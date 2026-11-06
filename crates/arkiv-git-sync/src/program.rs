//! Repo program enforcement (docs/p3/programs.md, "Enforcement"): the pinned source comes
//! from an `xyz.arkiv.program` publish op, verified ops and the delegates' signed-refs
//! updates become fold events in Arkiv order, and the fold starts from the canonical heads
//! at the pin. Fetch lists the program's accepted heads; the helper refuses a push the
//! program would reject.

use std::collections::{BTreeMap, HashMap};

use arkiv_git_core::{
    AttrValue, Body, CREATE, CobOpBody, Envelope, ErrorCode, Hash32, IdentityChain, Kind, Nid,
    ProgramPin, RefsBody, Revision, Rid, VerifiedOp, ZERO, canonical_refs, find_refs, verify_ops,
};
use arkiv_git_program::{Event, Outcome, Program, ProgramOp, RefUpdate, RepoInfo};
use arkiv_git_store::{Arkiv, Pred};
use serde_json::{Value, json};

use crate::git::Git;
use crate::keys::Keys;
use crate::observe::{all, b32, by_hashes, kind};
use crate::repo::{Config, Repo};
use crate::state::{PendingPush, unhex};
use crate::view::{View, Want};
use crate::{Result, SyncError, cob, fetch};

/// COB type of program source ops.
pub const PROGRAM_TYPE: &str = "xyz.arkiv.program";
/// Action of the op that carries `{engine, source}`.
pub const PUBLISH: &str = "publish";
/// Arkiv order of a would-be push: after everything on chain (scripts see orders as i64).
const PENDING_ORDER: u64 = i64::MAX as u64;

/// A pinned program folded over the repo; `outcome.heads` are the canonical heads.
#[derive(Debug, Clone)]
pub struct Enforced {
    pub program: Program,
    pub repo: RepoInfo,
    /// Canonical heads at the pin's identity revision (the fold's starting heads).
    pub seed: BTreeMap<String, Vec<u8>>,
    pub events: Vec<Event>,
    pub outcome: Outcome,
}

fn missing(msg: impl Into<String>) -> SyncError {
    SyncError::coded(ErrorCode::ProgramMissing, msg)
}

fn fold(e: &Enforced, extra: Vec<RefUpdate>, git: &Git) -> Result<Outcome> {
    let events = e
        .events
        .iter()
        .cloned()
        .chain(extra.into_iter().map(Event::Push));
    let ancestry = |a: &[u8], b: &[u8]| is_ancestor(git, a, b);
    e.program
        .fold_with(e.repo.clone(), e.seed.clone(), events, &ancestry)
        .map_err(|err| missing(format!("program fold failed: {err}")))
}

/// Git ancestry of commit `a` to commit `b` in `git`, or `None` if either is missing.
pub fn is_ancestor(git: &Git, a: &[u8], b: &[u8]) -> Option<bool> {
    let (a, b) = (hex::encode(a), hex::encode(b));
    let has = |o: &str| {
        git.run(&["cat-file", "-e", &format!("{o}^{{commit}}")], b"")
            .is_ok()
    };
    if !has(&a) || !has(&b) {
        return None;
    }
    // Exit 1 (no common ancestor) is a plain "not an ancestor".
    let base = git.run(&["merge-base", &a, &b], b"").unwrap_or_default();
    Some(String::from_utf8_lossy(&base).trim() == a)
}

impl Enforced {
    /// Folds again with `updates` appended after everything on chain; the first of them the
    /// program rejects is `E_PROGRAM_REJECTED` with the program's reason.
    pub fn check(&self, updates: Vec<RefUpdate>, git: &Git) -> Result<()> {
        let o = fold(self, updates, git)?;
        match o
            .rejected
            .iter()
            .find(|(u, _)| u.arkiv_order == PENDING_ORDER)
        {
            Some((_, why)) => Err(SyncError::coded(ErrorCode::ProgramRejected, why.clone())),
            None => Ok(()),
        }
    }
}

/// Ref moves of `author` from `old` to `new` refs, in ref-name order, at `order`.
pub fn ref_updates(
    author: Nid,
    old: Option<&RefsBody>,
    new: &RefsBody,
    order: u64,
) -> Vec<RefUpdate> {
    let map = |r: Option<&RefsBody>| -> BTreeMap<String, Vec<u8>> {
        r.iter()
            .flat_map(|r| &r.refs)
            .map(|e| (e.name.clone(), e.oid.clone()))
            .collect()
    };
    let (before, after) = (map(old), map(Some(new)));
    let mut names: Vec<&String> = before.keys().chain(after.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter(|n| before.get(*n) != after.get(*n))
        .map(|n| RefUpdate {
            author,
            name: n.clone(),
            old: before.get(n).cloned().unwrap_or_default(),
            new: after.get(n).cloned().unwrap_or_default(),
            arkiv_order: order,
        })
        .collect()
}

/// The program view of a verified op at `order`.
pub fn program_op(o: &VerifiedOp, order: u64) -> ProgramOp {
    ProgramOp {
        id: o.id,
        author: o.author,
        rid: o.body.rid,
        object: o.object(),
        type_name: o.body.type_name.clone(),
        action: o.body.action.clone(),
        parents: o.body.parents.clone(),
        payload: o.body.payload.clone(),
        ts_ms: o.body.ts_ms,
        arkiv_order: order,
    }
}

/// Rank of `key` in the one creation-ordered query that holds every event.
fn order(rank: &HashMap<Hash32, u64>, key: &Hash32) -> Result<u64> {
    rank.get(key).copied().ok_or_else(|| {
        SyncError::coded(
            ErrorCode::InconsistentView,
            format!(
                "entity {} missing from the ordered event query",
                hex::encode(key)
            ),
        )
    })
}

/// Compiles the source of the first publish op whose `Program::hash` is the pinned hash.
fn source(pin: &ProgramPin, ops: &[VerifiedOp]) -> Result<Program> {
    let src = ops
        .iter()
        .filter(|o| o.body.type_name == PROGRAM_TYPE && o.body.action == PUBLISH)
        .filter_map(|o| cob::json::to_json(&o.body.payload).ok())
        .find_map(|v| {
            let (engine, src) = (v["engine"].as_str()?, v["source"].as_str()?);
            (engine == pin.engine && Program::hash(src) == pin.hash).then(|| src.to_owned())
        })
        .ok_or_else(|| {
            missing(format!(
                "no {PROGRAM_TYPE} publish op carries the pinned program {} {}",
                pin.engine,
                hex::encode(pin.hash)
            ))
        })?;
    Program::load(&pin.engine, &src).map_err(|e| missing(e.to_string()))
}

/// The earliest revision of the run up to the head that carries the head's pin.
fn pin_revision(chain: &IdentityChain) -> &Revision {
    let pin = &chain.head().body.program;
    chain
        .revisions
        .iter()
        .rev()
        .take_while(|r| r.body.program == *pin)
        .last()
        .unwrap_or_else(|| chain.head())
}

/// Seed votes and post-pin ref updates of every delegate, from its verified anchor chain.
async fn ref_events<A: Arkiv>(
    a: &A,
    view: &View,
    rank: &HashMap<Hash32, u64>,
    pin_order: u64,
) -> Result<(BTreeMap<Nid, RefsBody>, Vec<RefUpdate>)> {
    let (mut votes, mut updates) = (BTreeMap::new(), vec![]);
    for nid in view.delegates() {
        let Some(pv) = view.peers.get(nid) else {
            continue;
        };
        let links = &pv.chain.links;
        let orders = links
            .iter()
            .map(|l| order(rank, &l.position.key))
            .collect::<Result<Vec<_>>>()?;
        // Only the last pre-pin push (the seed vote) and later ones need their refs.
        let from = orders.iter().rposition(|o| *o < pin_order).unwrap_or(0);
        let hashes: Vec<Hash32> = links[from..].iter().map(|l| l.body.refs).collect();
        let observed = by_hashes(a, kind(Kind::Refs), &hashes).await?;
        let mut prev: Option<RefsBody> = None;
        for (link, ord) in links.iter().zip(&orders).skip(from) {
            let refs =
                find_refs(&view.rid, &pv.info, &observed, &link.body.refs).ok_or_else(|| {
                    let h = hex::encode(link.body.refs);
                    SyncError::coded(
                        ErrorCode::NotLive,
                        format!("refs {h} after the program pin"),
                    )
                })?;
            match *ord < pin_order {
                true => _ = votes.insert(*nid, refs.clone()),
                false => updates.extend(ref_updates(*nid, prev.as_ref(), &refs, *ord)),
            }
            prev = Some(refs);
        }
    }
    Ok((votes, updates))
}

fn kind_is(o: &arkiv_git_core::Observed, k: Kind) -> bool {
    o.attr("ag.kind") == Some(&AttrValue::Str(k.name().to_owned()))
}

/// Folds the pinned program of `view`, or `None` when the identity pins none. A missing or
/// mismatched source is `E_PROGRAM_MISSING`: heads are never computed without the program.
/// `git` holds the pushed objects (fetch calls this after applying packs) for ancestry.
pub async fn enforce<A: Arkiv>(a: &A, view: &View, git: &Git) -> Result<Option<Enforced>> {
    let head = view.identity.head();
    let Some(pin) = head.body.program.clone() else {
        return Ok(None);
    };
    // One creation-ordered query, so ops, anchors and identity entities share one order.
    let kinds = [Kind::Cob, Kind::Anchor, Kind::Id, Kind::IdAcc].map(kind);
    let filter = Pred::And(vec![b32("ag.rid", &view.rid), Pred::Or(kinds.to_vec())]);
    let observed = all(a, filter).await?;
    let rank: HashMap<Hash32, u64> = (0u64..).zip(&observed).map(|(i, o)| (o.key, i)).collect();
    let cobs: Vec<_> = observed
        .into_iter()
        .filter(|o| kind_is(o, Kind::Cob))
        .collect();
    let ops = verify_ops(&view.rid, &cob::peer_keys(a, &cobs).await?, &cobs);
    let program = source(&pin, &ops)?;
    let pin_rev = pin_revision(&view.identity);
    let pin_order = order(&rank, &pin_rev.effective.key)?;
    let (votes, pushes) = ref_events(a, view, &rank, pin_order).await?;
    let seed = canonical_refs(&pin_rev.body, &votes)
        .refs
        .into_iter()
        .collect();
    let mut events = ops
        .iter()
        .map(|o| Ok(Event::Op(program_op(o, order(&rank, &o.arkiv_order.key)?))))
        .collect::<Result<Vec<_>>>()?;
    events.extend(pushes.into_iter().map(Event::Push));
    events.sort_by_key(Event::order);
    let repo = RepoInfo {
        rid: view.rid,
        delegates: head.body.delegates.clone(),
        quorum: head.body.threshold,
        name: head.body.name.clone(),
    };
    let mut e = Enforced {
        program,
        repo,
        seed,
        events,
        outcome: Outcome {
            state_json: String::new(),
            accepted: vec![],
            rejected: vec![],
            obligations: vec![],
            logs: vec![],
            heads: BTreeMap::new(),
        },
    };
    e.outcome = fold(&e, vec![], git)?;
    Ok(Some(e))
}

/// Refuses `pending` with `E_PROGRAM_REJECTED` if `view` enforces a program, `nid` is a
/// delegate, and the program rejects one of the ref moves the push signs.
pub fn check_push(view: &View, nid: &Nid, pending: &PendingPush, git: &Git) -> Result<()> {
    let Some(e) = &view.program else {
        return Ok(());
    };
    if !view.delegates().contains(nid) {
        return Ok(());
    }
    let new = RefsBody::decode(&Envelope::decode(&unhex(&pending.refs)?)?.body)?;
    let old = view
        .peers
        .get(nid)
        .and_then(|p| p.head.as_ref())
        .map(|h| &h.refs);
    e.check(ref_updates(*nid, old, &new, PENDING_ORDER), git)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Posts `{engine, source}` as a publish op on the repo's program object (created on first
/// use); returns the publish op id.
pub async fn publish<A: Arkiv>(
    a: &A,
    cfg: &Config,
    keys: &Keys,
    rid: &Rid,
    engine: &str,
    source: &str,
) -> Result<Hash32> {
    let op = |object: Hash32, action: &str, payload: &Value| -> Result<CobOpBody> {
        Ok(CobOpBody {
            rid: *rid,
            type_name: PROGRAM_TYPE.into(),
            object,
            parents: if object == ZERO { vec![] } else { vec![object] },
            action: action.into(),
            payload: cob::json::to_cbor(payload)?,
            ts_ms: now_ms(),
        })
    };
    let existing = cob::list(a, rid, None, Some(PROGRAM_TYPE)).await?;
    let object = match existing.iter().find(|o| o.body.is_create()) {
        Some(o) => o.id,
        None => cob::post(a, cfg, keys, &op(ZERO, CREATE, &json!({}))?).await?,
    };
    let payload = json!({ "engine": engine, "source": source });
    cob::post(a, cfg, keys, &op(object, PUBLISH, &payload)?).await
}

/// Obligations of the pinned program of `rid` (empty without a pin); the escrow hook. It
/// fetches into the cache under [`Config::default_home`], since fast-forwards need objects.
pub async fn obligations<A: Arkiv>(a: &A, rid: &Rid) -> Result<Vec<Value>> {
    obligations_in(a, &Repo::open(&Config::default_home(), rid)?).await
}

/// [`obligations`] with an explicit repo cache.
pub async fn obligations_in<A: Arkiv>(a: &A, repo: &Repo) -> Result<Vec<Value>> {
    let view = fetch::sync(a, repo, &Want::default()).await?;
    Ok(view
        .program
        .map(|e| e.outcome.obligations)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkiv_git_core::RefEntry;

    fn refs(entries: &[(&str, u8)]) -> RefsBody {
        RefsBody {
            rid: [0; 32],
            object_hash: 1,
            refs: entries
                .iter()
                .map(|(n, b)| RefEntry {
                    name: (*n).into(),
                    oid: vec![*b; 20],
                })
                .collect(),
        }
    }

    #[test]
    fn ref_updates_cover_new_moved_and_deleted_refs() {
        let old = refs(&[("refs/heads/a", 1), ("refs/heads/b", 2), ("refs/tags/t", 3)]);
        let new = refs(&[
            ("refs/heads/a", 1),
            ("refs/heads/b", 4),
            ("refs/heads/c", 5),
        ]);
        let u = ref_updates([9; 32], Some(&old), &new, 7);
        let got: Vec<_> = u
            .iter()
            .map(|u| {
                (
                    u.name.as_str(),
                    u.old.first().copied(),
                    u.new.first().copied(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("refs/heads/b", Some(2), Some(4)),
                ("refs/heads/c", None, Some(5)),
                ("refs/tags/t", Some(3), None),
            ]
        );
        assert!(u.iter().all(|u| u.arkiv_order == 7 && u.author == [9; 32]));
        assert_eq!(ref_updates([9; 32], None, &new, 1).len(), 3);
    }
}

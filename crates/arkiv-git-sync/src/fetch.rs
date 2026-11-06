//! Fetch and clone (design 4.3): verified view, pack download and `index-pack`, the
//! connectivity check, then the commit (marks first, then namespace refs, then applied
//! packs). Nothing reaches namespace refs before every check passed for that peer.

use std::collections::{BTreeMap, BTreeSet};

use arkiv_git_core::{ErrorCode, Hash32, IdMark, Nid, RefsBody, canonical_refs};
use arkiv_git_store::Arkiv;

use crate::git::RefOp;
use crate::ids::{hex32, to_text};
use crate::repo::{Repo, WARN_EXPIRY_BLOCKS, namespace};
use crate::state::State;
use crate::view::{self, View, Want};
use crate::{Result, SyncError, packs, warn};

/// Runs the fetch sync for `repo`; returns the verified view it committed.
pub async fn sync<A: Arkiv>(a: &A, repo: &Repo, want: &Want) -> Result<View> {
    let mut state = State::load(&repo.state_path())?;
    let mut view = view::build(a, &repo.rid, &state, want).await?;
    view.program = crate::program::enforce(a, &view).await?;
    warn_expiry(&view);
    let must: BTreeSet<Nid> = view.delegates().iter().chain(&want.must).copied().collect();
    let applied = apply_all(a, repo, &mut view, &state, &must).await?;
    check_connected(repo, &mut view, &must)?;
    commit(repo, &view, &mut state, applied)?;
    Ok(view)
}

fn skip_or_fail(view: &mut View, must: &BTreeSet<Nid>, nid: &Nid, e: SyncError) -> Result<()> {
    if must.contains(nid) {
        return Err(e);
    }
    warn(&format!("skipping peer {}: {e}", to_text(nid)));
    view.peers.remove(nid);
    Ok(())
}

/// Packs and manifests newly applied by this fetch.
#[derive(Debug, Default)]
pub struct Applied {
    pub packs: BTreeSet<Hash32>,
    pub manifests: BTreeSet<Hash32>,
}

async fn apply_all<A: Arkiv>(
    a: &A,
    repo: &Repo,
    view: &mut View,
    state: &State,
    must: &BTreeSet<Nid>,
) -> Result<Applied> {
    let mut applied = Applied::default();
    let nids: Vec<Nid> = view.peers.keys().copied().collect();
    for nid in nids {
        if let Err(e) = apply_peer(a, repo, view, &nid, state, &mut applied).await {
            skip_or_fail(view, must, &nid, e)?;
        }
    }
    Ok(applied)
}

async fn apply_peer<A: Arkiv>(
    a: &A,
    repo: &Repo,
    view: &View,
    nid: &Nid,
    state: &State,
    applied: &mut Applied,
) -> Result<()> {
    let Some(pv) = view.peers.get(nid) else {
        return Ok(());
    };
    let Some(head) = &pv.head else { return Ok(()) };
    let bulk = pv.info.binds.bulk_set();
    for (h, m) in &head.manifests {
        let known = state.applied_packs.contains(&hex32(&m.pack_id));
        if !known && !applied.packs.contains(&m.pack_id) {
            let tmp = repo.tmp().join(format!("{}.pack", hex32(&m.pack_id)));
            let r = packs::download(a, &view.rid, m, &bulk, &tmp).await;
            let r = r.and_then(|()| repo.git().index_pack(&tmp));
            let _ = std::fs::remove_file(&tmp);
            r?;
            applied.packs.insert(m.pack_id);
        }
        applied.manifests.insert(*h);
    }
    Ok(())
}

fn oids(refs: &RefsBody) -> Vec<String> {
    refs.refs.iter().map(|e| hex::encode(&e.oid)).collect()
}

fn check_connected(repo: &Repo, view: &mut View, must: &BTreeSet<Nid>) -> Result<()> {
    let old: Vec<String> = repo
        .git()
        .refs("refs/namespaces/")?
        .into_iter()
        .map(|(_, o)| o)
        .collect();
    let nids: Vec<Nid> = view.peers.keys().copied().collect();
    for nid in nids {
        let Some(head) = view.peers.get(&nid).and_then(|p| p.head.as_ref()) else {
            continue;
        };
        if !repo.git().connected(&oids(&head.refs), &old) {
            let e = SyncError::coded(
                ErrorCode::MissingObjects,
                format!("refs of peer {} name objects not on Arkiv", to_text(&nid)),
            );
            skip_or_fail(view, must, &nid, e)?;
        }
    }
    Ok(())
}

fn commit(repo: &Repo, view: &View, state: &mut State, applied: Applied) -> Result<()> {
    state.chain_id = Some(view.chain_id);
    let id = view.identity.head();
    let new_id = IdMark {
        rev: id.body.rev,
        hash: id.h,
    };
    let id_mark = match state.id_mark()? {
        Some(old) => old.advance(&new_id)?,
        None => new_id,
    };
    state.set_id_mark(&id_mark);
    for (nid, pv) in &view.peers {
        let Some(head) = &pv.head else { continue };
        let mark = match state.peer_mark(nid)? {
            Some(old) => old.advance(&head.mark)?,
            None => head.mark.clone(),
        };
        state.set_peer_mark(nid, &mark);
    }
    state.note_equivocations(&view.equivocations);
    state.save(&repo.state_path())?;
    for (nid, pv) in &view.peers {
        if let Some(head) = &pv.head {
            set_namespace(repo, nid, &head.refs)?;
        }
    }
    state.applied_packs.extend(applied.packs.iter().map(hex32));
    state
        .applied_manifests
        .extend(applied.manifests.iter().map(hex32));
    state.save(&repo.state_path())
}

/// Moves `refs/namespaces/<nid>/` to exactly `refs`, in one transaction with old values.
pub fn set_namespace(repo: &Repo, nid: &Nid, refs: &RefsBody) -> Result<()> {
    let ns = namespace(nid);
    let current: BTreeMap<String, String> = repo.git().refs(&ns)?.into_iter().collect();
    let desired: BTreeMap<String, String> = refs
        .refs
        .iter()
        .map(|e| (format!("{ns}{}", e.name), hex::encode(&e.oid)))
        .collect();
    let mut ops = vec![];
    for (name, new) in &desired {
        let old = current.get(name).cloned();
        if old.as_ref() != Some(new) {
            ops.push(RefOp::Update {
                name: name.clone(),
                new: new.clone(),
                old,
            });
        }
    }
    for (name, old) in current.iter().filter(|(n, _)| !desired.contains_key(*n)) {
        ops.push(RefOp::Delete {
            name: name.clone(),
            old: old.clone(),
        });
    }
    repo.git().update_refs(&ops)
}

fn warn_expiry(view: &View) {
    let min = view
        .peers
        .values()
        .flat_map(|p| p.refs.iter().chain(&p.manifests).chain(&p.anchors))
        .map(|o| o.expires_at)
        .min();
    if let Some(m) = min.filter(|m| *m < view.head_block + WARN_EXPIRY_BLOCKS) {
        let days = m.saturating_sub(view.head_block) / crate::repo::BLOCKS_PER_DAY;
        warn(&format!(
            "an entity this repo needs expires in {days} days (block {m}); run `arkiv-git keepalive`"
        ));
    }
}

/// What the helper prints for `list`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    /// `(ref name, oid hex)`.
    pub refs: Vec<(String, String)>,
    /// Target of the `HEAD` symref, if that ref is listed.
    pub head: Option<String>,
}

/// Canonical refs (design 3.8; with a pinned program, its accepted heads), or one peer's verified refs for `arkiv://<rid>/<nid>`.
pub fn listing(view: &View, peer: Option<&Nid>) -> Listing {
    let id = &view.identity.head().body;
    let refs: Vec<(String, String)> = match peer {
        Some(nid) => view
            .peers
            .get(nid)
            .and_then(|p| p.head.as_ref())
            .map(|h| oids_named(&h.refs))
            .unwrap_or_default(),
        None if view.program.is_some() => view
            .program
            .iter()
            .flat_map(|p| &p.outcome.heads)
            .map(|(n, o)| (n.clone(), hex::encode(o)))
            .collect(),
        None => {
            let votes: BTreeMap<Nid, RefsBody> = view
                .peers
                .iter()
                .filter_map(|(n, p)| p.head.as_ref().map(|h| (*n, h.refs.clone())))
                .collect();
            let c = canonical_refs(id, &votes);
            c.no_quorum
                .iter()
                .for_each(|r| warn(&format!("no quorum for {r}")));
            c.refs
                .into_iter()
                .map(|(n, o)| (n, hex::encode(o)))
                .collect()
        }
    };
    let default = format!("refs/heads/{}", id.default_branch);
    let head = refs.iter().any(|(n, _)| *n == default).then_some(default);
    Listing { refs, head }
}

fn oids_named(refs: &RefsBody) -> Vec<(String, String)> {
    refs.refs
        .iter()
        .map(|e| (e.name.clone(), hex::encode(&e.oid)))
        .collect()
}

/// The local signer's verified refs (`list for-push`).
pub fn own_refs(view: &View, nid: &Nid) -> Vec<(String, String)> {
    view.peers
        .get(nid)
        .and_then(|p| p.head.as_ref())
        .map(|h| oids_named(&h.refs))
        .unwrap_or_default()
}

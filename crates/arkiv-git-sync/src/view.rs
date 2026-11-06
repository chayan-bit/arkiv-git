//! The verified control-plane view of a repo (design 4.3 steps 1-4): identity chain, the
//! bind and anchor chains of every synced peer, and each peer's accepted head (refs and
//! manifests checked, no pack bytes yet).

use std::collections::{BTreeMap, BTreeSet};

use arkiv_git_core::{
    AcceptedHead, AnchorChain, CompletenessObs, Equivocation, HeadInputs, IdentityChain, Kind,
    ManifestBody, Nid, Observed, PeerInfo, Rid, Start, accept_head, anchor_chain, bind_chain,
    check_requires, identity_chain, peer_key,
};
use arkiv_git_store::{Arkiv, Pred};

use crate::observe::{self, all, b32, by_hashes, by_seq, creators, kind};
use crate::state::State;
use crate::{Result, SyncError, warn};

/// One synced peer.
#[derive(Debug, Clone)]
pub struct PeerView {
    pub info: PeerInfo,
    pub chain: AnchorChain,
    pub head: Option<AcceptedHead>,
    pub completeness: Vec<CompletenessObs>,
    /// The anchor entities as observed (keepalive and status read their keys and expiry).
    pub anchors: Vec<Observed>,
    /// The head refs entity and the manifests the head check saw.
    pub refs: Vec<Observed>,
    pub manifests: Vec<Observed>,
}

#[derive(Debug, Clone)]
pub struct View {
    pub rid: Rid,
    pub chain_id: u64,
    pub head_block: u64,
    pub identity: IdentityChain,
    /// Every resolvable signer of identity entities, plus synced peers.
    pub resolved: BTreeMap<Nid, PeerInfo>,
    pub peers: BTreeMap<Nid, PeerView>,
    pub equivocations: Vec<Equivocation>,
    /// The pinned repo program folded over the repo; set by fetch ([`crate::fetch::sync`]).
    pub program: Option<crate::program::Enforced>,
}

impl View {
    pub fn delegates(&self) -> &[Nid] {
        &self.identity.head().body.delegates
    }
}

/// Which peers to sync and which must verify (delegates always must).
#[derive(Debug, Clone, Default)]
pub struct Want {
    pub extra: BTreeSet<Nid>,
    pub must: BTreeSet<Nid>,
}

/// Peer key from its self-signed documents, then its bind chain for `rid`.
pub async fn resolve_peer<A: Arkiv>(a: &A, rid: &Rid, nid: &Nid) -> Result<Option<PeerInfo>> {
    let docs = all(a, Pred::And(vec![kind(Kind::Peer), b32("ag.nid", nid)])).await?;
    let Some(pk) = peer_key(nid, &docs) else {
        return Ok(None);
    };
    let filter = Pred::And(vec![
        kind(Kind::Bind),
        b32("ag.rid", rid),
        b32("ag.nid", nid),
    ]);
    let binds = by_seq(a, filter, "ag.seq").await?;
    let binds = bind_chain(rid, &pk, &binds)?;
    Ok(Some(PeerInfo { pk, binds }))
}

fn signer_nids(observed: &[Observed]) -> BTreeSet<Nid> {
    observed
        .iter()
        .filter_map(|o| match o.attr("ag.nid") {
            Some(arkiv_git_core::AttrValue::Bytes32(n)) => Some(*n),
            _ => None,
        })
        .collect()
}

/// The identity chain of `rid` and every signer the identity entities name.
pub async fn identity<A: Arkiv>(
    a: &A,
    rid: &Rid,
    state: &State,
) -> Result<(IdentityChain, BTreeMap<Nid, PeerInfo>)> {
    let kinds = Pred::Or(vec![kind(Kind::Id), kind(Kind::IdAcc)]);
    let observed = all(a, Pred::And(vec![b32("ag.rid", rid), kinds])).await?;
    let mut peers = BTreeMap::new();
    for nid in signer_nids(&observed) {
        if let Some(p) = resolve_peer(a, rid, &nid).await? {
            peers.insert(nid, p);
        }
    }
    let chain = identity_chain(rid, &peers, &observed, state.id_mark()?.as_ref())?;
    Ok((chain, peers))
}

/// Anchor entities of `nid` created by its control addresses, in seq order.
pub async fn anchors<A: Arkiv>(
    a: &A,
    rid: &Rid,
    nid: &Nid,
    info: &PeerInfo,
) -> Result<Vec<Observed>> {
    let control = info.binds.control_set();
    if control.is_empty() {
        return Ok(vec![]);
    }
    let filter = Pred::And(vec![
        kind(Kind::Anchor),
        b32("ag.rid", rid),
        b32("ag.nid", nid),
        creators(&control),
    ]);
    by_seq(a, filter, "ag.seq").await
}

/// The anchor chain of one peer from genesis (the high-water mark is checked on accept).
pub async fn chain_of<A: Arkiv>(
    a: &A,
    rid: &Rid,
    nid: &Nid,
    info: &PeerInfo,
    identity: &IdentityChain,
) -> Result<(AnchorChain, Vec<Observed>)> {
    let observed = anchors(a, rid, nid, info).await?;
    let chain = anchor_chain(rid, nid, info, identity, &observed, Start::Genesis)?;
    Ok((chain, observed))
}

/// Bind chain, completeness, anchor chain and head acceptance for one peer (design 3.6).
pub async fn verify_peer<A: Arkiv>(
    a: &A,
    rid: &Rid,
    nid: &Nid,
    info: PeerInfo,
    identity: &IdentityChain,
    state: &State,
) -> Result<PeerView> {
    let mut completeness = vec![];
    for c in info.binds.control_set() {
        completeness.push(observe::completeness(a, &c).await?);
    }
    let (chain, anchors) = chain_of(a, rid, nid, &info, identity).await?;
    let applied = state.applied_manifest_set()?;
    let (refs, manifests) = match chain.head() {
        None => (vec![], vec![]),
        Some(h) => {
            let refs = all(
                a,
                Pred::And(vec![kind(Kind::Refs), b32("ag.h", &h.body.refs)]),
            )
            .await?;
            let wanted: Vec<_> = chain
                .links
                .iter()
                .flat_map(|l| l.body.manifests.iter().copied())
                .filter(|m| !applied.contains(m))
                .collect();
            (refs, by_hashes(a, kind(Kind::Manifest), &wanted).await?)
        }
    };
    let mark = state.peer_mark(nid)?;
    let inputs = HeadInputs {
        completeness: &completeness,
        mark: mark.as_ref(),
        refs: &refs,
        manifests: &manifests,
        applied: &applied,
    };
    let head = accept_head(rid, &info, &chain, &inputs)?;
    Ok(PeerView {
        info,
        chain,
        head,
        completeness,
        anchors,
        refs,
        manifests,
    })
}

/// Builds the verified view: identity, then delegates, wanted peers and the transitive
/// closure of `requires`. A failing delegate or `must` peer aborts; others are skipped.
pub async fn build<A: Arkiv>(a: &A, rid: &Rid, state: &State, want: &Want) -> Result<View> {
    let chain_id = a.chain_id().await?;
    if let Some(expected) = state.chain_id.filter(|c| *c != chain_id) {
        return Err(arkiv_git_store::StoreError::WrongChain {
            expected,
            got: chain_id,
        }
        .into());
    }
    let head_block = a.head().await?;
    let (identity, mut resolved) = identity(a, rid, state).await?;
    let delegates: BTreeSet<Nid> = identity.head().body.delegates.iter().copied().collect();
    let must: BTreeSet<Nid> = delegates.union(&want.must).copied().collect();
    let mut queue: Vec<Nid> = must.iter().chain(&want.extra).copied().collect();
    let mut peers = BTreeMap::new();
    while let Some(nid) = queue.pop() {
        if peers.contains_key(&nid) {
            continue;
        }
        let info = match resolved.get(&nid) {
            Some(p) => Some(p.clone()),
            None => resolve_peer(a, rid, &nid).await?,
        };
        let Some(info) = info else { continue };
        resolved.insert(nid, info.clone());
        match verify_peer(a, rid, &nid, info, &identity, state).await {
            Ok(pv) => {
                queue.extend(requires_of(&pv));
                peers.insert(nid, pv);
            }
            Err(e) if must.contains(&nid) => return Err(e),
            Err(e) => warn(&format!("skipping peer {}: {e}", crate::ids::to_text(&nid))),
        }
    }
    check_all_requires(&peers)?;
    let equivocations = collect_equivocations(&peers);
    equivocations.iter().for_each(|e| {
        warn(&format!(
            "E_EQUIVOCATION peer {} seq {}: entities {}, {}",
            crate::ids::to_text(&e.nid),
            e.seq,
            hex::encode(e.keys[0]),
            hex::encode(e.keys[1])
        ))
    });
    Ok(View {
        rid: *rid,
        chain_id,
        head_block,
        identity,
        resolved,
        peers,
        equivocations,
        program: None,
    })
}

fn requires_of(pv: &PeerView) -> Vec<Nid> {
    pv.head
        .iter()
        .flat_map(|h| &h.manifests)
        .flat_map(|(_, m)| m.requires.iter().map(|r| r.nid))
        .collect()
}

fn check_all_requires(peers: &BTreeMap<Nid, PeerView>) -> Result<()> {
    let manifests: Vec<ManifestBody> = peers
        .values()
        .flat_map(|p| p.head.iter().flat_map(|h| h.manifests.iter()))
        .map(|(_, m)| m.clone())
        .collect();
    let chains: BTreeMap<Nid, AnchorChain> =
        peers.iter().map(|(n, p)| (*n, p.chain.clone())).collect();
    check_requires(&manifests, &chains).map_err(SyncError::from)
}

fn collect_equivocations(peers: &BTreeMap<Nid, PeerView>) -> Vec<Equivocation> {
    let mut v: Vec<Equivocation> = peers
        .values()
        .flat_map(|p| {
            p.chain
                .equivocations
                .iter()
                .chain(&p.info.binds.equivocations)
                .cloned()
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

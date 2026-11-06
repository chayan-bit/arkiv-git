//! Completeness predicate (design 3.3) and head acceptance (design 3.6, table rows that
//! need no pack bytes; chunk, pack and connectivity checks belong to the sync layer).

use super::{AnchorChain, AnchorLink, PeerInfo, open_all};
use crate::attrs::AttrValue;
use crate::marks::PeerMark;
use crate::types::{Address, Hash32, Nid, Observed, Rid};
use crate::{Body, Entity, ManifestBody, Missing, Plane, RefsBody, VerifyError};
use std::collections::{BTreeMap, BTreeSet};

/// One run of the completeness check for a control address: `entityNonce` before,
/// the live count of `$creator = address`, `entityNonce` after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletenessObs {
    pub address: Address,
    pub nonce_before: u64,
    pub live: u64,
    pub nonce_after: u64,
}

/// `n1 != n2` or `c > n1`: `E_INCONSISTENT_VIEW` (the caller retries up to 3 times);
/// `c < n1`: `E_CONTROL_NOT_LIVE`.
pub fn check_completeness(nid: &Nid, obs: &CompletenessObs) -> Result<(), VerifyError> {
    if obs.nonce_before != obs.nonce_after || obs.live > obs.nonce_before {
        return Err(VerifyError::InconsistentView);
    }
    if obs.live < obs.nonce_before {
        return Err(VerifyError::ControlNotLive {
            nid: *nid,
            address: obs.address,
            created: obs.nonce_before,
            live: obs.live,
        });
    }
    Ok(())
}

/// The completeness check for every control address of a peer.
pub fn check_control_set(
    nid: &Nid,
    control: &BTreeSet<Address>,
    obs: &[CompletenessObs],
) -> Result<(), VerifyError> {
    control.iter().try_for_each(|a| {
        let o = obs
            .iter()
            .find(|o| o.address == *a)
            .ok_or(VerifyError::MissingCompleteness(*a))?;
        check_completeness(nid, o)
    })
}

/// Inputs to [`accept_head`] besides the chain.
#[derive(Debug, Clone, Copy)]
pub struct HeadInputs<'a> {
    pub completeness: &'a [CompletenessObs],
    pub mark: Option<&'a PeerMark>,
    /// Candidate refs entities (e.g. the query `ag.h = A_s.refs`).
    pub refs: &'a [Observed],
    /// Candidate manifest entities named by the chain.
    pub manifests: &'a [Observed],
    /// Manifest `h` values already applied locally.
    pub applied: &'a BTreeSet<Hash32>,
}

/// A head that passed every check core can make without pack bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedHead {
    pub anchor: AnchorLink,
    pub refs: RefsBody,
    /// Manifests still to apply, in anchor order, as `(h, body)`.
    pub manifests: Vec<(Hash32, ManifestBody)>,
    /// The new high-water mark to store once the rest of the fetch succeeds.
    pub mark: PeerMark,
}

fn check_mark(chain: &AnchorChain, mark: Option<&PeerMark>) -> Result<(), VerifyError> {
    let Some(m) = mark else { return Ok(()) };
    let head = chain.head().map(|l| l.body.seq);
    if head.is_none_or(|s| s < m.seq) {
        return Err(VerifyError::StaleHead {
            seq: head,
            mark: m.seq,
        });
    }
    match chain.at_seq(m.seq) {
        Some(l) if l.h == m.hash => Ok(()),
        _ => Err(VerifyError::HistoryRewritten { seq: m.seq }),
    }
}

/// First valid bulk-plane entity with content hash `h`: signed by the peer, same rid,
/// creator rule against the bulk addresses.
fn find_bulk<B: Body + HasRid>(
    rid: &Rid,
    peer: &PeerInfo,
    observed: &[Observed],
    h: &Hash32,
) -> Option<Entity<B>> {
    open_all::<B>(observed).ok.into_iter().find(|e| {
        e.h == *h
            && e.envelope.signer == peer.binds.nid
            && e.body.rid() == rid
            && peer
                .binds
                .creator_ok(Plane::Bulk, &e.creator, e.position.block)
            && e.envelope.verify(&peer.pk).is_ok()
    })
}

pub(crate) trait HasRid {
    fn rid(&self) -> &Rid;
}
impl HasRid for RefsBody {
    fn rid(&self) -> &Rid {
        &self.rid
    }
}
impl HasRid for ManifestBody {
    fn rid(&self) -> &Rid {
        &self.rid
    }
}

fn head_refs(
    rid: &Rid,
    peer: &PeerInfo,
    refs: &[Observed],
    h: &Hash32,
) -> Result<RefsBody, VerifyError> {
    if let Some(e) = find_bulk::<RefsBody>(rid, peer, refs, h) {
        return Ok(e.body);
    }
    let present = refs
        .iter()
        .any(|o| o.attr("ag.h") == Some(&AttrValue::Bytes32(*h)));
    Err(match present {
        true => VerifyError::BadRefs("no valid refs entity with the head's hash".into()),
        false => VerifyError::NotLive(Missing::Refs(*h)),
    })
}

/// Accepts the chain head for one peer, or returns `None` if the peer has no anchors.
pub fn accept_head(
    rid: &Rid,
    peer: &PeerInfo,
    chain: &AnchorChain,
    inputs: &HeadInputs<'_>,
) -> Result<Option<AcceptedHead>, VerifyError> {
    check_control_set(&chain.nid, &peer.binds.control_set(), inputs.completeness)?;
    check_mark(chain, inputs.mark)?;
    let Some(head) = chain.head() else {
        return Ok(None);
    };
    let refs = head_refs(rid, peer, inputs.refs, &head.body.refs)?;
    let mut manifests = vec![];
    for m in chain.links.iter().flat_map(|l| &l.body.manifests) {
        if inputs.applied.contains(m) {
            continue;
        }
        let e = find_bulk::<ManifestBody>(rid, peer, inputs.manifests, m)
            .ok_or(VerifyError::NotLive(Missing::Manifest(*m)))?;
        manifests.push((*m, e.body));
    }
    Ok(Some(AcceptedHead {
        mark: PeerMark {
            seq: head.body.seq,
            hash: head.h,
            block: head.position.block,
        },
        anchor: head.clone(),
        refs,
        manifests,
    }))
}

/// Every `requires` entry must name an anchor in that peer's verified chain.
pub fn check_requires(
    manifests: &[ManifestBody],
    chains: &BTreeMap<Nid, AnchorChain>,
) -> Result<(), VerifyError> {
    manifests
        .iter()
        .flat_map(|m| &m.requires)
        .try_for_each(
            |r| match chains.get(&r.nid).is_some_and(|c| c.contains(&r.anchor)) {
                true => Ok(()),
                false => Err(VerifyError::MissingDependency {
                    nid: r.nid,
                    anchor: r.anchor,
                }),
            },
        )
}

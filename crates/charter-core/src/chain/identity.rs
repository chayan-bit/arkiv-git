//! Identity chain evaluation and key rotation (design 3.5).

use super::{PeerInfo, fail_unsupported, open_all};
use crate::marks::IdMark;
use crate::types::{Address, Hash32, Nid, Observed, Position, Rid};
use crate::{Body, Entity, IdAccBody, IdBody, Plane, VerifyError};
use std::collections::{BTreeMap, BTreeSet};

/// Heartwood's identity quorum: `floor(n / 2) + 1`.
pub const fn maj(n: usize) -> usize {
    n / 2 + 1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub body: IdBody,
    pub h: Hash32,
    pub proposer: Nid,
    pub position: Position,
    /// Position at which this revision took effect.
    pub effective: Position,
}

/// The accepted revisions from the root (rev 0) to the head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityChain {
    pub rid: Rid,
    pub revisions: Vec<Revision>,
}

impl IdentityChain {
    pub fn head(&self) -> &Revision {
        self.revisions
            .last()
            .unwrap_or_else(|| unreachable!("an identity chain always holds its root"))
    }

    pub fn contains(&self, h: &Hash32) -> bool {
        self.revisions.iter().any(|r| r.h == *h)
    }
}

/// Signature under the signer's key and the creator rule against the signer's binds.
fn signed_by_peer<B: Body>(e: &Entity<B>, peers: &BTreeMap<Nid, PeerInfo>) -> bool {
    peers.get(&e.envelope.signer).is_some_and(|p| {
        p.binds
            .creator_ok(Plane::Control, &e.creator, e.position.block)
            && e.envelope.verify(&p.pk).is_ok()
    })
}

type Acceptances = BTreeMap<Hash32, Vec<(Position, Nid)>>;

/// Position of the acceptance that first brings `|acceptors ∩ d|` to `maj(d)`.
fn quorum_at(acc: &Acceptances, rev: &Hash32, d: &[Nid]) -> Option<Position> {
    let mut seen = BTreeSet::new();
    for (pos, nid) in acc.get(rev)? {
        if d.contains(nid) && seen.insert(*nid) && seen.len() == maj(d.len()) {
            return Some(*pos);
        }
    }
    None
}

fn revision(e: &Entity<IdBody>, effective: Position) -> Revision {
    Revision {
        body: e.body.clone(),
        h: e.h,
        proposer: e.envelope.signer,
        position: e.position,
        effective,
    }
}

fn collect(
    rid: &Rid,
    peers: &BTreeMap<Nid, PeerInfo>,
    observed: &[Observed],
) -> Result<(Vec<Entity<IdBody>>, Acceptances), VerifyError> {
    let ids = open_all::<IdBody>(observed);
    let accs = open_all::<IdAccBody>(observed);
    let control: BTreeSet<Address> = peers.values().flat_map(|p| p.binds.control_set()).collect();
    fail_unsupported(&ids.unsupported, &control)?;
    fail_unsupported(&accs.unsupported, &control)?;
    let ids: Vec<Entity<IdBody>> = ids
        .ok
        .into_iter()
        .filter(|e| e.body.rid.map_or(e.h == *rid, |r| r == *rid) && signed_by_peer(e, peers))
        .collect();
    let mut acceptances: Acceptances = BTreeMap::new();
    for e in &ids {
        acceptances
            .entry(e.h)
            .or_default()
            .push((e.position, e.envelope.signer));
    }
    for e in accs
        .ok
        .iter()
        .filter(|e| e.body.rid == *rid && signed_by_peer(e, peers))
    {
        acceptances
            .entry(e.body.rev)
            .or_default()
            .push((e.position, e.envelope.signer));
    }
    acceptances.values_mut().for_each(|v| v.sort());
    Ok((ids, acceptances))
}

/// Evaluates the identity of `rid` from `id` and `idacc` entities in any order.
///
/// `peers` holds the key and bind chain of every signer the caller could resolve; entities
/// of other signers are ignored. Ties in effective position go to the earlier revision.
pub fn identity_chain(
    rid: &Rid,
    peers: &BTreeMap<Nid, PeerInfo>,
    observed: &[Observed],
    mark: Option<&IdMark>,
) -> Result<IdentityChain, VerifyError> {
    let (ids, acc) = collect(rid, peers, observed)?;
    let root = ids
        .iter()
        .filter(|e| {
            e.body.is_root() && e.h == *rid && e.body.delegates.contains(&e.envelope.signer)
        })
        .find_map(|e| quorum_at(&acc, &e.h, &e.body.delegates).map(|q| revision(e, q)))
        .ok_or(VerifyError::IdNotLive)?;
    let mut revisions = vec![root];
    while let Some(next) = revisions.last().and_then(|h| next_revision(&ids, &acc, h)) {
        revisions.push(next);
    }
    let chain = IdentityChain {
        rid: *rid,
        revisions,
    };
    check_mark(&chain, mark)?;
    Ok(chain)
}

fn next_revision(ids: &[Entity<IdBody>], acc: &Acceptances, head: &Revision) -> Option<Revision> {
    ids.iter()
        .filter(|e| Some(e.body.rev) == head.body.rev.checked_add(1) && e.body.parent == head.h)
        .filter_map(|e| {
            let q = quorum_at(acc, &e.h, &head.body.delegates)?;
            Some((q.max(head.effective), e.position, e))
        })
        .min_by_key(|(eff, pos, _)| (*eff, *pos))
        .map(|(eff, _, e)| revision(e, eff))
}

fn check_mark(chain: &IdentityChain, mark: Option<&IdMark>) -> Result<(), VerifyError> {
    let Some(m) = mark else { return Ok(()) };
    let head = chain.head().body.rev;
    if head < m.rev || !chain.contains(&m.hash) {
        return Err(VerifyError::StaleIdentity { rev: head });
    }
    Ok(())
}

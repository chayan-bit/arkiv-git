//! Bind chain and the creator rule (design 3.1, 3.4).

use super::{Equivocation, SeqItem, fail_unsupported, gap_and_equivocations, open_all};
use crate::types::{Address, Hash32, Nid, Observed, Position, Rid, ZERO};
use crate::{BindBody, Entity, Kind, Plane, VerifyError};
use charter_crypto::PublicKey;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindLink {
    pub body: BindBody,
    pub h: Hash32,
    pub position: Position,
}

/// `B_0..B_m` for one `(rid, nid)`; empty if the peer never bound to the repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindChain {
    pub nid: Nid,
    pub links: Vec<BindLink>,
    pub equivocations: Vec<Equivocation>,
}

impl BindChain {
    pub fn control_set(&self) -> BTreeSet<Address> {
        self.links.iter().map(|l| l.body.control).collect()
    }

    pub fn bulk_set(&self) -> BTreeSet<Address> {
        self.links.iter().map(|l| l.body.bulk).collect()
    }

    /// Creator rule: `creator = B_i.<plane>` for some `i`, and if `i < m` the entity was
    /// created no later than `B_{i+1}` (by `$createdAt` block).
    pub fn creator_ok(&self, plane: Plane, creator: &Address, block: u64) -> bool {
        self.links.iter().enumerate().any(|(i, l)| {
            let addr = match plane {
                Plane::Control => l.body.control,
                Plane::Bulk => l.body.bulk,
            };
            addr == *creator
                && self
                    .links
                    .get(i + 1)
                    .is_none_or(|next| block <= next.position.block)
        })
    }
}

fn link(cands: &[Entity<BindBody>]) -> Vec<BindLink> {
    let mut links: Vec<BindLink> = vec![];
    loop {
        let next = cands.iter().find(|e| match links.last() {
            None => e.body.seq == 0 && e.body.prev == ZERO && e.creator == e.body.control,
            Some(l) => {
                Some(e.body.seq) == l.body.seq.checked_add(1)
                    && e.body.prev == l.h
                    && e.creator == l.body.control
            }
        });
        match next {
            Some(e) => links.push(BindLink {
                body: e.body.clone(),
                h: e.h,
                position: e.position,
            }),
            None => return links,
        }
    }
}

/// Builds the bind chain of `pk`'s peer for `rid` from bind entities in any order.
pub fn bind_chain(
    rid: &Rid,
    pk: &PublicKey,
    observed: &[Observed],
) -> Result<BindChain, VerifyError> {
    let nid = pk.nid();
    let opened = open_all::<BindBody>(observed);
    let signed: Vec<Entity<BindBody>> = opened
        .ok
        .into_iter()
        .filter(|e| e.body.rid == *rid && e.envelope.verify(pk).is_ok())
        .collect();
    let links = link(&signed);
    let chain = BindChain {
        nid,
        links,
        equivocations: vec![],
    };
    let control = chain.control_set();
    fail_unsupported(&opened.unsupported, &control)?;
    let valid: Vec<SeqItem> = signed
        .iter()
        .filter(|e| match e.body.seq {
            0 => e.creator == e.body.control,
            _ => control.contains(&e.creator),
        })
        .map(|e| (e.body.seq, e.h, e.position.key))
        .collect();
    let link_items: Vec<SeqItem> = chain
        .links
        .iter()
        .map(|l| (l.body.seq, l.h, l.position.key))
        .collect();
    let equivocations = gap_and_equivocations(Kind::Bind, &nid, &link_items, &valid)?;
    Ok(BindChain {
        equivocations,
        ..chain
    })
}

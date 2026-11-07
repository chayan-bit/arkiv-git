//! Anchor chain (design 3.6, 3.7).

use super::{
    Equivocation, IdentityChain, PeerInfo, SeqItem, fail_unsupported, gap_and_equivocations,
    open_all,
};
use crate::marks::PeerMark;
use crate::types::{Hash32, Nid, Observed, Position, Rid};
use crate::{AnchorBody, Entity, Kind, Plane, VerifyError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorLink {
    pub body: AnchorBody,
    pub h: Hash32,
    pub position: Position,
}

/// `A_start..A_s` of one peer, first-created wins at every seq.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorChain {
    pub nid: Nid,
    pub links: Vec<AnchorLink>,
    pub equivocations: Vec<Equivocation>,
}

impl AnchorChain {
    pub fn head(&self) -> Option<&AnchorLink> {
        self.links.last()
    }

    pub fn contains(&self, h: &Hash32) -> bool {
        self.links.iter().any(|l| l.h == *h)
    }

    pub fn at_seq(&self, seq: u64) -> Option<&AnchorLink> {
        self.links.iter().find(|l| l.body.seq == seq)
    }
}

/// Where the chain starts: `A_0`, or the marked anchor for an incremental fetch
/// (anchors below the mark are then ignored).
#[derive(Debug, Clone, Copy)]
pub enum Start<'a> {
    Genesis,
    Mark(&'a PeerMark),
}

fn to_link(e: &Entity<AnchorBody>) -> AnchorLink {
    AnchorLink {
        body: e.body.clone(),
        h: e.h,
        position: e.position,
    }
}

fn first_link<'e>(
    valid: &[&'e Entity<AnchorBody>],
    start: Start<'_>,
) -> Result<Option<&'e Entity<AnchorBody>>, VerifyError> {
    let Start::Mark(m) = start else {
        return Ok(valid.iter().find(|e| e.body.seq == 0).copied());
    };
    match valid.iter().find(|e| e.body.seq == m.seq) {
        Some(e) if e.h == m.hash => Ok(Some(e)),
        Some(_) => Err(VerifyError::HistoryRewritten { seq: m.seq }),
        None => match valid
            .iter()
            .map(|e| e.body.seq)
            .filter(|s| *s > m.seq)
            .min()
        {
            Some(seq) => Err(VerifyError::ChainGap {
                kind: Kind::Anchor,
                seq,
            }),
            None => Err(VerifyError::StaleHead {
                seq: None,
                mark: m.seq,
            }),
        },
    }
}

fn link(valid: &[&Entity<AnchorBody>], start: Start<'_>) -> Result<Vec<AnchorLink>, VerifyError> {
    let Some(first) = first_link(valid, start)? else {
        return Ok(vec![]);
    };
    let mut links = vec![to_link(first)];
    while let Some(next) = links.last().and_then(|l| {
        valid
            .iter()
            .find(|e| Some(e.body.seq) == l.body.seq.checked_add(1) && e.body.prev == l.h)
    }) {
        links.push(to_link(next));
    }
    Ok(links)
}

/// Builds `nid`'s anchor chain for `rid` from anchor entities in any order.
///
/// Valid anchors (design 3.6 rules 1-6): canonical, signed by `nid`, attributes match,
/// creator rule, `id_head` in the verified identity chain, `at <= $createdAt`.
pub fn anchor_chain(
    rid: &Rid,
    nid: &Nid,
    peer: &PeerInfo,
    identity: &IdentityChain,
    observed: &[Observed],
    start: Start<'_>,
) -> Result<AnchorChain, VerifyError> {
    let opened = open_all::<AnchorBody>(observed);
    let signed: Vec<Entity<AnchorBody>> = opened
        .ok
        .into_iter()
        .filter(|e| {
            e.envelope.signer == *nid && e.body.rid == *rid && e.envelope.verify(&peer.pk).is_ok()
        })
        .collect();
    fail_unsupported(&opened.unsupported, &peer.binds.control_set())?;
    // Signed anchors imply a bind existed: a missing bind 0 fails closed.
    if peer.binds.links.is_empty() && !signed.is_empty() {
        return Err(VerifyError::ChainGap {
            kind: Kind::Bind,
            seq: 0,
        });
    }
    let from = match start {
        Start::Genesis => 0,
        Start::Mark(m) => m.seq,
    };
    let valid: Vec<&Entity<AnchorBody>> = signed
        .iter()
        .filter(|e| {
            e.body.seq >= from
                && peer
                    .binds
                    .creator_ok(Plane::Control, &e.creator, e.position.block)
                && identity.contains(&e.body.id_head)
                && e.body.at <= e.position.block
        })
        .collect();
    let links = link(&valid, start)?;
    let link_items: Vec<SeqItem> = links
        .iter()
        .map(|l| (l.body.seq, l.h, l.position.key))
        .collect();
    let cand_items: Vec<SeqItem> = valid
        .iter()
        .map(|e| (e.body.seq, e.h, e.position.key))
        .collect();
    let equivocations = gap_and_equivocations(Kind::Anchor, nid, &link_items, &cand_items)?;
    Ok(AnchorChain {
        nid: *nid,
        links,
        equivocations,
    })
}

//! Canonical refs by exact-match threshold quorum (design 3.8, open question Q7).

use crate::types::Nid;
use crate::{IdBody, RefsBody};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CanonicalRefs {
    /// `(name, oid)` ascending by name.
    pub refs: Vec<(String, Vec<u8>)>,
    /// Names omitted for a tie or no OID reaching the threshold (`warning: no quorum`).
    pub no_quorum: Vec<String>,
}

/// For `refs/heads/<default branch>` and every `refs/tags/*`: the OID held at that name by
/// at least `threshold` delegates, the most-voted one; ties omit the ref.
pub fn canonical_refs(head: &IdBody, delegate_refs: &BTreeMap<Nid, RefsBody>) -> CanonicalRefs {
    let default = format!("refs/heads/{}", head.default_branch);
    let voters: Vec<&RefsBody> = head
        .delegates
        .iter()
        .filter_map(|d| delegate_refs.get(d))
        .collect();
    let names: BTreeSet<&str> = voters
        .iter()
        .flat_map(|r| r.refs.iter().map(|e| e.name.as_str()))
        .filter(|n| *n == default || n.starts_with("refs/tags/"))
        .collect();
    let mut out = CanonicalRefs::default();
    for name in names {
        match winner(&voters, name, head.threshold) {
            Some(oid) => out.refs.push((name.to_owned(), oid.to_vec())),
            None => out.no_quorum.push(name.to_owned()),
        }
    }
    out
}

fn winner<'a>(voters: &[&'a RefsBody], name: &str, threshold: u64) -> Option<&'a [u8]> {
    let mut votes: BTreeMap<&[u8], u64> = BTreeMap::new();
    for oid in voters.iter().filter_map(|r| r.get(name)) {
        *votes.entry(oid).or_default() += 1;
    }
    let max = *votes.values().max()?;
    let mut top = votes.iter().filter(|(_, v)| **v == max);
    match (top.next(), top.next()) {
        (Some((oid, _)), None) if max >= threshold => Some(oid),
        _ => None,
    }
}

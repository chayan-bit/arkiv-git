//! Collaborative-object ops (docs/p3/cob-ops.md): [`post`] writes one signed op from the
//! author's control address; [`list`] queries ops and returns only the verified ones.

pub mod json;

use std::collections::{BTreeMap, BTreeSet};

use charter_core::{
    AttrValue, Body, CobOpBody, Hash32, Kind, Nid, Observed, Rid, VerifiedOp, peer_key, verify_ops,
};
use charter_store::{Arkiv, Pred, attr};

use crate::keys::Keys;
use crate::observe::{all, b32, kind};
use crate::repo::Config;
use crate::write::{entity_op, sign};
use crate::{Result, identity};

/// Signs and writes `body` (control plane, control lifetime); returns the op id. A writer
/// with no bind for the repo gets its `peer` and `bind_0` in the same tx.
pub async fn post<A: Arkiv>(a: &A, cfg: &Config, keys: &Keys, body: &CobOpBody) -> Result<Hash32> {
    body.validate()?;
    let s = sign(keys.sk(), body)?;
    let op = entity_op::<CobOpBody>(&s.bytes, cfg.control_lifetime)?;
    identity::with_bind(a, cfg, keys, &body.rid, op).await?;
    Ok(s.h)
}

/// Verified ops of `rid`, optionally of one object and one type, in Arkiv creation order.
pub async fn list<A: Arkiv>(
    a: &A,
    rid: &Rid,
    object: Option<&Hash32>,
    type_name: Option<&str>,
) -> Result<Vec<VerifiedOp>> {
    let mut terms = vec![kind(Kind::Cob), b32("ag.rid", rid)];
    terms.extend(object.map(|o| b32("ag.cob", o)));
    terms.extend(type_name.map(|t| Pred::eq(attr("ag.type"), t)));
    let observed = all(a, Pred::And(terms)).await?;
    let keys = peer_keys(a, &observed).await?;
    Ok(verify_ops(rid, &keys, &observed))
}

/// Keys of every `ag.nid` named in `observed` that has a valid self-signed `peer` document.
pub(crate) async fn peer_keys<A: Arkiv>(
    a: &A,
    observed: &[Observed],
) -> Result<BTreeMap<Nid, charter_crypto::PublicKey>> {
    let nids: BTreeSet<Nid> = observed
        .iter()
        .filter_map(|o| match o.attr("ag.nid") {
            Some(AttrValue::Bytes32(n)) => Some(*n),
            _ => None,
        })
        .collect();
    let mut keys = BTreeMap::new();
    for nid in nids {
        let docs = all(a, Pred::And(vec![kind(Kind::Peer), b32("ag.nid", &nid)])).await?;
        if let Some(pk) = peer_key(&nid, &docs) {
            keys.insert(nid, pk);
        }
    }
    Ok(keys)
}

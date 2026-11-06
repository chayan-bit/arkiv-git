//! MLS entities on Arkiv (`ag.kind = mls`, `ag.t` names the type) and the sealed lockbox.
//!
//! - `kp`: a key package, from the member's wallet; `ag.h = sha256(bytes)` is its id.
//! - `group`: the private-repo marker, `ag.tag = H(rid)`; its body is the suite code point.
//! - `commit`: from an admin's control address, `ag.rid = HMAC(rid)`, `ag.ep` = new epoch.
//! - `welcome`: `ag.kp` = the joiner's key package id, `ag.ep` = the joining epoch.
//! - `pub`: a published epoch grant (`id_key || content key`), `ag.tag = H(rid)`.
//!
//! Commits and welcomes carry `ag.box`: the keyring sealed under the new epoch key.

use arkiv_git_core::{AttrValue, Hash32, Kind, Observed, Rid};
use arkiv_git_store::{Arkiv, AttributeValue, Operation, Pred, attr, create_op, random_salt};
use arkiv_git_sync::observe;
use arkiv_git_sync::{Result, SyncError};

use crate::keyring::Keyring;
use crate::seal::{Key, aad, blind, open, seal};

pub const T_KP: &str = "kp";
pub const T_GROUP: &str = "group";
pub const T_COMMIT: &str = "commit";
pub const T_WELCOME: &str = "welcome";
pub const T_PUB: &str = "pub";
const BOX_KIND: &str = "box";

/// Public handle of a private repo for anyone who knows its rid.
pub fn tag(rid: &Rid) -> Hash32 {
    arkiv_git_crypto::h("private-tag", rid)
}

/// The blinded rid every commit carries.
pub fn blinded_rid(ring: &Keyring) -> Hash32 {
    blind(&ring.id_key, "rid", &ring.rid)
}

pub fn op(
    t: &str,
    extra: Vec<(&str, AttributeValue, bool)>,
    body: Vec<u8>,
    lifetime: u64,
) -> Result<Operation> {
    let mut attrs = vec![
        (
            "ag.kind",
            AttributeValue::Str(Kind::Mls.name().into()),
            true,
        ),
        (
            "ag.v",
            AttributeValue::U64(arkiv_git_core::ENTITY_LAYOUT_VERSION),
            false,
        ),
        ("ag.t", AttributeValue::Str(t.into()), true),
        ("ag.body", AttributeValue::Bytes(body), false),
    ];
    attrs.extend(extra);
    Ok(create_op(&attrs, lifetime, random_salt())?)
}

pub fn b32(v: Hash32) -> AttributeValue {
    AttributeValue::Bytes32(v)
}

/// Live `mls` entities of type `t` matching `extra`, in creation order.
pub async fn find<A: Arkiv>(a: &A, t: &str, extra: Vec<Pred>) -> Result<Vec<Observed>> {
    let mut all = vec![observe::kind(Kind::Mls), Pred::eq(attr("ag.t"), t)];
    all.extend(extra);
    observe::all(a, Pred::And(all)).await
}

pub fn bytes<'a>(o: &'a Observed, name: &str) -> Option<&'a [u8]> {
    match o.attr(name) {
        Some(AttrValue::Bytes(b)) => Some(b),
        _ => None,
    }
}

pub fn u64_of(o: &Observed, name: &str) -> Option<u64> {
    match o.attr(name) {
        Some(AttrValue::U64(n)) => Some(*n),
        _ => None,
    }
}

/// The keyring sealed under `key`, the key of `epoch`.
pub fn seal_box(ring: &Keyring, epoch: u64, key: &Key) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(ring)
        .map_err(|e| SyncError::coded(arkiv_git_core::ErrorCode::Internal, e.to_string()))?;
    seal(key, &aad(&ring.rid, BOX_KIND, epoch), &json)
}

pub fn open_box(rid: &Rid, epoch: u64, key: &Key, sealed: &[u8]) -> Option<Keyring> {
    let json = open(key, &aad(rid, BOX_KIND, epoch), sealed)?;
    serde_json::from_slice::<Keyring>(&json)
        .ok()
        .filter(|r| r.rid == *rid)
}

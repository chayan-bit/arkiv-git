//! `xyz.arkiv.escrow` op payloads and the helpers that post them (docs/p3/escrow.md).
//!
//! A delegate creates one escrow object per repo and appoints the agent in it; deposits,
//! payouts and refunds are follow-up ops of that object. Amounts are decimal wei strings
//! and tx hashes `0x` hex, so payloads stay canonical CBOR with no big integers.

use arkiv_git_core::{CobOpBody, Hash32, Rid, VerifiedOp, ZERO};
use arkiv_git_store::{Address, Arkiv, TxHash};
use arkiv_git_sync::cob::{self, json};
use arkiv_git_sync::ids::{parse_id, to_text};
use arkiv_git_sync::{Config, Keys};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{Result, bad};

pub const TYPE: &str = "xyz.arkiv.escrow";
pub const APPOINT: &str = "appoint";
pub const DEPOSIT: &str = "deposit";
pub const PAID: &str = "paid";
pub const REFUNDED: &str = "refunded";

/// Names the agent: its identity id (text form) and its escrow address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppointPayload {
    pub escrow: String,
    pub address: Address,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositPayload {
    pub tx: TxHash,
    pub amount: String,
    pub purpose: String,
    /// Block after which an unconsumed deposit is refunded to its sender.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry: Option<u64>,
}

/// A payout (`paid`) or a refund (`refunded`, which also names the deposit tx).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlePayload {
    pub key: String,
    pub tx: TxHash,
    pub to: Address,
    pub amount: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deposit: Option<TxHash>,
}

impl AppointPayload {
    pub fn nid(&self) -> Result<Hash32> {
        parse_id(&self.escrow)
    }
}

/// The payload of a verified op as `T`.
pub fn payload<T: DeserializeOwned>(op: &VerifiedOp) -> Result<T> {
    let v = json::to_json(&op.body.payload)?;
    serde_json::from_value(v).map_err(|e| bad(format!("{} payload: {e}", op.body.action)))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Posts one escrow op; `object` is zero for the create op.
pub async fn post<A: Arkiv, T: Serialize>(
    a: &A,
    cfg: &Config,
    keys: &Keys,
    rid: &Rid,
    object: Hash32,
    action: &str,
    payload: &T,
) -> Result<Hash32> {
    let v = serde_json::to_value(payload).map_err(|e| bad(format!("payload: {e}")))?;
    let body = CobOpBody {
        rid: *rid,
        type_name: TYPE.into(),
        object,
        parents: vec![],
        action: action.into(),
        payload: json::to_cbor(&v)?,
        ts_ms: now_ms(),
    };
    // Boxed here so callers' futures stay small (the write path's future is large).
    Box::pin(cob::post(a, cfg, keys, &body)).await
}

/// A delegate creates the repo's escrow object and appoints the agent in it; returns the
/// object id.
pub async fn appoint<A: Arkiv>(
    a: &A,
    cfg: &Config,
    delegate: &Keys,
    rid: &Rid,
    escrow: &Hash32,
    address: Address,
) -> Result<Hash32> {
    let object = post(
        a,
        cfg,
        delegate,
        rid,
        ZERO,
        "create",
        &serde_json::json!({}),
    )
    .await?;
    let p = AppointPayload {
        escrow: to_text(escrow),
        address,
    };
    post(a, cfg, delegate, rid, object, APPOINT, &p).await?;
    Ok(object)
}

/// Every verified escrow op of `rid`, in Arkiv order.
pub async fn list<A: Arkiv>(a: &A, rid: &Rid) -> Result<Vec<VerifiedOp>> {
    Box::pin(cob::list(a, rid, None, Some(TYPE))).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_round_trip_through_canonical_cbor() {
        let d = DepositPayload {
            tx: TxHash::repeat_byte(1),
            amount: "5".into(),
            purpose: "bounty:x".into(),
            expiry: None,
        };
        let v = serde_json::to_value(&d).expect("json");
        assert!(v.get("expiry").is_none());
        let cbor = json::to_cbor(&v).expect("cbor");
        let back: DepositPayload =
            serde_json::from_value(json::to_json(&cbor).expect("json")).expect("parse");
        assert_eq!(back, d);
        let s: std::result::Result<SettlePayload, _> =
            serde_json::from_value(serde_json::json!({"key": "k"}));
        assert!(s.is_err());
    }
}

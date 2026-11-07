//! One test per behaviour listed under "MemArkiv must model" (design 7.2).

use alloy_primitives::FixedBytes;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::SolValue;
use arkiv_bindings::{Attribute as WireAttr, Create, OP_CREATE};
use serde_json::json;

use super::*;
use crate::{
    AttributeValue as V, Attrs, Field, Pred, RowExt, Select, TxParams, Value, attr, create_op,
    extend_op,
};

const CHAIN: u64 = 7;
const LIFE: u64 = 1_000;

fn setup() -> (MemArkiv, PrivateKeySigner) {
    let fake = MemArkiv::new(CHAIN);
    let key = PrivateKeySigner::random();
    fake.fund(key.address(), U256::from(10u128.pow(20)));
    (fake, key)
}

fn tx(key: &PrivateKeySigner, nonce: u64, to: Address, input: Bytes, gas_limit: u64) -> SignedTx {
    let p = TxParams {
        nonce,
        to,
        value: U256::ZERO,
        input,
        gas_limit,
        max_fee_per_gas: 2 * MIN_BASE_FEE,
        max_priority_fee_per_gas: 0,
    };
    SignedTx::sign(key, CHAIN, p).expect("sign")
}

/// A tx at the next nonce with a 16M gas limit (so it fills a block's 30M on its own).
async fn sign(
    fake: &MemArkiv,
    key: &PrivateKeySigner,
    nonce: Option<u64>,
    to: Address,
    input: Bytes,
) -> SignedTx {
    let nonce = match nonce {
        Some(n) => n,
        None => fake.tx_count(key.address()).await.expect("nonce"),
    };
    tx(key, nonce, to, input, 16_000_000)
}

/// Send `ops` in one tx, mine a block, return the receipt.
async fn exec(fake: &MemArkiv, key: &PrivateKeySigner, ops: Vec<Operation>) -> Receipt {
    let tx = sign(fake, key, None, ARKIV_ADDRESS, execute_calldata(&ops)).await;
    let h = fake.send(tx).await.expect("send");
    fake.mine();
    fake.receipt(h).await.expect("receipt").expect("mined")
}

fn blob(kind: &str, i: u64, salt: u128) -> Operation {
    create_op(
        &[
            ("ag.kind", V::Str(kind.into()), true),
            ("ag.i", V::U64(i), true),
        ],
        LIFE,
        salt,
    )
    .expect("op")
}

async fn keys(fake: &MemArkiv, q: impl Into<Query>, opts: QueryOpts) -> Vec<B256> {
    let page = fake.query(&q.into(), &opts).await.expect("query");
    page.rows.iter().map(|r| r.key.expect("key")).collect()
}

async fn revert_reason(fake: &MemArkiv, from: Address, ops: &[Operation]) -> String {
    match fake.estimate(from, ops).await {
        Err(StoreError::Reverted { reason, .. }) => reason,
        other => panic!("expected a revert, got {other:?}"),
    }
}

fn raw_create(
    attributes: Vec<WireAttr>,
    flags: u8,
    min_lifetime: u64,
    expires_at: u64,
) -> Operation {
    let c = Create {
        salt: 1,
        expiresAt: expires_at,
        minLifetime: min_lifetime,
        creationFlags: flags,
        attributes,
    };
    Operation {
        operation: OP_CREATE,
        operationData: c.abi_encode().into(),
    }
}

fn wire(name: &str, type_id: u8, value: Vec<u8>) -> WireAttr {
    let mut n = [0u8; 32];
    n[..name.len()].copy_from_slice(name.as_bytes());
    WireAttr {
        name: FixedBytes(n),
        typeId: type_id,
        value: value.into(),
    }
}

mod chain;
mod entities;
mod queries;

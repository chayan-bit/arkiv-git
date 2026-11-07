//! Shared helpers for the `MemArkiv` integration tests.
#![allow(dead_code)]

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use alloy_signer_local::PrivateKeySigner;
use charter_store::fake::MemArkiv;
use charter_store::{
    ARKIV_ADDRESS, Arkiv, AttributeValue as V, Operation, SignedTx, TxParams, U256, create_op,
    execute_calldata,
};

pub const CHAIN: u64 = 7;
pub const DAY: u64 = 43_200;

pub fn funded(fake: &MemArkiv) -> PrivateKeySigner {
    let key = PrivateKeySigner::random();
    fake.fund(key.address(), U256::from(10u128.pow(21)));
    key
}

/// A small indexed entity of `kind` with `ag.seq = seq`.
pub fn small(kind: &str, seq: u64, salt: u128) -> Operation {
    create_op(
        &[
            ("ag.kind", V::Str(kind.into()), true),
            ("ag.seq", V::U64(seq), true),
        ],
        DAY,
        salt,
    )
    .expect("op")
}

/// A chunk-like create: `bytes` payload of `size` bytes.
pub fn blob(size: usize, lifetime: u64, salt: u128) -> Operation {
    let attrs = [
        ("ag.kind", V::Str("chunk".into()), true),
        ("ag.body", V::Bytes(vec![0xa5; size]), false),
    ];
    create_op(&attrs, lifetime, salt).expect("op")
}

/// Send `ops` in one tx outside any pipeline and mine it.
pub async fn exec(fake: &MemArkiv, key: &PrivateKeySigner, ops: Vec<Operation>) {
    let nonce = fake.tx_count(key.address()).await.expect("nonce");
    let p = TxParams {
        nonce,
        to: ARKIV_ADDRESS,
        value: U256::ZERO,
        input: execute_calldata(&ops),
        gas_limit: 16_000_000,
        max_fee_per_gas: 2_000_000_000,
        max_priority_fee_per_gas: 0,
    };
    let h = fake
        .send(SignedTx::sign(key, CHAIN, p).expect("sign"))
        .await
        .expect("send");
    fake.mine();
    assert!(fake.receipt(h).await.expect("r").expect("mined").success);
}

/// Run `f` while a background task mines a block every 2 s of (paused) tokio time.
pub async fn with_miner<F: Future>(fake: Arc<MemArkiv>, f: F) -> F::Output {
    let miner = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            fake.mine();
        }
    });
    let out = f.await;
    miner.abort();
    out
}

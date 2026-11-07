//! `TxPipeline` against `MemArkiv` with injected faults (design 4.4, 4.6, 8.3 "Pacing").
#![cfg(feature = "fake")]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use alloy_signer_local::PrivateKeySigner;
use charter_store::fake::{MAX_LIFETIME, MemArkiv};
use charter_store::pipeline::Landed;
use charter_store::{
    ARKIV_ADDRESS, Address, Arkiv, B256, PipelineConfig, SignedTx, StoreError, TxParams,
    TxPipeline, U256, execute_calldata, extend_op,
};
use common::{CHAIN, DAY, blob, exec, funded, small, with_miner};

fn setup() -> (Arc<MemArkiv>, PrivateKeySigner) {
    let fake = Arc::new(MemArkiv::new(CHAIN));
    let key = funded(&fake);
    (fake, key)
}

async fn run(
    fake: &Arc<MemArkiv>,
    key: &PrivateKeySigner,
    jobs: Vec<Vec<charter_store::Operation>>,
) -> Result<Vec<Landed>, StoreError> {
    let mut p = TxPipeline::new(&**fake, key.clone(), CHAIN, PipelineConfig::default());
    with_miner(fake.clone(), p.run(jobs)).await
}

/// Gas per block from the receipts of one run.
fn gas_by_block(landed: &[Landed]) -> BTreeMap<u64, (u64, usize)> {
    let mut m: BTreeMap<u64, (u64, usize)> = BTreeMap::new();
    for l in landed {
        let e = m.entry(l.receipt.block_number).or_default();
        e.0 += l.receipt.gas_used;
        e.1 += 1;
    }
    m
}

#[tokio::test(start_paused = true)]
async fn chunks_land_paced_at_40_percent_of_block_gas() {
    let (fake, key) = setup();
    let jobs: Vec<_> = (0..40).map(|i| vec![blob(65_536, DAY, i)]).collect();
    let landed = run(&fake, &key, jobs).await.expect("run");
    assert_eq!(landed.len(), 40);
    assert!(
        landed.windows(2).all(|w| w[0].job < w[1].job),
        "one receipt per job, in job order"
    );
    assert_eq!(fake.entity_nonce(key.address()).await.expect("n"), 40);
    let per_block = gas_by_block(&landed);
    for (n, (gas, _)) in &per_block {
        assert!(*gas <= 12_000_000, "block {n} carried {gas} gas of ours");
        assert_eq!(fake.block_gas_of(*n, key.address()), *gas);
    }
    let best = per_block.values().map(|(_, c)| *c).max().expect("blocks");
    assert_eq!(
        best, 4,
        "1-day 64 KiB chunks: 4 per block under the 12M budget"
    );
}

#[tokio::test(start_paused = true)]
async fn the_window_caps_small_txs_at_16_per_block() {
    let (fake, key) = setup();
    let jobs: Vec<_> = (0..40).map(|i| vec![small("w", i, i.into())]).collect();
    let landed = run(&fake, &key, jobs).await.expect("run");
    let per_block = gas_by_block(&landed);
    assert!(per_block.values().all(|(_, c)| *c <= 16));
    assert_eq!(per_block.values().map(|(_, c)| *c).max(), Some(16));
}

#[tokio::test(start_paused = true)]
async fn a_high_base_fee_halves_the_budget() {
    let (fake, key) = setup();
    fake.set_base_fee(20_000_000_000);
    let jobs: Vec<_> = (0..8).map(|i| vec![blob(65_536, DAY, i)]).collect();
    let landed = run(&fake, &key, jobs).await.expect("run");
    let first = gas_by_block(&landed).into_iter().next().expect("a block").1;
    assert!(first.0 <= 6_000_000 && first.1 == 2, "{first:?}");
}

#[tokio::test(start_paused = true)]
async fn dropped_txs_are_resent_until_they_land() {
    let (fake, key) = setup();
    fake.drop_sends(2, 1);
    let jobs: Vec<_> = (0..6).map(|i| vec![small("d", i, i.into())]).collect();
    let landed = run(&fake, &key, jobs).await.expect("run");
    assert_eq!(landed.len(), 6);
    assert_eq!(
        fake.entity_nonce(key.address()).await.expect("n"),
        6,
        "nothing landed twice"
    );
}

#[tokio::test(start_paused = true)]
async fn a_tx_dropped_on_every_resend_is_reported() {
    let (fake, key) = setup();
    fake.drop_next(100);
    let err = run(&fake, &key, vec![vec![small("d", 0, 0)]])
        .await
        .expect_err("dropped");
    assert!(matches!(err, StoreError::Dropped(_)), "{err:?}");
    assert_eq!(err.code(), "E_RPC");
}

#[tokio::test(start_paused = true)]
async fn reverts_are_decoded() {
    let (fake, key) = setup();
    let err = run(&fake, &key, vec![vec![extend_op(B256::repeat_byte(9), 10)]])
        .await
        .expect_err("revert");
    assert!(
        matches!(&err, StoreError::Reverted { tx: None, reason } if reason == "EntityNotFound"),
        "{err:?}"
    );
    assert_eq!(err.code(), "E_INTERNAL");
    fake.revert_next_at(0);
    let err = run(&fake, &key, vec![vec![small("r", 0, 0)]])
        .await
        .expect_err("mined revert");
    assert!(
        matches!(&err, StoreError::Reverted { tx: Some(_), .. }),
        "{err:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn jobs_over_the_byte_cap_are_split() {
    let (fake, key) = setup();
    let landed = run(
        &fake,
        &key,
        vec![(0..3).map(|i| blob(60_000, DAY, i)).collect()],
    )
    .await
    .expect("run");
    assert!(landed.len() >= 2 && landed.iter().all(|l| l.job == 0));
    assert_eq!(fake.entity_nonce(key.address()).await.expect("n"), 3);
}

#[tokio::test(start_paused = true)]
async fn jobs_over_the_gas_cap_are_split_and_single_ops_fail() {
    let (fake, key) = setup();
    let job: Vec<_> = (0..8).map(|i| blob(16_384, MAX_LIFETIME, i)).collect();
    let landed = run(&fake, &key, vec![job]).await.expect("run");
    assert!(landed.len() >= 2);
    assert_eq!(fake.entity_nonce(key.address()).await.expect("n"), 8);
    let err = run(&fake, &key, vec![vec![blob(126 * 1024, MAX_LIFETIME, 99)]])
        .await
        .expect_err("too big");
    assert_eq!(err.code(), "E_GAS_CAP");
}

#[tokio::test(start_paused = true)]
async fn a_stolen_nonce_is_reread() {
    let (fake, key) = setup();
    let mut p = TxPipeline::new(&*fake, key.clone(), CHAIN, PipelineConfig::default());
    with_miner(fake.clone(), p.run(vec![vec![small("n", 0, 0)]]))
        .await
        .expect("first");
    exec(&fake, &key, vec![small("n", 1, 1)]).await;
    with_miner(fake.clone(), p.run(vec![vec![small("n", 2, 2)]]))
        .await
        .expect("after re-read");
    assert_eq!(fake.entity_nonce(key.address()).await.expect("n"), 3);
}

#[tokio::test(start_paused = true)]
async fn transfers_and_insufficient_funds() {
    let (fake, key) = setup();
    let to = Address::repeat_byte(0x42);
    let mut p = TxPipeline::new(&*fake, key.clone(), CHAIN, PipelineConfig::default());
    assert_eq!(p.address(), key.address());
    let r = with_miner(fake.clone(), p.transfer(to, U256::from(1_000)))
        .await
        .expect("transfer");
    assert!(r.success);
    assert_eq!(fake.balance(to).await.expect("bal"), U256::from(1_000));
    let poor = PrivateKeySigner::random();
    let err = run(&fake, &poor, vec![vec![small("p", 0, 0)]])
        .await
        .expect_err("no funds");
    assert_eq!(err.code(), "E_INSUFFICIENT_FUNDS");
}

#[tokio::test]
async fn send_refuses_a_tx_already_too_large_for_the_rpc() {
    let fake = MemArkiv::new(CHAIN);
    let key = funded(&fake);
    let p = TxParams {
        nonce: 0,
        to: ARKIV_ADDRESS,
        value: U256::ZERO,
        input: execute_calldata(&[blob(131_000, DAY, 0)]),
        gas_limit: 6_000_000,
        max_fee_per_gas: 2_000_000_000,
        max_priority_fee_per_gas: 0,
    };
    let err = fake
        .send(SignedTx::sign(&key, CHAIN, p).expect("sign"))
        .await
        .expect_err("too big");
    assert_eq!(err.code(), "E_TX_TOO_LARGE");
}

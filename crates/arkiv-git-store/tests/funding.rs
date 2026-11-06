//! Rolling top-ups (design 4.2 step 6): a pipeline with a funder tops its sender up per
//! window of in-flight txs, never holding more than one window's max-fee reserve.
#![cfg(feature = "fake")]

mod common;

use std::sync::Arc;

use alloy_signer_local::PrivateKeySigner;
use arkiv_git_store::fake::MemArkiv;
use arkiv_git_store::{Arkiv, PipelineConfig, TxPipeline, U256};
use common::{CHAIN, DAY, blob, with_miner};

const CHUNKS: u128 = 40;

fn setup(wallet_wei: U256) -> (Arc<MemArkiv>, PrivateKeySigner, PrivateKeySigner) {
    let fake = Arc::new(MemArkiv::new(CHAIN));
    let wallet = PrivateKeySigner::random();
    fake.fund(wallet.address(), wallet_wei);
    (fake, wallet, PrivateKeySigner::random())
}

fn chunks() -> Vec<Vec<arkiv_git_store::Operation>> {
    (0..CHUNKS).map(|i| vec![blob(65_536, DAY, i)]).collect()
}

async fn chunk_gas(fake: &MemArkiv, sender: &PrivateKeySigner) -> u64 {
    fake.estimate(sender.address(), &[blob(65_536, DAY, 0)])
        .await
        .expect("estimate")
}

#[tokio::test(start_paused = true)]
async fn an_unfunded_sender_is_topped_up_per_window_from_the_funder() {
    let (fake, wallet, sender) = setup(U256::from(10u128.pow(21)));
    let cfg = PipelineConfig::default();
    let mut p =
        TxPipeline::new(&*fake, sender.clone(), CHAIN, cfg.clone()).funded_by(wallet.clone());
    let landed = with_miner(fake.clone(), p.run(chunks()))
        .await
        .expect("run");
    assert_eq!(landed.len() as u128, CHUNKS);

    let gas = chunk_gas(&fake, &sender).await;
    let base = fake.base_fee().await.expect("fee");
    let window = cfg.window_reserve(gas, base);
    let left = fake.balance(sender.address()).await.expect("balance");
    assert!(
        left <= window,
        "the sender keeps at most one window's reserve: {left} > {window}"
    );
    let top_ups = fake.tx_count(wallet.address()).await.expect("nonce");
    assert!(
        top_ups > 1,
        "several rolling top-ups, not one up-front transfer"
    );
    let spent: U256 = landed
        .iter()
        .map(|l| U256::from(l.receipt.gas_used) * U256::from(l.receipt.effective_gas_price))
        .sum();
    let moved = U256::from(10u128.pow(21)) - fake.balance(wallet.address()).await.expect("b");
    assert!(moved <= spent + window + U256::from(top_ups * 21_000) * U256::from(2 * base));
}

#[tokio::test(start_paused = true)]
async fn a_top_up_the_funder_cannot_cover_stops_the_run_with_insufficient_funds() {
    let probe = Arc::new(MemArkiv::new(CHAIN));
    let gas = chunk_gas(&probe, &PrivateKeySigner::random()).await;
    let cfg = PipelineConfig::default();
    let base = probe.base_fee().await.expect("fee");
    // One and a half windows: the first top-up fits, 40 chunks need about two.
    let (fake, wallet, sender) =
        setup(cfg.window_reserve(gas, base) * U256::from(3) / U256::from(2));
    let mut p = TxPipeline::new(&*fake, sender.clone(), CHAIN, cfg).funded_by(wallet);
    let err = with_miner(fake.clone(), p.run(chunks()))
        .await
        .expect_err("runs dry");
    assert_eq!(err.code(), "E_INSUFFICIENT_FUNDS");
    let landed = fake.entity_nonce(sender.address()).await.expect("n");
    assert!(
        landed > 0 && u128::from(landed) < CHUNKS,
        "stopped mid-run after {landed}"
    );
}

#[test]
fn the_window_reserve_is_a_full_window_at_the_max_fee() {
    let cfg = PipelineConfig::default();
    let gas = 2_724_658u64;
    let per_tx = U256::from(gas + gas / 10) * U256::from(2_000_000_000u128);
    assert_eq!(
        cfg.window_reserve(gas, 1_000_000_000),
        per_tx * U256::from(16)
    );
    let tipped = PipelineConfig {
        tip: 5,
        ..PipelineConfig::default()
    };
    assert_eq!(
        tipped.window_reserve(100_000, 1),
        U256::from(110_000u64 * 7 * 16)
    );
}

//! Funds and sweeps the wallet behind the live Rabdi demo repo in the README.
//!
//! Pays from the throwaway testnet key in `spike/arkiv-measure/.env`, which it never prints.
//! `ARKIV_GIT_DEMO_HOME` is the demo's `$ARKIV_GIT_HOME` (seed already created by `keys init`).
//!
//! `ARKIV_GIT_RABDI=1 ARKIV_GIT_DEMO_HOME=... cargo test -p arkiv-git-cli --test rabdi_demo fund -- --ignored`
//! then, after the demo, the same with `sweep`, which keeps a keepalive reserve on the wallet.

mod common;

use std::path::PathBuf;

use arkiv_git_store::{Arkiv, PipelineConfig, TxPipeline, U256};
use common::live::*;

const FUND_WEI: u128 = 30_000_000_000_000_000; // 0.03 tGLM
// `arkiv-git keepalive` signs with the wallet, so the keepalive reserve stays there.
const KEEP_WALLET_WEI: u128 = 5_000_000_000_000_000; // 0.005 tGLM

fn demo_home() -> PathBuf {
    PathBuf::from(std::env::var("ARKIV_GIT_DEMO_HOME").expect("set ARKIV_GIT_DEMO_HOME"))
}

#[test]
#[ignore = "live Rabdi: moves funds from the throwaway key to the demo wallet"]
fn fund() {
    if !enabled() {
        return;
    }
    let s = secrets();
    let rpc = connect(&s);
    let keys = arkiv_git_sync::Keys::require(&demo_home()).expect("keys");
    eprintln!(
        "throwaway before: {}",
        arkiv_git_cli::units::fmt_glm(balance(&rpc, s.key.address()))
    );
    // ARKIV_GIT_DEMO_FUND_WEI tops up a wallet that a push found short of its reserve.
    let amount = std::env::var("ARKIV_GIT_DEMO_FUND_WEI")
        .map(|v| v.parse::<u128>().expect("wei"))
        .unwrap_or(FUND_WEI);
    assert!(amount <= FUND_WEI, "demo budget is 0.03 tGLM per transfer");
    transfer(&rpc, &s.key, keys.wallet().address(), U256::from(amount));
    eprintln!(
        "demo wallet: {}",
        arkiv_git_cli::units::fmt_glm(balance(&rpc, keys.wallet().address()))
    );
}

#[test]
#[ignore = "live Rabdi: returns the demo wallet's leftover to the throwaway key"]
fn sweep() {
    if !enabled() {
        return;
    }
    let s = secrets();
    let rpc = connect(&s);
    let home = demo_home();
    let keys = arkiv_git_sync::Keys::require(&home).expect("keys");
    let rids: Vec<_> = std::fs::read_dir(home.join("repos"))
        .expect("repos")
        .flatten()
        .filter_map(|e| arkiv_git_sync::ids::unhex32(&e.file_name().to_string_lossy()).ok())
        .collect();
    arkiv_git_cli::block_on(async {
        let chain = rpc.chain_id().await.expect("chain");
        let base = rpc.base_fee().await.expect("fee");
        let cfg = PipelineConfig::default();
        let reserve = U256::from(30_000u64) * U256::from(2 * base + cfg.tip);
        let mut plan = vec![(keys.wallet(), U256::from(KEEP_WALLET_WEI))];
        for rid in &rids {
            plan.push((keys.bulk(rid), U256::ZERO));
            plan.push((keys.control(rid), U256::ZERO));
        }
        for (signer, keep) in plan {
            let have = rpc.balance(signer.address()).await.expect("balance");
            if have <= reserve + keep {
                continue;
            }
            let mut p = TxPipeline::new(&rpc, signer, chain, cfg.clone());
            if let Err(e) = p.transfer(s.key.address(), have - reserve - keep).await {
                eprintln!("sweep failed: {}", e.code());
            }
        }
        let wallet = keys.wallet().address();
        eprintln!(
            "wallet {wallet}: {}",
            arkiv_git_cli::units::fmt_glm(rpc.balance(wallet).await.expect("balance"))
        );
        for rid in &rids {
            for (name, a) in [
                ("bulk", keys.bulk(rid).address()),
                ("control", keys.control(rid).address()),
            ] {
                let b = rpc.balance(a).await.expect("balance");
                eprintln!("{name} {a}: {}", arkiv_git_cli::units::fmt_glm(b));
            }
        }
    })
    .expect("rt");
    eprintln!(
        "throwaway after: {}",
        arkiv_git_cli::units::fmt_glm(balance(&rpc, s.key.address()))
    );
}

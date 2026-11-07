//! Funds and sweeps the live AI-agent bounty demo (`demo/agent-bounty.sh`,
//! docs/live/agent-bounty.md). Every step is an ignored test on Rabdi:
//!
//! `CHARTER_RABDI=1 CHARTER_AGENT_ROOT=<demo root> cargo test -p charter-cli
//! --test rabdi_agent <step> -- --ignored --nocapture`
//!
//! - `bank_fund`: one transfer of `CHARTER_AGENT_FUND_WEI` (at most 0.08 tGLM) from the
//!   throwaway key to `<root>/bank`'s wallet; run it under the shared funding lock.
//! - `bank_pay`: bank to `CHARTER_AGENT_PAY=<0xaddr>:<wei>[,...]`, at most 0.04 each.
//! - `bank_collect`: every peer home under the root (and its escrow identity) back to the
//!   bank. The escrow payout key keeps its gas float, so the keyless audit stays clean.
//! - `bank_return`: the bank's balance back to the throwaway key.

mod common;

use std::path::PathBuf;

use charter_cli::units::fmt_glm;
use charter_store::{Address, U256};
use charter_sync::Keys;
use common::live::*;

const MAX_FUND_WEI: u128 = 80_000_000_000_000_000; // 0.08 tGLM
const MAX_PAY_WEI: u128 = 40_000_000_000_000_000; // 0.04 tGLM

fn root() -> PathBuf {
    PathBuf::from(std::env::var("CHARTER_AGENT_ROOT").expect("set CHARTER_AGENT_ROOT"))
}

fn bank() -> Keys {
    Keys::require(&root().join("bank/.charter")).expect("bank keys (demo setup)")
}

fn wei(v: &str) -> u128 {
    v.parse().expect("wei")
}

#[test]
#[ignore = "live Rabdi: moves at most 0.08 tGLM from the throwaway key to the demo bank"]
fn bank_fund() {
    if !enabled() {
        return;
    }
    let s = secrets();
    let rpc = connect(&s);
    let amount = wei(&std::env::var("CHARTER_AGENT_FUND_WEI").expect("fund wei"));
    assert!(amount <= MAX_FUND_WEI, "demo budget is 0.08 tGLM");
    let to = bank().wallet().address();
    eprintln!(
        "throwaway before: {}",
        fmt_glm(balance(&rpc, s.key.address()))
    );
    transfer(&rpc, &s.key, to, U256::from(amount));
    eprintln!("bank {to}: {}", fmt_glm(balance(&rpc, to)));
}

#[test]
#[ignore = "live Rabdi: moves bank funds to demo addresses"]
fn bank_pay() {
    if !enabled() {
        return;
    }
    let rpc = connect(&secrets());
    let plan = std::env::var("CHARTER_AGENT_PAY").expect("set CHARTER_AGENT_PAY");
    let bank = bank().wallet();
    for item in plan.split(',') {
        let (addr, amount) = item.split_once(':').expect("<0xaddr>:<wei>");
        let (to, amount): (Address, u128) = (addr.parse().expect("address"), wei(amount));
        assert!(amount <= MAX_PAY_WEI, "at most 0.04 tGLM per demo address");
        transfer(&rpc, &bank, to, U256::from(amount));
        eprintln!("paid {to} {}", fmt_glm(U256::from(amount)));
    }
    eprintln!("bank left: {}", fmt_glm(balance(&rpc, bank.address())));
}

#[test]
#[ignore = "live Rabdi: returns demo peer leftovers to the bank"]
fn bank_collect() {
    if !enabled() {
        return;
    }
    let rpc = connect(&secrets());
    let to = bank().wallet().address();
    for entry in std::fs::read_dir(root()).expect("root").flatten() {
        let home = entry.path();
        if entry.file_name() == "bank" {
            continue;
        }
        let rids = rids(&home);
        for dir in [home.join(".charter"), home.join(".charter/escrow")] {
            if let Ok(keys) = Keys::require(&dir) {
                sweep(&rpc, &keys, &rids, to);
            }
        }
    }
    eprintln!("bank after: {}", fmt_glm(balance(&rpc, to)));
}

#[test]
#[ignore = "live Rabdi: returns the demo bank's balance to the throwaway key"]
fn bank_return() {
    if !enabled() {
        return;
    }
    let s = secrets();
    let rpc = connect(&s);
    let keys = bank();
    sweep(&rpc, &keys, &[], s.key.address());
    eprintln!(
        "bank after: {}",
        fmt_glm(balance(&rpc, keys.wallet().address()))
    );
    eprintln!(
        "throwaway after: {}",
        fmt_glm(balance(&rpc, s.key.address()))
    );
}

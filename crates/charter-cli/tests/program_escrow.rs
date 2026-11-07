//! A pinned bounty program drives the escrow end to end on MemArkiv: the agent pays the
//! obligation the program emits after the merge, and a keyless audit passes.

mod common;

use charter_cli::backend::MEM_CHAIN_ID;
use charter_store::fake::FileMemArkiv;
use charter_store::{Address, U256};
use common::e2e::*;
use common::programs::*;

const TGLM: u128 = 1_000_000_000_000_000_000;

fn first_word<'a>(out: &'a str, key: &str) -> &'a str {
    field(out, key).split_whitespace().next().unwrap_or("")
}

fn fund_wei(log: &std::path::Path, addr: &str, wei: u128) {
    let mem = FileMemArkiv::open(log, MEM_CHAIN_ID).expect("open");
    let a: Address = addr.parse().expect("address");
    mem.fund(a, U256::from(wei)).expect("fund");
}

#[test]
fn bounty_obligation_is_paid_by_the_escrow_and_audits_clean() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (root, log) = (tmp.path(), tmp.path().join("chain.log"));
    let (alice, bob) = (user(root, "alice", &log), user(root, "bob", &log));
    let (url, work, _) = repo_with_main(&alice, root, "bounty-escrow");

    let agent = User::new(root, "agent", &log);
    let esc = agent.arkiv(&["escrow", "init"]);
    let (id, address) = (first_word(&esc, "escrow:"), first_word(&esc, "address:"));
    fund(&log, first_word(&esc, "wallet:"));
    fund_wei(&log, address, TGLM / 10); // gas for the payout transfer
    let appoint = [
        "escrow",
        "appoint",
        "--repo",
        &url,
        "--escrow",
        id,
        "--address",
        address,
    ];
    alice.arkiv(&appoint);
    alice.arkiv(&[
        "program",
        "pin",
        "--repo",
        &url,
        "--file",
        &program("bounty"),
    ]);

    let bounty = TGLM / 2;
    let issue = post(
        &alice,
        &url,
        "xyz.arkiv.issue",
        "create",
        None,
        &format!(r#"{{"title":"crash","bounty":{bounty}}}"#),
    );
    let purpose = format!("bounty:{issue}");
    let amount = TGLM.to_string();
    let deposit = [
        "escrow",
        "deposit",
        "--repo",
        &url,
        "--amount",
        &amount,
        "--purpose",
        &purpose,
    ];
    alice.arkiv(&deposit);

    let c1 = commit(&bob, &work, "fix\n");
    let patch = post(
        &bob,
        &url,
        "xyz.arkiv.patch",
        "create",
        None,
        &format!(r#"{{"head":"{c1}"}}"#),
    );
    let merged = format!(r#"{{"patch":"{patch}","closes":"{issue}"}}"#);
    post(
        &alice,
        &url,
        "xyz.arkiv.patch",
        "merged",
        Some(&patch),
        &merged,
    );
    let before = agent.arkiv(&["escrow", "run", "--repo", &url, "--once"]);
    assert!(
        !before.contains("paid"),
        "nothing owed before main moves: {before}"
    );

    alice.git(&work, &["push", "-q", &url, "main"]);
    let ran = agent.arkiv(&["escrow", "run", "--repo", &url, "--once"]);
    assert!(ran.contains(&format!("paid {purpose}")), "{ran}");
    let again = agent.arkiv(&["escrow", "run", "--repo", &url, "--once"]);
    assert!(!again.contains("paid"), "each key is paid once: {again}");

    let auditor = User::new(root, "auditor", &log);
    let report = auditor.arkiv(&["escrow", "audit", "--repo", &url]);
    assert!(!report.contains("FLAG"), "{report}");
    assert!(report.contains("settled:  1 op(s)"), "{report}");
}

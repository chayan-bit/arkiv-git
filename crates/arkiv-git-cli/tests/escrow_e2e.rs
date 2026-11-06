//! `arkiv-git escrow` end to end through the real binary against the file-persisted MemArkiv.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use arkiv_git_store::fake::FileMemArkiv;
use arkiv_git_store::{Address, U256};

const TGLM: u128 = 1_000_000_000_000_000_000;

struct User {
    home: PathBuf,
    log: PathBuf,
}

impl User {
    fn new(root: &Path, name: &str, log: &Path) -> Self {
        let home = root.join(name);
        std::fs::create_dir_all(&home).expect("home");
        let log = log.to_path_buf();
        Self { home, log }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_arkiv-git"))
            .current_dir(&self.home)
            .env("HOME", &self.home)
            .env("ARKIV_GIT_HOME", self.home.join(".arkiv-git"))
            .env("ARKIV_GIT_BACKEND", format!("mem:{}", self.log.display()))
            .args(args)
            .output()
            .expect("spawn")
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{args:?}: {err}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

fn field<'a>(out: &'a str, key: &str) -> &'a str {
    out.lines()
        .find_map(|l| l.strip_prefix(key))
        .map(|v| v.split_whitespace().next().unwrap_or(""))
        .unwrap_or_else(|| panic!("no {key} in {out}"))
}

fn fund(log: &Path, addr: &str, wei: u128) {
    let mem = FileMemArkiv::open(log, arkiv_git_cli::backend::MEM_CHAIN_ID).expect("open");
    let a: Address = addr.parse().expect("address");
    mem.fund(a, U256::from(wei)).expect("fund");
}

#[test]
fn appoint_deposit_pay_and_audit_through_the_cli() {
    let tmp = tempfile::tempdir().expect("tmp");
    let log = tmp.path().join("chain.log");
    let (alice, agent) = (
        User::new(tmp.path(), "alice", &log),
        User::new(tmp.path(), "agent", &log),
    );
    fund(
        &log,
        field(&alice.ok(&["keys", "init"]), "wallet:"),
        1000 * TGLM,
    );
    let url = field(&alice.ok(&["init", "--name", "paid"]), "url:").to_owned();
    let esc = agent.ok(&["escrow", "init"]);
    let (id, address) = (field(&esc, "escrow:"), field(&esc, "address:"));
    assert!(
        !esc.to_lowercase().contains("secret") && esc.lines().count() == 3,
        "{esc}"
    );
    fund(&log, field(&esc, "wallet:"), 10 * TGLM);
    fund(&log, address, TGLM / 10);
    assert!(
        agent.run(&["escrow", "init"]).status.code() == Some(1),
        "no overwrite"
    );

    alice.ok(&[
        "escrow",
        "appoint",
        "--repo",
        &url,
        "--escrow",
        id,
        "--address",
        address,
    ]);
    let amount = TGLM.to_string();
    alice.ok(&[
        "escrow",
        "deposit",
        "--repo",
        &url,
        "--amount",
        &amount,
        "--purpose",
        "bounty:1",
    ]);

    let bob = "0x00000000000000000000000000000000000000b0";
    let obligations = tmp.path().join("obligations.json");
    let ob = serde_json::json!([{ "kind": "pay", "to": bob, "amount": (TGLM / 2).to_string(),
        "reason": "fixed #1", "key": "bounty:1:alice", "purpose": "bounty:1" }]);
    std::fs::write(&obligations, ob.to_string()).expect("write");
    let obs = obligations.to_str().expect("utf8");
    let ran = agent.ok(&[
        "escrow",
        "run",
        "--repo",
        &url,
        "--obligations",
        obs,
        "--once",
    ]);
    assert!(ran.contains("paid bounty:1:alice"), "{ran}");
    let again = agent.ok(&[
        "escrow",
        "run",
        "--repo",
        &url,
        "--obligations",
        obs,
        "--once",
    ]);
    assert!(!again.contains("paid"), "{again}");

    let anyone = User::new(tmp.path(), "auditor", &log);
    let report = anyone.ok(&["escrow", "audit", "--repo", &url, "--obligations", obs]);
    assert!(
        report.contains("credited: 1000000000000000000 wei") && !report.contains("FLAG"),
        "{report}"
    );

    std::fs::write(&obligations, "[]").expect("write");
    let out = anyone.run(&["escrow", "audit", "--repo", &url, "--obligations", obs]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("FLAG unauthorized payout"), "{text}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("flagged: audit found"));
}

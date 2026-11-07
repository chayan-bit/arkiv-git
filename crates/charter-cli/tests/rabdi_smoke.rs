//! Live Rabdi smoke (design 8.5, minimal): a few-KiB repo pushed with `git push arkiv::`
//! and cloned with `git clone arkiv::` through the real binaries.
//!
//! Pays from the throwaway testnet key in `spike/arkiv-measure/.env` (`PRIVATE_KEY`, optional
//! `ARKIV_ACCESS_KEY`), which it never prints. It moves `BUDGET` to a fresh seed's wallet,
//! runs with 1 hour lifetimes, sweeps what is left back, and asserts the net spend stays
//! under the 0.02 tGLM cap.
//!
//! `CHARTER_RABDI=1 cargo test -p charter-cli --test rabdi_smoke -- --ignored --nocapture`

mod common;

use std::path::Path;

use charter_store::U256;
use common::live::*;

const CAP_WEI: u128 = 20_000_000_000_000_000; // 0.02 tGLM
const BUDGET_WEI: u128 = 19_000_000_000_000_000; // 0.019 tGLM, swept back afterwards
const LIFETIME: &str = "1800"; // blocks: 1 hour for data and control entities

#[test]
#[ignore = "live Rabdi: needs CHARTER_RABDI=1 and the funded throwaway key"]
fn live_rabdi_push_then_clone() {
    if !enabled() {
        return;
    }
    let s = secrets();
    let tmp = tempfile::tempdir().expect("tmp");
    let (alice, bob) = (tmp.path().join("alice"), tmp.path().join("bob"));
    [&alice, &bob]
        .iter()
        .for_each(|d| std::fs::create_dir_all(d).expect("home"));
    let rpc = connect(&s);
    let before = balance(&rpc, s.key.address());
    assert!(
        before >= U256::from(CAP_WEI),
        "throwaway key holds less than the cap"
    );
    let run = |home: &Path, program: &str, dir: &Path, args: &[&str]| {
        common::live::run(home, &s.access, LIFETIME, program, dir, args)
    };

    run(
        &alice,
        env!("CARGO_BIN_EXE_chart"),
        &alice,
        &["keys", "init"],
    );
    let keys = charter_sync::Keys::require(&alice.join(".charter")).expect("keys");
    transfer(
        &rpc,
        &s.key,
        keys.wallet().address(),
        U256::from(BUDGET_WEI),
    );

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let created = run(
            &alice,
            env!("CARGO_BIN_EXE_chart"),
            &alice,
            &["init", "--name", "smoke"],
        );
        let url = created
            .lines()
            .find_map(|l| l.strip_prefix("url: "))
            .expect("url")
            .to_owned();
        eprintln!("smoke repo {url}");
        let work = tmp.path().join("w");
        run(
            &alice,
            "git",
            tmp.path(),
            &["init", "-q", "-b", "main", &work.to_string_lossy()],
        );
        std::fs::write(work.join("README"), vec![b'x'; 3_000]).expect("write");
        run(&alice, "git", &work, &["add", "-A"]);
        run(&alice, "git", &work, &["commit", "-q", "-m", "feat: smoke"]);
        run(&alice, "git", &work, &["push", "-q", &url, "main"]);
        let dest = tmp.path().join("clone");
        run(
            &bob,
            "git",
            tmp.path(),
            &["clone", "-q", &url, &dest.to_string_lossy()],
        );
        let head = |d: &Path| run(&alice, "git", d, &["rev-parse", "HEAD"]);
        assert_eq!(head(&dest), head(&work));
    }));

    sweep(&rpc, &keys, &rids(&alice), s.key.address());
    let spent = before.saturating_sub(balance(&rpc, s.key.address()));
    eprintln!(
        "smoke: net spend {} tGLM",
        charter_cli::units::fmt_glm(spent)
    );
    assert!(spent <= U256::from(CAP_WEI), "spent over the cap");
    if let Err(p) = result {
        std::panic::resume_unwind(p);
    }
}

//! Live multi-peer collaboration on Rabdi through the real binaries (docs/live/collab-escrow.md):
//! patches, reviews, issues and merges between separate homes; a second delegate and
//! quorum; rollback by revert, by forced push and by a withholding view; a pinned
//! branch-protection program; and a bounty paid by the escrow agent.
//!
//! Money flow: `fund_bank` moves at most `BANK_CAP_WEI` in total from the throwaway key in
//! `spike/arkiv-measure/.env` (never printed) to a bank home under
//! `$ARKIV_GIT_COLLAB_ROOT/bank`, holding the shared `ARKIV_GIT_FUND_LOCK`; each test funds
//! its peers from the bank and sweeps them back, even on panic; `sweep_bank` returns the
//! rest. Every entity lives 1 day.
//!
//! ```sh
//! export ARKIV_GIT_RABDI=1 ARKIV_GIT_COLLAB_ROOT=<dir> ARKIV_GIT_FUND_LOCK=<file>
//! arkiv-git keys init   # with ARKIV_GIT_HOME=$ARKIV_GIT_COLLAB_ROOT/bank/.arkiv-git
//! cargo test --release -p arkiv-git-cli --test rabdi_collab fund_bank -- --ignored
//! cargo test --release -p arkiv-git-cli --test rabdi_collab live_ -- --ignored --test-threads 1
//! cargo test --release -p arkiv-git-cli --test rabdi_collab sweep_bank -- --ignored
//! ```

mod common;

use std::path::Path;

use arkiv_git_core::Kind;
use arkiv_git_store::{Address, B256, U256};
use arkiv_git_sync::git::Git;
use arkiv_git_sync::{Config, Keys, Push, Repo, Update, Want, sync};
use common::collab::*;
use common::live::{self, Hiding, entities, seq_of};

/// Most that may ever leave the throwaway key for these tests.
const BANK_CAP_WEI: u128 = 50 * MILLI;
/// Lent to the escrow identity per round (it posts at most one `paid` op).
const AGENT_FLOAT: u128 = 2 * MILLI;

fn field(out: &str, key: &str) -> String {
    out.lines()
        .find_map(|l| l.strip_prefix(key))
        .map(|v| v.split_whitespace().next().unwrap_or("").to_owned())
        .unwrap_or_else(|| panic!("no {key} in {out}"))
}

fn program(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../programs/{name}.rhai"));
    p.to_string_lossy().into_owned()
}

/// Creates repo `name` owned by `owner`, pushes `hello.c` v0 on `main`; returns
/// (url, work dir, c0).
fn repo_with_main(net: &Net, owner: &Peer, name: &str) -> (String, std::path::PathBuf, String) {
    let created = owner.write(&["init", "--name", name, "--description", "live test"]);
    let url = field(&created, "url:");
    let rid = rid_of(&url);
    let keys = Keys::require(&owner.ark_home()).expect("keys");
    net.log(&format!(
        "repo {url} rid-hex {} control {EXPLORER}/address/{} bulk {EXPLORER}/address/{}",
        hex::encode(rid),
        keys.control_address(&rid),
        keys.bulk_address(&rid)
    ));
    let work = net.dir.join(format!("{}-work", owner.name));
    owner.git(
        &net.dir,
        &["init", "-q", "-b", "main", &work.to_string_lossy()],
    );
    let c0 = owner.commit(&work, "hello.c", &hello_c("v0"), "feat: hello v0");
    owner.push_ok(&work, &["-q", &url, "main"]);
    (url, work, c0)
}

fn head(p: &Peer, dir: &Path) -> String {
    p.git(dir, &["rev-parse", "HEAD"])
}

fn has(ops: &[serde_json::Value], ty: &str, action: &str, author: &str) -> bool {
    ops.iter()
        .any(|o| o["type"] == ty && o["action"] == action && o["author"] == author)
}

#[test]
#[ignore = "live Rabdi: moves funds from the throwaway key to the bank (holds the fund lock)"]
fn fund_bank() {
    if !live::enabled() {
        return;
    }
    let amount: u128 = std::env::var("ARKIV_GIT_COLLAB_FUND_WEI")
        .map(|v| v.parse().expect("wei"))
        .unwrap_or(40 * MILLI);
    let ledger = root().join("funded-wei");
    let before: u128 = std::fs::read_to_string(&ledger)
        .map(|s| s.trim().parse().expect("ledger"))
        .unwrap_or(0);
    assert!(before + amount <= BANK_CAP_WEI, "over the 0.05 tGLM cap");
    let s = live::secrets();
    let rpc = live::connect(&s);
    let bank = Keys::require(&bank_home()).expect("bank: run keys init first");
    let _lock = fund_lock();
    live::transfer(&rpc, &s.key, bank.wallet().address(), U256::from(amount));
    std::fs::write(&ledger, (before + amount).to_string()).expect("ledger");
    eprintln!(
        "bank {}: {} tGLM (funded {} in total)",
        bank.wallet().address(),
        glm(live::balance(&rpc, bank.wallet().address())),
        glm(U256::from(before + amount))
    );
}

#[test]
#[ignore = "live Rabdi: returns the bank's leftover to the throwaway key (holds the fund lock)"]
fn sweep_bank() {
    if !live::enabled() {
        return;
    }
    let s = live::secrets();
    let rpc = live::connect(&s);
    let bank = Keys::require(&bank_home()).expect("bank");
    collect_leftovers(&rpc, bank.wallet().address());
    let _lock = fund_lock();
    sweep_signer(&rpc, &bank.wallet(), s.key.address());
    eprintln!(
        "bank {}: {} tGLM left",
        bank.wallet().address(),
        glm(live::balance(&rpc, bank.wallet().address()))
    );
}

/// Sweeps every keyed home of every run under the root (wallet, the repo addresses of every
/// repo the step log names, escrow identities and payout keys) into the bank, so a run that
/// stopped half way strands nothing.
fn collect_leftovers(rpc: &arkiv_git_store::RpcArkiv, bank: Address) {
    let log = std::fs::read_to_string(root().join("steps.log")).unwrap_or_default();
    let rids: Vec<arkiv_git_core::Rid> = log
        .split("rid-hex ")
        .skip(1)
        .filter_map(|r| arkiv_git_sync::ids::unhex32(r.split(' ').next()?).ok())
        .collect();
    let runs = std::fs::read_dir(root()).expect("root").flatten();
    let homes = runs
        .filter(|r| r.path().is_dir() && r.file_name() != "bank")
        .flat_map(|r| std::fs::read_dir(r.path()).into_iter().flatten().flatten())
        .map(|p| p.path().join(".arkiv-git"));
    for home in homes {
        if let Ok(keys) = Keys::require(&home) {
            live::sweep(rpc, &keys, &rids, bank);
        }
        let ehome = arkiv_git_escrow::keys::escrow_home(&home);
        if let Ok(keys) = Keys::require(&ehome) {
            live::sweep(rpc, &keys, &rids, bank);
            if let Ok(raw) = std::fs::read(ehome.join("keys/payout")) {
                let payout =
                    alloy_signer_local::PrivateKeySigner::from_bytes(&B256::from_slice(&raw));
                sweep_signer(rpc, &payout.expect("payout"), bank);
            }
        }
    }
}

/// Feature 1 and 3: a keyless clone, a non-delegate's patch, issue and review ops, a
/// merge, a fresh clone, a revert, old commits that still build, a forced rewind without
/// a program, a withholding view that gives `E_STALE_HEAD`, and a restore.
#[test]
#[ignore = "live Rabdi: spends testnet funds from the bank"]
fn live_collab_patch_merge_and_rollback() {
    Net::run("collab", |net| {
        let alice = net.peer("alice");
        let (ak, anid) = alice.keys_init();
        let (url, work, c0) = repo_with_main(net, &alice, "collab-demo");
        let rid = rid_of(&url);

        // Bob reads with no keys at all.
        let bob = net.peer("bob");
        let bw = bob.clone(&url, "bob-work");
        net.check(
            "1.keyless-clone",
            head(&bob, &bw) == c0 && !bob.ark_home().join("keys").exists(),
            &format!("bob cloned {c0} with no seed"),
        );
        let (_, bnid) = bob.keys_init();

        // Issue opened by bob, commented on by alice.
        let issue = bob.post(
            &url,
            "xyz.arkiv.issue",
            "create",
            None,
            r#"{"title":"greet the world"}"#,
        );
        alice.post(
            &url,
            "xyz.arkiv.issue",
            "comment",
            Some(&issue),
            r#"{"body":"agreed, patches welcome"}"#,
        );

        // Bob's commit travels through his own namespace; his patch op names it.
        bob.git(&bw, &["checkout", "-q", "-b", "fix"]);
        let cb = bob.commit(&bw, "hello.c", &hello_c("world"), "fix: greet the world");
        bob.push_ok(&bw, &["-q", "origin", "fix"]);
        let patch = bob.post(
            &url,
            "xyz.arkiv.patch",
            "create",
            None,
            &format!(
                r#"{{"title":"greet the world","head":"{cb}","base":"{c0}","branch":"refs/heads/fix","closes":"{issue}"}}"#
            ),
        );
        let ops = alice.ops(&url);
        let seen = ops
            .iter()
            .any(|o| o["id"] == patch && o["author"] == bnid && o["payload"]["head"] == cb);
        net.check(
            "1.op-list-sees-patch",
            seen,
            &format!("alice sees patch {patch} by bob at {cb}"),
        );

        // Alice moves main meanwhile, reviews, fetches bob's commit and merges it.
        let c1 = alice.commit(&work, "NOTES", "maintainer notes\n", "docs: notes");
        alice.push_ok(&work, &["-q", &url, "main"]);
        let review = alice.post(
            &url,
            "xyz.arkiv.review",
            "create",
            None,
            &format!(r#"{{"patch":"{patch}"}}"#),
        );
        alice.post(
            &url,
            "xyz.arkiv.review",
            "approve",
            Some(&review),
            &format!(r#"{{"patch":"{patch}","head":"{cb}"}}"#),
        );
        let bob_url = format!("{url}/{bnid}");
        alice.git(&work, &["fetch", "-q", &bob_url, "refs/heads/fix"]);
        let fetched = alice.git(&work, &["rev-parse", "FETCH_HEAD"]);
        net.check(
            "1.fetch-contributor-namespace",
            fetched == cb,
            &format!("FETCH_HEAD {fetched}"),
        );
        alice.git(
            &work,
            &[
                "merge",
                "--no-ff",
                "-q",
                "-m",
                "merge: greet the world",
                "FETCH_HEAD",
            ],
        );
        let m = head(&alice, &work);
        alice.push_ok(&work, &["-q", &url, "main"]);
        alice.post(
            &url,
            "xyz.arkiv.patch",
            "merged",
            Some(&patch),
            &format!(r#"{{"patch":"{patch}","head":"{m}","closes":"{issue}"}}"#),
        );
        alice.post(
            &url,
            "xyz.arkiv.issue",
            "close",
            Some(&issue),
            r#"{"reason":"fixed by bob's patch"}"#,
        );

        // Carol, a third home with no keys, gets the merge with bob's commit in it.
        let carol = net.peer("carol");
        let cw = carol.clone(&url, "carol-work");
        let ancestor = carol
            .git_out(&cw, &["merge-base", "--is-ancestor", &cb, "HEAD"])
            .status
            .success();
        let fsck = carol.git_out(&cw, &["fsck", "--full", "--strict"]);
        net.check(
            "1.fresh-clone-has-merge",
            head(&carol, &cw) == m && ancestor && fsck.status.success(),
            &format!("carol HEAD {m}, bob's {cb} is an ancestor, fsck clean"),
        );
        net.check(
            "1.merged-code-builds",
            build_and_run(&cw, &net.dir.join("carol-hello")) == "hello world",
            "carol's checkout compiles and prints hello world",
        );
        let ops = carol.ops(&url);
        let all = has(&ops, "xyz.arkiv.issue", "create", &bnid)
            && has(&ops, "xyz.arkiv.issue", "comment", &anid)
            && has(&ops, "xyz.arkiv.patch", "create", &bnid)
            && has(&ops, "xyz.arkiv.review", "create", &anid)
            && has(&ops, "xyz.arkiv.review", "approve", &anid)
            && has(&ops, "xyz.arkiv.patch", "merged", &anid)
            && has(&ops, "xyz.arkiv.issue", "close", &anid);
        net.check(
            "1.keyless-op-list",
            all && ops.len() == 7,
            &format!("carol verifies {} ops with the right authors", ops.len()),
        );

        // 3a: revert the merge; an existing clone and a fresh one both see it.
        alice.git(&work, &["revert", "--no-edit", "-m", "1", &m]);
        let r = head(&alice, &work);
        alice.push_ok(&work, &["-q", &url, "main"]);
        carol.git(&cw, &["pull", "-q", "--ff-only"]);
        let dave = net.peer("dave");
        let dw = dave.clone(&url, "dave-work");
        net.check(
            "3a.revert",
            head(&carol, &cw) == r && head(&dave, &dw) == r,
            &format!("carol pulled and dave cloned the revert {r}"),
        );

        // 3c: every earlier commit is in a fresh clone and still builds.
        let mut built = Vec::new();
        for (oid, want) in [
            (&c0, "hello v0"),
            (&cb, "hello world"),
            (&c1, "hello v0"),
            (&m, "hello world"),
            (&r, "hello v0"),
        ] {
            dave.git(&dw, &["checkout", "-q", "--detach", oid]);
            let got = build_and_run(&dw, &net.dir.join(format!("dave-{oid}")));
            built.push(got == want);
        }
        net.check(
            "3c.history-builds",
            built.iter().all(|b| *b),
            &format!("checked out and compiled c0, cb, c1, m, r: {built:?}"),
        );

        // 3b without a program: git refuses a plain rewind, a forced one is a new signed
        // state (seq + 1), so readers follow it and their marks do not object.
        alice.git(&work, &["reset", "-q", "--hard", &c0]);
        let plain = alice.push(&work, &[&url, "main"]);
        net.check(
            "3b.plain-rewind-refused",
            !plain.status.success() && stderr(&plain).contains("non-fast-forward"),
            "git refuses the non-fast-forward push before the helper sends anything",
        );
        alice.push_ok(&work, &["-q", "--force", &url, "main"]);
        carol.git(&cw, &["fetch", "-q"]);
        let tracked = carol.git(&cw, &["rev-parse", "origin/main"]);
        let erin = net.peer("erin");
        let ew = erin.clone(&url, "erin-work");
        net.check(
            "3b.forced-rewind-no-program",
            tracked == c0 && head(&erin, &ew) == c0,
            &format!("carol's fetch and erin's fresh clone follow main back to {c0}"),
        );

        // A view withholding alice's newest anchor fails closed for carol, who holds its mark.
        let code = stale_head(net, &carol.ark_home(), &rid, ak.control_address(&rid));
        net.check(
            "3b.withheld-anchor-stale-head",
            code.as_deref() == Some("E_STALE_HEAD"),
            &format!("sync over the withholding view: {code:?}"),
        );

        // Restore main to the revert so the demo repo keeps its full history.
        alice.git(&work, &["reset", "-q", "--hard", &r]);
        alice.push_ok(&work, &["-q", &url, "main"]);
        let frank = net.peer("frank");
        let fw = frank.clone(&url, "frank-work");
        net.check(
            "3b.restore",
            head(&frank, &fw) == r,
            &format!("fresh clone back at {r}"),
        );
        net.log(&format!(
            "collab evidence: url {url} alice {anid} bob {bnid} issue {issue} patch {patch} c0 {c0} cb {cb} c1 {c1} m {m} r {r}"
        ));
        log_anchors(net, &rid);
    });
}

/// Feature 2: a second delegate with threshold 2. Canonical refs need both delegates at
/// the same oid (design 3.8: no oid at the threshold omits the ref), and an identity
/// revision then needs both delegates too.
#[test]
#[ignore = "live Rabdi: spends testnet funds from the bank"]
fn live_quorum_two_delegates() {
    Net::run("quorum", |net| {
        let alice = net.peer("alice");
        let (_, anid) = alice.keys_init();
        let (url, work, c0) = repo_with_main(net, &alice, "quorum-demo");
        let bob = net.peer("bob");
        let (_, bnid) = bob.keys_init();

        let proposed = alice.write(&[
            "id",
            "propose",
            &url,
            "--add-delegate",
            &bnid,
            "--threshold",
            "2",
        ]);
        net.check(
            "2.add-delegate",
            proposed.contains("threshold:      2")
                && proposed.contains(&anid)
                && proposed.contains(&bnid),
            "revision 1 adds bob with threshold 2, effective on alice's own acceptance",
        );

        // Only alice voted for c0: no quorum, so a fresh clone gets no main.
        let carol = net.peer("carol");
        let lone = carol.git_out(
            &net.dir,
            &[
                "clone",
                "-q",
                &url,
                &net.dir.join("carol-work").to_string_lossy(),
            ],
        );
        let empty = carol
            .git_out(&net.dir.join("carol-work"), &["rev-parse", "HEAD"])
            .status
            .success();
        net.check(
            "2.no-quorum-one-vote",
            stderr(&lone).contains("no quorum for refs/heads/main") && !empty,
            "one of two delegates at c0: main omitted with a warning",
        );

        // Bob takes c0 from alice's namespace and pushes it: two votes, main is back.
        let bw = bob.clone(&format!("{url}/{anid}"), "bob-work");
        bob.push_ok(&bw, &["-q", &url, "main"]);
        let dave = net.peer("dave");
        let dw = dave.clone(&url, "dave-work");
        net.check(
            "2.quorum-both-at-c0",
            head(&dave, &dw) == c0,
            &format!("both delegates at {c0}"),
        );

        // Alice alone moves to c1: the canonical head does not move.
        let c1 = alice.commit(&work, "NOTES", "v1\n", "docs: v1");
        alice.push_ok(&work, &["-q", &url, "main"]);
        let fetched = dave.git_out(&dw, &["fetch"]);
        let tracked = dave.git(&dw, &["rev-parse", "origin/main"]);
        let erin = net.peer("erin");
        let lone = erin.git_out(
            &net.dir,
            &[
                "clone",
                "-q",
                &url,
                &net.dir.join("erin-work").to_string_lossy(),
            ],
        );
        net.check(
            "2.single-delegate-push-does-not-move-head",
            stderr(&fetched).contains("no quorum for refs/heads/main")
                && tracked == c0
                && stderr(&lone).contains("no quorum"),
            &format!("dave stays at {c0}, erin's fresh clone gets no main"),
        );

        // Bob agrees: c1 becomes canonical.
        bob.git(
            &bw,
            &["fetch", "-q", &format!("{url}/{anid}"), "refs/heads/main"],
        );
        bob.git(&bw, &["merge", "-q", "--ff-only", "FETCH_HEAD"]);
        bob.push_ok(&bw, &["-q", &url, "main"]);
        let frank = net.peer("frank");
        let fw = frank.clone(&url, "frank-work");
        dave.git(&dw, &["pull", "-q", "--ff-only"]);
        net.check(
            "2.quorum-both-at-c1",
            head(&frank, &fw) == c1 && head(&dave, &dw) == c1,
            &format!("both delegates at {c1}: fresh clone and dave's pull get it"),
        );

        // An identity change now needs both delegates (maj of 2 is 2).
        let p = alice.write(&["id", "propose", &url, "--threshold", "1"]);
        let rev = field(&p, "proposed revision");
        let shown = carol.arkiv(&["id", "show", &url]);
        let pending = shown.contains("threshold:      2") && shown.contains("revision:       1");
        let accepted = bob.write(&["id", "accept", &url, &rev]);
        net.check(
            "2.identity-needs-both",
            pending
                && accepted.contains("threshold:      1")
                && accepted.contains("revision:       2"),
            &format!("revision {rev} pending on alice alone, effective after bob accepts"),
        );
        net.log(&format!(
            "quorum evidence: url {url} alice {anid} bob {bnid} c0 {c0} c1 {c1} rev2 {rev}"
        ));
        log_anchors(net, &rid_of(&url));
    });
}

/// Feature 4 and 3b with a program: branch protection refuses direct pushes, ignores a
/// contributor's own approval and namespace push, accepts a delegate-approved patch, and
/// refuses a rewind even when the old head has an approved patch, also from a modified
/// client that skips the local check.
#[test]
#[ignore = "live Rabdi: spends testnet funds from the bank"]
fn live_program_branch_protection() {
    Net::run("program", |net| {
        let alice = net.peer("alice");
        let (_, anid) = alice.keys_init();
        let (url, work, c0) = repo_with_main(net, &alice, "guarded-demo");
        let pinned = alice.write(&[
            "program",
            "pin",
            "--repo",
            &url,
            "--file",
            &program("branch-protection"),
        ]);
        net.check(
            "4.pin",
            pinned.contains("program:        rhai/1"),
            "branch-protection.rhai pinned by hash in identity revision 1",
        );

        let c1 = alice.commit(&work, "NOTES", "direct\n", "docs: direct");
        let direct = alice.push(&work, &[&url, "main"]);
        let status = alice.arkiv(&["status", &url]);
        net.check(
            "4.direct-push-refused",
            !direct.status.success()
                && stderr(&direct).contains("E_PROGRAM_REJECTED: main needs a patch")
                && status.contains("delegate: seq 0"),
            &format!("direct push of {c1} refused locally, nothing anchored"),
        );

        // Bob (not a delegate) pushes main to his namespace and approves his own patch.
        let bob = net.peer("bob");
        let (_, bnid) = bob.keys_init();
        let bw = bob.clone(&url, "bob-work");
        let cb = bob.commit(&bw, "hello.c", &hello_c("world"), "fix: greet the world");
        bob.push_ok(&bw, &["-q", "origin", "main"]);
        let carol = net.peer("carol");
        let cw = carol.clone(&url, "carol-work");
        net.check(
            "4.contributor-namespace-push-not-canonical",
            head(&carol, &cw) == c0,
            &format!("bob's main {cb} is not an event; canonical stays {c0}"),
        );
        let patch = bob.post(
            &url,
            "xyz.arkiv.patch",
            "create",
            None,
            &format!(r#"{{"title":"greet the world","head":"{cb}"}}"#),
        );
        let own = bob.post(
            &url,
            "xyz.arkiv.review",
            "create",
            None,
            &format!(r#"{{"patch":"{patch}"}}"#),
        );
        bob.post(
            &url,
            "xyz.arkiv.review",
            "approve",
            Some(&own),
            &format!(r#"{{"patch":"{patch}","head":"{cb}"}}"#),
        );
        alice.git(&work, &["reset", "-q", "--hard", &c0]);
        alice.git(
            &work,
            &["fetch", "-q", &format!("{url}/{bnid}"), "refs/heads/main"],
        );
        alice.git(&work, &["merge", "-q", "--ff-only", "FETCH_HEAD"]);
        let self_approved = alice.push(&work, &[&url, "main"]);
        net.check(
            "4.contributor-self-approval-ignored",
            !self_approved.status.success()
                && stderr(&self_approved).contains("E_PROGRAM_REJECTED"),
            "bob's approval of his own patch does not count",
        );

        // A delegate approval at that exact head lets it through.
        let review = alice.post(
            &url,
            "xyz.arkiv.review",
            "create",
            None,
            &format!(r#"{{"patch":"{patch}"}}"#),
        );
        alice.post(
            &url,
            "xyz.arkiv.review",
            "approve",
            Some(&review),
            &format!(r#"{{"patch":"{patch}","head":"{cb}"}}"#),
        );
        alice.push_ok(&work, &["-q", &url, "main"]);
        let dave = net.peer("dave");
        let dw = dave.clone(&url, "dave-work");
        let report = dave.arkiv(&["program", "status", "--repo", &url]);
        net.check(
            "4.approved-patch-accepted",
            head(&dave, &dw) == cb && report.contains(&format!("head: refs/heads/main {cb}")),
            &format!("delegate-approved patch at {cb} accepted"),
        );

        // 3b with the program: a rewind is refused, even with an approved patch at c0.
        alice.git(&work, &["reset", "-q", "--hard", &c0]);
        let rewind = alice.push(&work, &["--force", &url, "main"]);
        let p0 = bob.post(
            &url,
            "xyz.arkiv.patch",
            "create",
            None,
            &format!(r#"{{"title":"roll back","head":"{c0}"}}"#),
        );
        alice.post(
            &url,
            "xyz.arkiv.review",
            "approve",
            Some(&review),
            &format!(r#"{{"patch":"{p0}","head":"{c0}"}}"#),
        );
        let approved_rewind = alice.push(&work, &["--force", &url, "main"]);
        net.check(
            "3b.rewind-refused-with-program",
            stderr(&rewind).contains("E_PROGRAM_REJECTED")
                && stderr(&approved_rewind)
                    .contains("E_PROGRAM_REJECTED: not a fast-forward of the accepted head"),
            "forced rewind refused; with an approved patch at c0 it is still not a fast-forward",
        );

        modified_client_ignored(net, &alice, &url, &work, &c0, &cb);
        net.log(&format!(
            "program evidence: url {url} alice {anid} bob {bnid} patch {patch} p0 {p0} c0 {c0} cb {cb}"
        ));
    });
}

/// A modified client skips the local check: the push lands, every reader ignores it.
fn modified_client_ignored(net: &Net, alice: &Peer, url: &str, work: &Path, c0: &str, cb: &str) {
    alice.float(WRITE_FLOAT, || forced_push(net, alice, url, work));
    let erin = net.peer("erin");
    let ew = erin.clone(url, "erin-work");
    let report = erin.arkiv(&["program", "status", "--repo", url]);
    let status = erin.arkiv(&["status", url]);
    net.check(
        "4.modified-client-ignored",
        head(&erin, &ew) == cb
            && status.contains("delegate: seq 2")
            && report.contains(&format!("rejected: refs/heads/main {cb}..{c0}")),
        &format!("anchored rewind to {c0} is rejected by the fold; clone stays at {cb}"),
    );
    log_anchors(net, &rid_of(url));
}

/// Resumes [`live_program_branch_protection`] at its last step against a run that stopped
/// there (`ARKIV_GIT_COLLAB_RESUME=<run dir> ARKIV_GIT_COLLAB_RESUME_ARGS="<url> <c0> <cb>"`),
/// reusing alice's home and work tree, whose main is already reset to c0.
#[test]
#[ignore = "live Rabdi: spends testnet funds from the bank"]
fn live_program_modified_client_resume() {
    Net::run("program-resume", |net| {
        let dir = std::path::PathBuf::from(std::env::var("ARKIV_GIT_COLLAB_RESUME").expect("dir"));
        let args = std::env::var("ARKIV_GIT_COLLAB_RESUME_ARGS").expect("args");
        let [url, c0, cb]: [&str; 3] = args
            .split_whitespace()
            .collect::<Vec<_>>()
            .try_into()
            .expect("url c0 cb");
        let alice = net.existing_peer("alice", dir.join("alice"));
        modified_client_ignored(net, &alice, url, &dir.join("alice-work"), c0, cb);
    });
}

/// Feature 5: a bounty program, an escrow agent with its own home and payout key, a
/// maintainer's deposit checked by tx, a payout to the contributor checked by balance and
/// tx, no pay before the merge, no double pay, a clean keyless audit, and a flagged audit
/// after the payout key moves funds outside any settlement.
///
/// Ordered for a small budget: the only push is the merge itself (a push's up-front check
/// is the largest float), and the appointment and deposit come after it. Bob's commit
/// reaches alice out of band (`git fetch` from his work tree); the on-Arkiv contributor
/// path is covered by `live_collab_patch_merge_and_rollback`.
#[test]
#[ignore = "live Rabdi: spends testnet funds from the bank"]
fn live_escrow_bounty() {
    const BOUNTY: u128 = 2 * MILLI;
    Net::run("escrow", |net| {
        let alice = net.peer("alice");
        let (ak, anid) = alice.keys_init();
        let created = alice.write(&[
            "init",
            "--name",
            "bounty-demo",
            "--description",
            "live test",
        ]);
        let url = field(&created, "url:");
        let rid = rid_of(&url);
        net.log(&format!(
            "repo {url} rid-hex {} control {EXPLORER}/address/{}",
            hex::encode(rid),
            ak.control_address(&rid)
        ));
        alice.write(&[
            "program",
            "pin",
            "--repo",
            &url,
            "--file",
            &program("bounty"),
        ]);
        let issue = alice.post(
            &url,
            "xyz.arkiv.issue",
            "create",
            None,
            &format!(r#"{{"title":"crash on empty input","bounty":{BOUNTY}}}"#),
        );

        // Bob writes the fix and posts the patch op; alice takes the commit from him.
        let bob = net.peer("bob");
        let (bk, bnid) = bob.keys_init();
        let bw = net.dir.join("bob-work");
        bob.git(
            &net.dir,
            &["init", "-q", "-b", "main", &bw.to_string_lossy()],
        );
        let cb = bob.commit(&bw, "hello.c", &hello_c("fixed"), "fix: empty input");
        let patch = bob.post(
            &url,
            "xyz.arkiv.patch",
            "create",
            None,
            &format!(r#"{{"title":"fix crash","head":"{cb}"}}"#),
        );
        let work = net.dir.join("alice-work");
        alice.git(
            &net.dir,
            &["init", "-q", "-b", "main", &work.to_string_lossy()],
        );
        alice.git(&work, &["fetch", "-q", &bw.to_string_lossy(), "main"]);
        alice.git(&work, &["reset", "-q", "--hard", "FETCH_HEAD"]);
        alice.post(
            &url,
            "xyz.arkiv.patch",
            "merged",
            Some(&patch),
            &format!(r#"{{"patch":"{patch}","closes":"{issue}"}}"#),
        );
        alice.push_ok(&work, &["-q", &url, "main"]);
        let report = alice.arkiv(&["program", "status", "--repo", &url]);
        net.check(
            "5.program-owes-bounty",
            report.contains(&format!("bounty:{issue}"))
                && report.contains(&format!("head: refs/heads/main {cb}")),
            "the merge push is accepted and bounty.rhai emits the pay obligation",
        );

        // The escrow agent: its own home, identity and payout key; appointed, then funded.
        let agent = net.peer("agent");
        let esc = agent.arkiv(&["escrow", "init"]);
        let (eid, eaddr) = (field(&esc, "escrow:"), field(&esc, "address:"));
        let ag = Agent::open(net, agent, rid);
        let escrow = ag.address;
        alice.write(&[
            "escrow",
            "appoint",
            "--repo",
            &url,
            "--escrow",
            &eid,
            "--address",
            &eaddr,
        ]);
        let early = ag.run(&url, WRITE_FLOAT);
        net.check(
            "5.no-pay-unfunded",
            !early.contains("paid"),
            &format!("agent round before any deposit: {:?}", early.trim()),
        );

        // The maintainer deposits the bounty; the deposit is checked against its chain tx.
        let purpose = format!("bounty:{issue}");
        let amount = BOUNTY.to_string();
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
        let tx1 = alice
            .float(WRITE_FLOAT + BOUNTY, || alice.arkiv(&deposit))
            .trim()
            .to_owned();
        let d1 = net.tx(&tx1).expect("deposit tx");
        net.check(
            "5.deposit-verified",
            d1.from == ak.wallet().address()
                && d1.to == Some(escrow)
                && d1.value == U256::from(BOUNTY),
            &format!(
                "deposit {EXPLORER}/tx/{tx1}: {} wei alice -> escrow",
                d1.value
            ),
        );
        let paid = payout_and_audit(net, &ag, &url, &purpose, bk.control_address(&rid), BOUNTY);
        net.log(&format!(
            "escrow evidence: url {url} alice {anid} bob {bnid} escrow {eid} address {EXPLORER}/address/{eaddr} issue {issue} patch {patch} cb {cb} deposit {tx1} payout {paid}"
        ));
    });
}

/// The escrow agent of a test: its peer (home), identity keys and payout key.
struct Agent<'n> {
    peer: Peer<'n>,
    keys: Keys,
    payout: alloy_signer_local::PrivateKeySigner,
    address: Address,
    rid: arkiv_git_core::Rid,
}

impl<'n> Agent<'n> {
    /// Reads the keys `escrow init` wrote under `peer`'s home; sweeps the identity later.
    fn open(net: &Net, peer: Peer<'n>, rid: arkiv_git_core::Rid) -> Self {
        let ehome = arkiv_git_escrow::keys::escrow_home(&peer.ark_home());
        let keys = Keys::require(&ehome).expect("escrow identity");
        let raw = std::fs::read(ehome.join("keys/payout")).expect("payout key");
        let payout = alloy_signer_local::PrivateKeySigner::from_bytes(&B256::from_slice(&raw))
            .expect("payout");
        for s in [keys.wallet(), keys.control(&rid), keys.bulk(&rid)] {
            net.sweep_later(s);
        }
        let address = payout.address();
        Agent {
            peer,
            keys,
            payout,
            address,
            rid,
        }
    }

    /// One `escrow run --once` round with `float` lent to the identity; returns stdout.
    fn run(&self, url: &str, float: u128) -> String {
        let net = self.peer_net();
        let out = net.float(
            &self.keys,
            || vec![self.rid],
            float,
            || {
                self.peer
                    .arkiv_out(&["escrow", "run", "--repo", url, "--once"])
            },
        );
        assert!(out.status.success(), "escrow run: {}", stderr(&out));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn peer_net(&self) -> &'n Net {
        self.peer.net()
    }
}

/// From a funded obligation on: the payout lands at `to` (balance delta and tx), then
/// [`after_payout`]. Returns the payout tx hash.
fn payout_and_audit(
    net: &Net,
    ag: &Agent,
    url: &str,
    purpose: &str,
    to: Address,
    amount: u128,
) -> String {
    net.fund(ag.address, MILLI / 4); // gas float for the payout transfer
    let before = net.balance(to);
    let ran = ag.run(url, AGENT_FLOAT);
    let delta = net.balance(to).saturating_sub(before);
    let paid = ag
        .peer
        .ops(url)
        .iter()
        .find(|o| o["type"] == "xyz.arkiv.escrow" && o["action"] == "paid")
        .map(|o| o["payload"]["tx"].as_str().unwrap_or("").to_owned())
        .unwrap_or_default();
    let tx_ok = net
        .tx(&paid)
        .is_some_and(|t| t.from == ag.address && t.to == Some(to) && t.value == U256::from(amount));
    net.check(
        "5.payout-on-chain",
        ran.contains(&format!("paid {purpose}")) && delta == U256::from(amount) && tx_ok,
        &format!("bob's control address {to} gained {delta} wei, payout {EXPLORER}/tx/{paid}"),
    );
    after_payout(net, ag, url, to);
    paid
}

/// A second round pays nothing (and needs no float, since it writes nothing), a keyless
/// audit is clean, and an unbooked payout-key transfer is flagged.
fn after_payout(net: &Net, ag: &Agent, url: &str, to: Address) {
    let before = net.balance(to);
    let again = ag.run(url, 0);
    net.check(
        "5.no-double-pay",
        !again.contains("paid") && net.balance(to) == before,
        &format!("a second round pays nothing: {:?}", again.trim()),
    );
    let auditor = net.peer("auditor");
    let clean = auditor.arkiv_out(&["escrow", "audit", "--repo", url]);
    let text = String::from_utf8_lossy(&clean.stdout).into_owned();
    net.check(
        "5.audit-clean",
        clean.status.success() && !text.contains("FLAG") && text.contains("settled:  1 op(s)"),
        &format!("audit: {}", text.trim().replace('\n', " | ")),
    );
    sweep_signer(&net.rpc, &ag.payout, net.bank.wallet().address());
    let caught = auditor.arkiv_out(&["escrow", "audit", "--repo", url]);
    let text = String::from_utf8_lossy(&caught.stdout).into_owned();
    net.check(
        "5.audit-flags-unbooked-transfer",
        caught.status.code() == Some(1) && text.contains("FLAG"),
        &format!(
            "after an unbooked payout-key transfer: {}",
            text.trim().replace('\n', " | ")
        ),
    );
}

/// Resumes [`live_escrow_bounty`] after its deposit, against a run that stopped there
/// (`ARKIV_GIT_COLLAB_RESUME=<run dir> ARKIV_GIT_COLLAB_RESUME_ARGS="<url> <issue id>"`),
/// reusing the homes of bob and the agent. With `ARKIV_GIT_COLLAB_RESUME_STAGE=after-payout`
/// it starts after a payout that already landed.
#[test]
#[ignore = "live Rabdi: spends testnet funds from the bank"]
fn live_escrow_resume() {
    Net::run("escrow-resume", |net| {
        let dir = std::path::PathBuf::from(std::env::var("ARKIV_GIT_COLLAB_RESUME").expect("dir"));
        let args = std::env::var("ARKIV_GIT_COLLAB_RESUME_ARGS").expect("args");
        let [url, issue]: [&str; 2] = args
            .split_whitespace()
            .collect::<Vec<_>>()
            .try_into()
            .expect("url issue");
        let rid = rid_of(url);
        let bob = net.existing_peer("bob", dir.join("bob"));
        let ag = Agent::open(net, net.existing_peer("agent", dir.join("agent")), rid);
        let bob_keys = Keys::require(&bob.ark_home()).expect("bob");
        let to = bob_keys.control_address(&rid);
        if std::env::var("ARKIV_GIT_COLLAB_RESUME_STAGE").as_deref() == Ok("after-payout") {
            after_payout(net, &ag, url, to);
        } else {
            let paid = payout_and_audit(net, &ag, url, &format!("bounty:{issue}"), to, 2 * MILLI);
            net.log(&format!(
                "escrow resume evidence: url {url} issue {issue} payout {paid}"
            ));
        }
        // Bob never cloned, so the final sweep does not know his repo; return the bounty.
        live::sweep(&net.rpc, &bob_keys, &[rid], net.bank.wallet().address());
    });
}

/// Design 8.5 step 4 against `home`'s cache: an honest sync sets the mark at the newest
/// anchor of `control`, then a view withholding that anchor must fail; returns the code.
fn stale_head(
    net: &Net,
    home: &Path,
    rid: &arkiv_git_core::Rid,
    control: Address,
) -> Option<String> {
    let newest = entities(&net.rpc, rid, Kind::Anchor)
        .into_iter()
        .filter(|o| Address::from(o.creator) == control)
        .max_by_key(seq_of)
        .expect("an anchor");
    let repo = Repo::open(home, rid).expect("repo");
    let want = Want::default();
    arkiv_git_cli::block_on(sync(&net.rpc, &repo, &want))
        .expect("rt")
        .expect("honest sync");
    let truth = arkiv_git_cli::block_on(arkiv_git_store::Arkiv::entity_nonce(&net.rpc, control))
        .expect("rt")
        .expect("nonce");
    let hiding = Hiding {
        inner: &net.rpc,
        key: B256::from(newest.key),
        control,
        nonce: truth - 1,
    };
    net.log(&format!(
        "withholding anchor seq {} key 0x{}",
        seq_of(&newest),
        hex::encode(newest.key)
    ));
    let stale = arkiv_git_cli::block_on(sync(&hiding, &repo, &want)).expect("rt");
    stale.err().map(|e| e.code().as_str().to_owned())
}

/// A modified client: pushes `main` through the sync library without the helper's
/// program check, so the signed refs and anchor land on Arkiv anyway.
fn forced_push(net: &Net, p: &Peer, url: &str, work: &Path) {
    let home = p.ark_home();
    let rid = rid_of(url);
    let lifetime: u64 = LIFETIME.parse().expect("blocks");
    arkiv_git_cli::block_on(async {
        let mut cfg = Config::new(home.clone());
        cfg.data_lifetime = lifetime;
        cfg.control_lifetime = lifetime;
        let keys = Keys::require(&home).expect("keys");
        let repo = Repo::open(&home, &rid).expect("repo");
        let want = Want {
            must: [keys.nid()].into(),
            ..Want::default()
        };
        let view = sync(&net.rpc, &repo, &want).await.expect("sync");
        let main = "refs/heads/main".to_owned();
        let up = [Update {
            src: Some(main.clone()),
            dst: main,
        }];
        let push = Push::prepare(
            &net.rpc,
            &repo,
            &cfg,
            &keys,
            view,
            &Git::work(Some(work)),
            &up,
        );
        push.expect("prepare").run().await.expect("forced push");
    })
    .expect("runtime");
    net.log("modified client pushed main without the program check");
}

/// Logs every anchor of `rid` (seq, creator, entity key) as evidence.
fn log_anchors(net: &Net, rid: &arkiv_git_core::Rid) {
    for o in entities(&net.rpc, rid, Kind::Anchor) {
        net.log(&format!(
            "anchor seq {} creator {} key 0x{}",
            seq_of(&o),
            Address::from(o.creator),
            hex::encode(o.key)
        ));
    }
}

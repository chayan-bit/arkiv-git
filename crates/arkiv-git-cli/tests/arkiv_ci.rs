//! arkiv-git developed on Arkiv (docs/p3/arkiv-ci.md), on MemArkiv: the delegate re-pins
//! `ci-gated.rhai` over `branch-protection.rhai`; a contributor opens patches with
//! `scripts/arkiv-patch.sh`; `scripts/arkiv-ci.sh --once` attests them; `main` moves only
//! through `scripts/arkiv-merge.sh` once a patch head has an approval and a runner pass.

mod common;

use std::path::Path;
use std::process::Output;

use common::ci::*;
use common::e2e::*;
use common::programs::*;

/// The delegate fast-forwards its main to bob's branch and pushes it directly.
fn direct_push(alice: &User, work: &Path, url: &str, bob_nid: &str, branch: &str) -> Output {
    let peer = format!("{url}/{bob_nid}");
    alice.git(
        work,
        &["fetch", "-q", &peer, &format!("refs/heads/{branch}")],
    );
    alice.git(work, &["reset", "-q", "--hard", "FETCH_HEAD"]);
    alice.run("git", work, &["push", url, "main"])
}

#[test]
fn main_moves_only_with_an_approval_and_a_runner_ci_pass() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (root, log) = (tmp.path(), tmp.path().join("chain.log"));
    let alice = user(root, "alice", &log);
    let (url, work, _) = repo_with_main(&alice, root, "selfhost");
    let pin = |file: &str| alice.arkiv(&["program", "pin", "--repo", &url, "--file", file]);
    pin(&program("branch-protection")); // the live repo's program today
    // Under branch-protection the delegate merges its own patch with no CI (need 0).
    alice.git(&work, &["checkout", "-q", "-b", "feat/ci"]);
    let c0 = commit(&alice, &work, "ci scripts\n");
    let p0 = patch(&alice, &work, &url, "feat/ci");
    let merged = ok(merge(&alice, &work, &url, &p0));
    assert!(merged.contains(&format!("main -> {c0}")), "{merged}");
    alice.git(&work, &["checkout", "-q", "main"]);
    let ci = user(root, "ci", &log);
    let ci_hex = hex_id(&nid(&ci));
    pin(&gated_for(&ci_hex, root).to_string_lossy()); // a plain re-pin replaces it
    let status = || alice.arkiv(&["program", "status", "--repo", &url]);
    assert!(status().contains(r#""need":1"#), "{}", status());

    let bob = user(root, "bob", &log);
    let bob_nid = nid(&bob);
    let bob_work = root.join("bob-work");
    bob.git(root, &["clone", "-q", &url, &bob_work.to_string_lossy()]);
    bob.git(&bob_work, &["checkout", "-q", "-b", "feat/one"]);
    let c1 = commit(&bob, &bob_work, "one\n");
    let p1 = patch(&bob, &bob_work, &url, "feat/one");
    assert_eq!(
        patch(&bob, &bob_work, &url, "feat/one"),
        p1,
        "same head, same patch"
    );

    let err = refused(direct_push(&alice, &work, &url, &bob_nid, "feat/one"));
    assert!(
        err.contains("E_PROGRAM_REJECTED") && err.contains("approval"),
        "{err}"
    );
    approve(&alice, &url, &p1, &c1);
    let err = refused(alice.run("git", &work, &["push", &url, "main"]));
    assert!(err.contains("CI") && !err.contains("approval"), "{err}");
    let err = refused(merge(&alice, &work, &url, &p1));
    assert!(err.contains("CI has 0 of 1 passes"), "{err}");

    let ran = run_ci(&ci, &url, "true");
    assert!(ran.contains(&format!("arkiv-ci: pass {c1}")), "{ran}");
    assert!(
        ran.contains(&format!("arkiv-ci: skip {c0}, already in main")),
        "{ran}"
    );
    let again = run_ci(&ci, &url, "false");
    assert!(
        !again.contains("arkiv-ci: patch"),
        "attested heads are not re-run: {again}"
    );
    let merged = ok(merge(&alice, &work, &url, &p1));
    assert!(merged.contains(&format!("main -> {c1}")), "{merged}");
    assert!(status().contains(&format!("head: refs/heads/main {c1}")));

    // A failing suite blocks the next patch, even with an approval and a direct push.
    bob.git(&bob_work, &["checkout", "-q", "-b", "feat/two"]);
    let c2 = commit(&bob, &bob_work, "two\n");
    let p2 = patch(&bob, &bob_work, &url, "feat/two");
    let ran = run_ci(&ci, &url, "false");
    assert!(ran.contains(&format!("arkiv-ci: fail {c2}")), "{ran}");
    let err = refused(merge(&alice, &work, &url, &p2));
    assert!(err.contains("CI has 0 of 1 passes"), "{err}");
    approve(&alice, &url, &p2, &c2);
    let err = refused(direct_push(&alice, &work, &url, &bob_nid, "feat/two"));
    assert!(
        err.contains("E_PROGRAM_REJECTED") && err.contains("CI"),
        "{err}"
    );
    let st = status();
    assert!(
        st.contains(&format!(r#""{c2}":{{"{ci_hex}":"fail"}}"#)),
        "{st}"
    );

    // The fix revises the same patch; its new head gets a fresh run and merges.
    let c3 = commit(&bob, &bob_work, "two, fixed\n");
    assert_eq!(patch(&bob, &bob_work, &url, "feat/two"), p2, "a revise");
    let ran = run_ci(&ci, &url, "true");
    assert!(ran.contains(&format!("arkiv-ci: pass {c3}")), "{ran}");
    let merged = ok(merge(&alice, &work, &url, &p2));
    assert!(merged.contains(&format!("main -> {c3}")), "{merged}");
    assert!(status().contains(&format!("head: refs/heads/main {c3}")));
    mirror(&alice, root, &url, &c1, &c3);
}

/// `mirror-to-github.sh` fast-forwards a stand-in GitHub remote and refuses a diverged one.
fn mirror(alice: &User, root: &Path, url: &str, behind: &str, canonical: &str) {
    let gh = root.join("github.git").to_string_lossy().into_owned();
    let clone = root.join("mirror-clone");
    alice.git(root, &["init", "-q", "--bare", &gh]);
    alice.git(root, &["clone", "-q", url, &clone.to_string_lossy()]);
    alice.git(
        &clone,
        &["push", "-q", &gh, &format!("{behind}:refs/heads/main")],
    );
    let out = ok(script(
        alice,
        &clone,
        "mirror-to-github.sh",
        &[url, &gh],
        "",
    ));
    assert!(out.contains(&format!("mirrored {canonical}")), "{out}");
    assert_eq!(
        alice.git(root, &["--git-dir", &gh, "rev-parse", "main"]),
        canonical
    );

    commit(alice, &clone, "github-only\n");
    alice.git(&clone, &["push", "-q", &gh, "HEAD:refs/heads/main"]);
    let err = refused(script(
        alice,
        &clone,
        "mirror-to-github.sh",
        &[url, &gh],
        "",
    ));
    assert!(err.contains("is not an ancestor"), "{err}");
}

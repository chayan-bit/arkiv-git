//! Signed, CI-attested release records (docs/p3/releases.md), on MemArkiv: a commit lands
//! through patch, CI and merge under `ci-gated.rhai`; the delegate publishes a release of
//! two local artifacts; `release verify` refuses until the runner attests them with
//! `scripts/chart-release-attest.sh`, then downloads them from a `file://` mirror, and a
//! tampered mirror makes it fail.

mod common;

use std::path::Path;

use common::ci::*;
use common::e2e::*;
use common::programs::*;

const BIN: &str = env!("CARGO_BIN_EXE_chart");

fn verify(u: &User, url: &str, dl: &Path) -> std::process::Output {
    let dl = dl.to_string_lossy();
    let args = [
        "release",
        "verify",
        "--repo",
        url,
        "--tag",
        "v1",
        "--download-dir",
        &dl,
    ];
    u.run(BIN, &u.home, &args)
}

#[test]
fn a_release_verifies_only_after_ci_and_artifact_attestations() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (root, log) = (tmp.path(), tmp.path().join("chain.log"));
    let alice = user(root, "alice", &log);
    let (url, work, _) = repo_with_main(&alice, root, "rel");
    let ci = user(root, "ci", &log);
    let gated = gated_for(&hex_id(&nid(&ci)), root);
    alice.arkiv(&[
        "program",
        "pin",
        "--repo",
        &url,
        "--file",
        &gated.to_string_lossy(),
    ]);

    alice.git(&work, &["checkout", "-q", "-b", "feat/rel"]);
    let c1 = commit(&alice, &work, "release me\n");
    let p1 = patch(&alice, &work, &url, "feat/rel");
    approve(&alice, &url, &p1, &c1);
    run_ci(&ci, &url, "true");
    ok(merge(&alice, &work, &url, &p1));

    let (dist, mirror, dl) = (root.join("dist"), root.join("mirror"), root.join("dl"));
    for d in [&dist, &mirror] {
        std::fs::create_dir_all(d).expect("dir");
    }
    for (name, bytes) in [("a.tar.gz", b"aaa".as_slice()), ("b.tar.gz", b"bbbb")] {
        write(&dist, name, bytes);
        write(&mirror, name, bytes);
    }
    let art = |n: &str, t: &str| format!("{n}={}@{t}", dist.join(n).display());
    let loc = |n: &str| format!("{n}=file://{}", mirror.join(n).display());
    let (a, b) = (
        art("a.tar.gz", "aarch64-apple-darwin"),
        art("b.tar.gz", "x86_64"),
    );
    let (la, lb) = (loc("a.tar.gz"), loc("b.tar.gz"));
    let id = alice.arkiv(&[
        "release",
        "publish",
        "--repo",
        &url,
        "--tag",
        "v1",
        "--commit",
        &c1,
        "--artifact",
        &a,
        "--artifact",
        &b,
        "--location",
        &la,
        "--location",
        &lb,
    ]);
    assert_eq!(id.trim().len(), 64, "{id}");

    let err = refused(verify(&alice, &url, &dl));
    assert!(
        err.contains("not valid") && err.contains("artifact a.tar.gz"),
        "{err}"
    );

    let attest = || {
        ok(script(
            &ci,
            &ci.home,
            "chart-release-attest.sh",
            &["--repo", &url, "--tag", "v1"],
            "",
        ))
    };
    let out = attest();
    assert!(
        out.contains("attested a.tar.gz") && out.contains("attested b.tar.gz"),
        "{out}"
    );
    assert!(
        attest().contains("already attested a.tar.gz"),
        "attests once"
    );

    let out = ok(verify(&alice, &url, &dl));
    assert!(
        out.contains("OK a.tar.gz") && out.contains("OK b.tar.gz"),
        "{out}"
    );
    assert_eq!(
        std::fs::read(dl.join("b.tar.gz")).expect("downloaded"),
        b"bbbb"
    );

    write(&mirror, "a.tar.gz", b"evil");
    let err = refused(verify(&alice, &url, &dl));
    assert!(
        err.contains("MISMATCH") && err.contains("a.tar.gz"),
        "{err}"
    );
}

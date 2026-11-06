//! Repo programs on private repos (docs/p3/programs.md, "Private repos"): ops and the pin go
//! through the `Sealed` adapter, so members fold them and outsiders see no plaintext rid.

mod common;

use arkiv_git_cli::backend::MEM_CHAIN_ID;
use arkiv_git_core::{AttrValue, Kind};
use arkiv_git_store::fake::FileMemArkiv;
use arkiv_git_sync::ids::parse_url;
use arkiv_git_sync::observe;
use common::e2e::*;
use common::programs::*;

#[test]
fn private_repo_enforces_its_pinned_program_for_members() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (root, log) = (tmp.path(), tmp.path().join("chain.log"));
    let (alice, bob) = (user(root, "alice", &log), user(root, "bob", &log));
    let url = field(
        &alice.arkiv(&["init", "--private", "--name", "vault"]),
        "url:",
    )
    .to_owned();
    let kp = field(&bob.arkiv(&["member", "keypackage"]), "key package:").to_owned();
    alice.arkiv(&[
        "member",
        "add",
        "--repo",
        &url,
        "--key-package",
        &kp,
        "--role",
        "write",
    ]);
    let work = root.join("vault-work");
    alice.git(root, &["init", "-q", &work.to_string_lossy()]);
    write(&work, "README", b"v0\n");
    alice.git(&work, &["add", "-A"]);
    alice.git(&work, &["commit", "-q", "-m", "feat: v0"]);
    alice.git(&work, &["push", "-q", &url, "main"]);

    let pin = program("branch-protection");
    let pinned = alice.arkiv(&["program", "pin", "--repo", &url, "--file", &pin]);
    assert!(pinned.contains("program:        rhai/1"), "{pinned}");

    // A delegate's direct push is refused by the program, through the sealed view.
    let c1 = commit(&alice, &work, "v1\n");
    let direct = alice.run("git", &work, &["push", &url, "main"]);
    let err = String::from_utf8_lossy(&direct.stderr);
    assert!(!direct.status.success());
    assert!(err.contains("E_PROGRAM_REJECTED"), "{err}");

    // A member's patch plus the delegate's approval lets it through; a member clone sees it.
    let head = format!(r#"{{"title":"v1","head":"{c1}"}}"#);
    let patch = post(&bob, &url, "xyz.arkiv.patch", "create", None, &head);
    let review = post(
        &alice,
        &url,
        "xyz.arkiv.review",
        "create",
        None,
        &format!(r#"{{"patch":"{patch}"}}"#),
    );
    let approve = format!(r#"{{"patch":"{patch}","head":"{c1}"}}"#);
    post(
        &alice,
        &url,
        "xyz.arkiv.review",
        "approve",
        Some(&review),
        &approve,
    );
    let listed = bob.arkiv(&["op", "list", "--repo", &url]);
    assert!(
        listed.contains(&patch),
        "a member lists sealed ops: {listed}"
    );
    alice.git(&work, &["push", "-q", &url, "main"]);
    let dest = root.join("bob-clone");
    bob.git(root, &["clone", "-q", &url, &dest.to_string_lossy()]);
    assert_eq!(bob.git(&dest, &["rev-parse", "HEAD"]), c1);
    let status = bob.arkiv(&["program", "status", "--repo", &url]);
    assert!(
        status.contains(&format!("head: refs/heads/main {c1}")),
        "{status}"
    );

    // An outsider cannot read the program, its state or the ops.
    let carol = user(root, "carol", &log);
    let outsider = carol.run(
        env!("CARGO_BIN_EXE_arkiv-git"),
        &carol.home,
        &["program", "status", "--repo", &url],
    );
    assert!(String::from_utf8_lossy(&outsider.stderr).contains("E_NOT_MEMBER"));

    // No op entity carries the plaintext rid in any attribute.
    let rid = parse_url(url.trim_start_matches("arkiv::"))
        .expect("url")
        .rid;
    let cobs = arkiv_git_cli::block_on(async {
        let mem = FileMemArkiv::open(&log, MEM_CHAIN_ID).expect("open");
        observe::all(&mem, observe::kind(Kind::Cob))
            .await
            .expect("query")
    })
    .expect("runtime");
    assert!(cobs.len() >= 5, "program create + publish + 3 ops");
    let rid_hex = hex::encode(rid);
    for o in &cobs {
        for a in &o.attributes {
            let leaks = match &a.value {
                AttrValue::Bytes32(b) => *b == rid,
                AttrValue::Bytes(b) => b.windows(32).any(|w| w == rid),
                AttrValue::Str(s) => s.contains(&rid_hex),
                AttrValue::U64(_) => false,
            };
            assert!(!leaks, "op entity {:?} leaks the rid in {}", o.key, a.name);
        }
    }
}

//! Push then clone through `MemArkiv` (design 8.3 first two bullets).

mod common;

use arkiv_git_sync::ids::to_text;
use common::*;

#[tokio::test(flavor = "multi_thread")]
async fn push_then_fresh_clone_gives_identical_refs_and_a_clean_fsck() {
    let net = Net::new();
    let alice = Peer::new(&net, 1);
    let rid = init(&net, &alice, "demo").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let work = work_repo(tmp.path(), "work");
    commit(
        &work,
        &[("a.txt", b"hello"), ("bin/x", &noise(1, 200_000))],
        "feat: one",
    );
    git(&work, &["tag", "-a", "v1", "-m", "v1"]);
    push(
        &net,
        &alice,
        &rid,
        &work,
        &["refs/heads/main", "refs/tags/v1"],
    )
    .await
    .expect("push");

    let bob = Peer::new(&net, 2);
    let dest = tmp.path().join("dest");
    let (view, listing) = clone_into(&net, &bob, &rid, &dest).await.expect("clone");
    assert_eq!(listing.head.as_deref(), Some("refs/heads/main"));
    assert_eq!(
        listing.refs,
        vec![
            ("refs/heads/main".to_string(), rev(&work, "refs/heads/main")),
            ("refs/tags/v1".to_string(), rev(&work, "refs/tags/v1")),
        ]
    );
    assert_eq!(rev(&dest, "main^{tree}"), rev(&work, "main^{tree}"));
    git(&dest, &["fsck", "--full", "--strict"]);
    assert_eq!(view.identity.head().body.name, "demo");
    assert!(to_text(&rid).starts_with('z'));
}

#[tokio::test(flavor = "multi_thread")]
async fn incremental_pushes_deletion_and_no_new_objects() {
    let net = Net::new();
    let alice = Peer::new(&net, 1);
    let rid = init(&net, &alice, "inc").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let work = work_repo(tmp.path(), "work");
    for i in 0..10u8 {
        let name = format!("f{i}");
        commit(
            &work,
            &[(name.as_str(), &noise(u64::from(i), 3000))],
            "feat: step",
        );
        push(&net, &alice, &rid, &work, &["refs/heads/main"])
            .await
            .expect("push");
    }
    git(&work, &["branch", "topic", "HEAD~3"]);
    push(&net, &alice, &rid, &work, &["refs/heads/topic"])
        .await
        .expect("push with no new objects");
    let repo = alice.repo(&rid);
    let state = arkiv_git_sync::state::State::load(&repo.state_path()).expect("state");
    assert_eq!(
        state.applied_packs.len(),
        10,
        "no pack for a ref moved to known objects"
    );
    assert_eq!(state.peers.values().next().expect("mark").seq, 10);
    drop(repo);
    push(&net, &alice, &rid, &work, &[":refs/heads/topic"])
        .await
        .expect("delete");

    let bob = Peer::new(&net, 2);
    let dest = tmp.path().join("dest");
    let (view, _) = clone_into(&net, &bob, &rid, &dest).await.expect("clone");
    let head = view
        .peers
        .values()
        .next()
        .and_then(|p| p.head.clone())
        .expect("head");
    assert_eq!(head.anchor.body.seq, 11);
    assert_eq!(head.refs.refs.len(), 1, "topic deleted");
    assert_eq!(rev(&dest, "main"), rev(&work, "main"));
    git(&dest, &["fsck", "--full"]);

    // An incremental fetch into the existing cache sees a new push.
    commit(&work, &[("late", b"late")], "feat: late");
    push(&net, &alice, &rid, &work, &["refs/heads/main"])
        .await
        .expect("push");
    let (_, listing) = clone_into(&net, &bob, &rid, &dest).await.expect("fetch");
    assert_eq!(listing.refs[0].1, rev(&work, "main"));
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_ref_names_and_unknown_repos_fail_cleanly() {
    let net = Net::new();
    let alice = Peer::new(&net, 1);
    let rid = init(&net, &alice, "names").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let work = work_repo(tmp.path(), "work");
    commit(&work, &[("a", b"a")], "feat: a");
    git(&work, &["update-ref", "refs/rad/x", "HEAD"]);
    let r = push(&net, &alice, &rid, &work, &["refs/rad/x"]).await;
    assert_eq!(code(r), "E_BAD_REFS");
    let ghost = [9u8; 32];
    let r = clone_into(&net, &alice, &ghost, &tmp.path().join("g")).await;
    assert_eq!(code(r), "E_ID_NOT_LIVE");
}

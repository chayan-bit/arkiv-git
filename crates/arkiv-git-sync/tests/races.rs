//! Concurrent pushers and partial uploads (design 4.4, 4.5, 8.3).

mod common;

use arkiv_git_sync::{Gate, Want};
use common::*;

#[tokio::test(flavor = "multi_thread")]
async fn two_peers_push_the_same_repo_and_canonical_refs_follow_the_delegate() {
    let net = Net::new();
    let alice = Peer::new(&net, 1);
    let mut bob = Peer::new(&net, 2);
    let rid = init(&net, &alice, "two").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let wa = work_repo(tmp.path(), "a");
    commit(&wa, &[("a", &noise(1, 70_000))], "feat: a");
    push(&net, &alice, &rid, &wa, &["refs/heads/main"])
        .await
        .expect("alice");

    // Bob builds on alice's history and pushes to his own namespace.
    let wb = tmp.path().join("b");
    clone_into(&net, &bob, &rid, &wb).await.expect("bob clones");
    git(&wb, &["checkout", "-q", "main"]);
    commit(&wb, &[("b", &noise(2, 70_000))], "feat: b");
    bob.cfg.extra_peers = vec![bob.keys.nid()];
    push(&net, &bob, &rid, &wb, &["refs/heads/main"])
        .await
        .expect("bob");

    let carol = Peer::new(&net, 3);
    let repo = carol.repo(&rid);
    let want = Want {
        extra: [bob.keys.nid()].into(),
        ..Want::default()
    };
    let view = arkiv_git_sync::sync(&net, &repo, &want)
        .await
        .expect("sync");
    let canonical = arkiv_git_sync::listing(&view, None);
    assert_eq!(canonical.refs[0].1, rev(&wa, "main"), "delegate decides");
    let bobs = arkiv_git_sync::listing(&view, Some(&bob.keys.nid()));
    assert_eq!(bobs.refs[0].1, rev(&wb, "main"));
    let head = view.peers[&bob.keys.nid()].head.clone().expect("bob head");
    assert_eq!(head.manifests[0].1.requires[0].nid, alice.keys.nid());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copied_key_racing_itself_gets_concurrent_push_then_lost_race() {
    let net = Net::new();
    let a1 = Peer::new(&net, 1);
    let a2 = Peer::new(&net, 1); // same seed, second home: a copied key
    let rid = init(&net, &a1, "race").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let w1 = work_repo(tmp.path(), "w1");
    let w2 = work_repo(tmp.path(), "w2");
    commit(&w1, &[("one", b"1")], "feat: one");
    commit(&w2, &[("two", b"2")], "feat: two");

    // E_CONCURRENT_PUSH: the head moves between a2's sync and its anchor gate.
    let r2 = a2.repo(&rid);
    let p2 = prepare(&net, &a2, &r2, &w2, &[upd("refs/heads/main")])
        .await
        .expect("prepare");
    p2.upload().await.expect("upload");
    push(&net, &a1, &rid, &w1, &["refs/heads/main"])
        .await
        .expect("a1 pushes");
    assert_eq!(code(p2.gate().await), "E_CONCURRENT_PUSH");
    drop(p2);
    assert!(r2.pending_path().exists(), "pending kept for the retry");

    // E_LOST_RACE: both pass the gate, both anchor seq 1, first created wins.
    let r1 = a1.repo(&rid);
    commit(&w1, &[("three", b"3")], "feat: three");
    let mut p1 = prepare(&net, &a1, &r1, &w1, &[upd("refs/heads/main")])
        .await
        .expect("p1");
    let mut p2 = prepare(&net, &a2, &r2, &w2, &[upd("refs/heads/main")])
        .await
        .expect("p2");
    assert!(!r2.outgoing().join("stale").exists());
    p1.upload().await.expect("u1");
    p2.upload().await.expect("u2");
    assert_eq!(p1.gate().await.expect("g1"), Gate::Clear);
    assert_eq!(p2.gate().await.expect("g2"), Gate::Clear);
    p1.send_anchor().await.expect("a1");
    p2.send_anchor().await.expect("a2");
    p1.verify().await.expect("p1 wins");
    assert_eq!(code(p2.verify().await), "E_LOST_RACE");
    p1.commit().expect("commit");

    let bob = Peer::new(&net, 2);
    let (view, listing) = clone_into(&net, &bob, &rid, &tmp.path().join("d"))
        .await
        .expect("clone");
    assert_eq!(listing.refs[0].1, rev(&w1, "main"));
    assert_eq!(view.equivocations.len(), 1, "E_EQUIVOCATION reported");
    assert_eq!(view.equivocations[0].seq, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn partial_upload_fails_incomplete_then_the_retry_sends_only_what_is_missing() {
    let net = Net::new();
    let mut alice = Peer::new(&net, 1);
    alice.cfg.chunk_batch = 2;
    let rid = init(&net, &alice, "partial").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let work = work_repo(tmp.path(), "w");
    commit(&work, &[("big", &noise(7, 600_000))], "feat: big");
    let bulk = alice.keys.bulk_address(&rid);
    net.drop_sends_from(bulk, 3);
    let r = push(&net, &alice, &rid, &work, &["refs/heads/main"]).await;
    assert_eq!(code(r), "E_UPLOAD_INCOMPLETE");
    assert_eq!(created_by(&net, bulk).await.len(), 3, "three chunks landed");
    let repo = alice.repo(&rid);
    assert!(repo.pending_path().exists());
    drop(repo);

    net.heal();
    push(&net, &alice, &rid, &work, &["refs/heads/main"])
        .await
        .expect("retry");
    let chunks = entities(&net, &rid, arkiv_git_core::Kind::Chunk)
        .await
        .len();
    assert_eq!(chunks, 10, "no chunk sent twice");
    assert_eq!(
        created_by(&net, bulk).await.len(),
        chunks + 2,
        "plus one manifest, one refs"
    );
    let anchors = entities(&net, &rid, arkiv_git_core::Kind::Anchor).await;
    assert_eq!(anchors.len(), 1, "exactly one new anchor");
    let repo = alice.repo(&rid);
    assert!(!repo.pending_path().exists());
    let (_, l) = clone_into(&net, &Peer::new(&net, 5), &rid, &tmp.path().join("d"))
        .await
        .expect("clone");
    assert_eq!(l.refs[0].1, rev(&work, "main"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_after_the_anchor_send_is_finished_by_the_next_push() {
    let net = Net::new();
    let alice = Peer::new(&net, 1);
    let rid = init(&net, &alice, "crash").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let work = work_repo(tmp.path(), "w");
    commit(&work, &[("x", b"x")], "feat: x");
    {
        let repo = alice.repo(&rid);
        let mut p = prepare(&net, &alice, &repo, &work, &[upd("refs/heads/main")])
            .await
            .expect("p");
        p.upload().await.expect("upload");
        p.gate().await.expect("gate");
        p.send_anchor().await.expect("anchor");
        // crash: no verify, no commit
    }
    push(&net, &alice, &rid, &work, &["refs/heads/main"])
        .await
        .expect("resume");
    assert_eq!(
        entities(&net, &rid, arkiv_git_core::Kind::Anchor)
            .await
            .len(),
        1
    );
    let repo = alice.repo(&rid);
    assert!(!repo.pending_path().exists());
    let ns = repo.git().refs("refs/namespaces/").expect("refs");
    assert_eq!(ns[0].1, rev(&work, "main"));
}

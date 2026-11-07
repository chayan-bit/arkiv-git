//! P1 gate (design 8.3, 8.6) against MemArkiv: a 100 MB repo of incompressible blobs is
//! pushed and cloned byte-identically, then old anchor and refs are replayed (foreign and
//! own control address) and must not roll back a fresh clone or a marked one.
//!
//! Run (release, temp data under `$TMPDIR`):
//! `TMPDIR=<scratch> cargo test --release -p charter-sync --all-features --test gate -- --ignored --nocapture`

mod common;

use std::time::Instant;

use charter_core::{AnchorBody, Kind, RefsBody};
use charter_store::B256;
use charter_sync::write::entity_op;
use common::*;

const FILE_BYTES: usize = 1_000_000;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "P1 gate: 100 MB through MemArkiv, run in release (command in the file header)"]
async fn gate_100mb_push_clone_and_replay_rejection() {
    gate(100).await;
}

/// The default-CI size of design 8.3.
#[tokio::test(flavor = "multi_thread")]
async fn eight_mb_push_clone_and_replay_rejection() {
    gate(8).await;
}

/// `files` blobs of 1 MB each.
async fn gate(files: u64) {
    let net = Net::new();
    let alice = Peer::new(&net, 1);
    let rid = init(&net, &alice, "gate").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let work = work_repo(tmp.path(), "w");
    for i in 0..files {
        std::fs::write(
            work.join(format!("blob-{i:03}")),
            noise(1_000 + i, FILE_BYTES),
        )
        .expect("write");
    }
    let a = commit(&work, &[], "feat: noise");

    let t = Instant::now();
    push(&net, &alice, &rid, &work, &["refs/heads/main"])
        .await
        .expect("push A");
    eprintln!("gate: push of {files} MB took {:.1?}", t.elapsed());
    let chunks = entities(&net, &rid, Kind::Chunk).await.len();
    eprintln!("gate: {chunks} chunks on MemArkiv");
    assert!(
        chunks as u64 >= files * 15,
        "1 MB is about 15.3 chunks of 64 KiB"
    );

    let b = commit(&work, &[("B", b"b")], "feat: B");
    push(&net, &alice, &rid, &work, &["refs/heads/main"])
        .await
        .expect("push B");
    let c = commit(&work, &[("C", b"c")], "feat: C");
    push(&net, &alice, &rid, &work, &["refs/heads/main"])
        .await
        .expect("push C");
    assert_ne!(a, b);

    // Replay A's anchor and refs from a foreign address, and A's anchor from alice's own
    // control address, both after C.
    let anchors = entities(&net, &rid, Kind::Anchor).await;
    let anchor_a = anchors.iter().find(|o| seq_of(o) == 0).expect("A");
    let refs_a = entities(&net, &rid, Kind::Refs).await[0].clone();
    let replay_anchor = entity_op::<AnchorBody>(&body_of(anchor_a), 100_000).expect("op");
    let replay_refs = entity_op::<RefsBody>(&body_of(&refs_a), 100_000).expect("op");
    send_ops(
        &net,
        &outsider(&net),
        vec![replay_anchor.clone(), replay_refs],
    )
    .await;
    // Pushes leave control only its spend plus a reserve; fund the replay itself.
    net.fund(alice.keys.control_address(&rid));
    send_ops(&net, &alice.keys.control(&rid), vec![replay_anchor]).await;

    let bob = Peer::new(&net, 2);
    let dest = tmp.path().join("clone");
    let t = Instant::now();
    let (view, listing) = clone_into(&net, &bob, &rid, &dest).await.expect("clone");
    eprintln!("gate: fresh clone of {files} MB took {:.1?}", t.elapsed());
    assert_eq!(
        listing.refs,
        vec![("refs/heads/main".to_owned(), c.clone())],
        "replay rejected: clone gets C"
    );
    assert!(view.equivocations.is_empty());
    git(&dest, &["checkout", "-q", "main"]);
    assert_eq!(
        git(&dest, &["ls-tree", "-r", "HEAD"]),
        git(&work, &["ls-tree", "-r", "HEAD"])
    );
    git(&dest, &["fsck", "--full", "--strict"]);
    for i in [0, files / 2, files - 1] {
        let name = format!("blob-{i:03}");
        let same = std::fs::read(dest.join(&name)).expect("read") == noise(1_000 + i, FILE_BYTES);
        assert!(same, "{name} differs");
    }

    // Bob holds the mark at C; a view withholding C with a lying entityNonce shows A (the
    // replay) or B at best, and must give E_STALE_HEAD rather than roll back.
    let c_anchor = anchors.iter().find(|o| seq_of(o) == 2).expect("C");
    net.mem.withhold(B256::from(c_anchor.key));
    let control = alice.keys.control_address(&rid);
    let truth = charter_store::Arkiv::entity_nonce(&net, control)
        .await
        .expect("nonce");
    net.mem.lie_entity_nonce(control, truth - 1);
    let stale = clone_into(&net, &bob, &rid, &dest).await;
    assert_eq!(code(stale), "E_STALE_HEAD");
}

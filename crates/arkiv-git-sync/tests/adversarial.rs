//! Fail-closed cases and the replay gate rehearsal (design 3.3, 3.6, 3.9, 8.3).

mod common;

use arkiv_git_core::{AnchorBody, Kind, RefsBody};
use arkiv_git_store::{B256, Operation};
use arkiv_git_sync::write::entity_op;
use common::*;

/// Pushes three commits A, B, C (anchors seq 0, 1, 2); returns the work repo path.
async fn three_pushes(
    net: &Net,
    p: &Peer,
    rid: &arkiv_git_core::Rid,
    dir: &std::path::Path,
) -> Vec<String> {
    let work = work_repo(dir, "w");
    let mut tips = vec![];
    for (i, name) in ["A", "B", "C"].iter().enumerate() {
        tips.push(commit(
            &work,
            &[(name, &noise(i as u64, 1000))],
            "feat: step",
        ));
        push(net, p, rid, &work, &["refs/heads/main"])
            .await
            .expect("push");
    }
    tips
}

fn anchor_at(anchors: &[arkiv_git_core::Observed], seq: u64) -> arkiv_git_core::Observed {
    anchors
        .iter()
        .find(|o| seq_of(o) == seq)
        .cloned()
        .expect("anchor")
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_chunk_fails_the_clone_with_not_live() {
    let net = Net::new();
    let mut alice = Peer::new(&net, 1);
    alice.cfg.data_lifetime = 400;
    let rid = init(&net, &alice, "expiry").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let work = work_repo(tmp.path(), "w");
    commit(&work, &[("f", &noise(3, 200_000))], "feat: f");
    push(&net, &alice, &rid, &work, &["refs/heads/main"])
        .await
        .expect("push");
    let sponsor = outsider(&net);
    let mut keep: Vec<_> = entities(&net, &rid, Kind::Refs).await;
    keep.extend(entities(&net, &rid, Kind::Manifest).await);
    let keys: Vec<_> = keep.iter().map(|o| o.key).collect();
    extend(&net, &sponsor, &keys, 100_000).await;
    net.mine(500);
    let r = clone_into(&net, &Peer::new(&net, 2), &rid, &tmp.path().join("d")).await;
    let err = r.expect_err("chunks expired");
    assert_eq!(err.code().as_str(), "E_NOT_LIVE");
    assert!(err.to_string().contains("chunk 0 of 4"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_deleted_head_anchor_fails_closed() {
    let net = Net::new();
    let alice = Peer::new(&net, 1);
    let rid = init(&net, &alice, "deleted").await;
    let tmp = tempfile::tempdir().expect("tmp");
    three_pushes(&net, &alice, &rid, tmp.path()).await;
    let bob = Peer::new(&net, 2);
    clone_into(&net, &bob, &rid, &tmp.path().join("b"))
        .await
        .expect("bob clones");
    let head = anchor_at(&entities(&net, &rid, Kind::Anchor).await, 2);
    send_ops(
        &net,
        &alice.keys.control(&rid),
        vec![Operation::delete(B256::from(head.key))],
    )
    .await;
    let fresh = clone_into(&net, &Peer::new(&net, 3), &rid, &tmp.path().join("c")).await;
    assert_eq!(code(fresh), "E_CONTROL_NOT_LIVE");
    let marked = clone_into(&net, &bob, &rid, &tmp.path().join("b")).await;
    assert_eq!(code(marked), "E_CONTROL_NOT_LIVE");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_withheld_newer_anchor_is_caught_by_completeness_or_the_high_water_mark() {
    let net = Net::new();
    let alice = Peer::new(&net, 1);
    let rid = init(&net, &alice, "withheld").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let tips = three_pushes(&net, &alice, &rid, tmp.path()).await;
    let bob = Peer::new(&net, 2);
    clone_into(&net, &bob, &rid, &tmp.path().join("b"))
        .await
        .expect("mark at C");
    let c = anchor_at(&entities(&net, &rid, Kind::Anchor).await, 2);
    net.mem.withhold(B256::from(c.key));
    let honest = clone_into(&net, &Peer::new(&net, 3), &rid, &tmp.path().join("h")).await;
    assert_eq!(
        code(honest),
        "E_CONTROL_NOT_LIVE",
        "truthful entityNonce exposes it"
    );

    let control = alice.keys.control_address(&rid);
    let truth = arkiv_git_store::Arkiv::entity_nonce(&net, control)
        .await
        .expect("nonce");
    net.mem.lie_entity_nonce(control, truth - 1);
    let marked = clone_into(&net, &bob, &rid, &tmp.path().join("b")).await;
    assert_eq!(code(marked), "E_STALE_HEAD");
    // A fresh client facing a node that lies about both is outside the trust model.
    let (_, fooled) = clone_into(&net, &Peer::new(&net, 4), &rid, &tmp.path().join("f"))
        .await
        .expect("served B");
    assert_eq!(fooled.refs[0].1, tips[1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn replayed_old_anchor_and_refs_are_rejected() {
    let net = Net::new();
    let mut alice = Peer::new(&net, 1);
    alice.cfg.control_lifetime = 3_000;
    let rid = init(&net, &alice, "replay").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let tips = three_pushes(&net, &alice, &rid, tmp.path()).await;
    let anchors = entities(&net, &rid, Kind::Anchor).await;
    let a = anchor_at(&anchors, 0);
    let a_refs = entities(&net, &rid, Kind::Refs).await[0].clone();

    // A foreign address re-posts A's anchor and refs; the peer's own control key re-posts A.
    let foreign = outsider(&net);
    let replay_anchor = entity_op::<AnchorBody>(&body_of(&a), 100_000).expect("op");
    let replay_refs = entity_op::<RefsBody>(&body_of(&a_refs), 100_000).expect("op");
    send_ops(&net, &foreign, vec![replay_anchor.clone(), replay_refs]).await;
    // Pushes leave control only its spend plus a reserve; fund the replay itself.
    net.fund(alice.keys.control_address(&rid));
    send_ops(&net, &alice.keys.control(&rid), vec![replay_anchor]).await;

    let bob = Peer::new(&net, 2);
    let (view, listing) = clone_into(&net, &bob, &rid, &tmp.path().join("b"))
        .await
        .expect("clone");
    assert_eq!(listing.refs[0].1, tips[2], "fresh clone gets C");
    assert!(
        view.equivocations.is_empty(),
        "identical bytes are a duplicate, not a fork"
    );

    // C's anchor expires while a third party keeps every older control entity alive.
    let sponsor = outsider(&net);
    let control = alice.keys.control_address(&rid);
    let c_key = anchor_at(&anchors, 2).key;
    let keep: Vec<_> = created_by(&net, control)
        .await
        .into_iter()
        .map(|o| o.key)
        .filter(|k| *k != c_key)
        .collect();
    extend(&net, &sponsor, &keep, 100_000).await;
    net.mine(3_100);
    let fresh = clone_into(&net, &Peer::new(&net, 3), &rid, &tmp.path().join("c")).await;
    assert_eq!(code(fresh), "E_CONTROL_NOT_LIVE");
    let marked = clone_into(&net, &bob, &rid, &tmp.path().join("b")).await;
    assert_eq!(code(marked), "E_CONTROL_NOT_LIVE");
}

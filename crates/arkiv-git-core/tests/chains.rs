//! Bind chain and anchor chain (design 3.1, 3.4, 3.6, 3.7).

mod common;

use arkiv_git_core::*;
use common::history::*;
use common::*;
use std::collections::BTreeSet;

fn all_ok(n: usize) -> Outcome {
    let served = honest(n);
    let comp = completeness(&controls(n), &served, &served, true);
    run(&served, &comp, None)
}

#[test]
fn honest_history_gives_newest_head() {
    for n in [1, ROTATE_AT, ROTATE_AT + 1, PUSHES] {
        let (head, chain, eq) = all_ok(n).expect("ok").expect("head");
        assert_eq!(head, world().anchors[n - 1].1, "n = {n}");
        assert_eq!(chain.len(), n);
        assert!(eq.is_empty());
    }
}

#[test]
fn peer_key_needs_a_valid_self_signed_document() {
    let w = world();
    assert_eq!(
        peer_key(&alice().nid, std::slice::from_ref(&w.peer_obs)),
        Some(alice().pk.clone())
    );
    assert_eq!(
        peer_key(&peers()[1].nid, std::slice::from_ref(&w.peer_obs)),
        None
    );
    let mut forged = w.peer_obs.clone();
    forged.attributes.retain(|a| a.name != "ag.h");
    assert_eq!(peer_key(&alice().nid, &[forged]), None);
}

#[test]
fn bind_chain_follows_rotation_and_creator_rule() {
    let w = world();
    let info = peer_info(&w.binds);
    assert_eq!(info.binds.links.len(), 2);
    assert_eq!(
        info.binds.control_set(),
        BTreeSet::from([CONTROL0, CONTROL1])
    );
    assert_eq!(info.binds.bulk_set(), BTreeSet::from([BULK0, BULK1]));
    let b = &info.binds;
    assert!(b.creator_ok(Plane::Control, &CONTROL0, rotation_block()));
    assert!(!b.creator_ok(Plane::Control, &CONTROL0, rotation_block() + 1));
    assert!(b.creator_ok(Plane::Control, &CONTROL1, 99_999));
    assert!(b.creator_ok(Plane::Bulk, &BULK0, 1));
    assert!(!b.creator_ok(Plane::Bulk, &CONTROL1, 1));
    assert!(!b.creator_ok(Plane::Control, &FOREIGN, 1));
}

#[test]
fn bind_seq0_by_wrong_creator_is_ignored() {
    let w = world();
    let moved = relocate(&w.binds[0], FOREIGN, 1, 1);
    let info = peer_info(&[moved]);
    assert!(info.binds.links.is_empty());
}

#[test]
fn bind_above_head_is_a_chain_gap() {
    let w = world();
    // bind 1 without bind 0: seq 1 created by a control address not in an (empty) chain
    // is ignored; with bind 0 replaced by a foreign copy the chain is empty too.
    let r = bind_chain(&w.rid, &alice().pk, &[w.binds[1].clone()]).expect("ok");
    assert!(r.links.is_empty());
    // A validly created seq-1 bind whose prev does not link is a gap.
    let p = alice();
    let (b0, _) = (w.binds[0].clone(), ());
    let stray = BindBody {
        rid: w.rid,
        ..bind_body(2, [9; 32], CONTROL1, BULK1)
    };
    let (stray_obs, _) = observe(p, &stray, CONTROL0, 5, 0);
    let err = bind_chain(&w.rid, &p.pk, &[b0, stray_obs]).expect_err("gap");
    assert_eq!(err.code(), ErrorCode::ChainGap);
}

#[test]
fn bind_equivocation_is_reported_not_fatal() {
    let w = world();
    let p = alice();
    let alt = BindBody {
        rid: w.rid,
        ..bind_body(1, content_h(&w.binds[0]), [0xc2; 20], [0xb2; 20])
    };
    let (alt_obs, _) = observe(p, &alt, CONTROL0, rotation_block() + 1, 0);
    let mut set = w.binds.clone();
    set.push(alt_obs);
    let r = bind_chain(&w.rid, &p.pk, &set).expect("ok");
    assert_eq!(r.links.len(), 2);
    assert_eq!(r.equivocations.len(), 1);
    assert_eq!(r.equivocations[0].seq, 1);
}

fn content_h(o: &Observed) -> Hash32 {
    match o.attr("ag.h") {
        Some(AttrValue::Bytes32(h)) => *h,
        _ => panic!("no ag.h"),
    }
}

#[test]
fn anchors_without_any_bind_fail_closed() {
    let w = world();
    // The full pipeline stops at the identity (the root fails the creator rule)...
    let served: Vec<Observed> = honest(3)
        .into_iter()
        .filter(|o| o.key != w.binds[0].key)
        .collect();
    let comp = completeness(&controls(3), &served, &served, true);
    assert_eq!(run(&served, &comp, None), Err(ErrorCode::IdNotLive));
    // ...and the anchor chain alone refuses signed anchors with no bind chain.
    let identity = identity_chain(
        &w.rid,
        &peers_map(peer_info(&w.binds)),
        std::slice::from_ref(&w.root_obs),
        None,
    )
    .expect("id");
    let unbound = peer_info(&[]);
    let anchors: Vec<Observed> = (0..3).map(|k| w.anchors[k].0.clone()).collect();
    let r = anchor_chain(
        &w.rid,
        &alice().nid,
        &unbound,
        &identity,
        &anchors,
        Start::Genesis,
    );
    assert_eq!(
        r.map(|_| ()).map_err(|e| e.code()),
        Err(ErrorCode::ChainGap)
    );
    let none = anchor_chain(
        &w.rid,
        &alice().nid,
        &unbound,
        &identity,
        &[],
        Start::Genesis,
    );
    assert!(none.expect("ok").links.is_empty());
}

#[test]
fn foreign_replay_of_old_anchor_is_inert() {
    let n = 5;
    let w = world();
    let mut served = honest(n);
    served.push(relocate(&w.anchors[0].0, FOREIGN, 500, 0));
    served.push(relocate(&w.refs[0].0, FOREIGN, 500, 1));
    let comp = completeness(&controls(n), &served, &served, true);
    let (head, ..) = run(&served, &comp, None).expect("ok").expect("head");
    assert_eq!(head, w.anchors[n - 1].1);
}

#[test]
fn own_control_replay_cannot_displace_the_first_created_anchor() {
    let n = 6;
    let w = world();
    let mut served = honest(n);
    served.push(relocate(&w.anchors[1].0, CONTROL1, 600, 0));
    let comp = completeness(&controls(n), &served, &served, true);
    let (head, chain, eq) = run(&served, &comp, None).expect("ok").expect("head");
    assert_eq!(head, w.anchors[n - 1].1);
    assert_eq!(chain[1], w.anchors[1].1);
    assert!(
        eq.is_empty(),
        "identical bytes are a duplicate, not equivocation"
    );
}

#[test]
fn forks_are_equivocations_and_first_created_wins() {
    let n = PUSHES;
    let w = world();
    let mut served = honest(n);
    served.extend(w.forks.iter().cloned());
    let comp = completeness(&controls(n), &served, &served, true);
    let (head, _, eq) = run(&served, &comp, None).expect("ok").expect("head");
    assert_eq!(head, w.anchors[n - 1].1);
    let seqs: Vec<u64> = eq.iter().map(|e| e.0).collect();
    assert_eq!(seqs, vec![2, 4]);
}

#[test]
fn missing_head_anchor_with_truthful_nonce_is_control_not_live() {
    let n = 5;
    let created = honest(n);
    let served: Vec<Observed> = created
        .iter()
        .filter(|o| o.key != world().anchors[n - 1].0.key)
        .cloned()
        .collect();
    let comp = completeness(&controls(n), &created, &served, true);
    assert_eq!(run(&served, &comp, None), Err(ErrorCode::ControlNotLive));
}

#[test]
fn withheld_head_with_lying_nonce_is_stopped_by_the_mark() {
    let n = 5;
    let w = world();
    let created = honest(n);
    let served: Vec<Observed> = created
        .iter()
        .filter(|o| o.key != w.anchors[n - 1].0.key)
        .cloned()
        .collect();
    let comp = completeness(&controls(n), &created, &served, false);
    let (head, ..) = run(&served, &comp, None).expect("ok").expect("head");
    assert_eq!(head, w.anchors[n - 2].1, "without a mark the lie succeeds");
    let mark = PeerMark {
        seq: (n - 1) as u64,
        hash: w.anchors[n - 1].1,
        block: push_block(n - 1),
    };
    assert_eq!(run(&served, &comp, Some(&mark)), Err(ErrorCode::StaleHead));
}

#[test]
fn mark_on_a_different_anchor_is_history_rewritten() {
    let n = 4;
    let served = honest(n);
    let comp = completeness(&controls(n), &served, &served, true);
    let mark = PeerMark {
        seq: 2,
        hash: [7; 32],
        block: 0,
    };
    assert_eq!(
        run(&served, &comp, Some(&mark)),
        Err(ErrorCode::HistoryRewritten)
    );
}

#[test]
fn anchor_with_at_after_creation_is_invalid() {
    let w = world();
    let p = alice();
    let late = AnchorBody {
        rid: w.rid,
        ..anchor_body(0, ZERO, w.refs[0].1, w.rid, 10_000)
    };
    let (late_obs, _) = observe(p, &late, CONTROL0, push_block(0), 1);
    let mut served = honest(0);
    served.push(w.refs[0].0.clone());
    served.push(late_obs);
    let comp = completeness(&controls(0), &served, &served, true);
    assert_eq!(run(&served, &comp, None), Ok(None));
}

#[test]
fn anchor_with_unknown_identity_head_is_invalid() {
    let w = world();
    let p = alice();
    let bad = AnchorBody {
        rid: w.rid,
        ..anchor_body(0, ZERO, w.refs[0].1, [3; 32], 1)
    };
    let (obs, _) = observe(p, &bad, CONTROL0, push_block(0), 1);
    let mut served = honest(0);
    served.push(obs);
    let comp = completeness(&controls(0), &served, &served, true);
    assert_eq!(run(&served, &comp, None), Ok(None));
}

#[test]
fn unsupported_version_from_tracked_control_fails_closed_else_ignored() {
    let n = 2;
    let w = world();
    let mut future = w.anchors[n - 1].0.clone();
    future.attributes.iter_mut().for_each(|a| {
        if a.name == "ag.v" {
            a.value = AttrValue::U64(2);
        }
    });
    let mut served = honest(n);
    served.push(relocate(&future, CONTROL0, 900, 0));
    let comp = completeness(&controls(n), &served, &served, true);
    assert_eq!(
        run(&served, &comp, None),
        Err(ErrorCode::UnsupportedVersion)
    );
    let mut served = honest(n);
    served.push(relocate(&future, FOREIGN, 900, 0));
    let comp = completeness(&controls(n), &served, &served, true);
    assert!(run(&served, &comp, None).expect("ok").is_some());
}

#[test]
fn incremental_fetch_from_a_mark() {
    let n = 6;
    let w = world();
    let peer = peer_info(&w.binds);
    let peers = peers_map(peer.clone());
    let identity =
        identity_chain(&w.rid, &peers, std::slice::from_ref(&w.root_obs), None).expect("id");
    let anchors: Vec<Observed> = (3..n).map(|k| w.anchors[k].0.clone()).collect();
    let mark = PeerMark {
        seq: 3,
        hash: w.anchors[3].1,
        block: push_block(3),
    };
    let chain = anchor_chain(
        &w.rid,
        &alice().nid,
        &peer,
        &identity,
        &anchors,
        Start::Mark(&mark),
    )
    .expect("chain");
    assert_eq!(chain.links.first().map(|l| l.body.seq), Some(3));
    assert_eq!(chain.head().map(|l| l.h), Some(w.anchors[n - 1].1));
    let wrong = PeerMark {
        hash: [1; 32],
        ..mark
    };
    let err = anchor_chain(
        &w.rid,
        &alice().nid,
        &peer,
        &identity,
        &anchors,
        Start::Mark(&wrong),
    );
    assert_eq!(
        err.map(|_| ()).map_err(|e| e.code()),
        Err(ErrorCode::HistoryRewritten)
    );
    let gone = PeerMark { seq: 7, ..mark };
    let err = anchor_chain(
        &w.rid,
        &alice().nid,
        &peer,
        &identity,
        &anchors,
        Start::Mark(&gone),
    );
    assert_eq!(
        err.map(|_| ()).map_err(|e| e.code()),
        Err(ErrorCode::StaleHead)
    );
}

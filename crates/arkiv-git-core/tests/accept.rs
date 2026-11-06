//! Head acceptance, completeness and marks (design 3.3, 3.6 table, 3.9).

mod common;

use arkiv_git_core::*;
use common::history::*;
use common::*;
use std::collections::BTreeSet;

#[test]
fn missing_head_refs_is_not_live_and_old_refs_may_expire() {
    let n = 4;
    let w = world();
    let without_head_refs: Vec<Observed> = honest(n)
        .into_iter()
        .filter(|o| o.key != w.refs[n - 1].0.key)
        .collect();
    let comp = completeness(&controls(n), &without_head_refs, &without_head_refs, true);
    assert_eq!(
        run(&without_head_refs, &comp, None),
        Err(ErrorCode::NotLive)
    );
    let without_old_refs: Vec<Observed> = honest(n)
        .into_iter()
        .filter(|o| o.key != w.refs[0].0.key)
        .collect();
    let comp = completeness(&controls(n), &without_old_refs, &without_old_refs, true);
    assert!(run(&without_old_refs, &comp, None).expect("ok").is_some());
}

#[test]
fn head_refs_from_a_foreign_creator_is_bad_refs() {
    let n = 2;
    let w = world();
    let mut served: Vec<Observed> = honest(n)
        .into_iter()
        .filter(|o| o.key != w.refs[n - 1].0.key)
        .collect();
    served.push(relocate(&w.refs[n - 1].0, FOREIGN, 300, 0));
    let comp = completeness(&controls(n), &served, &served, true);
    assert_eq!(run(&served, &comp, None), Err(ErrorCode::BadRefs));
}

#[test]
fn completeness_predicate() {
    let nid = alice().nid;
    let obs = |b, l, a| CompletenessObs {
        address: CONTROL0,
        nonce_before: b,
        live: l,
        nonce_after: a,
    };
    assert!(check_completeness(&nid, &obs(5, 5, 5)).is_ok());
    assert_eq!(
        check_completeness(&nid, &obs(5, 4, 5)).map_err(|e| e.code()),
        Err(ErrorCode::ControlNotLive)
    );
    assert_eq!(
        check_completeness(&nid, &obs(5, 6, 5)).map_err(|e| e.code()),
        Err(ErrorCode::InconsistentView)
    );
    assert_eq!(
        check_completeness(&nid, &obs(5, 5, 6)).map_err(|e| e.code()),
        Err(ErrorCode::InconsistentView)
    );
    let set = BTreeSet::from([CONTROL0, CONTROL1]);
    assert_eq!(
        check_control_set(&nid, &set, &[obs(1, 1, 1)]).map_err(|e| e.code()),
        Err(ErrorCode::Internal)
    );
}

#[test]
fn marks_only_advance() {
    let m = PeerMark {
        seq: 3,
        hash: [1; 32],
        block: 30,
    };
    let up = PeerMark {
        seq: 4,
        hash: [2; 32],
        block: 40,
    };
    assert_eq!(m.advance(&up), Ok(up.clone()));
    assert_eq!(
        up.advance(&m).map_err(|e| e.code()),
        Err(ErrorCode::StaleHead)
    );
    let same_seq_other = PeerMark {
        hash: [9; 32],
        ..m.clone()
    };
    assert_eq!(
        m.advance(&same_seq_other).map_err(|e| e.code()),
        Err(ErrorCode::HistoryRewritten)
    );
    let i = IdMark {
        rev: 2,
        hash: [1; 32],
    };
    assert!(
        i.advance(&IdMark {
            rev: 1,
            hash: [0; 32]
        })
        .is_err()
    );
}

#[test]
fn manifests_named_by_the_chain_must_be_live_unless_applied() {
    let w = world();
    let p = alice();
    let m = ManifestBody {
        rid: w.rid,
        ..manifest_body(1)
    };
    let (m_obs, m_h) = observe(p, &m, BULK0, push_block(0), 0);
    let a = AnchorBody {
        rid: w.rid,
        manifests: vec![m_h],
        ..anchor_body(0, ZERO, w.refs[0].1, w.rid, 1)
    };
    let (a_obs, _) = observe(p, &a, CONTROL0, push_block(0), 2);
    let base = vec![
        w.peer_obs.clone(),
        w.binds[0].clone(),
        w.root_obs.clone(),
        w.refs[0].0.clone(),
        a_obs,
    ];
    let peer = peer_info(&w.binds[..1]);
    let identity = identity_chain(
        &w.rid,
        &peers_map(peer.clone()),
        std::slice::from_ref(&w.root_obs),
        None,
    )
    .expect("id");
    let chain =
        anchor_chain(&w.rid, &p.nid, &peer, &identity, &base[4..], Start::Genesis).expect("chain");
    let comp = completeness(&[CONTROL0], &base, &base, true);
    let empty = BTreeSet::new();
    let inputs = |manifests: &[Observed], applied: &BTreeSet<Hash32>| {
        accept_head(
            &w.rid,
            &peer,
            &chain,
            &HeadInputs {
                completeness: &comp,
                mark: None,
                refs: &base[3..4],
                manifests,
                applied,
            },
        )
        .map(|h| h.map(|h| h.manifests.len()))
        .map_err(|e| e.code())
    };
    assert_eq!(inputs(&[], &empty), Err(ErrorCode::NotLive));
    assert_eq!(inputs(std::slice::from_ref(&m_obs), &empty), Ok(Some(1)));
    assert_eq!(inputs(&[], &BTreeSet::from([m_h])), Ok(Some(0)));
    let foreign = relocate(&m_obs, FOREIGN, 50, 0);
    assert_eq!(inputs(&[foreign], &empty), Err(ErrorCode::NotLive));

    let req = ManifestBody {
        requires: vec![Require {
            nid: peers()[1].nid,
            anchor: [4; 32],
        }],
        ..m
    };
    let chains = std::collections::BTreeMap::from([(p.nid, chain.clone())]);
    assert_eq!(
        check_requires(std::slice::from_ref(&req), &chains).map_err(|e| e.code()),
        Err(ErrorCode::MissingDependency)
    );
    let ok = ManifestBody {
        requires: vec![Require {
            nid: p.nid,
            anchor: chain.links[0].h,
        }],
        ..req
    };
    assert!(check_requires(&[ok], &chains).is_ok());
}

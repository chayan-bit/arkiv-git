//! Identity evaluation and key rotation (design 3.5), unit and property tests.

mod common;

use charter_core::*;
use common::*;
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::sync::OnceLock;

fn control(i: usize) -> Address {
    [0xd0 + i as u8; 20]
}

struct Fx {
    rid: Rid,
    peers: BTreeMap<Nid, PeerInfo>,
    root: Observed,
    root_acc: Observed,
    x: (IdBody, Observed, Hash32),
    y: (IdBody, Observed, Hash32),
    /// acc[peer][0 = x, 1 = y], placed at block 0 (relocated by tests).
    acc: Vec<[Observed; 2]>,
}

fn delegates(idx: &[usize]) -> Vec<Nid> {
    let mut d: Vec<Nid> = idx.iter().map(|i| peers()[*i].nid).collect();
    d.sort();
    d
}

fn rev(parent: Hash32, rid: Rid, rev: u64, name: &str, d: Vec<Nid>) -> IdBody {
    IdBody {
        rid: Some(rid),
        rev,
        parent,
        name: name.into(),
        nonce: None,
        ..root_body(d, 1)
    }
}

fn acc(p: usize, rid: Rid, target: Hash32, block: u64, tx: u32) -> Observed {
    observe(
        &peers()[p],
        &IdAccBody { rid, rev: target },
        control(p),
        block,
        tx,
    )
    .0
}

fn build() -> Fx {
    let root = root_body(delegates(&[0, 1, 2]), 1);
    let (root_bytes, rid) = sign(&peers()[0], &root);
    let root_obs = observe_bytes(&root_bytes, &root, control(0), 2, 0);
    let mut map = BTreeMap::new();
    for (i, p) in peers().iter().enumerate() {
        let b = BindBody {
            rid,
            ..bind_body(0, ZERO, control(i), [0xe0 + i as u8; 20])
        };
        let binds =
            bind_chain(&rid, &p.pk, &[observe(p, &b, control(i), 1, i as u32).0]).expect("bind");
        map.insert(
            p.nid,
            PeerInfo {
                pk: p.pk.clone(),
                binds,
            },
        );
    }
    let xb = rev(rid, rid, 1, "x", delegates(&[0, 1, 2]));
    let yb = rev(rid, rid, 1, "y", delegates(&[0, 1, 3]));
    let (xo, xh) = observe(&peers()[0], &xb, control(0), 3, 0);
    let (yo, yh) = observe(&peers()[1], &yb, control(1), 3, 1);
    let accs = (0..peers().len())
        .map(|p| [acc(p, rid, xh, 0, 0), acc(p, rid, yh, 0, 0)])
        .collect();
    Fx {
        rid,
        peers: map,
        root_acc: acc(1, rid, rid, 2, 1),
        root: root_obs,
        x: (xb, xo, xh),
        y: (yb, yo, yh),
        acc: accs,
    }
}

fn fx() -> &'static Fx {
    static F: OnceLock<Fx> = OnceLock::new();
    F.get_or_init(build)
}

fn place(o: &Observed, block: u64, tx: u32) -> Observed {
    let mut c = o.clone();
    c.block = block;
    c.tx = tx;
    c.key[..8].copy_from_slice(&block.to_be_bytes());
    c.key[8..12].copy_from_slice(&tx.to_be_bytes());
    c
}

fn eval(obs: &[Observed]) -> Result<IdentityChain, ErrorCode> {
    identity_chain(&fx().rid, &fx().peers, obs, None).map_err(|e| e.code())
}

fn head_rev(obs: &[Observed]) -> Result<(u64, Hash32), ErrorCode> {
    eval(obs).map(|c| {
        let h = c.head();
        (h.body.rev, h.h)
    })
}

#[test]
fn quorum_math() {
    assert_eq!([1, 2, 3, 4, 5, 16].map(maj), [1, 2, 2, 3, 3, 9]);
}

#[test]
fn root_needs_quorum_of_its_own_delegates() {
    let f = fx();
    assert_eq!(
        head_rev(std::slice::from_ref(&f.root)),
        Err(ErrorCode::IdNotLive)
    );
    assert_eq!(
        head_rev(&[f.root.clone(), f.root_acc.clone()]),
        Ok((0, f.rid))
    );
    let outsider = place(&acc(3, f.rid, f.rid, 0, 0), 2, 5);
    assert_eq!(
        head_rev(&[f.root.clone(), outsider]),
        Err(ErrorCode::IdNotLive)
    );
}

#[test]
fn root_signed_by_non_delegate_is_rejected() {
    let root = root_body(delegates(&[1, 2]), 1);
    let (bytes, rid) = sign(&peers()[0], &root);
    let obs = observe_bytes(&bytes, &root, control(0), 2, 0);
    let a = place(&acc(1, rid, rid, 0, 0), 2, 1);
    let b = place(&acc(2, rid, rid, 0, 0), 2, 2);
    let r = identity_chain(&rid, &fx().peers, &[obs, a, b], None);
    assert_eq!(
        r.map(|_| ()).map_err(|e| e.code()),
        Err(ErrorCode::IdNotLive)
    );
}

fn base() -> Vec<Observed> {
    let f = fx();
    vec![
        f.root.clone(),
        f.root_acc.clone(),
        f.x.1.clone(),
        f.y.1.clone(),
    ]
}

#[test]
fn first_quorum_wins_among_siblings() {
    let f = fx();
    // x created first but reaches quorum at block 9; y reaches it at block 5.
    let mut obs = base();
    obs.push(place(&f.acc[2][0], 9, 0));
    obs.push(place(&f.acc[0][1], 5, 0));
    assert_eq!(head_rev(&obs), Ok((1, f.y.2)));
    // a later acceptance of x never revives it
    obs.push(place(&f.acc[1][0], 20, 0));
    assert_eq!(head_rev(&obs), Ok((1, f.y.2)));
}

#[test]
fn dead_sibling_children_are_never_accepted() {
    let f = fx();
    let mut obs = base();
    obs.push(place(&f.acc[0][1], 5, 0));
    let child = rev(f.x.2, f.rid, 2, "x2", delegates(&[0, 1, 2]));
    let (c_obs, c_h) = observe(&peers()[0], &child, control(0), 30, 0);
    obs.push(c_obs);
    obs.push(place(&acc(1, f.rid, c_h, 0, 0), 31, 0));
    obs.push(place(&acc(2, f.rid, c_h, 0, 0), 31, 1));
    assert_eq!(head_rev(&obs), Ok((1, f.y.2)));
}

#[test]
fn only_parent_delegates_count() {
    let f = fx();
    // peer 3 and 4 are not delegates of the root
    let mut obs = base();
    obs.push(place(&f.acc[3][0], 6, 0));
    obs.push(place(&f.acc[4][0], 6, 1));
    assert_eq!(head_rev(&obs), Ok((0, f.rid)));
}

#[test]
fn rotation_replaces_the_delegate_set() {
    let f = fx();
    // y replaces peer 2 with peer 3; accepted by peers 1 (proposer) and 0.
    let mut obs = base();
    obs.push(place(&f.acc[0][1], 5, 0));
    let next = rev(f.y.2, f.rid, 2, "z", delegates(&[0, 1, 3]));
    let (n_obs, n_h) = observe(&peers()[0], &next, control(0), 40, 0);
    obs.push(n_obs);
    // old delegate 2 accepting no longer counts
    obs.push(place(&acc(2, f.rid, n_h, 0, 0), 41, 0));
    assert_eq!(head_rev(&obs), Ok((1, f.y.2)));
    // new delegate 3 does
    obs.push(place(&acc(3, f.rid, n_h, 0, 0), 42, 0));
    let chain = eval(&obs).expect("ok");
    assert_eq!(chain.head().body.rev, 2);
    assert_eq!(chain.head().body.delegates, delegates(&[0, 1, 3]));
    assert!(chain.contains(&f.y.2) && chain.contains(&f.rid));
}

#[test]
fn identity_mark_is_enforced() {
    let f = fx();
    let obs = vec![f.root.clone(), f.root_acc.clone()];
    let ahead = IdMark {
        rev: 1,
        hash: f.y.2,
    };
    let r = identity_chain(&f.rid, &f.peers, &obs, Some(&ahead));
    assert_eq!(
        r.map(|_| ()).map_err(|e| e.code()),
        Err(ErrorCode::StaleIdentity)
    );
    let wrong = IdMark {
        rev: 0,
        hash: [5; 32],
    };
    let r = identity_chain(&f.rid, &f.peers, &obs, Some(&wrong));
    assert_eq!(
        r.map(|_| ()).map_err(|e| e.code()),
        Err(ErrorCode::StaleIdentity)
    );
    let ok = IdMark {
        rev: 0,
        hash: f.rid,
    };
    assert!(identity_chain(&f.rid, &f.peers, &obs, Some(&ok)).is_ok());
}

#[test]
fn expired_newer_revision_fails_closed_via_completeness() {
    let f = fx();
    let mut created = base();
    created.push(place(&f.acc[0][1], 5, 0));
    let served: Vec<Observed> = created
        .iter()
        .filter(|o| o.key != f.y.1.key)
        .cloned()
        .collect();
    let c1 = control(1);
    let count = |s: &[Observed]| s.iter().filter(|o| o.creator == c1).count() as u64;
    let obs = CompletenessObs {
        address: c1,
        nonce_before: count(&created),
        live: count(&served),
        nonce_after: count(&created),
    };
    let err = check_completeness(&peers()[1].nid, &obs).expect_err("not live");
    assert_eq!(err.code(), ErrorCode::ControlNotLive);
}

#[test]
fn entities_failing_the_creator_rule_are_ignored() {
    let f = fx();
    let mut obs = base();
    let mut foreign = place(&f.acc[0][1], 5, 0);
    foreign.creator = [0xf0; 20];
    obs.push(foreign);
    assert_eq!(head_rev(&obs), Ok((0, f.rid)));
}

#[test]
fn unsupported_id_version_from_tracked_control_fails_closed() {
    let f = fx();
    let mut future = place(&f.x.1, 50, 0);
    future.attributes.iter_mut().for_each(|a| {
        if a.name == "ag.v" {
            a.value = AttrValue::U64(9);
        }
    });
    let mut obs = base();
    obs.push(future.clone());
    assert_eq!(head_rev(&obs), Err(ErrorCode::UnsupportedVersion));
    future.creator = [0xf0; 20];
    let mut obs = base();
    obs.push(future);
    assert!(head_rev(&obs).is_ok());
}

/// Reference model: position at which `target` gets `maj(D)` distinct delegate acceptors.
fn reference_q(
    target: &Observed,
    signer: usize,
    accs: &[(usize, Observed)],
    d: &[usize],
) -> Option<Position> {
    let mut events: Vec<(Position, usize)> = vec![(target.position(), signer)];
    events.extend(accs.iter().map(|(p, o)| (o.position(), *p)));
    events.sort();
    let mut seen = std::collections::BTreeSet::new();
    events
        .into_iter()
        .filter(|(_, p)| d.contains(p))
        .find_map(|(pos, p)| (seen.insert(p) && seen.len() == maj(d.len())).then_some(pos))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]

    #[test]
    fn sibling_choice_matches_reference_and_ignores_delivery_order(
        picks in proptest::collection::vec((0usize..5, 0usize..2, 4u64..60, 0u32..4), 0..10),
        seed in any::<u64>(),
    ) {
        let f = fx();
        let mut accs: [Vec<(usize, Observed)>; 2] = [vec![], vec![]];
        for (i, (p, t, b, tx)) in picks.iter().enumerate() {
            accs[*t].push((*p, place(&f.acc[*p][*t], *b, tx * 16 + i as u32)));
        }
        let d = [0usize, 1, 2];
        let p0 = f.root_acc.position();
        let effective = |o: &Observed, s: usize, a: &[(usize, Observed)]| {
            reference_q(o, s, a, &d).map(|q| (q.max(p0), o.position()))
        };
        let ex = effective(&f.x.1, 0, &accs[0]);
        let ey = effective(&f.y.1, 1, &accs[1]);
        let expected = match (ex, ey) {
            (None, None) => f.rid,
            (Some(_), None) => f.x.2,
            (None, Some(_)) => f.y.2,
            (Some(a), Some(b)) => if a < b { f.x.2 } else { f.y.2 },
        };
        let mut obs = base();
        obs.extend(accs.iter().flatten().map(|(_, o)| o.clone()));
        let got = head_rev(&obs).map(|r| r.1);
        prop_assert_eq!(got, Ok(expected));
        // any delivery order gives the same head
        let mut shuffled = obs.clone();
        let n = shuffled.len();
        for i in 0..n {
            let j = ((seed.rotate_left(i as u32) ^ i as u64) as usize) % n;
            shuffled.swap(i, j);
        }
        prop_assert_eq!(head_rev(&shuffled).map(|r| r.1), Ok(expected));
    }
}

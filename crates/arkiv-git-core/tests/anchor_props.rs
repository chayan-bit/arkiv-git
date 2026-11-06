//! Design section 8.2 property tests for the anchor chain and head acceptance.
//!
//! An honest one-peer history (binds with an EVM rotation, root identity, refs, anchors) is
//! signed once; each case lays out a random adversarial schedule over it: replays by a
//! foreign address and by the peer's own control address, forks, shuffled delivery, expiry
//! of control-plane entities, and withholding of the newest anchors with a truthful or
//! lying `entityNonce`.

mod common;

use arkiv_git_core::*;
use common::history::*;
use proptest::prelude::*;

#[derive(Debug, Clone)]
struct Schedule {
    n: usize,
    foreign_replays: Vec<usize>,
    own_replays: Vec<usize>,
    forks: bool,
    expire: Vec<usize>,
    withhold: usize,
    nonce_truthful: bool,
    perm_seed: u64,
}

fn schedule() -> impl Strategy<Value = Schedule> {
    (1usize..=PUSHES).prop_flat_map(|n| {
        (
            Just(n),
            proptest::collection::vec(0..n, 0..4),
            proptest::collection::vec(0..n, 0..3),
            any::<bool>(),
            proptest::collection::vec(0usize..64, 0..3),
            0usize..=n.min(3),
            any::<bool>(),
            any::<u64>(),
        )
            .prop_map(
                |(n, fr, or, forks, expire, withhold, nonce_truthful, perm_seed)| Schedule {
                    n,
                    foreign_replays: fr,
                    own_replays: or,
                    forks,
                    expire,
                    withhold,
                    nonce_truthful,
                    perm_seed,
                },
            )
    })
}

struct Laid {
    /// Every entity created on chain (honest, own replays, forks, foreign copies).
    created: Vec<Observed>,
    /// What the node serves.
    served: Vec<Observed>,
    /// Keys of honest control-plane entities removed by expiry.
    expired_control: usize,
}

fn honest_control_keys(n: usize) -> Vec<Hash32> {
    honest(n)
        .into_iter()
        .filter(|o| controls(n).contains(&o.creator))
        .map(|o| o.key)
        .collect()
}

fn foreign_copies(s: &Schedule) -> Vec<Observed> {
    let w = world();
    let mut v = vec![];
    for (i, k) in s.foreign_replays.iter().enumerate() {
        let b = 2000 + i as u64;
        v.push(relocate(&w.anchors[*k].0, FOREIGN, b, 0));
        v.push(relocate(&w.refs[*k].0, FOREIGN, b, 1));
        v.push(relocate(&w.binds[0], FOREIGN, b, 2));
    }
    v
}

fn lay(s: &Schedule, with_foreign: bool) -> Laid {
    let w = world();
    let mut created = honest(s.n);
    let last_control = *controls(s.n).last().expect("control");
    for (i, k) in s.own_replays.iter().enumerate() {
        created.push(relocate(&w.anchors[*k].0, last_control, 3000 + i as u64, 0));
    }
    if s.forks && s.n > ROTATE_AT {
        created.extend(w.forks.iter().filter(|f| seq_of(f) < s.n as u64).cloned());
    }
    if with_foreign {
        created.extend(foreign_copies(s));
    }
    let control_keys = honest_control_keys(s.n);
    let expired: Vec<Hash32> = s
        .expire
        .iter()
        .map(|i| control_keys[i % control_keys.len()])
        .collect();
    let withheld: Vec<Hash32> = (s.n - s.withhold..s.n)
        .map(|k| w.anchors[k].0.key)
        .collect();
    let mut served: Vec<Observed> = created
        .iter()
        .filter(|o| !expired.contains(&o.key) && !withheld.contains(&o.key))
        .cloned()
        .collect();
    shuffle(&mut served, s.perm_seed);
    let expired_control = expired
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    Laid {
        created,
        served,
        expired_control,
    }
}

fn seq_of(o: &Observed) -> u64 {
    match o.attr("ag.seq") {
        Some(AttrValue::U64(s)) => *s,
        _ => u64::MAX,
    }
}

fn shuffle(v: &mut [Observed], seed: u64) {
    let mut x = seed | 1;
    for i in (1..v.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

fn outcome(s: &Schedule, laid: &Laid, mark: Option<&PeerMark>) -> Outcome {
    let comp = completeness(
        &controls(s.n),
        &laid.created,
        &laid.served,
        s.nonce_truthful,
    );
    run(&laid.served, &comp, mark)
}

fn honest_hashes(n: usize) -> Vec<Hash32> {
    world().anchors[..n].iter().map(|a| a.1).collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// 1. Soundness: any returned head heads a prefix of the honest chain.
    #[test]
    fn soundness(s in schedule()) {
        let laid = lay(&s, true);
        if let Ok(Some((head, chain, _))) = outcome(&s, &laid, None) {
            let honest = honest_hashes(s.n);
            prop_assert_eq!(&chain[..], &honest[..chain.len()]);
            prop_assert_eq!(Some(&head), chain.last());
        }
    }

    /// 2. Freshness: everything live and served gives exactly the honest newest anchor.
    #[test]
    fn freshness(mut s in schedule()) {
        s.expire.clear();
        s.withhold = 0;
        let laid = lay(&s, true);
        let (head, ..) = outcome(&s, &laid, None).expect("ok").expect("head");
        prop_assert_eq!(head, world().anchors[s.n - 1].1);
    }

    /// 3. Fail closed: a missing honest control entity with a truthful nonce never yields a head.
    #[test]
    fn fail_closed(mut s in schedule()) {
        s.nonce_truthful = true;
        s.withhold = 0;
        prop_assume!(!s.expire.is_empty());
        let laid = lay(&s, true);
        prop_assert!(laid.expired_control > 0);
        let r = outcome(&s, &laid, None);
        prop_assert!(
            // E_ID_NOT_LIVE: bind 0 itself expired, so no control set is known and the
            // root identity revision fails the creator rule.
            matches!(
                r,
                Err(ErrorCode::ControlNotLive) | Err(ErrorCode::ChainGap) | Err(ErrorCode::IdNotLive)
            ),
            "got {:?}", r
        );
    }

    /// 3b. Withholding with a truthful nonce also fails closed.
    #[test]
    fn withholding_with_truthful_nonce_fails_closed(mut s in schedule()) {
        s.nonce_truthful = true;
        s.expire.clear();
        prop_assume!(s.withhold > 0);
        let laid = lay(&s, true);
        let r = outcome(&s, &laid, None);
        prop_assert!(matches!(r, Err(ErrorCode::ControlNotLive)), "got {:?}", r);
    }

    /// 4. Monotonicity: with a mark at seq w the result is never a head below w.
    #[test]
    fn monotonicity(s in schedule(), w in 0usize..PUSHES) {
        let w = w % s.n;
        let mark = PeerMark { seq: w as u64, hash: world().anchors[w].1, block: push_block(w) };
        let laid = lay(&s, true);
        match outcome(&s, &laid, Some(&mark)) {
            Ok(Some((_, chain, _))) => prop_assert!(chain.len() > w),
            Ok(None) => prop_assert!(false, "no head despite a mark"),
            Err(_) => {}
        }
    }

    /// 5. Determinism: two delivery orders give the same head and equivocation set.
    #[test]
    fn determinism(s in schedule(), other in any::<u64>()) {
        let a = lay(&s, true);
        let mut b = lay(&s, true);
        shuffle(&mut b.served, other);
        prop_assert_eq!(outcome(&s, &a, None), outcome(&s, &b, None));
    }

    /// 6. Creator rule: entities created outside the bind chain never change the result.
    #[test]
    fn creator_rule(s in schedule()) {
        let with = lay(&s, true);
        let without = lay(&s, false);
        prop_assert_eq!(outcome(&s, &with, None), outcome(&s, &without, None));
    }
}

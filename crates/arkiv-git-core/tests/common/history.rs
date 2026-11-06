//! An honest one-peer history laid out in creation order, signed once and cached.
#![allow(dead_code)]

use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub const CONTROL0: Address = [0xc0; 20];
pub const BULK0: Address = [0xb0; 20];
pub const CONTROL1: Address = [0xc1; 20];
pub const BULK1: Address = [0xb1; 20];
pub const FOREIGN: Address = [0xf0; 20];
pub const PUSHES: usize = 8;
/// Bind seq 1 (rotation to CONTROL1/BULK1) lands just before push ROTATE_AT.
pub const ROTATE_AT: usize = 4;

pub fn push_block(k: usize) -> u64 {
    10 + 10 * k as u64
}

pub fn rotation_block() -> u64 {
    push_block(ROTATE_AT) - 5
}

pub fn control_for(k: usize) -> Address {
    if k >= ROTATE_AT { CONTROL1 } else { CONTROL0 }
}

pub fn bulk_for(k: usize) -> Address {
    if k >= ROTATE_AT { BULK1 } else { BULK0 }
}

pub struct World {
    pub rid: Rid,
    pub peer_obs: Observed,
    pub root_obs: Observed,
    pub root_h: Hash32,
    pub binds: Vec<Observed>,
    pub refs: Vec<(Observed, Hash32)>,
    pub anchors: Vec<(Observed, Hash32)>,
    /// Validly signed forks: seq 2 with other refs (right prev), seq 4 with a wrong prev.
    pub forks: Vec<Observed>,
}

fn build() -> World {
    let p = alice();
    let root = root_body(vec![p.nid], 1);
    let (root_bytes, rid) = sign(p, &root);
    let root_obs = observe_bytes(&root_bytes, &root, CONTROL0, 1, 2);
    let peer_obs = observe(p, &peer_body(p), CONTROL0, 1, 0).0;
    let b0 = BindBody {
        rid,
        ..bind_body(0, ZERO, CONTROL0, BULK0)
    };
    let (b0_obs, b0_h) = observe(p, &b0, CONTROL0, 1, 1);
    let b1 = BindBody {
        rid,
        ..bind_body(1, b0_h, CONTROL1, BULK1)
    };
    let b1_obs = observe(p, &b1, CONTROL0, rotation_block(), 0).0;
    let mut refs = vec![];
    let mut anchors = vec![];
    let mut prev = ZERO;
    for k in 0..PUSHES {
        let rb = RefsBody {
            rid,
            ..refs_body(k as u8 + 1)
        };
        let (r_obs, r_h) = observe(p, &rb, bulk_for(k), push_block(k), 0);
        let ab = AnchorBody {
            rid,
            ..anchor_body(k as u64, prev, r_h, rid, push_block(k) - 1)
        };
        let (a_obs, a_h) = observe(p, &ab, control_for(k), push_block(k), 1);
        refs.push((r_obs, r_h));
        anchors.push((a_obs, a_h));
        prev = a_h;
    }
    let fork2 = AnchorBody {
        rid,
        ..anchor_body(2, anchors[1].1, refs[7].1, rid, 5)
    };
    let fork4 = AnchorBody {
        rid,
        ..anchor_body(4, [0xee; 32], refs[0].1, rid, 5)
    };
    let forks = vec![
        observe(p, &fork2, CONTROL1, 1000, 0).0,
        observe(p, &fork4, CONTROL1, 1001, 0).0,
    ];
    World {
        rid,
        peer_obs,
        root_obs,
        root_h: rid,
        binds: vec![b0_obs, b1_obs],
        refs,
        anchors,
        forks,
    }
}

pub fn world() -> &'static World {
    static W: OnceLock<World> = OnceLock::new();
    W.get_or_init(build)
}

/// A copy of `o` created by `creator` at a new position (signature untouched).
pub fn relocate(o: &Observed, creator: Address, block: u64, tx: u32) -> Observed {
    let mut c = o.clone();
    c.creator = creator;
    c.block = block;
    c.tx = tx;
    c.key[..8].copy_from_slice(&block.to_be_bytes());
    c.key[8..12].copy_from_slice(&tx.to_be_bytes());
    c.key[12..20].copy_from_slice(&creator[..8]);
    c
}

/// Completeness observations for `controls`, with `created` counting every entity each
/// control address ever created and `served` what the (possibly withholding) node returns.
pub fn completeness(
    controls: &[Address],
    created: &[Observed],
    served: &[Observed],
    nonce_truthful: bool,
) -> Vec<CompletenessObs> {
    let count =
        |set: &[Observed], a: &Address| set.iter().filter(|o| o.creator == *a).count() as u64;
    controls
        .iter()
        .map(|a| {
            let live = count(served, a);
            let nonce = if nonce_truthful {
                count(created, a)
            } else {
                live
            };
            CompletenessObs {
                address: *a,
                nonce_before: nonce,
                live,
                nonce_after: nonce,
            }
        })
        .collect()
}

pub fn peer_info(binds: &[Observed]) -> PeerInfo {
    let w = world();
    let pk = alice().pk.clone();
    PeerInfo {
        binds: bind_chain(&w.rid, &pk, binds).expect("bind chain"),
        pk,
    }
}

pub fn peers_map(info: PeerInfo) -> BTreeMap<Nid, PeerInfo> {
    BTreeMap::from([(alice().nid, info)])
}

pub type Outcome =
    Result<Option<(Hash32, Vec<Hash32>, BTreeSet<(u64, Hash32, Hash32)>)>, ErrorCode>;

/// The full core pipeline for one peer over `served` entities (any order).
pub fn run(served: &[Observed], comp: &[CompletenessObs], mark: Option<&PeerMark>) -> Outcome {
    let w = world();
    let of = |k: &str| -> Vec<Observed> {
        served
            .iter()
            .filter(|o| o.attr("ag.kind") == Some(&AttrValue::Str(k.into())))
            .cloned()
            .collect()
    };
    // Any valid copy of a peer document gives the same key (nid commits to it); another
    // repo's copy stays reachable when this repo's copy is gone.
    let mut peer_docs = of("peer");
    peer_docs.push(relocate(&w.peer_obs, FOREIGN, 0, 0));
    let pk = peer_key(&alice().nid, &peer_docs).ok_or(ErrorCode::NotLive)?;
    let binds = bind_chain(&w.rid, &pk, &of("bind")).map_err(|e| e.code())?;
    check_control_set(&alice().nid, &binds.control_set(), comp).map_err(|e| e.code())?;
    let peer = PeerInfo { pk, binds };
    let ids = of("id");
    let peers = peers_map(peer.clone());
    let identity = identity_chain(&w.rid, &peers, &ids, None).map_err(|e| e.code())?;
    let chain = anchor_chain(
        &w.rid,
        &alice().nid,
        &peer,
        &identity,
        &of("anchor"),
        Start::Genesis,
    )
    .map_err(|e| e.code())?;
    let applied = BTreeSet::new();
    let inputs = HeadInputs {
        completeness: comp,
        mark,
        refs: &of("refs"),
        manifests: &of("manifest"),
        applied: &applied,
    };
    let head = accept_head(&w.rid, &peer, &chain, &inputs).map_err(|e| e.code())?;
    let eq = chain
        .equivocations
        .iter()
        .map(|e| (e.seq, e.keys[0], e.keys[1]))
        .collect();
    Ok(head.map(|h| (h.anchor.h, chain.links.iter().map(|l| l.h).collect(), eq)))
}

/// Every honest entity of the first `n` pushes, in creation order.
pub fn honest(n: usize) -> Vec<Observed> {
    let w = world();
    let mut v = vec![w.peer_obs.clone(), w.binds[0].clone(), w.root_obs.clone()];
    if n > ROTATE_AT {
        v.push(w.binds[1].clone());
    }
    for k in 0..n {
        v.push(w.refs[k].0.clone());
        v.push(w.anchors[k].0.clone());
    }
    v
}

pub fn controls(n: usize) -> Vec<Address> {
    if n > ROTATE_AT {
        vec![CONTROL0, CONTROL1]
    } else {
        vec![CONTROL0]
    }
}

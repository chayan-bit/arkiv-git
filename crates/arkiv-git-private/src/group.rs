//! The private-repo API that the CLI and, later, repo programs (through obligations) call:
//! create, key packages, add and remove members (each an MLS commit, so a new epoch and a
//! new content key), and epoch-key publication or export for sponsorware and escrow.

use arkiv_git_core::{Body, ErrorCode, Hash32, IdBody, Nid, Rid, ZERO};
use arkiv_git_store::{Arkiv, AttributeValue};
use arkiv_git_sync::identity::RepoMeta;
use arkiv_git_sync::push_send::first_write_ops;
use arkiv_git_sync::write::{self, entity_op, sign};
use arkiv_git_sync::{Config, Keys, Result, SyncError, observe};

use crate::arkiv::Sealed;
use crate::join::refresh;
use crate::keyring::{Keyring, Role, not_member};
use crate::mls::{Mls, SUITE};
use crate::msg::{self, T_COMMIT, T_GROUP, T_KP, T_PUB, T_WELCOME, b32, blinded_rid, tag};
use crate::seal::blind;

/// `id_key || content key`: everything needed to find and open one epoch's entities.
pub type Grant = Vec<u8>;

/// Publishes a fresh key package from the wallet; returns its id for `member add`.
pub async fn publish_key_package<A: Arkiv>(a: &A, cfg: &Config, keys: &Keys) -> Result<Hash32> {
    let mut mls = Mls::open(&cfg.home, keys.nid())?;
    let kp = mls.key_package()?;
    let id = arkiv_git_crypto::sha256(&kp);
    mls.kps.push(id);
    mls.save()?;
    let op = msg::op(
        T_KP,
        vec![("ag.h", b32(id), true)],
        kp,
        cfg.control_lifetime,
    )?;
    write::send(a, &keys.wallet(), &cfg.pipeline, vec![vec![op]]).await?;
    Ok(id)
}

/// Creates a private repo with this peer as its single delegate and only (admin) member.
pub async fn init<A: Arkiv>(a: &A, cfg: &Config, keys: &Keys, meta: &RepoMeta) -> Result<Rid> {
    let root = IdBody {
        rid: None,
        rev: 0,
        parent: ZERO,
        name: meta.name.clone(),
        description: meta.description.clone(),
        default_branch: meta.default_branch.clone(),
        delegates: vec![keys.nid()],
        threshold: 1,
        visibility: 1,
        lang: meta.lang.clone(),
        license: meta.license.clone(),
        nonce: Some(arkiv_git_store::random_salt().to_be_bytes()),
    };
    root.validate()?;
    let s = sign(keys.sk(), &root)?;
    let rid = s.h;
    let mls = Mls::open(&cfg.home, keys.nid())?;
    let g = mls.create()?;
    let (epoch, key) = mls.epoch_key(&g, &rid)?;
    let ring = Keyring {
        rid,
        id_key: mls.id_key(&g, &rid)?,
        epochs: [(epoch, key)].into(),
        roles: [(hex::encode(keys.nid()), Role::Admin)].into(),
        group: g.group_id().as_slice().to_vec(),
    };
    let mut ops = first_write_ops(keys, &rid, cfg)?;
    ops.push(entity_op::<IdBody>(&s.bytes, cfg.control_lifetime)?);
    let suite = u16::from(SUITE).to_be_bytes().to_vec();
    let marker = vec![("ag.tag", b32(tag(&rid)), true)];
    ops.push(msg::op(T_GROUP, marker, suite, cfg.control_lifetime)?);
    let sealed = Sealed::new(a, ring.clone());
    write::send_funded(
        &sealed,
        &keys.control(&rid),
        &keys.wallet(),
        &cfg.pipeline,
        vec![ops],
    )
    .await?;
    mls.save()?;
    ring.save(&cfg.home)?;
    Ok(rid)
}

/// This member's current keyring, or `E_NOT_MEMBER`.
pub async fn member_ring<A: Arkiv>(
    a: &A,
    cfg: &Config,
    keys: &Keys,
    rid: &Rid,
) -> Result<(Mls, Keyring)> {
    match refresh(a, cfg, keys, rid).await? {
        (mls, Some(ring)) => Ok((mls, ring)),
        _ => Err(not_member(rid)),
    }
}

/// Admin only: adds the owner of key package `kp_id` with `role`; returns their nid.
pub async fn add_member<A: Arkiv>(
    a: &A,
    cfg: &Config,
    keys: &Keys,
    rid: &Rid,
    kp_id: &Hash32,
    role: Role,
) -> Result<Nid> {
    let (mls, mut ring) = member_ring(a, cfg, keys, rid).await?;
    ring.require(&keys.nid(), Role::Admin)?;
    let found = msg::find(a, T_KP, vec![observe::b32("ag.h", kp_id)]).await?;
    let kp = found
        .iter()
        .find_map(|o| msg::bytes(o, "ag.body").filter(|b| arkiv_git_crypto::sha256(b) == *kp_id))
        .ok_or_else(|| SyncError::coded(ErrorCode::NotLive, "no live key package with that id"))?;
    let mut g = mls.load(&ring.group)?;
    let (commit, welcome, nid) = mls.add(&mut g, kp)?;
    ring.roles.insert(hex::encode(nid), role);
    let (epoch, key) = mls.epoch_key(&g, rid)?;
    ring.epochs.insert(epoch, key);
    publish_commit(a, cfg, keys, &ring, commit, Some((*kp_id, welcome))).await?;
    mls.save()?;
    ring.save(&cfg.home)?;
    Ok(nid)
}

/// Admin only: removes `nid`. The commit starts a new epoch whose key `nid` cannot derive.
pub async fn remove_member<A: Arkiv>(
    a: &A,
    cfg: &Config,
    keys: &Keys,
    rid: &Rid,
    nid: &Nid,
) -> Result<()> {
    let (mls, mut ring) = member_ring(a, cfg, keys, rid).await?;
    ring.require(&keys.nid(), Role::Admin)?;
    let mut g = mls.load(&ring.group)?;
    let commit = mls.remove(&mut g, nid)?;
    ring.roles.remove(&hex::encode(nid));
    let (epoch, key) = mls.epoch_key(&g, rid)?;
    ring.epochs.insert(epoch, key);
    publish_commit(a, cfg, keys, &ring, commit, None).await?;
    mls.save()?;
    ring.save(&cfg.home)
}

/// One tx from the admin's control address: the commit, and the Welcome if any, both
/// carrying the keyring sealed under the new epoch key.
async fn publish_commit<A: Arkiv>(
    a: &A,
    cfg: &Config,
    keys: &Keys,
    ring: &Keyring,
    commit: Vec<u8>,
    welcome: Option<(Hash32, Vec<u8>)>,
) -> Result<()> {
    let (epoch, key) = ring.current()?;
    let bx = msg::seal_box(ring, epoch, key)?;
    let ep = || ("ag.ep", AttributeValue::U64(epoch), true);
    let lockbox = || ("ag.box", AttributeValue::Bytes(bx.clone()), false);
    let rid_attr = ("ag.rid", b32(blinded_rid(ring)), true);
    let mut ops = vec![msg::op(
        T_COMMIT,
        vec![rid_attr, ep(), lockbox()],
        commit,
        cfg.control_lifetime,
    )?];
    if let Some((kp, w)) = welcome {
        let attrs = vec![("ag.kp", b32(kp), true), ep(), lockbox()];
        ops.push(msg::op(T_WELCOME, attrs, w, cfg.control_lifetime)?);
    }
    let control = keys.control(&ring.rid);
    write::send_funded(a, &control, &keys.wallet(), &cfg.pipeline, vec![ops]).await?;
    Ok(())
}

/// Admin only: publishes epoch `epoch`'s grant in plaintext, so anyone who knows the rid
/// can read what was sealed in that epoch (sponsorware, source escrow release).
pub async fn publish_epoch_key<A: Arkiv>(
    a: &A,
    cfg: &Config,
    keys: &Keys,
    rid: &Rid,
    epoch: u64,
) -> Result<Grant> {
    let (_, ring) = member_ring(a, cfg, keys, rid).await?;
    ring.require(&keys.nid(), Role::Admin)?;
    let grant = grant(&ring, epoch)?;
    let attrs = vec![
        ("ag.tag", b32(tag(rid)), true),
        ("ag.ep", AttributeValue::U64(epoch), true),
    ];
    let op = msg::op(T_PUB, attrs, grant.clone(), cfg.control_lifetime)?;
    write::send_funded(
        a,
        &keys.control(rid),
        &keys.wallet(),
        &cfg.pipeline,
        vec![vec![op]],
    )
    .await?;
    Ok(grant)
}

/// Epoch `epoch`'s grant from the local keyring, for an escrow agent or a timelock.
pub fn export_epoch_key(home: &std::path::Path, rid: &Rid, epoch: u64) -> Result<Grant> {
    grant(
        &Keyring::load(home, rid)?.ok_or_else(|| not_member(rid))?,
        epoch,
    )
}

fn grant(ring: &Keyring, epoch: u64) -> Result<Grant> {
    let key = ring.epochs.get(&epoch).ok_or_else(|| {
        SyncError::coded(
            ErrorCode::NotMember,
            format!("no key held for epoch {epoch}"),
        )
    })?;
    Ok([ring.id_key.as_slice(), key].concat())
}

/// `HMAC(name)` as stored in `ag.name`, for tests and tooling.
pub fn blinded_name(ring: &Keyring, name: &str) -> Hash32 {
    blind(&ring.id_key, "name", name.as_bytes())
}

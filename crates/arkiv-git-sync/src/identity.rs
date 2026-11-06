//! Identity writes (design 3.5): `init` (root revision plus `peer` and `bind_0` in one
//! control tx), revision proposals and acceptances. A writer with no bind for the repo gets
//! `peer` and `bind_0` in the same control tx.

use arkiv_git_core::{Body, IdAccBody, IdBody, IdentityChain, Nid, Rid, ZERO};
use arkiv_git_store::{Arkiv, Operation};

use crate::keys::Keys;
use crate::push_send::first_write_ops;
use crate::repo::Config;
use crate::state::State;
use crate::write::{self, entity_op, sign};
use crate::{Result, view, warn};

/// Repo metadata of a root revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoMeta {
    pub name: String,
    pub description: String,
    pub default_branch: String,
    pub lang: Option<String>,
    pub license: Option<String>,
}

fn nonce() -> [u8; 16] {
    arkiv_git_store::random_salt().to_be_bytes()
}

/// Sends `ops` in one tx from the control address, topped up from the wallet if short.
async fn send_control<A: Arkiv>(
    a: &A,
    cfg: &Config,
    keys: &Keys,
    rid: &Rid,
    ops: Vec<Operation>,
) -> Result<()> {
    write::send_funded(
        a,
        &keys.control(rid),
        &keys.wallet(),
        &cfg.pipeline,
        vec![ops],
    )
    .await?;
    Ok(())
}

/// Creates a repo with this peer as its single delegate; returns the rid.
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
        visibility: 0,
        lang: meta.lang.clone(),
        license: meta.license.clone(),
        nonce: Some(nonce()),
        program: None,
    };
    root.validate()?;
    let s = sign(keys.sk(), &root)?;
    let rid = s.h;
    let mut ops = first_write_ops(keys, &rid, cfg)?;
    ops.push(entity_op::<IdBody>(&s.bytes, cfg.control_lifetime)?);
    send_control(a, cfg, keys, &rid, ops).await?;
    warn(
        "single-delegate repo: losing this key loses the repo (add delegates with `arkiv-git id propose`)",
    );
    Ok(rid)
}

/// The verified identity chain of `rid`.
pub async fn show<A: Arkiv>(a: &A, rid: &Rid, state: &State) -> Result<IdentityChain> {
    Ok(view::identity(a, rid, state).await?.0)
}

pub(crate) async fn with_bind<A: Arkiv>(
    a: &A,
    cfg: &Config,
    keys: &Keys,
    rid: &Rid,
    op: Operation,
) -> Result<()> {
    let bound = view::resolve_peer(a, rid, &keys.nid())
        .await?
        .is_some_and(|p| !p.binds.links.is_empty());
    let mut ops = match bound {
        true => vec![],
        false => first_write_ops(keys, rid, cfg)?,
    };
    ops.push(op);
    send_control(a, cfg, keys, rid, ops).await
}

/// Proposes the next revision, produced by `edit` from the current head body; returns its
/// `h`. The proposer's own envelope counts as its acceptance.
pub async fn propose<A: Arkiv>(
    a: &A,
    cfg: &Config,
    keys: &Keys,
    rid: &Rid,
    state: &State,
    edit: impl FnOnce(&IdBody) -> IdBody,
) -> Result<arkiv_git_core::Hash32> {
    let chain = show(a, rid, state).await?;
    let head = chain.head();
    let mut body = edit(&head.body);
    body.rid = Some(*rid);
    body.rev = head.body.rev + 1;
    body.parent = head.h;
    body.nonce = None;
    body.delegates.sort();
    body.delegates.dedup();
    body.validate()?;
    let s = sign(keys.sk(), &body)?;
    with_bind(
        a,
        cfg,
        keys,
        rid,
        entity_op::<IdBody>(&s.bytes, cfg.control_lifetime)?,
    )
    .await?;
    Ok(s.h)
}

/// Accepts revision `rev` of `rid` as a delegate.
pub async fn accept<A: Arkiv>(
    a: &A,
    cfg: &Config,
    keys: &Keys,
    rid: &Rid,
    rev: &arkiv_git_core::Hash32,
) -> Result<()> {
    let body = IdAccBody {
        rid: *rid,
        rev: *rev,
    };
    let s = sign(keys.sk(), &body)?;
    with_bind(
        a,
        cfg,
        keys,
        rid,
        entity_op::<IdAccBody>(&s.bytes, cfg.control_lifetime)?,
    )
    .await
}

/// Short display form of a delegate list.
pub fn delegates_text(d: &[Nid]) -> String {
    d.iter()
        .map(crate::ids::to_text)
        .collect::<Vec<_>>()
        .join(", ")
}

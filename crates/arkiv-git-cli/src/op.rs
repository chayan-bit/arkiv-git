//! `arkiv-git op`: post and list collaborative-object ops (docs/p3/cob-ops.md).

use arkiv_git_core::{CobOpBody, Hash32, VerifiedOp, ZERO};
use arkiv_git_store::Arkiv;
use arkiv_git_sync::cob::{self, json};
use arkiv_git_sync::ids::{hex32, parse_id, to_text};
use arkiv_git_sync::{Config, Keys};
use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::cmd::rid_of;
use crate::{CliError, Result};

#[derive(Debug, Subcommand)]
pub enum OpCmd {
    /// Sign and write one op; prints its id.
    Post(PostArgs),
    /// Print the verified ops of a repo in Arkiv creation order.
    List(ListArgs),
}

#[derive(Debug, Args)]
pub struct PostArgs {
    #[arg(long)]
    pub repo: String,
    /// Object type, e.g. `xyz.arkiv.issue`.
    #[arg(long = "type")]
    pub type_name: String,
    /// `create` starts a new object; anything else needs `--object`.
    #[arg(long)]
    pub action: String,
    #[arg(long)]
    pub object: Option<String>,
    /// Ids of earlier ops of the same object this op saw.
    #[arg(long = "parent")]
    pub parents: Vec<String>,
    /// Payload as JSON (no floats); stored as canonical CBOR.
    #[arg(long)]
    pub payload_json: String,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    #[arg(long)]
    pub repo: String,
    #[arg(long)]
    pub object: Option<String>,
    #[arg(long = "type")]
    pub type_name: Option<String>,
    /// One JSON array instead of one line per op.
    #[arg(long)]
    pub json: bool,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn body(p: &PostArgs) -> Result<CobOpBody> {
    let v: Value = serde_json::from_str(&p.payload_json)
        .map_err(|e| CliError::Usage(format!("--payload-json: {e}")))?;
    Ok(CobOpBody {
        rid: rid_of(&p.repo)?,
        object: p
            .object
            .as_deref()
            .map(parse_id)
            .transpose()?
            .unwrap_or(ZERO),
        type_name: p.type_name.clone(),
        action: p.action.clone(),
        parents: p
            .parents
            .iter()
            .map(|s| parse_id(s))
            .collect::<Result<_, _>>()?,
        payload: json::to_cbor(&v)?,
        ts_ms: now_ms(),
    })
}

fn view(o: &VerifiedOp) -> Result<Value> {
    let hexes = |v: &[Hash32]| v.iter().map(hex32).collect::<Vec<_>>();
    Ok(json!({
        "id": hex32(&o.id),
        "object": hex32(&o.object()),
        "author": to_text(&o.author),
        "type": o.body.type_name,
        "action": o.body.action,
        "parents": hexes(&o.body.parents),
        "ts_ms": o.body.ts_ms,
        "block": o.arkiv_order.block,
        "payload": json::to_json(&o.body.payload)?,
    }))
}

pub async fn run<A: Arkiv>(a: &A, cfg: &Config, cmd: &OpCmd) -> Result<()> {
    match cmd {
        OpCmd::Post(p) => {
            let body = body(p)?;
            let id = cob::post(a, cfg, &Keys::require(&cfg.home)?, &body).await?;
            println!("{}", hex32(&id));
        }
        OpCmd::List(l) => list(a, l).await?,
    }
    Ok(())
}

async fn list<A: Arkiv>(a: &A, l: &ListArgs) -> Result<()> {
    let object = l.object.as_deref().map(parse_id).transpose()?;
    let rid = rid_of(&l.repo)?;
    let ops = cob::list(a, &rid, object.as_ref(), l.type_name.as_deref()).await?;
    let views = ops.iter().map(view).collect::<Result<Vec<_>>>()?;
    if l.json {
        println!("{}", Value::Array(views));
        return Ok(());
    }
    for (o, v) in ops.iter().zip(&views) {
        let b = &o.body;
        let (id, obj) = (hex32(&o.id), hex32(&o.object()));
        let author = to_text(&o.author);
        println!(
            "{id} {author} {} {} {obj} {}",
            b.type_name, b.action, v["payload"]
        );
    }
    Ok(())
}

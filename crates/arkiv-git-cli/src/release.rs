//! `arkiv-git release`: publish a signed release record and verify one (docs/p3/releases.md).
//! The op carries only names, hashes, sizes and locations; the bytes live on any mirror.

use std::path::{Path, PathBuf};
use std::time::Duration;

use arkiv_git_core::{CobOpBody, Hash32, Rid, ZERO};
use arkiv_git_store::Arkiv;
use arkiv_git_sync::cob::{self, json};
use arkiv_git_sync::program::proofs::sha256_hex;
use arkiv_git_sync::{Config, Keys, Repo, Want, sync};
use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::cmd::rid_of;
use crate::{CliError, Result};

pub const RELEASE_TYPE: &str = "xyz.arkiv.release";

#[derive(Debug, Subcommand)]
pub enum ReleaseCmd {
    /// Hash the artifacts and post one signed `xyz.arkiv.release` op; prints its id.
    Publish(PublishArgs),
    /// Refuse unless the pinned program says the release is valid, then download every
    /// artifact and check its sha256 and size.
    Verify(VerifyArgs),
}

#[derive(Debug, Args)]
pub struct PublishArgs {
    #[arg(long)]
    pub repo: String,
    #[arg(long)]
    pub tag: String,
    /// The released commit (hex oid).
    #[arg(long)]
    pub commit: String,
    /// `<name>=<path-or-url>[@<target>]`; a URL is downloaded to hash it and becomes a location.
    #[arg(long = "artifact", required = true)]
    pub artifacts: Vec<String>,
    /// `<name>=<url>`: another place the artifact can be downloaded from.
    #[arg(long = "location")]
    pub locations: Vec<String>,
    #[arg(long)]
    pub notes: Option<String>,
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
    #[arg(long)]
    pub repo: String,
    #[arg(long)]
    pub tag: String,
    /// Write each verified artifact here as `<name>`.
    #[arg(long)]
    pub download_dir: Option<PathBuf>,
}

impl ReleaseCmd {
    pub fn rid(&self) -> Result<Rid> {
        rid_of(match self {
            Self::Publish(p) => &p.repo,
            Self::Verify(v) => &v.repo,
        })
    }
}

fn usage(s: impl Into<String>) -> CliError {
    CliError::Usage(s.into())
}

/// A name that is safe as a file name in the download directory.
fn plain_name(n: &str) -> bool {
    !n.is_empty() && !n.starts_with('.') && !n.contains(['/', '\\', '\0'])
}

fn is_url(s: &str) -> bool {
    ["http://", "https://", "file://"]
        .iter()
        .any(|p| s.starts_with(p))
}

/// `<name>=<source>[@<target>]` as (name, source, target); a target has no `/` or `:`.
fn parse_spec(s: &str) -> Result<(&str, &str, &str)> {
    let (name, rest) = s
        .split_once('=')
        .filter(|(n, r)| plain_name(n) && !r.is_empty())
        .ok_or_else(|| {
            usage(format!(
                "--artifact {s}: want <name>=<path-or-url>[@<target>]"
            ))
        })?;
    Ok(match rest.rsplit_once('@') {
        Some((src, t)) if !src.is_empty() && !t.is_empty() && !t.contains(['/', ':']) => {
            (name, src, t)
        }
        _ => (name, rest, ""),
    })
}

/// The bytes at a `file://`, `http://` or `https://` location.
async fn fetch(url: &str) -> std::result::Result<Vec<u8>, String> {
    // shortcut: file:// paths are not percent-decoded and whole artifacts sit in memory;
    // stream to disk if artifacts grow past a few hundred MB.
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read(path).map_err(|e| e.to_string());
    }
    if !is_url(url) {
        return Err("unsupported location scheme".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).send().await;
    let resp = resp
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?;
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| e.to_string())
}

fn is_oid(s: &str) -> bool {
    matches!(s.len(), 40 | 64)
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

async fn artifacts(p: &PublishArgs) -> Result<Vec<Value>> {
    let mut out: Vec<Value> = Vec::new();
    for spec in &p.artifacts {
        let (name, src, target) = parse_spec(spec)?;
        if out.iter().any(|a| a["name"] == name) {
            return Err(usage(format!("artifact {name} given twice")));
        }
        let (bytes, mut locations) = match is_url(src) {
            true => (
                fetch(src).await.map_err(|e| usage(format!("{src}: {e}")))?,
                vec![src],
            ),
            false => (
                std::fs::read(src).map_err(|e| usage(format!("{src}: {e}")))?,
                vec![],
            ),
        };
        let more = p.locations.iter().filter_map(|l| l.split_once('='));
        locations.extend(more.filter(|(n, _)| *n == name).map(|(_, u)| u));
        if locations.is_empty() {
            return Err(usage(format!(
                "artifact {name} has no location; pass --location {name}=<url>"
            )));
        }
        out.push(json!({
            "name": name, "target": target, "sha256": sha256_hex(&bytes),
            "size": bytes.len(), "locations": locations,
        }));
    }
    for l in &p.locations {
        let name = l.split_once('=').map(|(n, _)| n);
        if !name.is_some_and(|n| out.iter().any(|a| a["name"] == n)) {
            return Err(usage(format!(
                "--location {l}: want <name>=<url> for a given artifact"
            )));
        }
    }
    Ok(out)
}

/// Hashes the artifacts and posts the release op.
pub async fn publish<A: Arkiv>(a: &A, cfg: &Config, p: &PublishArgs) -> Result<Hash32> {
    if p.tag.is_empty() || !is_oid(&p.commit) {
        return Err(usage(
            "--tag must be non-empty and --commit a lowercase hex oid",
        ));
    }
    let keys = Keys::require(&cfg.home)?;
    let mut payload = json!({ "tag": p.tag, "commit": p.commit, "artifacts": artifacts(p).await? });
    if let Some(n) = &p.notes {
        payload["notes"] = json!(n);
    }
    let body = CobOpBody {
        rid: rid_of(&p.repo)?,
        object: ZERO,
        type_name: RELEASE_TYPE.into(),
        action: "create".into(),
        parents: vec![],
        payload: json::to_cbor(&payload)?,
        ts_ms: crate::op::now_ms(),
    };
    Ok(cob::post(a, cfg, &keys, &body).await?)
}

/// Downloads `art` from its first working location; Ok is the line to print.
async fn check(art: &Value, dir: Option<&Path>) -> std::result::Result<String, String> {
    let name = art["name"]
        .as_str()
        .filter(|n| plain_name(n))
        .ok_or("unsafe artifact name")?;
    let (sha, size) = (art["sha256"].as_str().unwrap_or(""), art["size"].as_u64());
    let locations = art["locations"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for loc in locations.iter().filter_map(Value::as_str) {
        let bytes = match fetch(loc).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{name}: {loc}: {e}");
                continue;
            }
        };
        let got = sha256_hex(&bytes);
        if got != sha || Some(bytes.len() as u64) != size {
            return Err(format!(
                "MISMATCH {name} at {loc}: sha256 {got} size {}",
                bytes.len()
            ));
        }
        if let Some(d) = dir {
            std::fs::write(d.join(name), &bytes).map_err(|e| format!("{name}: {e}"))?;
        }
        return Ok(format!("OK {name} {sha} {} from {loc}", bytes.len()));
    }
    Err(format!("{name}: no location could be downloaded"))
}

/// Checks the release against the pinned program's state, then every artifact.
pub async fn verify<A: Arkiv>(a: &A, cfg: &Config, rid: &Rid, v: &VerifyArgs) -> Result<()> {
    let view = sync(a, &Repo::open(&cfg.home, rid)?, &Want::default()).await?;
    let e = view
        .program
        .as_ref()
        .ok_or_else(|| CliError::Flagged("no program pinned".into()))?;
    let state: Value =
        serde_json::from_str(&e.outcome.state_json).map_err(|e| usage(e.to_string()))?;
    let r = &state["releases"][&v.tag];
    if !r.is_object() {
        return Err(CliError::Flagged(format!(
            "no release {} in the program state",
            v.tag
        )));
    }
    if r["valid"] != true {
        let missing: Vec<&str> = r["missing"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        return Err(CliError::Flagged(format!(
            "release {} is not valid: needs {}",
            v.tag,
            missing.join("; ")
        )));
    }
    println!(
        "release {} at {}: valid under the pinned program",
        v.tag,
        r["commit"].as_str().unwrap_or("")
    );
    if let Some(d) = &v.download_dir {
        std::fs::create_dir_all(d).map_err(|e| usage(format!("{}: {e}", d.display())))?;
    }
    let mut bad = 0;
    for art in r["artifacts"].as_array().into_iter().flatten() {
        match check(art, v.download_dir.as_deref()).await {
            Ok(line) => println!("{line}"),
            Err(why) => {
                eprintln!("{why}");
                bad += 1;
            }
        }
    }
    match bad {
        0 => Ok(()),
        n => Err(CliError::Flagged(format!(
            "{n} artifact(s) failed verification"
        ))),
    }
}

pub async fn run<A: Arkiv>(a: &A, cfg: &Config, cmd: &ReleaseCmd) -> Result<()> {
    match cmd {
        ReleaseCmd::Publish(p) => {
            println!("{}", arkiv_git_sync::ids::hex32(&publish(a, cfg, p).await?))
        }
        ReleaseCmd::Verify(v) => verify(a, cfg, &cmd.rid()?, v).await?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_spec_splits_name_source_and_target() {
        let p = |s| parse_spec(s).expect("spec");
        assert_eq!(
            p("a=d/a.tgz@x86_64-unknown-linux-gnu"),
            ("a", "d/a.tgz", "x86_64-unknown-linux-gnu")
        );
        assert_eq!(p("a=https://h/a@b/c"), ("a", "https://h/a@b/c", ""));
        assert_eq!(p("a=https://u@h/a"), ("a", "https://u@h/a", ""));
        assert!(parse_spec("../a=x").is_err() && parse_spec("a=").is_err());
    }

    #[test]
    fn plain_names_cannot_escape_the_download_dir() {
        assert!(plain_name("arkiv-git-v0.1.0-x86_64.tar.gz"));
        for n in ["", ".", "..", "a/b", "a\\b", ".hidden"] {
            assert!(!plain_name(n), "{n}");
        }
    }
}

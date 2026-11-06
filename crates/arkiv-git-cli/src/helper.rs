//! The git remote-helper protocol (design 4.1): capabilities `option`, `fetch`, `push`.
//! stdout carries only protocol lines; diagnostics go to stderr.

use std::collections::BTreeSet;
use std::io::{BufRead, Write};

use arkiv_git_core::Nid;
use arkiv_git_store::Arkiv;
use arkiv_git_sync::git::Git;
use arkiv_git_sync::ids::{RemoteUrl, parse_id};
use arkiv_git_sync::{Config, Keys, Push, Repo, Update, View, Want, listing, own_refs, sync};

use crate::{CliError, Result};

/// One helper session: the repo lock is taken by the first `list` and held to the end.
pub struct Session<'a, A: Arkiv> {
    a: &'a A,
    cfg: Config,
    url: RemoteUrl,
    repo: Option<Repo>,
    view: Option<View>,
    dry_run: bool,
}

/// What the main loop does after a command.
enum Next {
    Continue,
    Stop,
}

/// Nids from `remote.<name>.arkivPeers` (whitespace or comma separated, repeatable).
pub fn configured_peers(remote: &str) -> Result<Vec<Nid>> {
    let key = format!("remote.{remote}.arkivPeers");
    let out = Git::work(None).run(&["config", "--get-all", &key], b"");
    let text = out
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    text.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .map(|s| parse_id(s).map_err(CliError::from))
        .collect()
}

impl<'a, A: Arkiv> Session<'a, A> {
    pub fn new(a: &'a A, cfg: Config, url: RemoteUrl) -> Self {
        Self {
            a,
            cfg,
            url,
            repo: None,
            view: None,
            dry_run: false,
        }
    }

    fn repo(&mut self) -> Result<&Repo> {
        if self.repo.is_none() {
            self.repo = Some(Repo::open(&self.cfg.home, &self.url.rid)?);
        }
        self.repo
            .as_ref()
            .ok_or_else(|| CliError::Usage("repo not open".into()))
    }

    async fn sync(&mut self, must: BTreeSet<Nid>) -> Result<View> {
        let want = Want {
            extra: self.cfg.extra_peers.iter().copied().collect(),
            must,
        };
        self.repo()?;
        let repo = self
            .repo
            .as_ref()
            .ok_or_else(|| CliError::Usage("repo not open".into()))?;
        Ok(sync(self.a, repo, &want).await?)
    }

    fn option(&mut self, rest: &str, out: &mut impl Write) -> Result<()> {
        let (name, value) = rest.split_once(' ').unwrap_or((rest, ""));
        let reply = match name {
            "progress" | "verbosity" => "ok",
            "dry-run" => {
                self.dry_run = value == "true";
                "ok"
            }
            _ => "unsupported",
        };
        writeln!(out, "{reply}").map_err(io)
    }

    async fn list(&mut self, for_push: bool, out: &mut impl Write) -> Result<()> {
        let refs = match for_push {
            true => {
                let nid = Keys::require(&self.cfg.home)?.nid();
                let view = self.sync([nid].into()).await?;
                let refs = own_refs(&view, &nid);
                self.view = Some(view);
                refs
            }
            false => {
                let view = self.sync(BTreeSet::new()).await?;
                let l = listing(&view, self.url.nid.as_ref());
                if let Some(h) = &l.head {
                    writeln!(out, "@{h} HEAD").map_err(io)?;
                }
                l.refs
            }
        };
        for (name, oid) in refs {
            writeln!(out, "{oid} {name}").map_err(io)?;
        }
        writeln!(out).map_err(io)
    }

    fn fetch(&mut self, oids: &[String], out: &mut impl Write) -> Result<()> {
        let cache = self.repo()?.git_dir();
        let work = Git::work(None);
        let kept = work.fetch_pack_from(&cache, oids)?;
        if !kept.is_empty() {
            let dir = work.run(&["rev-parse", "--absolute-git-dir"], b"")?;
            let dir = String::from_utf8_lossy(&dir).trim().to_owned();
            for k in kept {
                writeln!(out, "lock {dir}/objects/pack/pack-{k}.keep").map_err(io)?;
            }
        }
        writeln!(out).map_err(io)
    }

    async fn push_all(&mut self, updates: &[Update]) -> Result<()> {
        let keys = Keys::require(&self.cfg.home)?;
        let view = match self.view.take() {
            Some(v) => v,
            None => self.sync([keys.nid()].into()).await?,
        };
        let repo = self
            .repo
            .as_ref()
            .ok_or_else(|| CliError::Usage("repo not open".into()))?;
        let resumed = repo.pending_path().exists();
        let p = Push::prepare(
            self.a,
            repo,
            &self.cfg,
            &keys,
            view,
            &Git::work(None),
            updates,
        )?;
        if !self.dry_run {
            return Ok(p.run().await?);
        }
        let cost = p.cost().await?;
        eprintln!(
            "arkiv-git: dry run: about {} gas, {} tGLM at 2x base fee; nothing sent",
            cost.control_gas + cost.bulk_gas,
            crate::units::fmt_glm(cost.wei())
        );
        if !resumed {
            p.discard(&p.pending);
        }
        Ok(())
    }

    async fn push(&mut self, specs: &[String], out: &mut impl Write) -> Result<()> {
        let updates: Vec<Update> = specs.iter().map(|s| parse_push(s)).collect();
        let result = self.push_all(&updates).await;
        for u in &updates {
            match &result {
                Ok(()) => writeln!(out, "ok {}", u.dst),
                Err(e) => writeln!(out, "error {} {}", u.dst, e.line()),
            }
            .map_err(io)?;
        }
        writeln!(out).map_err(io)
    }

    /// Reads lines of a batch (`fetch` / `push`) up to the terminating blank line.
    fn batch(first: &str, input: &mut impl BufRead, prefix: &str) -> Result<Vec<String>> {
        let mut items = vec![first.to_owned()];
        loop {
            let line = read_line(input)?.unwrap_or_default();
            if line.is_empty() {
                return Ok(items);
            }
            match line.strip_prefix(prefix) {
                Some(rest) => items.push(rest.to_owned()),
                None => return Err(CliError::Usage(format!("unexpected line in batch: {line}"))),
            }
        }
    }

    async fn step(
        &mut self,
        line: &str,
        input: &mut impl BufRead,
        out: &mut impl Write,
    ) -> Result<Next> {
        match line {
            "" => return Ok(Next::Stop),
            "capabilities" => writeln!(out, "option\nfetch\npush\n").map_err(io)?,
            "list" => self.list(false, out).await?,
            "list for-push" => self.list(true, out).await?,
            _ => {
                if let Some(rest) = line.strip_prefix("option ") {
                    self.option(rest, out)?;
                } else if let Some(rest) = line.strip_prefix("fetch ") {
                    let items = Self::batch(rest, input, "fetch ")?;
                    let oids: Vec<String> = items
                        .iter()
                        .filter_map(|i| i.split_whitespace().next().map(str::to_owned))
                        .collect();
                    self.fetch(&oids, out)?;
                } else if let Some(rest) = line.strip_prefix("push ") {
                    let specs = Self::batch(rest, input, "push ")?;
                    self.push(&specs, out).await?;
                } else {
                    return Err(CliError::Usage(format!("unknown helper command: {line}")));
                }
            }
        }
        out.flush().map_err(io)?;
        Ok(Next::Continue)
    }

    /// Serves commands until EOF or a blank line.
    pub async fn serve(&mut self, input: &mut impl BufRead, out: &mut impl Write) -> Result<()> {
        while let Some(line) = read_line(input)? {
            if let Next::Stop = self.step(&line, input, out).await? {
                break;
            }
        }
        Ok(())
    }
}

fn io(e: std::io::Error) -> CliError {
    CliError::Sync(arkiv_git_sync::SyncError::io("helper stdout", e))
}

fn read_line(input: &mut impl BufRead) -> Result<Option<String>> {
    let mut s = String::new();
    let n = input.read_line(&mut s).map_err(io)?;
    Ok((n > 0).then(|| s.trim_end_matches(['\n', '\r']).to_owned()))
}

/// `[+]<src>:<dst>`; an empty `src` deletes `dst`. Force is git's business (it checks
/// fast-forwards against `list for-push` before sending the batch).
pub fn parse_push(spec: &str) -> Update {
    let spec = spec.strip_prefix('+').unwrap_or(spec);
    let (src, dst) = spec.split_once(':').unwrap_or((spec, spec));
    Update {
        src: (!src.is_empty()).then(|| src.to_owned()),
        dst: dst.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_specs_parse_like_git_sends_them() {
        let u = parse_push("+refs/heads/a:refs/heads/b");
        assert_eq!(
            (u.src.as_deref(), u.dst.as_str()),
            (Some("refs/heads/a"), "refs/heads/b")
        );
        let d = parse_push(":refs/heads/gone");
        assert_eq!((d.src, d.dst.as_str()), (None, "refs/heads/gone"));
    }

    #[test]
    fn batches_end_at_the_blank_line() {
        let mut input = std::io::Cursor::new(b"push b:b\n\nlist\n".to_vec());
        let items = Session::<arkiv_git_store::fake::MemArkiv>::batch("a:a", &mut input, "push ")
            .expect("batch");
        assert_eq!(items, vec!["a:a", "b:b"]);
        assert_eq!(
            read_line(&mut input).expect("read").as_deref(),
            Some("list")
        );
        let mut bad = std::io::Cursor::new(b"list\n".to_vec());
        assert!(Session::<arkiv_git_store::fake::MemArkiv>::batch("x", &mut bad, "push ").is_err());
    }
}

//! Configuration and the per-repo cache layout under `$ARKIV_GIT_HOME` (design 3.9):
//! `repos/<rid-hex>/{git/, state.json, outgoing/, lock}`; the lock is held while a
//! [`Repo`] is alive, for the whole of a push, fetch or keepalive.

use std::path::{Path, PathBuf};
use std::time::Duration;

use arkiv_git_core::{Nid, Rid};
use arkiv_git_store::PipelineConfig;

use crate::git::Git;
use crate::ids::hex32;
use crate::{Result, SyncError};

/// Data lifetime at create (design 2.3): 30 days of 2 s blocks.
pub const DATA_LIFETIME: u64 = 1_296_000;
/// Control-plane lifetime at create: the maximum, 182.5 days.
pub const CONTROL_LIFETIME: u64 = 7_884_000;
/// Warn when a required entity expires sooner than this (7 days).
pub const WARN_EXPIRY_BLOCKS: u64 = 302_400;
/// Blocks per day on Arkiv (2 s blocks).
pub const BLOCKS_PER_DAY: u64 = 43_200;

#[derive(Debug, Clone)]
pub struct Config {
    pub home: PathBuf,
    /// `minLifetime` for refs, manifests and chunks (test override: `--lifetime`).
    pub data_lifetime: u64,
    /// `minLifetime` for peer, bind, id, idacc and anchor entities.
    pub control_lifetime: u64,
    pub pipeline: PipelineConfig,
    /// Peers to sync besides the delegates (`remote.<name>.arkivPeers`).
    pub extra_peers: Vec<Nid>,
    /// Chunk txs handed to the pipeline at once (bounds memory for 100 MiB packs).
    pub chunk_batch: usize,
    /// Display alias written into this peer's `peer` document.
    pub alias: String,
}

impl Config {
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            data_lifetime: DATA_LIFETIME,
            control_lifetime: CONTROL_LIFETIME,
            pipeline: PipelineConfig::default(),
            extra_peers: vec![],
            chunk_batch: 64,
            alias: String::new(),
        }
    }

    /// A config for the in-memory fake: no waiting between receipt polls.
    pub fn for_fake(home: PathBuf) -> Self {
        let mut c = Self::new(home);
        c.pipeline.poll = Duration::ZERO;
        c
    }

    /// `$ARKIV_GIT_HOME`, else `~/.arkiv-git`.
    pub fn default_home() -> PathBuf {
        if let Some(h) = std::env::var_os("ARKIV_GIT_HOME").filter(|h| !h.is_empty()) {
            return PathBuf::from(h);
        }
        let home = std::env::var_os("HOME").unwrap_or_default();
        PathBuf::from(home).join(".arkiv-git")
    }
}

/// An opened, locked repo cache.
#[derive(Debug)]
pub struct Repo {
    pub rid: Rid,
    pub dir: PathBuf,
    git: Git,
    _lock: std::fs::File,
}

impl Repo {
    /// Creates the layout if needed and takes the repo lock (blocks while another process
    /// holds it).
    pub fn open(home: &Path, rid: &Rid) -> Result<Self> {
        let dir = home.join("repos").join(hex32(rid));
        for d in [dir.join("outgoing"), dir.join("tmp")] {
            std::fs::create_dir_all(&d).map_err(|e| SyncError::io(d.display(), e))?;
        }
        let lock_path = dir.join("lock");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|e| SyncError::io(lock_path.display(), e))?;
        lock.lock()
            .map_err(|e| SyncError::io(lock_path.display(), e))?;
        let git = Git::init_cache(&dir.join("git"))?;
        Ok(Self {
            rid: *rid,
            dir,
            git,
            _lock: lock,
        })
    }

    pub fn git(&self) -> &Git {
        &self.git
    }
    pub fn git_dir(&self) -> PathBuf {
        self.dir.join("git")
    }
    pub fn state_path(&self) -> PathBuf {
        self.dir.join("state.json")
    }
    pub fn outgoing(&self) -> PathBuf {
        self.dir.join("outgoing")
    }
    pub fn pending_path(&self) -> PathBuf {
        self.outgoing().join("pending.json")
    }
    pub fn tmp(&self) -> PathBuf {
        self.dir.join("tmp")
    }
}

/// `refs/namespaces/<nid-hex>/` (verified, anchored refs of one peer).
pub fn namespace(nid: &Nid) -> String {
    format!("refs/namespaces/{}/", hex32(nid))
}

/// `refs/arkiv/staging/<nid-hex>/` (local, not yet anchored).
pub fn staging(nid: &Nid) -> String {
    format!("refs/arkiv/staging/{}/", hex32(nid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_and_lock() {
        let d = tempfile::tempdir().expect("tmp");
        let r = Repo::open(d.path(), &[1u8; 32]).expect("open");
        assert!(r.git_dir().join("HEAD").exists());
        assert!(r.outgoing().is_dir() && r.tmp().is_dir());
        assert!(r.pending_path().starts_with(r.outgoing()));
        assert!(r.state_path().ends_with("state.json"));
        assert!(r.git().refs("refs/").expect("refs").is_empty());
        assert!(namespace(&[0xab; 32]).starts_with("refs/namespaces/abab"));
        assert!(staging(&[0; 32]).starts_with("refs/arkiv/staging/0000"));
        let c = Config::for_fake(d.path().into());
        assert_eq!(c.pipeline.poll, Duration::ZERO);
        assert_eq!(c.data_lifetime, DATA_LIFETIME);
        assert!(!Config::default_home().as_os_str().is_empty());
    }
}

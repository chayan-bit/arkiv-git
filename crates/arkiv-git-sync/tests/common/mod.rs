//! Shared fixtures for the sync integration tests: an auto-mining `MemArkiv` wrapper with a
//! send-dropping fault, peers with their own `$ARKIV_GIT_HOME`, and real git work repos.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use alloy_signer_local::PrivateKeySigner;
use arkiv_git_core::{Hash32, Rid};
use arkiv_git_crypto::MasterSeed;
use arkiv_git_store::fake::MemArkiv;
use arkiv_git_store::{
    Address, Arkiv, Operation, Query, QueryOpts, QueryPage, Receipt, SignedTx, StoreError, TxHash,
    U256,
};
use arkiv_git_sync::identity::{self, RepoMeta};
use arkiv_git_sync::{Config, Keys, Listing, Push, Repo, Update, View, Want};

pub const CHAIN: u64 = 7;

/// `MemArkiv` that mines one block whenever a receipt is polled and not found, and can
/// accept sends without ever mining them (a lossy RPC edge).
pub struct Net {
    pub mem: Arc<MemArkiv>,
    /// `(sender, sends still let through)`: later sends from `sender` are dropped.
    drop: Mutex<Option<(Address, u64)>>,
}

impl Net {
    pub fn new() -> Self {
        Self {
            mem: Arc::new(MemArkiv::new(CHAIN)),
            drop: Mutex::new(None),
        }
    }

    /// Accept but never mine every send from `sender` after the next `after` ones.
    pub fn drop_sends_from(&self, sender: Address, after: u64) {
        *self.drop.lock().expect("lock") = Some((sender, after));
    }

    pub fn heal(&self) {
        *self.drop.lock().expect("lock") = None;
    }

    pub fn fund(&self, a: Address) {
        self.mem.fund(a, U256::from(10u128.pow(24)));
    }

    pub fn mine(&self, n: u64) {
        (0..n).for_each(|_| {
            self.mem.mine();
        });
    }
}

impl Arkiv for Net {
    async fn chain_id(&self) -> Result<u64, StoreError> {
        self.mem.chain_id().await
    }
    async fn head(&self) -> Result<u64, StoreError> {
        self.mem.head().await
    }
    async fn base_fee(&self) -> Result<u128, StoreError> {
        self.mem.base_fee().await
    }
    async fn query(&self, q: &Query, o: &QueryOpts) -> Result<QueryPage, StoreError> {
        self.mem.query(q, o).await
    }
    async fn entity_nonce(&self, c: Address) -> Result<u64, StoreError> {
        self.mem.entity_nonce(c).await
    }
    async fn balance(&self, a: Address) -> Result<U256, StoreError> {
        self.mem.balance(a).await
    }
    async fn tx_count(&self, a: Address) -> Result<u64, StoreError> {
        self.mem.tx_count(a).await
    }
    async fn estimate(&self, from: Address, ops: &[Operation]) -> Result<u64, StoreError> {
        self.mem.estimate(from, ops).await
    }
    async fn send(&self, tx: SignedTx) -> Result<TxHash, StoreError> {
        let dropped = match self.drop.lock().expect("lock").as_mut() {
            Some((s, left)) if *s == tx.from() => match *left {
                0 => true,
                _ => {
                    *left -= 1;
                    false
                }
            },
            _ => false,
        };
        if dropped {
            return Ok(tx.hash());
        }
        self.mem.send(tx).await
    }
    async fn receipt(&self, h: TxHash) -> Result<Option<Receipt>, StoreError> {
        if let Some(r) = self.mem.receipt(h).await? {
            return Ok(Some(r));
        }
        self.mem.mine();
        self.mem.receipt(h).await
    }
}

/// A peer: its own home directory, config and keys (wallet funded).
pub struct Peer {
    pub home: tempfile::TempDir,
    pub cfg: Config,
    pub keys: Keys,
}

impl Peer {
    pub fn new(net: &Net, seed: u8) -> Self {
        let home = tempfile::tempdir().expect("home");
        let mut cfg = Config::for_fake(home.path().to_path_buf());
        cfg.pipeline.resend_after_blocks = 3;
        let keys = Keys::from_seed(MasterSeed::from_bytes([seed; 32]));
        net.fund(keys.wallet().address());
        Self { home, cfg, keys }
    }

    pub fn repo(&self, rid: &Rid) -> Repo {
        Repo::open(self.home.path(), rid).expect("repo")
    }

    pub fn want(&self) -> Want {
        Want {
            must: [self.keys.nid()].into(),
            ..Want::default()
        }
    }
}

pub fn meta(name: &str) -> RepoMeta {
    RepoMeta {
        name: name.into(),
        description: "test repo".into(),
        default_branch: "main".into(),
        lang: Some("rust".into()),
        license: Some("MIT".into()),
    }
}

pub async fn init(net: &Net, p: &Peer, name: &str) -> Rid {
    identity::init(net, &p.cfg, &p.keys, &meta(name))
        .await
        .expect("init")
}

/// Fetch sync including the peer itself, then prepare a push of `updates`.
pub async fn prepare<'a>(
    net: &'a Net,
    p: &'a Peer,
    repo: &'a Repo,
    work: &Path,
    updates: &[Update],
) -> arkiv_git_sync::Result<Push<'a, Net>> {
    let view = arkiv_git_sync::sync(net, repo, &p.want()).await?;
    Push::prepare(
        net,
        repo,
        &p.cfg,
        &p.keys,
        view,
        &arkiv_git_sync::git::Git::work(Some(work)),
        updates,
    )
}

pub async fn push(
    net: &Net,
    p: &Peer,
    rid: &Rid,
    work: &Path,
    refs: &[&str],
) -> arkiv_git_sync::Result<()> {
    let repo = p.repo(rid);
    let updates: Vec<Update> = refs.iter().map(|r| upd(r)).collect();
    prepare(net, p, &repo, work, &updates).await?.run().await
}

pub fn upd(r: &str) -> Update {
    match r.strip_prefix(':') {
        Some(dst) => Update {
            src: None,
            dst: dst.into(),
        },
        None => Update {
            src: Some(r.into()),
            dst: r.into(),
        },
    }
}

/// Fresh-or-existing cache fetch, then the working repo `dest` gets the listed objects and
/// refs (what `git clone arkiv::<rid>` does through the helper).
pub async fn clone_into(
    net: &Net,
    p: &Peer,
    rid: &Rid,
    dest: &Path,
) -> arkiv_git_sync::Result<(View, Listing)> {
    let repo = p.repo(rid);
    let view = arkiv_git_sync::sync(net, &repo, &Want::default()).await?;
    let listing = arkiv_git_sync::listing(&view, None);
    if !dest.join(".git").exists() {
        git(
            dest.parent().expect("parent"),
            &["init", "-q", &dest.to_string_lossy()],
        );
    }
    let work = arkiv_git_sync::git::Git::work(Some(dest));
    let oids: Vec<String> = listing.refs.iter().map(|(_, o)| o.clone()).collect();
    work.fetch_pack_from(&repo.git_dir(), &oids)?;
    for (name, oid) in &listing.refs {
        git(dest, &["update-ref", name, oid]);
    }
    Ok((view, listing))
}

/// Runs git with a config isolated from the machine's (no global hooks or signing).
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A new working repo with no commits.
pub fn work_repo(parent: &Path, name: &str) -> PathBuf {
    let d = parent.join(name);
    git(parent, &["init", "-q", &d.to_string_lossy()]);
    d
}

/// Writes `files` (path, bytes) and commits them; returns the commit oid.
pub fn commit(work: &Path, files: &[(&str, &[u8])], msg: &str) -> String {
    for (p, b) in files {
        let f = work.join(p);
        std::fs::create_dir_all(f.parent().expect("parent")).expect("mkdir");
        std::fs::write(&f, b).expect("write");
    }
    git(work, &["add", "-A"]);
    git(work, &["commit", "-q", "--allow-empty", "-m", msg]);
    git(work, &["rev-parse", "HEAD"])
}

/// Deterministic incompressible bytes.
pub fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

pub fn rev(dir: &Path, r: &str) -> String {
    git(dir, &["rev-parse", r])
}

/// A funded key that is no peer of any repo (third parties, replayers, sponsors).
pub fn outsider(net: &Net) -> PrivateKeySigner {
    let k = PrivateKeySigner::random();
    net.fund(k.address());
    k
}

pub fn code<T>(r: arkiv_git_sync::Result<T>) -> &'static str {
    match r {
        Ok(_) => "OK",
        Err(e) => e.code().as_str(),
    }
}

pub fn h32_hex(h: &Hash32) -> String {
    hex::encode(h)
}

pub fn fast() -> arkiv_git_store::PipelineConfig {
    arkiv_git_store::PipelineConfig {
        poll: std::time::Duration::ZERO,
        resend_after_blocks: 3,
        ..Default::default()
    }
}

/// Live entities of `kind` carrying `ag.rid = rid`, in creation order.
pub async fn entities(
    net: &Net,
    rid: &Rid,
    kind: arkiv_git_core::Kind,
) -> Vec<arkiv_git_core::Observed> {
    use arkiv_git_sync::observe;
    let f = arkiv_git_store::Pred::And(vec![observe::kind(kind), observe::b32("ag.rid", rid)]);
    observe::all(net, f).await.expect("query")
}

/// Every live entity created by `creator`.
pub async fn created_by(net: &Net, creator: Address) -> Vec<arkiv_git_core::Observed> {
    use arkiv_git_sync::observe;
    observe::all(net, observe::creators(&[creator.into_array()].into()))
        .await
        .expect("query")
}

pub fn body_of(o: &arkiv_git_core::Observed) -> Vec<u8> {
    match o.attr("ag.body") {
        Some(arkiv_git_core::AttrValue::Bytes(b)) => b.clone(),
        other => panic!("no body: {other:?}"),
    }
}

pub fn seq_of(o: &arkiv_git_core::Observed) -> u64 {
    match o.attr("ag.seq") {
        Some(arkiv_git_core::AttrValue::U64(s)) => *s,
        other => panic!("no seq: {other:?}"),
    }
}

/// Sends `ops` in one tx from `signer` and waits for it to land.
pub async fn send_ops(net: &Net, signer: &PrivateKeySigner, ops: Vec<Operation>) {
    arkiv_git_sync::write::send(net, signer, &fast(), vec![ops])
        .await
        .expect("send");
}

/// Extends each entity (permissionless) to `lifetime` blocks from now.
pub async fn extend(net: &Net, signer: &PrivateKeySigner, keys: &[Hash32], lifetime: u64) {
    for group in keys.chunks(40) {
        let ops = group
            .iter()
            .map(|k| arkiv_git_store::extend_op(arkiv_git_store::B256::from(*k), lifetime))
            .collect();
        send_ops(net, signer, ops).await;
    }
}

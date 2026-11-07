//! Helpers for the live multi-peer Rabdi tests (`rabdi_collab.rs`): a bank wallet funded
//! once from the throwaway key, isolated peers that run the real binaries, a step log for
//! the report, and a sweep of every peer back to the bank, even on panic.
#![allow(dead_code)]

use std::cell::RefCell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Instant;

use alloy_signer_local::PrivateKeySigner;
use charter_core::Rid;
use charter_store::{Address, Arkiv, PipelineConfig, RpcArkiv, TxPipeline, U256};
use charter_sync::Keys;

use super::live::{self, Secrets};

pub const EXPLORER: &str = "https://rabdi.explorer.arkiv.network";
/// Every entity lives 1 day (43200 blocks at 2 s), so the repos stay readable for judges.
pub const LIFETIME: &str = "43200";
pub const ARKIV: &str = env!("CARGO_BIN_EXE_chart");
pub const MILLI: u128 = 1_000_000_000_000_000; // 0.001 tGLM
/// First float for a push or a control write (init, op, pin, identity revision, escrow
/// round). A push's up-front check wants predicted spend plus one window of max fees
/// (design 4.2 step 6), far above what a tiny push spends (about 0.001 tGLM), so on
/// `E_INSUFFICIENT_FUNDS` the write is retried once with what the error says it needs.
pub const WRITE_FLOAT: u128 = 3 * MILLI;
/// Retry float when the error names no amount.
pub const RETRY_FLOAT: u128 = 8 * MILLI;

/// The float to retry with after `out` failed for lack of funds, else `None`.
fn retry_float(out: &Output) -> Option<u128> {
    let err = stderr(out);
    if !err.contains("E_INSUFFICIENT_FUNDS") {
        return None;
    }
    let need = err
        .split("this push needs ")
        .nth(1)
        .and_then(|r| r.split(' ').next())
        .and_then(|n| n.parse::<u128>().ok());
    Some(need.map_or(RETRY_FLOAT, |n| n + MILLI / 2))
}

/// `CHARTER_COLLAB_ROOT`: holds `bank/` (the funded home) and one dir per test run.
pub fn root() -> PathBuf {
    PathBuf::from(std::env::var("CHARTER_COLLAB_ROOT").expect("set CHARTER_COLLAB_ROOT"))
}

pub fn bank_home() -> PathBuf {
    root().join("bank/.charter")
}

/// Exclusive lock shared with every other user of the throwaway key
/// (`CHARTER_FUND_LOCK`, the same file `lockf(1)` takes).
pub fn fund_lock() -> std::fs::File {
    let p = std::env::var("CHARTER_FUND_LOCK").expect("set CHARTER_FUND_LOCK");
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(p)
        .expect("lock file");
    f.lock().expect("flock");
    f
}

pub fn glm(w: U256) -> String {
    charter_cli::units::fmt_glm(w)
}

pub fn rid_of(url: &str) -> Rid {
    charter_sync::ids::parse_url(url.trim_start_matches("arkiv::"))
        .expect("url")
        .rid
}

/// One live test run.
pub struct Net {
    pub s: Secrets,
    pub rpc: RpcArkiv,
    pub dir: PathBuf,
    pub name: String,
    pub bank: Keys,
    bank_start: U256,
    homes: RefCell<Vec<PathBuf>>,
    extra: RefCell<Vec<PrivateKeySigner>>,
    failures: RefCell<Vec<String>>,
    t0: Instant,
}

impl Net {
    /// Runs `body` against Rabdi, then sweeps every peer back to the bank and fails if any
    /// [`Net::check`] failed. Returns early (skips) unless `CHARTER_RABDI=1`.
    pub fn run(name: &str, body: impl FnOnce(&Net)) {
        if !live::enabled() {
            return;
        }
        let s = live::secrets();
        let rpc = live::connect(&s);
        let bank = Keys::require(&bank_home()).expect("bank home: run fund_bank first");
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        let dir = root().join(format!("{name}-{ts}"));
        std::fs::create_dir_all(&dir).expect("run dir");
        let bank_start = live::balance(&rpc, bank.wallet().address());
        let net = Net {
            s,
            rpc,
            dir,
            name: name.to_owned(),
            bank,
            bank_start,
            homes: RefCell::default(),
            extra: RefCell::default(),
            failures: RefCell::default(),
            t0: Instant::now(),
        };
        net.log(&format!("start, bank {} tGLM", glm(bank_start)));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&net)));
        net.sweep_all();
        let end = live::balance(&net.rpc, net.bank.wallet().address());
        let spent = net.bank_start.saturating_sub(end);
        net.log(&format!(
            "end after {} s, net spend {} tGLM, bank {} tGLM",
            net.t0.elapsed().as_secs(),
            glm(spent),
            glm(end)
        ));
        if let Err(p) = result {
            std::panic::resume_unwind(p);
        }
        let failures = net.failures.borrow();
        assert!(failures.is_empty(), "failed steps: {failures:#?}");
    }

    /// Appends `line` to stderr and to `$CHARTER_COLLAB_ROOT/steps.log`.
    pub fn log(&self, line: &str) {
        let line = format!("[{} +{}s] {line}", self.name, self.t0.elapsed().as_secs());
        eprintln!("{line}");
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root().join("steps.log"))
            .expect("steps log");
        writeln!(f, "{line}").expect("write log");
    }

    /// Records a step as PASS or FAIL and keeps going, so one run shows every failure.
    pub fn check(&self, step: &str, ok: bool, detail: &str) -> bool {
        let verdict = if ok { "PASS" } else { "FAIL" };
        self.log(&format!("{verdict} {step}: {detail}"));
        if !ok {
            self.failures.borrow_mut().push(format!("{step}: {detail}"));
        }
        ok
    }

    pub fn peer(&self, name: &str) -> Peer<'_> {
        let home = self.dir.join(name);
        std::fs::create_dir_all(&home).expect("home");
        Peer {
            name: name.to_owned(),
            home,
            net: self,
        }
    }

    /// A peer whose home (with keys) already exists, swept at the end like any other.
    pub fn existing_peer(&self, name: &str, home: PathBuf) -> Peer<'_> {
        self.homes.borrow_mut().push(home.join(".charter"));
        Peer {
            name: name.to_owned(),
            home,
            net: self,
        }
    }

    /// Sends `wei` from the bank wallet.
    pub fn fund(&self, to: Address, wei: u128) {
        live::transfer(&self.rpc, &self.bank.wallet(), to, U256::from(wei));
        self.log(&format!("funded {to} with {} tGLM", glm(U256::from(wei))));
    }

    /// Lends `keys` up to `wei` (wallet plus the repo addresses of `rids`) for `f`, then
    /// sweeps everything back, so the bank only ever floats one write at a time.
    pub fn float<T>(
        &self,
        keys: &Keys,
        rids: impl Fn() -> Vec<Rid>,
        wei: u128,
        f: impl FnOnce() -> T,
    ) -> T {
        let wallet = keys.wallet().address();
        let mut have = self.balance(wallet);
        for rid in rids() {
            have +=
                self.balance(keys.control_address(&rid)) + self.balance(keys.bulk_address(&rid));
        }
        if have < U256::from(wei) {
            let short = U256::from(wei) - have;
            live::transfer(&self.rpc, &self.bank.wallet(), wallet, short);
        }
        let out = f();
        live::sweep(&self.rpc, keys, &rids(), self.bank.wallet().address());
        out
    }

    /// Runs `f` with a [`WRITE_FLOAT`], once more with a bigger one if it ran out of funds.
    pub fn funded(
        &self,
        keys: &Keys,
        rids: impl Fn() -> Vec<Rid>,
        f: impl Fn() -> Output,
    ) -> Output {
        let out = self.float(keys, &rids, WRITE_FLOAT, &f);
        match retry_float(&out) {
            Some(wei) => {
                self.log(&format!(
                    "retrying with a {} tGLM float",
                    glm(U256::from(wei))
                ));
                self.float(keys, rids, wei, f)
            }
            None => out,
        }
    }

    /// Also sweep `signer` (an escrow payout or identity key) at the end.
    pub fn sweep_later(&self, signer: PrivateKeySigner) {
        self.extra.borrow_mut().push(signer);
    }

    pub fn balance(&self, a: Address) -> U256 {
        live::balance(&self.rpc, a)
    }

    pub fn head(&self) -> u64 {
        charter_cli::block_on(self.rpc.head())
            .expect("rt")
            .expect("head")
    }

    pub fn tx(&self, h: &str) -> Option<charter_store::TxInfo> {
        let h = h.parse().expect("tx hash");
        charter_cli::block_on(self.rpc.transaction(h))
            .expect("rt")
            .expect("tx")
    }

    /// Waits (polling every 4 s) until the chain head reaches `block`.
    pub fn wait_for_block(&self, block: u64) {
        while self.head() < block {
            std::thread::sleep(std::time::Duration::from_secs(4));
        }
    }

    fn sweep_all(&self) {
        let to = self.bank.wallet().address();
        for home in self.homes.borrow().iter() {
            if let Ok(keys) = Keys::require(home) {
                live::sweep(
                    &self.rpc,
                    &keys,
                    &live::rids(home.parent().expect("home")),
                    to,
                );
            }
        }
        for signer in self.extra.borrow().iter() {
            sweep_signer(&self.rpc, signer, to);
        }
    }
}

/// Moves all but the transfer's own gas from `signer` to `to`.
pub fn sweep_signer(rpc: &RpcArkiv, signer: &PrivateKeySigner, to: Address) {
    charter_cli::block_on(async {
        let chain = rpc.chain_id().await.expect("chain");
        let base = rpc.base_fee().await.expect("fee");
        let cfg = PipelineConfig::default();
        let reserve = U256::from(30_000u64) * U256::from(2 * base + cfg.tip);
        let have = rpc.balance(signer.address()).await.expect("balance");
        if have > reserve {
            let mut p = TxPipeline::new(rpc, signer.clone(), chain, cfg);
            if let Err(e) = p.transfer(to, have - reserve).await {
                eprintln!("sweep failed: {}", e.code());
            }
        }
    })
    .expect("rt");
}

/// A user with its own `$HOME`, `$CHARTER_HOME` and git config.
pub struct Peer<'n> {
    pub name: String,
    pub home: PathBuf,
    net: &'n Net,
}

impl<'n> Peer<'n> {
    /// Runs `program`, timing it into the step log; never prints keys (none are in args).
    pub fn out(&self, program: &str, dir: &Path, args: &[&str]) -> Output {
        let n = self.net;
        let start = Instant::now();
        let out = live::command(&self.home, &n.s.access, LIFETIME, program, dir, args)
            .output()
            .expect("spawn");
        live::note_stderr(&String::from_utf8_lossy(&out.stderr));
        let shown = if program == ARKIV { "chart" } else { program };
        n.log(&format!(
            "{}$ {shown} {} -> {} ({:.1} s)",
            self.name,
            args.join(" "),
            out.status.code().unwrap_or(-1),
            start.elapsed().as_secs_f64()
        ));
        out
    }

    pub fn ok(&self, program: &str, dir: &Path, args: &[&str]) -> String {
        let out = self.out(program, dir, args);
        assert!(
            out.status.success(),
            "{}: {program} {args:?} failed: {}",
            self.name,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn arkiv(&self, args: &[&str]) -> String {
        self.ok(ARKIV, &self.home, args)
    }

    pub fn arkiv_out(&self, args: &[&str]) -> Output {
        self.out(ARKIV, &self.home, args)
    }

    pub fn git(&self, dir: &Path, args: &[&str]) -> String {
        self.ok("git", dir, args).trim().to_owned()
    }

    pub fn git_out(&self, dir: &Path, args: &[&str]) -> Output {
        self.out("git", dir, args)
    }

    pub fn net(&self) -> &'n Net {
        self.net
    }

    /// [`Net::float`] for this peer's own keys and cached repos.
    pub fn float<T>(&self, wei: u128, f: impl FnOnce() -> T) -> T {
        let keys = Keys::require(&self.ark_home()).expect("keys");
        self.net.float(&keys, || live::rids(&self.home), wei, f)
    }

    fn funded(&self, f: impl Fn() -> Output) -> Output {
        let keys = Keys::require(&self.ark_home()).expect("keys");
        self.net.funded(&keys, || live::rids(&self.home), f)
    }

    /// `git push <args>` in `work`, funded; returns the output.
    pub fn push(&self, work: &Path, args: &[&str]) -> Output {
        let full: Vec<&str> = std::iter::once("push")
            .chain(args.iter().copied())
            .collect();
        self.funded(|| self.git_out(work, &full))
    }

    /// [`Peer::push`] that must succeed.
    pub fn push_ok(&self, work: &Path, args: &[&str]) {
        let out = self.push(work, args);
        assert!(
            out.status.success(),
            "{}: push {args:?} failed: {}",
            self.name,
            stderr(&out)
        );
    }

    /// A `chart` write, funded; returns stdout.
    pub fn write(&self, args: &[&str]) -> String {
        let out = self.funded(|| self.arkiv_out(args));
        assert!(
            out.status.success(),
            "{}: chart {args:?} failed: {}",
            self.name,
            stderr(&out)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn ark_home(&self) -> PathBuf {
        self.home.join(".charter")
    }

    /// `keys init`, registering the home for the final sweep; returns (keys, nid text).
    pub fn keys_init(&self) -> (Keys, String) {
        let out = self.arkiv(&["keys", "init"]);
        self.net.homes.borrow_mut().push(self.ark_home());
        let nid = out
            .lines()
            .find_map(|l| l.strip_prefix("nid:"))
            .expect("nid")
            .trim()
            .to_owned();
        (Keys::require(&self.ark_home()).expect("keys"), nid)
    }

    /// `op post`, returning the op id (hex).
    pub fn post(
        &self,
        url: &str,
        ty: &str,
        action: &str,
        object: Option<&str>,
        payload: &str,
    ) -> String {
        let mut args = vec![
            "op", "post", "--repo", url, "--type", ty, "--action", action,
        ];
        if let Some(o) = object {
            args.extend(["--object", o, "--parent", o]);
        }
        args.extend(["--payload-json", payload]);
        self.write(&args).trim().to_owned()
    }

    /// `op list --json`.
    pub fn ops(&self, url: &str) -> Vec<serde_json::Value> {
        let out = self.arkiv(&["op", "list", "--repo", url, "--json"]);
        serde_json::from_str(out.trim()).expect("op list json")
    }

    /// Writes `file` in `work`, commits everything, returns the new oid.
    pub fn commit(&self, work: &Path, file: &str, text: &str, msg: &str) -> String {
        std::fs::write(work.join(file), text).expect("write");
        self.git(work, &["add", "-A"]);
        self.git(work, &["commit", "-q", "-m", msg]);
        self.git(work, &["rev-parse", "HEAD"])
    }

    /// `git clone -q url <dir>`; returns the dir.
    pub fn clone(&self, url: &str, name: &str) -> PathBuf {
        let dest = self.net.dir.join(name);
        self.git(
            &self.net.dir,
            &["clone", "-q", url, &dest.to_string_lossy()],
        );
        dest
    }
}

/// A one-file C program whose output names its version, so "builds" is a real compile.
pub fn hello_c(version: &str) -> String {
    format!("#include <stdio.h>\nint main(void) {{ puts(\"hello {version}\"); return 0; }}\n")
}

/// Compiles `hello.c` in `work` and runs it; returns its output.
pub fn build_and_run(work: &Path, out: &Path) -> String {
    let cc = std::process::Command::new("cc")
        .current_dir(work)
        .args(["-o", &out.to_string_lossy(), "hello.c"])
        .status()
        .expect("cc");
    assert!(cc.success(), "cc failed in {}", work.display());
    let run = std::process::Command::new(out).output().expect("run");
    String::from_utf8_lossy(&run.stdout).trim().to_owned()
}

pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

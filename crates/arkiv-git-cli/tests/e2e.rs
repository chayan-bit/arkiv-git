//! End to end through the real binaries: stock `git push arkiv::<rid>` and
//! `git clone arkiv::<rid>` against the file-persisted MemArkiv backend.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use arkiv_git_store::fake::FileMemArkiv;
use arkiv_git_store::{Address, U256};

/// An isolated user: own `$HOME`, `$ARKIV_GIT_HOME` and git config, shared fake chain.
struct User {
    home: PathBuf,
    log: PathBuf,
}

fn bin_dir() -> PathBuf {
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_git-remote-arkiv"));
    helper.parent().expect("bin dir").to_path_buf()
}

impl User {
    fn new(root: &Path, name: &str, log: &Path) -> Self {
        let home = root.join(name);
        std::fs::create_dir_all(&home).expect("home");
        Self {
            home,
            log: log.to_path_buf(),
        }
    }

    fn cmd(&self, program: &str, dir: &Path) -> Command {
        let path = format!(
            "{}:{}",
            bin_dir().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut c = Command::new(program);
        c.current_dir(dir)
            .env("PATH", path)
            .env("HOME", &self.home)
            .env("ARKIV_GIT_HOME", self.home.join(".arkiv-git"))
            .env("ARKIV_GIT_BACKEND", format!("mem:{}", self.log.display()))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("ARKIV_GIT_DATA_LIFETIME")
            .env_remove("ARKIV_GIT_CONTROL_LIFETIME");
        c
    }

    fn run(&self, program: &str, dir: &Path, args: &[&str]) -> Output {
        self.cmd(program, dir).args(args).output().expect("spawn")
    }

    fn ok(&self, program: &str, dir: &Path, args: &[&str]) -> String {
        let out = self.run(program, dir, args);
        assert!(
            out.status.success(),
            "{program} {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn git(&self, dir: &Path, args: &[&str]) -> String {
        let mut full = vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ];
        full.extend_from_slice(args);
        self.ok("git", dir, &full).trim().to_owned()
    }

    fn arkiv(&self, args: &[&str]) -> String {
        self.ok(env!("CARGO_BIN_EXE_arkiv-git"), &self.home, args)
    }
}

fn field<'a>(out: &'a str, key: &str) -> &'a str {
    out.lines()
        .find_map(|l| l.strip_prefix(key))
        .map(str::trim)
        .unwrap_or_else(|| panic!("no {key} in {out}"))
}

fn fund(log: &Path, wallet: &str) {
    let mem = FileMemArkiv::open(log, arkiv_git_cli::backend::MEM_CHAIN_ID).expect("open");
    let addr: Address = wallet.parse().expect("address");
    mem.fund(addr, U256::from(10u128.pow(24))).expect("fund");
}

fn write(dir: &Path, name: &str, bytes: &[u8]) {
    std::fs::write(dir.join(name), bytes).expect("write");
}

#[test]
fn git_push_and_clone_through_the_remote_helper() {
    let tmp = tempfile::tempdir().expect("tmp");
    let log = tmp.path().join("chain.log");
    let (alice, bob) = (
        User::new(tmp.path(), "alice", &log),
        User::new(tmp.path(), "bob", &log),
    );
    let keys = alice.arkiv(&["keys", "init"]);
    fund(&log, field(&keys, "wallet:"));
    let created = alice.arkiv(&[
        "init",
        "--name",
        "demo",
        "--lang",
        "rust",
        "--license",
        "MIT",
    ]);
    let url = field(&created, "url:").to_owned();
    assert!(url.starts_with("arkiv::z"));

    let work = tmp.path().join("work");
    alice.git(tmp.path(), &["init", "-q", &work.to_string_lossy()]);
    write(&work, "README", b"hello arkiv\n");
    alice.git(&work, &["add", "-A"]);
    alice.git(&work, &["commit", "-q", "-m", "feat: first"]);
    alice.git(&work, &["push", "-q", &url, "main"]);
    std::fs::create_dir_all(work.join("src")).expect("mkdir");
    write(
        &work,
        "src/lib.rs",
        &(0..50_000u32)
            .flat_map(|i| i.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    alice.git(&work, &["add", "-A"]);
    alice.git(&work, &["commit", "-q", "-m", "feat: second"]);
    alice.git(&work, &["tag", "v1"]);
    alice.git(&work, &["push", "-q", &url, "main", "refs/tags/v1"]);

    let dest = tmp.path().join("clone");
    bob.git(tmp.path(), &["clone", "-q", &url, &dest.to_string_lossy()]);
    assert_eq!(
        bob.git(&dest, &["rev-parse", "HEAD"]),
        alice.git(&work, &["rev-parse", "HEAD"])
    );
    assert_eq!(
        bob.git(&dest, &["ls-tree", "-r", "HEAD"]),
        alice.git(&work, &["ls-tree", "-r", "HEAD"])
    );
    assert_eq!(
        bob.git(&dest, &["rev-parse", "v1"]),
        alice.git(&work, &["rev-parse", "v1"])
    );
    assert_eq!(bob.git(&dest, &["branch", "--show-current"]), "main");
    bob.git(&dest, &["fsck", "--full", "--strict"]);
    let packs = std::fs::read_dir(dest.join(".git/objects/pack")).expect("packs");
    assert!(
        packs
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().ends_with(".keep")),
        "no stale .keep"
    );

    write(&work, "README", b"hello again\n");
    alice.git(&work, &["commit", "-qam", "fix: readme"]);
    alice.git(&work, &["push", "-q", &url, "main"]);
    bob.git(&dest, &["pull", "-q", "--ff-only"]);
    assert_eq!(
        std::fs::read(dest.join("README")).expect("read"),
        b"hello again\n"
    );

    let status = alice.arkiv(&["status", &url]);
    assert!(status.contains("delegate: seq 2"), "{status}");
    assert!(alice.arkiv(&["search", "--prefix", "de"]).contains(&url));
    assert!(
        alice
            .arkiv(&["id", "show", &url])
            .contains("name:           demo")
    );
    assert!(
        alice
            .arkiv(&["keepalive", &url, "--dry-run"])
            .contains("would extend")
    );
    assert!(
        alice
            .arkiv(&["fund", "--bytes", "1000000"])
            .contains("estimated need")
    );
}

#[test]
fn errors_map_to_git_and_exit_codes() {
    let tmp = tempfile::tempdir().expect("tmp");
    let log = tmp.path().join("chain.log");
    let alice = User::new(tmp.path(), "alice", &log);
    let arkiv = env!("CARGO_BIN_EXE_arkiv-git");
    let unknown = format!("arkiv::z{}", "1".repeat(32));
    let clone = alice.run("git", tmp.path(), &["clone", "-q", &unknown, "x"]);
    let err = String::from_utf8_lossy(&clone.stderr);
    assert!(!clone.status.success());
    assert!(err.contains("fatal: arkiv-git E_ID_NOT_LIVE"), "{err}");

    assert_eq!(
        alice.run(arkiv, tmp.path(), &["search"]).status.code(),
        Some(2)
    );
    assert_eq!(
        alice.run(arkiv, tmp.path(), &["bogus"]).status.code(),
        Some(2)
    );
    alice.arkiv(&["keys", "init"]);
    let poor = alice.run(arkiv, tmp.path(), &["init", "--name", "poor"]);
    assert_eq!(
        poor.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&poor.stderr)
    );
    assert!(String::from_utf8_lossy(&poor.stderr).contains("E_INSUFFICIENT_FUNDS"));

    fund(&log, field(&alice.arkiv(&["keys", "show"]), "wallet:"));
    let url = field(&alice.arkiv(&["init", "--name", "rich"]), "url:").to_owned();
    let work = tmp.path().join("w");
    alice.git(tmp.path(), &["init", "-q", &work.to_string_lossy()]);
    alice.git(&work, &["commit", "-q", "--allow-empty", "-m", "feat: x"]);
    let bad = alice.run("git", &work, &["push", &url, "main:refs/weird/x"]);
    assert!(!bad.status.success());
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("E_BAD_REFS"),
        "{}",
        String::from_utf8_lossy(&bad.stderr)
    );
    let empty = tmp.path().join("empty");
    let cloned = alice.run(
        "git",
        tmp.path(),
        &["clone", "-q", &url, &empty.to_string_lossy()],
    );
    assert!(
        cloned.status.success(),
        "an empty repo clones: {}",
        String::from_utf8_lossy(&cloned.stderr)
    );
}

#[test]
fn private_repo_members_push_and_clone_outsiders_get_not_member() {
    let tmp = tempfile::tempdir().expect("tmp");
    let log = tmp.path().join("chain.log");
    let [alice, bob, carol] = ["alice", "bob", "carol"].map(|n| User::new(tmp.path(), n, &log));
    for u in [&alice, &bob, &carol] {
        fund(&log, field(&u.arkiv(&["keys", "init"]), "wallet:"));
    }
    let url = field(
        &alice.arkiv(&["init", "--private", "--name", "secret"]),
        "url:",
    )
    .to_owned();
    let kp = field(&bob.arkiv(&["member", "keypackage"]), "key package:").to_owned();
    let added = alice.arkiv(&[
        "member",
        "add",
        "--repo",
        &url,
        "--key-package",
        &kp,
        "--role",
        "read",
    ]);
    assert!(added.contains("as Read"), "{added}");
    assert!(
        alice
            .arkiv(&["member", "list", "--repo", &url])
            .contains("Admin")
    );

    let work = tmp.path().join("work");
    alice.git(tmp.path(), &["init", "-q", &work.to_string_lossy()]);
    write(&work, "README", b"private\n");
    alice.git(&work, &["add", "-A"]);
    alice.git(&work, &["commit", "-q", "-m", "feat: first"]);
    alice.git(&work, &["push", "-q", &url, "main"]);

    let dest = tmp.path().join("bob-clone");
    bob.git(tmp.path(), &["clone", "-q", &url, &dest.to_string_lossy()]);
    assert_eq!(
        std::fs::read(dest.join("README")).expect("read"),
        b"private\n"
    );
    write(&dest, "README", b"bob\n");
    bob.git(&dest, &["commit", "-qam", "feat: bob"]);
    let denied = bob.run("git", &dest, &["push", "-q", &url, "main"]);
    assert!(String::from_utf8_lossy(&denied.stderr).contains("E_ROLE_DENIED"));

    let out = carol.run("git", tmp.path(), &["clone", "-q", &url, "carol-clone"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("E_NOT_MEMBER"));
}

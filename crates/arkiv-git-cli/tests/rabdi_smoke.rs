//! Live Rabdi smoke (design 8.5, minimal): a few-KiB repo pushed with `git push arkiv::`
//! and cloned with `git clone arkiv::` through the real binaries.
//!
//! Pays from the throwaway testnet key in `spike/arkiv-measure/.env` (`PRIVATE_KEY`, optional
//! `ARKIV_ACCESS_KEY`), which it never prints. It moves `BUDGET` to a fresh seed's wallet,
//! runs with 1 hour lifetimes, sweeps what is left back, and asserts the net spend stays
//! under the 0.02 tGLM cap.
//!
//! `ARKIV_GIT_RABDI=1 cargo test -p arkiv-git-cli --test rabdi_smoke -- --ignored --nocapture`

use std::path::{Path, PathBuf};
use std::process::Command;

use alloy_signer_local::PrivateKeySigner;
use arkiv_git_store::{Arkiv, PipelineConfig, RpcArkiv, RpcConfig, TxPipeline, U256};

const CAP_WEI: u128 = 20_000_000_000_000_000; // 0.02 tGLM
const BUDGET_WEI: u128 = 19_000_000_000_000_000; // 0.019 tGLM, swept back afterwards
const LIFETIME: &str = "1800"; // blocks: 1 hour for data and control entities

struct Secrets {
    key: PrivateKeySigner,
    access: Option<String>,
}

fn secrets() -> Secrets {
    let env = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spike/arkiv-measure/.env");
    let text = std::fs::read_to_string(&env).expect("spike/arkiv-measure/.env is readable");
    let get = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_owned())
            .filter(|v| !v.is_empty())
    };
    let key = get("PRIVATE_KEY")
        .and_then(|k| k.parse::<PrivateKeySigner>().ok())
        .unwrap_or_else(|| panic!("PRIVATE_KEY in .env is missing or malformed"));
    Secrets {
        key,
        access: get("ARKIV_ACCESS_KEY"),
    }
}

fn run(home: &Path, access: &Option<String>, program: &str, dir: &Path, args: &[&str]) -> String {
    let bins = PathBuf::from(env!("CARGO_BIN_EXE_git-remote-arkiv"));
    let path = format!(
        "{}:{}",
        bins.parent().expect("dir").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut c = Command::new(program);
    c.current_dir(dir)
        .env("PATH", path)
        .env("HOME", home)
        .env("ARKIV_GIT_HOME", home.join(".arkiv-git"))
        .env_remove("ARKIV_GIT_BACKEND")
        .env("ARKIV_GIT_DATA_LIFETIME", LIFETIME)
        .env("ARKIV_GIT_CONTROL_LIFETIME", LIFETIME)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    if program == "git" {
        c.args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ]);
    }
    c.args(args);
    match access {
        Some(k) => c.env("ARKIV_ACCESS_KEY", k),
        None => c.env_remove("ARKIV_ACCESS_KEY"),
    };
    let out = c.output().expect("spawn");
    let stderr = String::from_utf8_lossy(&out.stderr);
    eprint!("{stderr}");
    assert!(out.status.success(), "{program} {args:?} failed");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
#[ignore = "live Rabdi: needs ARKIV_GIT_RABDI=1 and the funded throwaway key"]
fn live_rabdi_push_then_clone() {
    if std::env::var("ARKIV_GIT_RABDI").as_deref() != Ok("1") {
        eprintln!("skipped: set ARKIV_GIT_RABDI=1");
        return;
    }
    let s = secrets();
    let tmp = tempfile::tempdir().expect("tmp");
    let (alice, bob) = (tmp.path().join("alice"), tmp.path().join("bob"));
    [&alice, &bob]
        .iter()
        .for_each(|d| std::fs::create_dir_all(d).expect("home"));
    let rpc = arkiv_git_cli::block_on(RpcArkiv::connect(RpcConfig::rabdi(s.access.clone())))
        .expect("runtime")
        .expect("connect to Rabdi");
    let before = arkiv_git_cli::block_on(rpc.balance(s.key.address()))
        .expect("rt")
        .expect("balance");
    assert!(
        before >= U256::from(CAP_WEI),
        "throwaway key holds less than the cap"
    );

    run(
        &alice,
        &s.access,
        env!("CARGO_BIN_EXE_arkiv-git"),
        &alice,
        &["keys", "init"],
    );
    let keys = arkiv_git_sync::Keys::require(&alice.join(".arkiv-git")).expect("keys");
    arkiv_git_cli::block_on(async {
        let chain = rpc.chain_id().await.expect("chain");
        let mut p = TxPipeline::new(&rpc, s.key.clone(), chain, PipelineConfig::default());
        let r = p
            .transfer(keys.wallet().address(), U256::from(BUDGET_WEI))
            .await
            .expect("fund");
        assert!(r.success);
    })
    .expect("rt");

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let created = run(
            &alice,
            &s.access,
            env!("CARGO_BIN_EXE_arkiv-git"),
            &alice,
            &["init", "--name", "smoke"],
        );
        let url = created
            .lines()
            .find_map(|l| l.strip_prefix("url: "))
            .expect("url")
            .to_owned();
        eprintln!("smoke repo {url}");
        let work = tmp.path().join("w");
        run(
            &alice,
            &s.access,
            "git",
            tmp.path(),
            &["init", "-q", "-b", "main", &work.to_string_lossy()],
        );
        std::fs::write(work.join("README"), vec![b'x'; 3_000]).expect("write");
        run(&alice, &s.access, "git", &work, &["add", "-A"]);
        run(
            &alice,
            &s.access,
            "git",
            &work,
            &["commit", "-q", "-m", "feat: smoke"],
        );
        run(
            &alice,
            &s.access,
            "git",
            &work,
            &["push", "-q", &url, "main"],
        );
        let dest = tmp.path().join("clone");
        run(
            &bob,
            &s.access,
            "git",
            tmp.path(),
            &["clone", "-q", &url, &dest.to_string_lossy()],
        );
        let head = |d: &Path| run(&alice, &s.access, "git", d, &["rev-parse", "HEAD"]);
        assert_eq!(head(&dest), head(&work));
    }));

    let rid_sweep = std::fs::read_dir(alice.join(".arkiv-git/repos"))
        .map(|d| {
            d.flatten()
                .filter_map(|e| arkiv_git_sync::ids::unhex32(&e.file_name().to_string_lossy()).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    arkiv_git_cli::block_on(async {
        let chain = rpc.chain_id().await.expect("chain");
        let base = rpc.base_fee().await.expect("fee");
        let mut signers = vec![keys.wallet()];
        for rid in &rid_sweep {
            signers.push(keys.control(rid));
            signers.push(keys.bulk(rid));
        }
        let cfg = PipelineConfig::default();
        let reserve = U256::from(30_000u64) * U256::from(2 * base + cfg.tip);
        for signer in signers.into_iter().rev() {
            let have = rpc.balance(signer.address()).await.expect("balance");
            if have <= reserve {
                continue;
            }
            let mut p = TxPipeline::new(&rpc, signer, chain, cfg.clone());
            if let Err(e) = p.transfer(s.key.address(), have - reserve).await {
                eprintln!("sweep failed: {}", e.code());
            }
        }
    })
    .expect("rt");
    let after = arkiv_git_cli::block_on(rpc.balance(s.key.address()))
        .expect("rt")
        .expect("balance");
    let spent = before.saturating_sub(after);
    eprintln!(
        "smoke: net spend {} tGLM",
        arkiv_git_cli::units::fmt_glm(spent)
    );
    assert!(spent <= U256::from(CAP_WEI), "spent over the cap");
    if let Err(p) = result {
        std::panic::resume_unwind(p);
    }
}

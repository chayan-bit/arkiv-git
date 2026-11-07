//! Helpers for the Arkiv-hosted development tests (docs/p3/arkiv-ci.md): the `scripts/`
//! wrappers, patch, CI run, approval and merge, on top of [`super::programs`].
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Output;

use arkiv_git_sync::ids::parse_id;

use super::e2e::*;
use super::programs::*;

/// The CI runner identity of the live self-hosted repo; the template's default attester.
pub const RUNNER_NID: &str = "z76sCb7au55NQrrbUC8YdoGoLSnwqw5vBvFzumNtjCBoi";

pub fn script(u: &User, dir: &Path, name: &str, args: &[&str], ci_cmd: &str) -> Output {
    let path = format!("{}/../../scripts/{name}", env!("CARGO_MANIFEST_DIR"));
    let mut c = u.cmd("bash", dir);
    c.arg(path).args(args).env("ARKIV_CI_CMD", ci_cmd);
    c.output().expect("spawn")
}

pub fn ok(out: Output) -> String {
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn refused(out: Output) -> String {
    assert!(
        !out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

pub fn nid(u: &User) -> String {
    field(&u.arkiv(&["keys", "show"]), "nid:").to_owned()
}

pub fn hex_id(nid: &str) -> String {
    hex::encode(parse_id(nid).expect("nid"))
}

/// `ci-gated.rhai` with `runner_hex` as its only attester instead of the live runner.
pub fn gated_for(runner_hex: &str, dir: &Path) -> PathBuf {
    let src = std::fs::read_to_string(program("ci-gated")).expect("template");
    let live = hex_id(RUNNER_NID);
    assert!(
        src.contains(&live),
        "the template attests with the live CI runner"
    );
    let path = dir.join("ci-gated.rhai");
    std::fs::write(&path, src.replace(&live, runner_hex)).expect("write");
    path
}

pub fn patch(bob: &User, work: &Path, url: &str, branch: &str) -> String {
    ok(script(bob, work, "arkiv-patch.sh", &[url, branch], ""))
        .trim()
        .to_owned()
}

pub fn run_ci(ci: &User, url: &str, cmd: &str) -> String {
    ok(script(
        ci,
        &ci.home,
        "arkiv-ci.sh",
        &["--repo", url, "--once"],
        cmd,
    ))
}

pub fn merge(alice: &User, work: &Path, url: &str, patch: &str) -> Output {
    script(alice, work, "arkiv-merge.sh", &[url, patch], "")
}

pub fn approve(alice: &User, url: &str, patch: &str, head: &str) {
    let review = post(
        alice,
        url,
        "xyz.arkiv.review",
        "create",
        None,
        &format!(r#"{{"patch":"{patch}"}}"#),
    );
    let body = format!(r#"{{"patch":"{patch}","head":"{head}"}}"#);
    post(
        alice,
        url,
        "xyz.arkiv.review",
        "approve",
        Some(&review),
        &body,
    );
}

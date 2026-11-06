//! Helpers for the repo-program end-to-end tests (MemArkiv users from [`super::e2e`]).
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use super::e2e::*;

pub fn program(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../programs/{name}.rhai"));
    p.to_string_lossy().into_owned()
}

/// A funded user with keys.
pub fn user(root: &Path, name: &str, log: &Path) -> User {
    let u = User::new(root, name, log);
    fund(log, field(&u.arkiv(&["keys", "init"]), "wallet:"));
    u
}

/// Repo `name` owned by `owner` with one pre-pin push of `main`; returns (url, work, c0).
pub fn repo_with_main(owner: &User, root: &Path, name: &str) -> (String, PathBuf, String) {
    let url = field(&owner.arkiv(&["init", "--name", name]), "url:").to_owned();
    let work = root.join(format!("{name}-work"));
    owner.git(root, &["init", "-q", &work.to_string_lossy()]);
    write(&work, "README", b"v0\n");
    owner.git(&work, &["add", "-A"]);
    owner.git(&work, &["commit", "-q", "-m", "feat: v0"]);
    owner.git(&work, &["push", "-q", &url, "main"]);
    let c0 = owner.git(&work, &["rev-parse", "HEAD"]);
    (url, work, c0)
}

pub fn commit(u: &User, work: &Path, text: &str) -> String {
    write(work, "README", text.as_bytes());
    u.git(work, &["commit", "-qam", &format!("feat: {}", text.trim())]);
    u.git(work, &["rev-parse", "HEAD"])
}

/// `op post`, returning the op id.
pub fn post(
    u: &User,
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
    u.arkiv(&args).trim().to_owned()
}

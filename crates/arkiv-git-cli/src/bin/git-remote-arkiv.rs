//! `git-remote-arkiv <remote> <url>`: git runs this for `arkiv::<rid>` and `arkiv://<rid>`.

use arkiv_git_cli::{CliError, backend, block_on, helper, member};
use arkiv_git_sync::ids::parse_url;

fn session(remote: &str, url: &str) -> Result<(), CliError> {
    let url = parse_url(url.strip_prefix("arkiv::").unwrap_or(url))?;
    let peers = helper::configured_peers(remote)?;
    block_on(async {
        let (a, mut cfg) = backend::connect().await?;
        cfg.extra_peers = peers;
        member::serve(&a, cfg, url).await
    })?
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, remote, url] = args.as_slice() else {
        eprintln!("usage: git-remote-arkiv <remote> <url> (run by git for arkiv:: URLs)");
        std::process::exit(2);
    };
    if let Err(e) = session(remote, url) {
        eprintln!("fatal: arkiv-git {}", e.line());
        std::process::exit(128);
    }
}

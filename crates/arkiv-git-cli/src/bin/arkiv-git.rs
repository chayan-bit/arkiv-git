//! `arkiv-git`: keys, repo identity, keepalive, search, status, fund.

use arkiv_git_cli::cmd::{Cli, run};
use clap::Parser;

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli) {
        eprintln!("arkiv-git: {}", e.line());
        std::process::exit(e.exit_code());
    }
}

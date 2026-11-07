//! `chart`: keys, repo identity, keepalive, search, status, fund.

use charter_cli::cmd::{Cli, run};
use clap::Parser;

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli) {
        eprintln!("chart: {}", e.line());
        std::process::exit(e.exit_code());
    }
}

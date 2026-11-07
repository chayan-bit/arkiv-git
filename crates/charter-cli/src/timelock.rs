//! `chart timelock seal|open`: drand quicknet timelock encryption of a file.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use charter_timelock::{SealedBlob, fetch_beacon, fmt_unix, open_when_due, parse_until, seal};
use clap::{Args, Subcommand};

use crate::{CliError, Result};

#[derive(Debug, Subcommand)]
pub enum TimelockCmd {
    /// Encrypt a file so anyone can open it once the given time has passed.
    Seal(SealArgs),
    /// Decrypt a sealed file; fails with E_TOO_EARLY before its time.
    Open(OpenArgs),
}

#[derive(Debug, Args)]
pub struct SealArgs {
    /// RFC 3339 instant (2027-01-01T00:00:00Z) or +90s, +30m, +12h, +7d from now.
    #[arg(long)]
    pub until: String,
    #[arg(long = "in")]
    pub input: PathBuf,
    #[arg(long)]
    pub out: PathBuf,
}

#[derive(Debug, Args)]
pub struct OpenArgs {
    #[arg(long = "in")]
    pub input: PathBuf,
    #[arg(long)]
    pub out: PathBuf,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn read(p: &Path) -> Result<Vec<u8>> {
    std::fs::read(p).map_err(|e| CliError::Usage(format!("{}: {e}", p.display())))
}

fn write(p: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(p, bytes).map_err(|e| CliError::Usage(format!("{}: {e}", p.display())))
}

pub fn run(cmd: &TimelockCmd) -> Result<()> {
    match cmd {
        TimelockCmd::Seal(a) => {
            let until = parse_until(&a.until, now())?;
            let blob = seal(&read(&a.input)?, until)?;
            write(&a.out, &blob.to_bytes()?)?;
            println!(
                "sealed until {} (drand quicknet round {})",
                fmt_unix(blob.unlock_at()),
                blob.round
            );
        }
        TimelockCmd::Open(a) => {
            let blob = SealedBlob::from_bytes(&read(&a.input)?)?;
            write(&a.out, &open_when_due(&blob, now(), fetch_beacon)?)?;
            println!("opened (drand quicknet round {})", blob.round);
        }
    }
    Ok(())
}

//! `chart zk verify`: check a sealed-patch receipt offline (docs/p4/zk-sealed-patch.md).

use std::path::{Path, PathBuf};

use charter_core::ErrorCode;
use charter_sync::SyncError;
use charter_zk::Public;
use clap::Subcommand;

use crate::{CliError, Result};

#[derive(Debug, Subcommand)]
pub enum ZkCmd {
    /// Verify a receipt proves the sealed-patch relation for the given public statement.
    Verify {
        #[arg(long)]
        receipt: PathBuf,
        /// JSON: key_hash, ct_hash, base_hash, spec_hash (hex) and spec.
        #[arg(long)]
        public: PathBuf,
    },
}

pub fn zk_cmd(cmd: &ZkCmd) -> Result<()> {
    let ZkCmd::Verify { receipt, public } = cmd;
    verify_files(receipt, public)?;
    println!("ok: receipt proves the sealed-patch relation");
    Ok(())
}

pub fn verify_files(receipt: &Path, public: &Path) -> Result<()> {
    let read =
        |p: &Path| std::fs::read(p).map_err(|e| CliError::Sync(SyncError::io(p.display(), e)));
    let expected: Public = serde_json::from_slice(&read(public)?)
        .map_err(|e| CliError::Usage(format!("{}: {e}", public.display())))?;
    charter_zk::verify(&read(receipt)?, &expected)
        .map_err(|e| CliError::Sync(SyncError::coded(ErrorCode::BadSig, format!("zk: {e}"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../charter-zk/tests/fixtures")
            .join(name)
    }

    #[test]
    fn verify_accepts_the_demo_receipt_and_rejects_a_mismatched_statement() {
        let receipt = fixture("demo.receipt");
        verify_files(&receipt, &fixture("demo.public.json")).expect("demo verifies");
        let err = verify_files(&fixture("demo.sealed"), &fixture("demo.public.json"));
        assert_eq!(err.map_err(|e| e.exit_code()).err(), Some(1));
    }
}

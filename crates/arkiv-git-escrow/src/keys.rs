//! The agent's keys under `$ARKIV_GIT_HOME/escrow/`: an arkiv-git identity (`keys/seed`)
//! that signs escrow ops, and a dedicated payout EVM key (`keys/payout`) that holds the
//! funds. Both files are mode 0600 and are never printed.
//!
//! The payout key sends payouts and refunds only; ops are funded from the identity's own
//! wallet. The audit relies on this: every mined tx of the payout key needs a settlement op.

use std::path::{Path, PathBuf};

use alloy_signer_local::PrivateKeySigner;
use arkiv_git_store::{Address, B256};
use arkiv_git_sync::{Keys, SyncError};

use crate::{Result, bad, internal};

pub struct EscrowKeys {
    pub id: Keys,
    payout: PrivateKeySigner,
}

impl std::fmt::Debug for EscrowKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EscrowKeys({:?}, {})", self.id, self.address())
    }
}

/// `<home>/escrow`.
pub fn escrow_home(home: &Path) -> PathBuf {
    home.join("escrow")
}

fn payout_path(dir: &Path) -> PathBuf {
    dir.join("keys").join("payout")
}

impl EscrowKeys {
    pub fn new(id: Keys, payout: PrivateKeySigner) -> Self {
        Self { id, payout }
    }

    /// Creates both keys under `<home>/escrow`; refuses to overwrite either.
    pub fn init(home: &Path) -> Result<Self> {
        let dir = escrow_home(home);
        let p = payout_path(&dir);
        if p.exists() {
            return Err(internal(format!(
                "{} exists; refusing to overwrite",
                p.display()
            )));
        }
        let id = Keys::init(&dir)?;
        let payout = PrivateKeySigner::random();
        write_secret(&p, payout.to_bytes().as_slice())?;
        Ok(Self { id, payout })
    }

    /// Loads both keys or fails with a hint to run `arkiv-git escrow init`.
    pub fn require(home: &Path) -> Result<Self> {
        let dir = escrow_home(home);
        let p = payout_path(&dir);
        let bytes = std::fs::read(&p).map_err(|e| {
            internal(format!(
                "{}: {e} (run `arkiv-git escrow init`)",
                p.display()
            ))
        })?;
        let b: [u8; 32] = bytes
            .try_into()
            .map_err(|_| bad(format!("{} is not 32 bytes", p.display())))?;
        let payout = PrivateKeySigner::from_bytes(&B256::from(b))
            .map_err(|e| bad(format!("{}: {e}", p.display())))?;
        Ok(Self {
            id: Keys::require(&dir)?,
            payout,
        })
    }

    /// The escrow address deposits go to.
    pub fn address(&self) -> Address {
        self.payout.address()
    }

    pub(crate) fn payout(&self) -> &PrivateKeySigner {
        &self.payout
    }
}

fn write_secret(p: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let dir = p
        .parent()
        .ok_or_else(|| SyncError::io(p.display(), "no parent"))?;
    std::fs::create_dir_all(dir).map_err(|e| SyncError::io(dir.display(), e))?;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(p).map_err(|e| SyncError::io(p.display(), e))?;
    f.write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| SyncError::io(p.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_writes_private_keys_once_and_reloads_them() {
        let d = tempfile::tempdir().expect("tmp");
        assert!(EscrowKeys::require(d.path()).is_err());
        let k = EscrowKeys::init(d.path()).expect("init");
        let again = EscrowKeys::require(d.path()).expect("load");
        assert_eq!((k.address(), k.id.nid()), (again.address(), again.id.nid()));
        assert!(EscrowKeys::init(d.path()).is_err(), "no overwrite");
        assert_ne!(
            k.address(),
            k.id.wallet().address(),
            "payout key is dedicated"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = payout_path(&escrow_home(d.path()));
            let mode = std::fs::metadata(p).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(!format!("{k:?}").contains(&B256::from(k.payout.to_bytes()).to_string()));
    }
}

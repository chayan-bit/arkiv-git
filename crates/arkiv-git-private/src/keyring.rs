//! A member's keys for one private repo: the id-blinding key, every epoch content key it
//! holds, and the roles map. The same struct is the sealed lockbox a commit or welcome
//! carries, and the local file under `$ARKIV_GIT_HOME/private/`, mode 600.

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use arkiv_git_core::{ErrorCode, Nid, Rid};
use arkiv_git_sync::{Result, SyncError};
use serde::{Deserialize, Serialize};

use crate::seal::Key;

/// GitHub-style roles: admin manages members, write pushes, read clones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Read,
    Write,
    Admin,
}

impl std::str::FromStr for Role {
    type Err = SyncError;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "admin" => Ok(Role::Admin),
            "write" => Ok(Role::Write),
            "read" => Ok(Role::Read),
            _ => Err(SyncError::coded(
                ErrorCode::BadFormat,
                format!("role must be admin, write or read, got {s:?}"),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keyring {
    pub rid: Rid,
    /// HMAC key for `ag.rid` / `ag.name`, exported at the creating epoch and never rotated.
    pub id_key: Key,
    pub epochs: BTreeMap<u64, Key>,
    /// Hex nid to role.
    pub roles: BTreeMap<String, Role>,
    /// Local MLS group id (also visible in MLS message headers, so not secret).
    pub group: Vec<u8>,
}

impl Keyring {
    /// Newest epoch and its key; every keyring holds at least one.
    pub fn current(&self) -> Result<(u64, &Key)> {
        self.epochs
            .iter()
            .next_back()
            .map(|(e, k)| (*e, k))
            .ok_or_else(|| not_member(&self.rid))
    }

    pub fn role(&self, nid: &Nid) -> Option<Role> {
        self.roles.get(&hex::encode(nid)).copied()
    }

    /// `E_ROLE_DENIED` unless `nid` holds at least `min`.
    pub fn require(&self, nid: &Nid, min: Role) -> Result<()> {
        match self.role(nid) {
            Some(r) if r >= min => Ok(()),
            have => Err(SyncError::coded(
                ErrorCode::RoleDenied,
                format!("{min:?} role needed in this private repo, you have {have:?}"),
            )),
        }
    }

    /// Adopts a newer lockbox: its roles, plus every epoch key either side holds.
    pub fn merge(&mut self, newer: Keyring) {
        self.epochs.extend(newer.epochs);
        self.roles = newer.roles;
        self.id_key = newer.id_key;
    }

    pub fn path(home: &Path, rid: &Rid) -> PathBuf {
        home.join("private")
            .join(format!("{}.json", hex::encode(rid)))
    }

    pub fn load(home: &Path, rid: &Rid) -> Result<Option<Self>> {
        let p = Self::path(home, rid);
        match std::fs::read(&p) {
            Ok(b) => Ok(Some(
                serde_json::from_slice(&b).map_err(|e| bad_file(&p, e))?,
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(SyncError::io("read private keyring", e)),
        }
    }

    pub fn save(&self, home: &Path) -> Result<()> {
        let bytes = serde_json::to_vec(self).map_err(|e| bad_file(&home.join("private"), e))?;
        write_private(&Self::path(home, &self.rid), &bytes)
    }
}

pub fn not_member(rid: &Rid) -> SyncError {
    SyncError::coded(
        ErrorCode::NotMember,
        format!(
            "{} is a private repo and you hold no key for it (ask an admin for `arkiv-git member add`)",
            arkiv_git_sync::ids::to_text(rid)
        ),
    )
}

fn bad_file(p: &Path, e: serde_json::Error) -> SyncError {
    SyncError::coded(ErrorCode::BadFormat, format!("{}: {e}", p.display()))
}

/// Writes `bytes` to `path` with mode 600 (directory 700), via a temp file and rename.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let io = |e| SyncError::io("write private state", e);
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(io)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(io)?;
    f.write_all(bytes).and_then(|()| f.sync_all()).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_order_and_files_are_private() {
        let mut ring = Keyring {
            rid: [1; 32],
            id_key: [2; 32],
            epochs: BTreeMap::from([(1, [3; 32]), (2, [4; 32])]),
            roles: BTreeMap::from([(hex::encode([5u8; 32]), Role::Write)]),
            group: vec![7],
        };
        assert_eq!(ring.current().expect("cur").0, 2);
        assert!(ring.require(&[5; 32], Role::Read).is_ok());
        assert!(ring.require(&[5; 32], Role::Admin).is_err());
        assert!(ring.require(&[6; 32], Role::Read).is_err());
        let home = tempfile::tempdir().expect("tmp");
        ring.save(home.path()).expect("save");
        let mode = std::fs::metadata(Keyring::path(home.path(), &ring.rid))
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let back = Keyring::load(home.path(), &ring.rid).expect("load");
        assert_eq!(back.as_ref(), Some(&ring));
        ring.epochs.clear();
        assert_eq!(
            ring.current().map_err(|e| e.code()).err(),
            Some(ErrorCode::NotMember)
        );
    }
}

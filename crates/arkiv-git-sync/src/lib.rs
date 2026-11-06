//! arkiv-git-sync: moves verified data between a local git cache and Arkiv (design 4-6).
//!
//! The git side shells out to stock `git` ([`git`]); the Arkiv side is generic over
//! [`arkiv_git_store::Arkiv`], so the same code runs against Rabdi and `MemArkiv`.
//! Verification rules live in `arkiv-git-core`; this crate feeds them what Arkiv returns
//! and adds the checks core leaves to sync (chunks, packs, connectivity).

mod error;
pub mod fetch;
pub mod git;
pub mod identity;
pub mod ids;
pub mod keepalive;
pub mod keys;
pub mod observe;
pub mod packs;
pub mod push;
pub mod push_send;
pub mod repo;
pub mod search;
pub mod state;
pub mod view;
pub mod write;

pub use error::{Result, SyncError, store_code};
pub use fetch::{Listing, listing, own_refs, sync};
pub use keys::Keys;
pub use push::{Push, Update};
pub use push_send::{Cost, Gate};
pub use repo::{Config, Repo};
pub use view::{View, Want};

/// Diagnostics go to stderr (stdout belongs to the git remote-helper protocol).
pub fn warn(msg: &str) {
    eprintln!("warning: {msg}");
}

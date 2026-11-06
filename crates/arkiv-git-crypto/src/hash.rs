use crate::Hash32;
use sha2::{Digest, Sha256};

/// Plain SHA-256, for raw byte blobs (packs, chunks).
pub fn sha256(x: &[u8]) -> Hash32 {
    Sha256::digest(x).into()
}

/// Tagged hash for structured objects:
/// `H(tag, x) = SHA-256("arkiv-git/v1/" || tag || 0x00 || x)` (design section 2.1).
pub fn h(tag: &str, x: &[u8]) -> Hash32 {
    let mut d = Sha256::new();
    d.update(b"arkiv-git/v1/");
    d.update(tag.as_bytes());
    d.update([0u8]);
    d.update(x);
    d.finalize().into()
}

//! Pack chunking (design section 2.2, `manifest` and `chunk`).

use crate::ManifestBody;
use arkiv_git_crypto::sha256;

pub const CHUNK_SIZE: usize = 65_536;
/// 100 MiB, so one manifest (1,600 chunk hashes) always fits in one entity.
pub const MAX_PACK_SIZE: u64 = 104_857_600;

pub fn chunk_count(size: u64) -> u64 {
    size.div_ceil(CHUNK_SIZE as u64)
}

/// The stored chunks of a pack, in index order; the last one may be shorter.
pub fn split(pack: &[u8]) -> std::slice::Chunks<'_, u8> {
    pack.chunks(CHUNK_SIZE)
}

/// Manifest field 6: concatenated SHA-256 of every chunk.
pub fn chunk_hashes(pack: &[u8]) -> Vec<u8> {
    split(pack).flat_map(sha256).collect()
}

/// Whether `bytes` is chunk `i` of the manifest's pack (hash and length).
pub fn verify_chunk(m: &ManifestBody, i: u64, bytes: &[u8]) -> bool {
    let Some(expected) = m.chunk_hash(i) else {
        return false;
    };
    let start = i.saturating_mul(CHUNK_SIZE as u64);
    let want_len = m.size.saturating_sub(start).min(CHUNK_SIZE as u64);
    bytes.len() as u64 == want_len && sha256(bytes) == expected
}

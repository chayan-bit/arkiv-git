//! Design section 8.1 chunking: split and reassemble at boundary sizes.

use arkiv_git_core::*;
use arkiv_git_crypto::sha256;

fn manifest_for(pack: &[u8]) -> ManifestBody {
    ManifestBody {
        rid: [1; 32],
        pack_id: sha256(pack),
        trailer: vec![0; 20],
        size: pack.len() as u64,
        chunk_size: CHUNK_SIZE as u64,
        chunk_hashes: chunk_hashes(pack),
        requires: vec![],
        pack_kind: 1,
    }
}

fn round_trip(size: usize) {
    let pack: Vec<u8> = (0..size).map(|i| (i * 31 % 251) as u8).collect();
    let chunks: Vec<&[u8]> = split(&pack).collect();
    assert_eq!(chunks.len() as u64, chunk_count(size as u64), "size {size}");
    assert!(
        chunks
            .iter()
            .all(|c| c.len() <= CHUNK_SIZE && !c.is_empty())
    );
    assert_eq!(chunks.concat(), pack);
    if size == 0 {
        return;
    }
    let m = manifest_for(&pack);
    ManifestBody::decode(&m.encode()).expect("valid manifest");
    for (i, c) in chunks.iter().enumerate() {
        assert!(verify_chunk(&m, i as u64, c));
        assert!(!verify_chunk(&m, i as u64 + 1, c), "index binds the chunk");
    }
}

#[test]
fn chunking_boundaries() {
    for size in [0, 1, 65_535, 65_536, 65_537] {
        round_trip(size);
    }
}

#[test]
fn chunking_100_mib() {
    round_trip(MAX_PACK_SIZE as usize);
    assert_eq!(chunk_count(MAX_PACK_SIZE), 1600);
}

#[test]
fn empty_and_oversized_packs_are_invalid_manifests() {
    let mut m = manifest_for(&[1]);
    m.size = 0;
    m.chunk_hashes = vec![];
    assert!(ManifestBody::decode(&m.encode()).is_err());
    let mut m = manifest_for(&[1]);
    m.size = MAX_PACK_SIZE + 1;
    assert!(ManifestBody::decode(&m.encode()).is_err());
}

#[test]
fn tampered_chunk_fails() {
    let pack = vec![7u8; 70_000];
    let m = manifest_for(&pack);
    let mut c = pack[..CHUNK_SIZE].to_vec();
    c[0] ^= 1;
    assert!(!verify_chunk(&m, 0, &c));
    assert!(!verify_chunk(&m, 5, &pack[..10]));
}

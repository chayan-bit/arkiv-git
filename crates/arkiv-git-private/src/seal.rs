//! Content sealing: XChaCha20-Poly1305 under an epoch key with a random 24-byte nonce,
//! plaintext padded to a bucket, and HMAC-SHA-256 blinding of queryable identifiers.

use arkiv_git_core::{ErrorCode, Hash32};
use arkiv_git_sync::{Result, SyncError};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// A 32-byte symmetric key (epoch content key or id-blinding key).
pub type Key = [u8; 32];

pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
/// The padded plaintext starts with the real length as a big-endian u32.
pub const LEN_PREFIX: usize = 4;
pub const MIN_BUCKET: usize = 256;
/// Largest power-of-two bucket: every signed `ag.body` and every chunk is at most 64 KiB.
pub const MAX_POW2_BUCKET: usize = 65_536;
/// Above 64 KiB (a full chunk or near-maximal envelope plus its few hidden attributes) the
/// bucket is the next 4 KiB multiple, so all full chunks seal to one size under the
/// 128 KiB attribute and tx caps that a doubled bucket would break.
const BIG_STEP: usize = 4096;

/// Padded size for `len` plaintext bytes: a power of two from 256 B to 64 KiB.
pub fn bucket(len: usize) -> usize {
    if len <= MAX_POW2_BUCKET {
        len.max(MIN_BUCKET).next_power_of_two()
    } else {
        len.div_ceil(BIG_STEP) * BIG_STEP
    }
}

/// Sealed length of `len` plaintext bytes.
pub fn sealed_len(len: usize) -> usize {
    NONCE_LEN + LEN_PREFIX + bucket(len) + TAG_LEN
}

/// `rid || kind || u64be(epoch)`.
pub fn aad(rid: &Hash32, kind: &str, epoch: u64) -> Vec<u8> {
    [rid.as_slice(), kind.as_bytes(), &epoch.to_be_bytes()].concat()
}

fn internal(msg: &str) -> SyncError {
    SyncError::coded(ErrorCode::Internal, msg.to_owned())
}

/// `nonce || XChaCha20-Poly1305(key, nonce, u32be(len) || plaintext || zeros, aad)`.
pub fn seal(key: &Key, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    let len = u32::try_from(plaintext.len()).map_err(|_| internal("plaintext over 4 GiB"))?;
    let mut padded = Vec::with_capacity(LEN_PREFIX + bucket(plaintext.len()));
    padded.extend_from_slice(&len.to_be_bytes());
    padded.extend_from_slice(plaintext);
    padded.resize(LEN_PREFIX + bucket(plaintext.len()), 0);
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| internal("no OS randomness"))?;
    let ct = XChaCha20Poly1305::new(key.into())
        .encrypt(&XNonce::from(nonce), Payload { msg: &padded, aad })
        .map_err(|_| internal("seal failed"))?;
    Ok([nonce.as_slice(), &ct].concat())
}

/// Inverse of [`seal`]; `None` for a wrong key, wrong aad, tampering or bad padding.
pub fn open(key: &Key, aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    let (nonce, ct) = sealed.split_at_checked(NONCE_LEN)?;
    let nonce: [u8; NONCE_LEN] = nonce.try_into().ok()?;
    let padded = XChaCha20Poly1305::new(key.into())
        .decrypt(&XNonce::from(nonce), Payload { msg: ct, aad })
        .ok()?;
    let (len, rest) = padded.split_at_checked(LEN_PREFIX)?;
    let len = usize::try_from(u32::from_be_bytes(len.try_into().ok()?)).ok()?;
    let (body, pad) = rest.split_at_checked(len)?;
    (pad.len() == bucket(len) - len && pad.iter().all(|b| *b == 0)).then(|| body.to_vec())
}

/// `HMAC-SHA-256(id_key, label || 0x00 || value)`: a stable, unlinkable stand-in for an id.
pub fn blind(id_key: &Key, label: &str, value: &[u8]) -> Hash32 {
    let mut m = Hmac::<Sha256>::new_from_slice(id_key)
        .unwrap_or_else(|_| unreachable!("HMAC takes any key length"));
    m.update(label.as_bytes());
    m.update(&[0]);
    m.update(value);
    m.finalize().into_bytes().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_powers_of_two_up_to_64k_then_4k_steps() {
        assert_eq!(bucket(0), 256);
        assert_eq!(bucket(257), 512);
        assert_eq!(bucket(3_500), 4_096);
        assert_eq!(bucket(65_536), 65_536);
        assert_eq!(bucket(65_600), 69_632);
        assert!(
            sealed_len(65_600) < 131_030,
            "a sealed full chunk fits one tx"
        );
    }

    #[test]
    fn seal_round_trips_and_rejects_wrong_key_or_aad() {
        let (k, a) = ([1u8; 32], aad(&[2; 32], "anchor", 3));
        let s = seal(&k, &a, b"hello").expect("seal");
        assert_eq!(s.len(), sealed_len(5));
        assert_eq!(open(&k, &a, &s).as_deref(), Some(&b"hello"[..]));
        assert!(open(&[9; 32], &a, &s).is_none());
        assert!(open(&k, &aad(&[2; 32], "anchor", 4), &s).is_none());
        assert_ne!(seal(&k, &a, b"hello").expect("seal"), s, "random nonce");
        assert_ne!(blind(&k, "rid", b"x"), blind(&k, "name", b"x"));
    }
}

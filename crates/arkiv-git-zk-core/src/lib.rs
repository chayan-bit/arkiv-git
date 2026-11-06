//! The sealed-patch relation (docs/p4/zk-sealed-patch.md).
//!
//! A prover knows key `k` and ciphertext `C` such that `C` decrypts under `k` to a patch `P`,
//! and `P` applied to base file `F` yields a file that satisfies spec `S`.
//! [`check`] evaluates the relation and returns the [`Public`] statement the zkVM guest commits.
#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// XChaCha20 nonce length; a sealed blob is `nonce || ciphertext || tag`.
pub const NONCE_LEN: usize = 24;

/// What the verifier learns: hashes of the secrets and the public spec itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Public {
    /// `sha256(k)`.
    #[serde(with = "hex::serde")]
    pub key_hash: [u8; 32],
    /// `sha256(C)`.
    #[serde(with = "hex::serde")]
    pub ct_hash: [u8; 32],
    /// `sha256(F)`.
    #[serde(with = "hex::serde")]
    pub base_hash: [u8; 32],
    /// `sha256(S)`.
    #[serde(with = "hex::serde")]
    pub spec_hash: [u8; 32],
    /// The spec `S`, in the line format of [`spec_holds`].
    pub spec: String,
}

/// The prover's secret witness.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Private {
    pub key: [u8; 32],
    pub sealed: Vec<u8>,
    pub base: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RelationError {
    Decrypt,
    PatchNotUtf8,
    Patch(&'static str),
    SpecFailed,
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// Evaluates the relation and returns the public statement, or why it does not hold.
pub fn check(private: &Private, spec: &str) -> Result<Public, RelationError> {
    let patch = open(&private.key, &private.sealed)?;
    let patch = core::str::from_utf8(&patch).map_err(|_| RelationError::PatchNotUtf8)?;
    let result = apply_patch(&private.base, patch)?;
    if !spec_holds(spec, &result) {
        return Err(RelationError::SpecFailed);
    }
    Ok(Public {
        key_hash: sha256(&private.key),
        ct_hash: sha256(&private.sealed),
        base_hash: sha256(private.base.as_bytes()),
        spec_hash: sha256(spec.as_bytes()),
        spec: spec.into(),
    })
}

/// Encrypts `patch` as `nonce || ciphertext || tag` (host side; the guest only opens).
pub fn seal(key: &[u8; 32], nonce: &[u8; NONCE_LEN], patch: &[u8]) -> Vec<u8> {
    let ct = XChaCha20Poly1305::new(key.into())
        .encrypt(&(*nonce).into(), patch)
        .unwrap_or_default(); // shortcut: encrypt only fails past 256 GiB, unreachable here
    [nonce.as_slice(), &ct].concat()
}

/// Decrypts and authenticates a sealed blob.
pub fn open(key: &[u8; 32], sealed: &[u8]) -> Result<Vec<u8>, RelationError> {
    if sealed.len() < NONCE_LEN {
        return Err(RelationError::Decrypt);
    }
    let (nonce, ct) = sealed.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| RelationError::Decrypt)?;
    XChaCha20Poly1305::new(key.into())
        .decrypt(&nonce.into(), ct)
        .map_err(|_| RelationError::Decrypt)
}

/// Applies a line patch to `base` (lines split on `\n`, rejoined with `\n`).
///
/// Each patch line is one op, applied in order with a cursor over the base lines:
/// `= N` copies the next N base lines, `- TEXT` drops the next base line which must equal TEXT,
/// `+ TEXT` emits TEXT. Base lines left after the last op are copied unchanged.
pub fn apply_patch(base: &str, patch: &str) -> Result<String, RelationError> {
    let mut src = base.split('\n');
    let mut out: Vec<&str> = Vec::new();
    for op in patch.lines() {
        match op.split_at_checked(2) {
            Some(("= ", n)) => {
                let n: usize = n.parse().map_err(|_| RelationError::Patch("bad count"))?;
                for _ in 0..n {
                    out.push(src.next().ok_or(RelationError::Patch("copy past end"))?);
                }
            }
            Some(("- ", text)) => {
                if src.next() != Some(text) {
                    return Err(RelationError::Patch("delete does not match base"));
                }
            }
            Some(("+ ", text)) => out.push(text),
            _ => return Err(RelationError::Patch("unknown op")),
        }
    }
    out.extend(src);
    Ok(out.join("\n"))
}

/// Evaluates a spec: each line is `+ TEXT` (result must contain TEXT) or `- TEXT` (must not).
/// Any other non-empty line fails the spec, so a typo can never pass silently.
pub fn spec_holds(spec: &str, result: &str) -> bool {
    spec.lines()
        .filter(|l| !l.is_empty())
        .all(|l| match l.split_at_checked(2) {
            Some(("+ ", t)) => result.contains(t),
            Some(("- ", t)) => !result.contains(t),
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "fn add(a: i32, b: i32) -> i32 {\n    a - b\n}\n";
    const PATCH: &str = "= 1\n-     a - b\n+     a + b\n";
    const SPEC: &str = "+ a + b\n- a - b\n";
    const KEY: [u8; 32] = [7; 32];

    fn private(patch: &str) -> Private {
        Private {
            key: KEY,
            sealed: seal(&KEY, &[1; NONCE_LEN], patch.as_bytes()),
            base: BASE.into(),
        }
    }

    #[test]
    fn patch_applies_and_keeps_trailing_newline() {
        let out = apply_patch(BASE, PATCH);
        assert_eq!(
            out.as_deref(),
            Ok("fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n")
        );
    }

    #[test]
    fn patch_rejects_mismatched_delete_and_overrun() {
        assert!(apply_patch(BASE, "-     a * b").is_err());
        assert!(apply_patch(BASE, "= 9").is_err());
        assert!(apply_patch(BASE, "x").is_err());
    }

    #[test]
    fn spec_checks_required_and_forbidden_and_rejects_junk() {
        assert!(spec_holds(SPEC, "a + b"));
        assert!(!spec_holds(SPEC, "a - b; a + b"));
        assert!(!spec_holds("oops", "anything"));
    }

    #[test]
    fn check_returns_hashes_of_the_witness() {
        let p = private(PATCH);
        let public = check(&p, SPEC).expect("relation holds");
        assert_eq!(public.key_hash, sha256(&KEY));
        assert_eq!(public.ct_hash, sha256(&p.sealed));
        assert_eq!(public.spec_hash, sha256(SPEC.as_bytes()));
    }

    #[test]
    fn check_fails_on_wrong_key_or_failing_spec() {
        let mut p = private(PATCH);
        assert_eq!(check(&p, "+ a * b").err(), Some(RelationError::SpecFailed));
        p.key = [8; 32];
        assert_eq!(check(&p, SPEC).err(), Some(RelationError::Decrypt));
    }
}

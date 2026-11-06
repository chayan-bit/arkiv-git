//! Composite ML-DSA-65 + Ed25519 (RFC 10073) behaviour tests.

use arkiv_git_crypto::{Alg, CryptoError, MasterSeed, PK_LEN, PublicKey, SIG_LEN, SecretKey, Sig};
use proptest::prelude::*;
use std::sync::OnceLock;

const ML_SIG_LEN: usize = 3309;

fn key() -> &'static SecretKey {
    static K: OnceLock<SecretKey> = OnceLock::new();
    K.get_or_init(|| MasterSeed::from_bytes([7u8; 32]).signing_key())
}

fn signed(msg: &[u8], ctx: &[u8]) -> Sig {
    key().sign(msg, ctx).expect("sign")
}

fn flip(sig: &Sig, bit: usize) -> Sig {
    let mut b = sig.as_bytes().to_vec();
    b[bit / 8] ^= 1 << (bit % 8);
    Sig::from_bytes(Alg::MlDsa65Ed25519, &b).expect("length unchanged")
}

fn splice(ml_from: &Sig, ed_from: &Sig) -> Sig {
    let mut b = ml_from.as_bytes()[..ML_SIG_LEN].to_vec();
    b.extend_from_slice(&ed_from.as_bytes()[ML_SIG_LEN..]);
    Sig::from_bytes(Alg::MlDsa65Ed25519, &b).expect("length")
}

#[test]
fn sizes_match_rfc10073_table() {
    assert_eq!(PK_LEN, 1984);
    assert_eq!(SIG_LEN, 3373);
    assert_eq!(key().public_key().as_bytes().len(), PK_LEN);
    assert_eq!(signed(b"m", b"c").as_bytes().len(), SIG_LEN);
}

#[test]
fn round_trip_verifies() {
    let pk = key().public_key();
    let sig = signed(b"hello", b"arkiv-git/anchor/v1");
    assert!(pk.verify(b"hello", b"arkiv-git/anchor/v1", &sig).is_ok());
}

#[test]
fn public_key_and_sig_round_trip_through_bytes() {
    let pk = key().public_key();
    let pk2 = PublicKey::from_bytes(Alg::MlDsa65Ed25519, pk.as_bytes()).expect("pk");
    let sig = signed(b"x", b"");
    let sig2 = Sig::from_bytes(Alg::MlDsa65Ed25519, sig.as_bytes()).expect("sig");
    assert!(pk2.verify(b"x", b"", &sig2).is_ok());
    assert_eq!(pk, pk2);
}

#[test]
fn wrong_context_fails() {
    let pk = key().public_key();
    let sig = signed(b"hello", b"arkiv-git/anchor/v1");
    assert_eq!(
        pk.verify(b"hello", b"arkiv-git/refs/v1", &sig),
        Err(CryptoError::BadSignature)
    );
    assert!(pk.verify(b"hello", b"", &sig).is_err());
}

#[test]
fn wrong_message_fails() {
    let pk = key().public_key();
    let sig = signed(b"hello", b"c");
    assert!(pk.verify(b"hellp", b"c", &sig).is_err());
}

#[test]
fn wrong_key_fails() {
    let other = MasterSeed::from_bytes([8u8; 32]).signing_key().public_key();
    assert!(
        other
            .verify(b"hello", b"c", &signed(b"hello", b"c"))
            .is_err()
    );
}

#[test]
fn every_ed25519_bit_flip_fails() {
    let pk = key().public_key();
    let sig = signed(b"bits", b"c");
    for bit in ML_SIG_LEN * 8..SIG_LEN * 8 {
        assert!(
            pk.verify(b"bits", b"c", &flip(&sig, bit)).is_err(),
            "bit {bit}"
        );
    }
}

#[test]
fn strided_ml_dsa_bit_flips_fail() {
    let pk = key().public_key();
    let sig = signed(b"bits", b"c");
    for bit in (0..ML_SIG_LEN * 8).step_by(211) {
        assert!(
            pk.verify(b"bits", b"c", &flip(&sig, bit)).is_err(),
            "bit {bit}"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn any_single_bit_flip_fails(bit in 0usize..SIG_LEN * 8) {
        let pk = key().public_key();
        let sig = signed(b"prop", b"c");
        prop_assert!(pk.verify(b"prop", b"c", &flip(&sig, bit)).is_err());
    }
}

#[test]
fn valid_ml_dsa_half_with_invalid_ed25519_half_fails() {
    let pk = key().public_key();
    let a = signed(b"message a", b"c");
    let b = signed(b"message b", b"c");
    assert!(pk.verify(b"message a", b"c", &splice(&a, &b)).is_err());
}

#[test]
fn valid_ed25519_half_with_invalid_ml_dsa_half_fails() {
    let pk = key().public_key();
    let a = signed(b"message a", b"c");
    let b = signed(b"message b", b"c");
    assert!(pk.verify(b"message a", b"c", &splice(&b, &a)).is_err());
}

#[test]
fn signing_is_hedged_in_the_ml_dsa_half_only() {
    let a = signed(b"same", b"c");
    let b = signed(b"same", b"c");
    assert_ne!(a.as_bytes()[..ML_SIG_LEN], b.as_bytes()[..ML_SIG_LEN]);
    assert_eq!(a.as_bytes()[ML_SIG_LEN..], b.as_bytes()[ML_SIG_LEN..]);
}

#[test]
fn context_longer_than_255_bytes_is_rejected() {
    assert_eq!(
        key().sign(b"m", &[0u8; 256]),
        Err(CryptoError::ContextTooLong)
    );
    let sig = signed(b"m", b"");
    assert!(key().public_key().verify(b"m", &[0u8; 256], &sig).is_err());
}

#[test]
fn malformed_lengths_are_rejected() {
    assert_eq!(
        PublicKey::from_bytes(Alg::MlDsa65Ed25519, &[0u8; PK_LEN - 1]),
        Err(CryptoError::BadLength)
    );
    assert_eq!(
        Sig::from_bytes(Alg::MlDsa65Ed25519, &[0u8; SIG_LEN + 1]),
        Err(CryptoError::BadLength)
    );
}

#[test]
fn invalid_ed25519_point_in_public_key_is_rejected() {
    let mut b = key().public_key().as_bytes().to_vec();
    // y = 2 is not on the curve (no square root for x).
    b[PK_LEN - 32..].copy_from_slice(&{
        let mut y = [0u8; 32];
        y[0] = 2;
        y
    });
    assert_eq!(
        PublicKey::from_bytes(Alg::MlDsa65Ed25519, &b),
        Err(CryptoError::BadKey)
    );
}

#[test]
fn algorithm_registry_round_trips() {
    assert_eq!(
        Alg::from_id(Alg::MlDsa65Ed25519.id()),
        Some(Alg::MlDsa65Ed25519)
    );
    assert_eq!(Alg::from_id(0), None);
    assert_eq!(Alg::MlDsa65Ed25519.pk_len(), PK_LEN);
    assert_eq!(Alg::MlDsa65Ed25519.sig_len(), SIG_LEN);
}

#[test]
fn error_codes_are_stable() {
    assert_eq!(CryptoError::BadSignature.code(), "E_BAD_SIG");
    assert_eq!(CryptoError::BadLength.code(), "E_BAD_FORMAT");
    assert_eq!(CryptoError::BadKey.code(), "E_BAD_FORMAT");
}

#[test]
fn secret_key_debug_does_not_leak() {
    let s = format!("{:?}", key());
    assert!(!s.contains("07"), "{s}");
}

#[test]
fn secret_key_and_seed_zeroize_on_drop() {
    fn assert_zod<T: zeroize::ZeroizeOnDrop>() {}
    assert_zod::<SecretKey>();
    assert_zod::<MasterSeed>();
}

#[test]
fn generated_seeds_are_fresh_and_debug_is_redacted() {
    let a = MasterSeed::generate().expect("os rng");
    let b = MasterSeed::generate().expect("os rng");
    assert_ne!(a.expose(), b.expose());
    assert_eq!(MasterSeed::from_bytes(*a.expose()).expose(), a.expose());
    assert_eq!(format!("{a:?}"), "MasterSeed(..)");
}

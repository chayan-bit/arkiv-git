//! Verifies the committed demo receipt (produced by scripts/zk-prove.sh) on this machine.

use arkiv_git_zk::{Public, ZkError, verify};
use arkiv_git_zk_core::sha256;

const RECEIPT: &[u8] = include_bytes!("fixtures/demo.receipt");
const SEALED: &[u8] = include_bytes!("fixtures/demo.sealed");
const BASE: &[u8] = include_bytes!("../../../zk/demo/base.rs");
const SPEC: &str = include_str!("../../../zk/demo/spec.txt");

fn public() -> Public {
    serde_json::from_str(include_str!("fixtures/demo.public.json")).expect("fixture json")
}

#[test]
fn demo_receipt_verifies_against_its_public_statement() {
    verify(RECEIPT, &public()).expect("demo receipt verifies");
}

#[test]
fn public_statement_binds_the_published_ciphertext_base_and_spec() {
    let p = public();
    assert_eq!(p.ct_hash, sha256(SEALED));
    assert_eq!(p.base_hash, sha256(BASE));
    assert_eq!(
        (p.spec.as_str(), p.spec_hash),
        (SPEC, sha256(SPEC.as_bytes()))
    );
}

#[test]
fn tampered_public_input_fails() {
    let mut p = public();
    p.key_hash[0] ^= 1;
    assert!(matches!(verify(RECEIPT, &p), Err(ZkError::Mismatch)));
    let mut p = public();
    p.spec = "+ anything\n".into();
    assert!(matches!(verify(RECEIPT, &p), Err(ZkError::Mismatch)));
}

#[test]
fn tampered_receipt_fails() {
    let mut bytes = RECEIPT.to_vec();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 1;
    assert!(verify(&bytes, &public()).is_err());
    assert!(matches!(
        verify(b"junk", &public()),
        Err(ZkError::Decode(_))
    ));
}

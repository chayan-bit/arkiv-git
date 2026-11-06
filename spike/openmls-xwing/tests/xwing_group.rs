//! P0 spike: MLS group lifecycle on X-Wing (ML-KEM-768 + X25519) ciphersuites.
//! Create -> add via Welcome -> agree on exporter secret -> remove -> epoch advanced, exporter rotated.
//! Ed25519 suites run on the libcrux provider; the X-Wing + ML-DSA-44 suite runs on the RustCrypto
//! provider, the only released provider that implements ML-DSA (libcrux gains it on OpenMLS main).

use openmls::prelude::{tls_codec::*, *};
use openmls_basic_credential::SignatureKeyPair;
use openmls_libcrux_crypto::Provider as Libcrux;
use openmls_rust_crypto::OpenMlsRustCrypto as RustCrypto;
use openmls_traits::{signatures::Signer, OpenMlsProvider};

const LABEL: &str = "arkiv-git/content-key/v1";
const CTX: &[u8] = b"repo:example";

struct Member<P> {
    provider: P,
    signer: SignatureKeyPair,
    credential: CredentialWithKey,
}

fn member<P: OpenMlsProvider + Default>(name: &str, cs: Ciphersuite) -> Member<P> {
    let provider = P::default();
    let signer = SignatureKeyPair::new(cs.signature_algorithm()).unwrap();
    signer.store(provider.storage()).unwrap();
    let credential = CredentialWithKey {
        credential: BasicCredential::new(name.into()).into(),
        signature_key: signer.to_public_vec().into(),
    };
    Member { provider, signer, credential }
}

fn key_package<P: OpenMlsProvider>(m: &Member<P>, cs: Ciphersuite) -> KeyPackage {
    KeyPackage::builder()
        .build(cs, &m.provider, &m.signer, m.credential.clone())
        .unwrap()
        .key_package()
        .clone()
}

/// Round-trip through TLS wire bytes, as a real delivery service would.
fn wire(m: &MlsMessageOut) -> (MlsMessageBodyIn, usize) {
    let bytes = m.tls_serialize_detached().unwrap();
    (MlsMessageIn::tls_deserialize_exact(&bytes).unwrap().extract(), bytes.len())
}

fn export<P: OpenMlsProvider>(g: &MlsGroup, m: &Member<P>) -> Vec<u8> {
    g.export_secret(m.provider.crypto(), LABEL, CTX, 32).unwrap()
}

fn lifecycle<P: OpenMlsProvider + Default>(cs: Ciphersuite) {
    assert!(P::default().crypto().supports(cs).is_ok(), "{cs:?} unsupported by provider");

    let alice: Member<P> = member("alice", cs);
    let (bob, carol): (Member<P>, Member<P>) = (member("bob", cs), member("carol", cs));
    let create_cfg = MlsGroupCreateConfig::builder()
        .ciphersuite(cs)
        .use_ratchet_tree_extension(true)
        .build();
    let mut a = MlsGroup::new(&alice.provider, &alice.signer, &create_cfg, alice.credential.clone()).unwrap();

    // Add bob and carol in one commit, one Welcome.
    let kps = [key_package(&bob, cs), key_package(&carol, cs)];
    // X-Wing public key is 1216 bytes (IANA HPKE KEM 0x647A); X25519 alone would be 32.
    assert_eq!(cs.hpke_kem_algorithm(), HpkeKemType::XWingKemDraft6);
    assert_eq!(kps[0].hpke_init_key().as_slice().len(), 1216, "init key is not X-Wing");
    let kp_len = kps[0].tls_serialize_detached().unwrap().len();
    let (_commit, welcome, _gi) = a.add_members(&alice.provider, &alice.signer, &kps).unwrap();
    a.merge_pending_commit(&alice.provider).unwrap();

    let (body, welcome_len) = wire(&welcome);
    let MlsMessageBodyIn::Welcome(w) = body else { panic!("expected Welcome") };
    let join = |m: &Member<P>| {
        let w = w.clone();
        StagedWelcome::new_from_welcome(&m.provider, create_cfg.join_config(), w, None)
            .unwrap()
            .into_group(&m.provider)
            .unwrap()
    };
    let mut b = join(&bob);
    let c = join(&carol);

    let epoch_before = a.epoch();
    let secret_before = export(&a, &alice);
    assert_eq!(b.epoch(), epoch_before);
    assert_eq!(export(&b, &bob), secret_before, "bob exporter mismatch");
    assert_eq!(export(&c, &carol), secret_before, "carol exporter mismatch");

    // Remove carol: the commit carries an update path, so a fresh X-Wing HPKE encryption happens here too.
    let (commit, _w, _gi) = a.remove_members(&alice.provider, &alice.signer, &[c.own_leaf_index()]).unwrap();
    a.merge_pending_commit(&alice.provider).unwrap();
    let (body, commit_len) = wire(&commit);
    let msg: ProtocolMessage = match body {
        MlsMessageBodyIn::PublicMessage(m) => m.into(),
        MlsMessageBodyIn::PrivateMessage(m) => m.into(),
        _ => panic!("expected commit message"),
    };
    match b.process_message(&bob.provider, msg).unwrap().into_content() {
        ProcessedMessageContent::StagedCommitMessage(sc) => b.merge_staged_commit(&bob.provider, *sc).unwrap(),
        other => panic!("expected commit, got {other:?}"),
    }

    let secret_after = export(&a, &alice);
    assert_eq!(a.epoch().as_u64(), epoch_before.as_u64() + 1, "epoch did not advance");
    assert_eq!(b.epoch(), a.epoch());
    assert_eq!(a.members().count(), 2);
    assert_eq!(export(&b, &bob), secret_after, "post-remove exporter mismatch");
    assert_ne!(secret_after, secret_before, "exporter did not rotate");
    println!(
        "{cs:?} ok: epoch {} -> {}, exporter rotated, key package {kp_len} B, welcome {welcome_len} B (2 joiners), remove commit {commit_len} B",
        epoch_before.as_u64(),
        a.epoch().as_u64()
    );
}

/// Legacy OpenMLS-only suite, the only X-Wing suite libcrux 0.4.0 advertises.
#[test]
fn xwing_legacy_0x004d() {
    lifecycle::<Libcrux>(Ciphersuite::MLS_256_XWING_CHACHA20POLY1305_SHA256_Ed25519);
}

/// draft-ietf-mls-pq-ciphersuites X-Wing suite (OpenMLS provisional 0x004E; draft-07 says 0x0011).
#[test]
fn xwing_draft_ietf_aes256_sha384() {
    lifecycle::<Libcrux>(Ciphersuite::MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519);
}

/// X-Wing + ML-DSA-44, the only X-Wing suite with a PQ signature (OpenMLS provisional 0x0052; draft-07 says 0x0020).
/// PQ authenticity as well as PQ confidentiality: credentials, KeyPackages, commits and the Welcome are ML-DSA signed.
#[test]
fn xwing_mldsa44_0x0052() {
    let cs = Ciphersuite::MLS_128_MLKEM768X25519_CHACHA20POLY1305_SHA384_MLDSA44;
    assert_eq!(cs.signature_algorithm(), SignatureScheme::MLDSA44);
    assert_eq!(u16::from(cs), 0x0052);

    // FIPS 204 Table 2 sizes for ML-DSA-44: public key 1312 B, signature 2420 B.
    let signer = SignatureKeyPair::new(SignatureScheme::MLDSA44).unwrap();
    assert_eq!(signer.public().len(), 1312, "ML-DSA-44 public key size");
    let sig = signer.sign(b"arkiv-git/anchor/v1").unwrap();
    assert_eq!(sig.len(), 2420, "ML-DSA-44 signature size");
    // The provider verifies what basic_credential signs (two different code paths over the RustCrypto ml-dsa crate).
    RustCrypto::default()
        .crypto()
        .verify_signature(SignatureScheme::MLDSA44, b"arkiv-git/anchor/v1", signer.public(), &sig)
        .expect("ML-DSA-44 signature does not verify");

    lifecycle::<RustCrypto>(cs);
}

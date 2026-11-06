//! P0 spike: MLS group lifecycle on X-Wing (ML-KEM-768 + X25519) ciphersuites with the libcrux provider.
//! Create -> add via Welcome -> agree on exporter secret -> remove -> epoch advanced, exporter rotated.

use openmls::prelude::{tls_codec::*, *};
use openmls_basic_credential::SignatureKeyPair;
use openmls_libcrux_crypto::Provider;
use openmls_traits::OpenMlsProvider;

const LABEL: &str = "arkiv-git/content-key/v1";
const CTX: &[u8] = b"repo:example";

struct Member {
    provider: Provider,
    signer: SignatureKeyPair,
    credential: CredentialWithKey,
}

fn member(name: &str, cs: Ciphersuite) -> Member {
    let provider = Provider::default();
    let signer = SignatureKeyPair::new(cs.signature_algorithm()).unwrap();
    signer.store(provider.storage()).unwrap();
    let credential = CredentialWithKey {
        credential: BasicCredential::new(name.into()).into(),
        signature_key: signer.to_public_vec().into(),
    };
    Member { provider, signer, credential }
}

fn key_package(m: &Member, cs: Ciphersuite) -> KeyPackage {
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

fn export(g: &MlsGroup, m: &Member) -> Vec<u8> {
    g.export_secret(m.provider.crypto(), LABEL, CTX, 32).unwrap()
}

fn lifecycle(cs: Ciphersuite) {
    assert!(Provider::default().crypto().supports(cs).is_ok(), "{cs:?} unsupported by libcrux provider");

    let (alice, bob, carol) = (member("alice", cs), member("bob", cs), member("carol", cs));
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
    let (_commit, welcome, _gi) = a.add_members(&alice.provider, &alice.signer, &kps).unwrap();
    a.merge_pending_commit(&alice.provider).unwrap();

    let (body, welcome_len) = wire(&welcome);
    let MlsMessageBodyIn::Welcome(w) = body else { panic!("expected Welcome") };
    let join = |m: &Member| {
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
        "{cs:?} ok: epoch {} -> {}, exporter rotated, welcome {welcome_len} B (2 joiners), remove commit {commit_len} B",
        epoch_before.as_u64(),
        a.epoch().as_u64()
    );
}

/// Legacy OpenMLS-only suite, the only X-Wing suite libcrux 0.4.0 advertises.
#[test]
fn xwing_legacy_0x004d() {
    lifecycle(Ciphersuite::MLS_256_XWING_CHACHA20POLY1305_SHA256_Ed25519);
}

/// draft-ietf-mls-pq-ciphersuites X-Wing suite (OpenMLS provisional 0x004E; draft-07 says 0x0011).
#[test]
fn xwing_draft_ietf_aes256_sha384() {
    lifecycle(Ciphersuite::MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519);
}

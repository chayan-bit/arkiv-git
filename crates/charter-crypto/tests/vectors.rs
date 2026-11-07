//! Known-answer tests.
//!
//! - RFC 10073 vector: published by the IETF lamps-wg (see the fixture's `source`).
//! - `H`, HKDF outputs and EVM scalars: values computed independently with Python
//!   `hashlib`/`hmac` (HKDF by hand) for these inputs.
//! - EVM addresses for private keys 1 and Hardhat #0: well-known public values.
//! - Master-seed -> nid / EVM address: self-derived with this crate, checked in so any
//!   change to the derivation is caught.

use charter_crypto::{Alg, MasterSeed, PublicKey, SecretKey, Sig, evm, h, sha256};

fn hex32(s: &str) -> [u8; 32] {
    hex::decode(s).expect("hex").try_into().expect("32")
}

struct Rfc {
    m: Vec<u8>,
    ctx: Vec<u8>,
    pk: Vec<u8>,
    sk: Vec<u8>,
    s: Vec<u8>,
    s_ctx: Vec<u8>,
}

fn rfc() -> Rfc {
    let raw = include_str!("vectors/rfc10073_mldsa65_ed25519.json");
    let v: serde_json::Value = serde_json::from_str(raw).expect("json");
    let f = |k: &str| hex::decode(v[k].as_str().expect(k)).expect("hex");
    Rfc {
        m: f("m"),
        ctx: f("ctx"),
        pk: f("pk"),
        sk: f("sk"),
        s: f("s"),
        s_ctx: f("s_with_context"),
    }
}

#[test]
fn rfc10073_secret_key_derives_published_public_key() {
    let v = rfc();
    let sk = SecretKey::from_bytes(&v.sk).expect("sk");
    assert_eq!(sk.public_key().as_bytes(), v.pk.as_slice());
}

#[test]
fn rfc10073_signature_with_empty_context_verifies() {
    let v = rfc();
    let pk = PublicKey::from_bytes(Alg::MlDsa65Ed25519, &v.pk).expect("pk");
    let s = Sig::from_bytes(Alg::MlDsa65Ed25519, &v.s).expect("sig");
    assert!(pk.verify(&v.m, b"", &s).is_ok());
    assert!(pk.verify(&v.m, &v.ctx, &s).is_err());
}

#[test]
fn rfc10073_signature_with_context_verifies() {
    let v = rfc();
    let pk = PublicKey::from_bytes(Alg::MlDsa65Ed25519, &v.pk).expect("pk");
    let s = Sig::from_bytes(Alg::MlDsa65Ed25519, &v.s_ctx).expect("sig");
    assert!(pk.verify(&v.m, &v.ctx, &s).is_ok());
    assert!(pk.verify(&v.m, b"", &s).is_err());
}

#[test]
fn rfc10073_key_signs_and_our_output_verifies() {
    let v = rfc();
    let sk = SecretKey::from_bytes(&v.sk).expect("sk");
    let sig = sk.sign(&v.m, &v.ctx).expect("sign");
    let pk = PublicKey::from_bytes(Alg::MlDsa65Ed25519, &v.pk).expect("pk");
    assert!(pk.verify(&v.m, &v.ctx, &sig).is_ok());
}

#[test]
fn tagged_hash_matches_python() {
    assert_eq!(
        h("nid", b"abc"),
        hex32("f739d2dee6b10968c6ada96e7f64f39d8c128f6b3f0c15decdc10446a069a761")
    );
    assert_eq!(
        h("id", b""),
        hex32("0e413a2a184cbf7fef1bc37dfe9e22ef3fc334e048ea570612d2fb1ff452bce5")
    );
    assert_eq!(sha256(b"")[..4], [0xe3, 0xb0, 0xc4, 0x42]);
}

#[test]
fn signing_key_component_seeds_match_python_hkdf() {
    let seed = MasterSeed::from_bytes(core::array::from_fn(|i| i as u8));
    let sk = seed.signing_key();
    let bytes = sk.to_bytes();
    assert_eq!(
        bytes[..32],
        hex32("9ce9f63465a8ec3f7582cc6b40dbe6be39ec620d994f2872e3b91cdca9ac1628")
    );
    assert_eq!(
        bytes[32..],
        hex32("d79dea8628ec31069abbd8ce87768534f044e9505d37cbb2c5ac854cbc8d0372")
    );
}

#[test]
fn master_seed_gives_fixed_nid() {
    let seed = MasterSeed::from_bytes(core::array::from_fn(|i| i as u8));
    let nid = seed.signing_key().public_key().nid();
    assert_eq!(
        hex::encode(nid),
        "ff42d6f44adcfde2616bb123c506ade38e3e96d44c5003b87b4e834be9b4051d",
        "self-derived vector; update only with a deliberate format change"
    );
}

#[test]
fn evm_address_of_well_known_keys() {
    let mut one = [0u8; 32];
    one[31] = 1;
    assert_eq!(
        hex::encode(evm::address_of(&one).expect("valid")),
        "7e5f4552091a69125d5dfcb7b8c2659029395bdf"
    );
    let hardhat0 = hex32("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80");
    assert_eq!(
        hex::encode(evm::address_of(&hardhat0).expect("valid")),
        "f39fd6e51aad88f6f4ce6ab8827279cfffb92266"
    );
    assert!(evm::address_of(&[0u8; 32]).is_none());
}

#[test]
fn evm_scalars_match_python_hkdf() {
    let seed = MasterSeed::from_bytes(core::array::from_fn(|i| i as u8));
    let rid = [0xaa; 32];
    assert_eq!(
        *evm::wallet(&seed, 0).secret(),
        hex32("39d8eb3eb7659783949ae25c7606d3167e6fd94cad6915c52ea5eb0ce7366071")
    );
    assert_eq!(
        *evm::control(&seed, &rid, 0).secret(),
        hex32("f0e76a932fb61853940bb20a0e79aff0302ecc758f1e17a59ba4ac760d6bad9d")
    );
    assert_eq!(
        *evm::bulk(&seed, &rid, 1).secret(),
        hex32("2a2963386799114a2eef6cd216acb60e4ceb2f6de50437996463a283dc5478c7")
    );
}

#[test]
fn master_seed_gives_fixed_evm_addresses() {
    let seed = MasterSeed::from_bytes(core::array::from_fn(|i| i as u8));
    let rid = [0xaa; 32];
    let got = [
        hex::encode(evm::wallet(&seed, 0).address()),
        hex::encode(evm::control(&seed, &rid, 0).address()),
        hex::encode(evm::bulk(&seed, &rid, 1).address()),
    ];
    assert_eq!(
        got,
        [
            "cdf2c3453cd16d52461bafb24b6dfcf34dda9caf",
            "ae684cd3d5847a5c13eb8486ab5e6a2e47264c9e",
            "63cd8fe2cc297f2ab5cb46dac455e6c7d16f1d48",
        ],
        "self-derived vector"
    );
}

#[test]
fn evm_roles_and_indices_give_distinct_addresses() {
    let seed = MasterSeed::from_bytes([3u8; 32]);
    let rid = [1u8; 32];
    let addrs = [
        evm::wallet(&seed, 0).address(),
        evm::wallet(&seed, 1).address(),
        evm::control(&seed, &rid, 0).address(),
        evm::control(&seed, &rid, 1).address(),
        evm::control(&seed, &[2u8; 32], 0).address(),
        evm::bulk(&seed, &rid, 0).address(),
    ];
    for (i, a) in addrs.iter().enumerate() {
        for b in &addrs[i + 1..] {
            assert_ne!(a, b);
        }
    }
    let k = evm::control(&seed, &rid, 0);
    assert_eq!(evm::address_of(k.secret()), Some(k.address()));
}

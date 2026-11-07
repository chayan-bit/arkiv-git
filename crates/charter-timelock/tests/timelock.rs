//! Timelock tests run offline against real quicknet beacons stored as fixtures.

use charter_timelock::{
    CHAIN_HASH, SealedBlob, TimelockError, fetch_beacon, open, open_when_due, parse_until,
    round_at, seal, unlock_time,
};

const BEACON_1000: &str = include_str!("fixtures/quicknet-1000.json");
const BEACON_1001: &str = include_str!("fixtures/quicknet-1001.json");

fn signature(fixture: &str) -> Vec<u8> {
    let v: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    hex::decode(v["signature"].as_str().expect("signature")).expect("hex")
}

/// Emission time of quicknet round 1000.
const T1000: u64 = 1_692_806_364;

#[test]
fn round_matches_known_time_round_pairs() {
    // Round r is published at genesis + (r - 1) * 3 s; fetched live: round 33730737 at 1793995575.
    assert_eq!(round_at(T1000), 1000);
    assert_eq!(round_at(1_793_995_575), 33_730_737);
    assert_eq!(unlock_time(1000), T1000);
    assert_eq!(unlock_time(1), 1_692_803_367);
    // Never earlier than asked: a time between two rounds rounds up.
    assert_eq!(round_at(T1000 + 1), 1001);
    assert_eq!(round_at(T1000 - 2), 1000);
    assert_eq!(round_at(0), 1);
}

#[test]
fn seal_then_open_with_a_real_beacon() {
    let secret = b"embargoed fix: patch key \x00\x00 trailing zeros".to_vec();
    let blob = seal(&secret, T1000).expect("seal");
    assert_eq!(blob.round, 1000);
    assert_eq!(blob.chain_hash, CHAIN_HASH);
    assert_ne!(blob.ciphertext, secret);
    assert_eq!(open(&blob, &signature(BEACON_1000)).expect("open"), secret);
}

#[test]
fn blob_survives_cbor() {
    let blob = seal(b"x", T1000).expect("seal");
    let back = SealedBlob::from_bytes(&blob.to_bytes().expect("encode")).expect("decode");
    assert_eq!(back, blob);
    assert!(SealedBlob::from_bytes(b"\xff\x00garbage").is_err());
}

#[test]
fn wrong_beacons_are_rejected() {
    let blob = seal(b"secret", T1000).expect("seal");
    // Genuine beacon, wrong round.
    let other = open(&blob, &signature(BEACON_1001));
    assert!(
        matches!(other, Err(TimelockError::BadBeacon(_))),
        "{other:?}"
    );
    // Tampered signature.
    let mut bad = signature(BEACON_1000);
    bad[5] ^= 1;
    assert!(matches!(
        open(&blob, &bad),
        Err(TimelockError::BadBeacon(_))
    ));
    assert!(matches!(
        open(&blob, b"short"),
        Err(TimelockError::BadBeacon(_))
    ));
}

#[test]
fn foreign_chain_blob_is_rejected() {
    let mut blob = seal(b"secret", T1000).expect("seal");
    blob.chain_hash[0] ^= 1;
    let e = open(&blob, &signature(BEACON_1000));
    assert!(matches!(e, Err(TimelockError::WrongChain)), "{e:?}");
}

#[test]
fn open_before_the_round_is_too_early_and_never_fetches() {
    let blob = seal(b"secret", T1000).expect("seal");
    let e = open_when_due(&blob, T1000 - 1, |_| {
        panic!("must not fetch before the round")
    });
    match e {
        Err(TimelockError::TooEarly { round, unlock_at }) => {
            assert_eq!((round, unlock_at), (1000, T1000));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(e.expect_err("too early").code(), "E_TOO_EARLY");
}

#[test]
fn relay_without_the_round_yet_is_too_early() {
    let blob = seal(b"secret", T1000).expect("seal");
    let e = open_when_due(&blob, T1000, |r| {
        Err(TimelockError::TooEarly {
            round: r,
            unlock_at: unlock_time(r),
        })
    });
    assert!(matches!(e, Err(TimelockError::TooEarly { .. })));
}

#[test]
fn open_when_due_opens_with_the_fetched_beacon() {
    let blob = seal(b"secret", T1000).expect("seal");
    let got = open_when_due(&blob, T1000 + 5, |r| {
        assert_eq!(r, 1000);
        Ok(signature(BEACON_1000))
    });
    assert_eq!(got.expect("open"), b"secret");
}

#[test]
fn until_parses_rfc3339_and_relative() {
    assert_eq!(parse_until("2023-08-23T15:59:24Z", 0).expect("rfc"), T1000);
    assert_eq!(
        parse_until("2023-08-23T17:59:24+02:00", 0).expect("offset"),
        T1000
    );
    assert_eq!(parse_until("+90s", 100), Ok(190));
    assert_eq!(parse_until("+2h", 100), Ok(100 + 7200));
    assert_eq!(parse_until("+1d", 0), Ok(86_400));
    for bad in ["", "+", "+5", "+5x", "+-5m", "tomorrow", "2023-08-23"] {
        assert!(parse_until(bad, 0).is_err(), "{bad}");
    }
}

/// Live network: fetches and verifies a real beacon from the public relays.
#[test]
#[ignore = "needs network"]
fn live_fetch_verifies_against_pinned_key() {
    let sig = fetch_beacon(1000).expect("fetch");
    assert_eq!(sig, signature(BEACON_1000));
    let future = fetch_beacon(u64::MAX / 2);
    assert!(future.is_err());
}

#[test]
fn pinned_chain_hash_bytes_match_the_hex() {
    assert_eq!(hex::encode(CHAIN_HASH), charter_timelock::CHAIN_HASH_HEX);
}

//! Pinned drand quicknet parameters, round arithmetic, beacon verification and fetching.

use drand_core::DrandError;
use drand_core::HttpClient;
use drand_core::beacon::{ApiBeacon, BeaconError};
use drand_core::chain::{ChainInfo, ChainOptions, ChainVerification};
use sha2::{Digest, Sha256};

use crate::{Result, TimelockError};

/// drand quicknet: unchained, 3 s period, signatures on G1 (`bls-unchained-g1-rfc9380`).
pub const CHAIN_HASH_HEX: &str = "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971";
/// Raw chain hash, the first 32 bytes of every sealed blob's identity.
pub const CHAIN_HASH: [u8; 32] = [
    0x52, 0xdb, 0x9b, 0xa7, 0x0e, 0x0c, 0xc0, 0xf6, 0xea, 0xf7, 0x80, 0x3d, 0xd0, 0x74, 0x47, 0xa1,
    0xf5, 0x47, 0x77, 0x35, 0xfd, 0x3f, 0x66, 0x17, 0x92, 0xba, 0x94, 0x60, 0x0c, 0x84, 0xe9, 0x71,
];
pub const PUBLIC_KEY_HEX: &str = "83cf0f2896adee7eb8b5f01fcad3912212c437e0073e911fb90022d3e760183c8c4b450b6a0a6c3ac6a5776a2d1064510d1fec758c921cc22b0e17e63aaf4bcb5ed66304de9cf809bd274ca73bab4af5a6e9c76a4bc09e76eae8991ef5ece45a";
pub const GENESIS_TIME: u64 = 1_692_803_367;
pub const PERIOD: u64 = 3;
const GROUP_HASH_HEX: &str = "f477d5c89f21a17c863a7f937c6a6d15859414d2be09cd448d4279af331c5d3e";
const SCHEME_ID: &str = "bls-unchained-g1-rfc9380";

/// Public relays, tried in order. Each answer is verified against the pinned key, so none is trusted.
pub const RELAYS: [&str; 3] = [
    "https://api.drand.sh",
    "https://api2.drand.sh",
    "https://drand.cloudflare.com",
];

/// Smallest round whose beacon is published at or after `unlock_at` (unix seconds).
/// Round `r` is published at `GENESIS_TIME + (r - 1) * PERIOD`, and round 1 is the first.
pub fn round_at(unlock_at: u64) -> u64 {
    unlock_at.saturating_sub(GENESIS_TIME).div_ceil(PERIOD) + 1
}

/// Unix time at which `round` is published.
pub fn unlock_time(round: u64) -> u64 {
    GENESIS_TIME.saturating_add(round.saturating_sub(1).saturating_mul(PERIOD))
}

fn pinned_info() -> Result<ChainInfo> {
    let json = format!(
        r#"{{"public_key":"{PUBLIC_KEY_HEX}","period":{PERIOD},"genesis_time":{GENESIS_TIME},"hash":"{CHAIN_HASH_HEX}","groupHash":"{GROUP_HASH_HEX}","schemeID":"{SCHEME_ID}","metadata":{{"beaconID":"quicknet"}}}}"#
    );
    serde_json::from_str(&json)
        .map_err(|e| TimelockError::Internal(format!("pinned chain info: {e}")))
}

/// Checks that `signature` is quicknet's BLS signature for `round`, against the pinned public key.
pub fn verify_signature(round: u64, signature: &[u8]) -> Result<()> {
    let bad = |why: &str| TimelockError::BadBeacon(format!("round {round}: {why}"));
    if signature.len() != 48 {
        return Err(bad("signature is not 48 bytes"));
    }
    // Unchained beacons sign only the round; randomness is sha256(signature).
    let beacon: ApiBeacon = serde_json::from_value(serde_json::json!({
        "round": round,
        "randomness": hex::encode(Sha256::digest(signature)),
        "signature": hex::encode(signature),
    }))
    .map_err(|e| bad(&e.to_string()))?;
    match beacon.verify(pinned_info()?) {
        Ok(true) => Ok(()),
        Ok(false) => Err(bad("signature does not verify")),
        Err(e) => Err(bad(&e.to_string())),
    }
}

fn relay_get(relay: &str, round: u64) -> std::result::Result<Vec<u8>, DrandError> {
    let pinned =
        ChainVerification::new(Some(CHAIN_HASH.to_vec()), hex::decode(PUBLIC_KEY_HEX).ok());
    let url = format!("{relay}/{CHAIN_HASH_HEX}");
    let client = HttpClient::new(&url, Some(ChainOptions::new(true, true, Some(pinned))))?;
    Ok(client.get(round)?.signature())
}

fn is_not_found(e: &DrandError) -> bool {
    matches!(e, DrandError::Beacon(b) if matches!(**b, BeaconError::NotFound))
}

/// Fetches the beacon signature for `round`, trying each relay and verifying before returning.
/// A round no relay has published yet is `TooEarly`.
pub fn fetch_beacon(round: u64) -> Result<Vec<u8>> {
    let mut not_found = false;
    let mut last = String::from("no relays");
    for relay in RELAYS {
        match relay_get(relay, round) {
            Ok(sig) => match verify_signature(round, &sig) {
                Ok(()) => return Ok(sig),
                Err(e) => last = format!("{relay}: {e}"),
            },
            Err(e) => {
                not_found |= is_not_found(&e);
                last = format!("{relay}: {e}");
            }
        }
    }
    match not_found {
        true => Err(TimelockError::TooEarly {
            round,
            unlock_at: unlock_time(round),
        }),
        false => Err(TimelockError::Fetch(last)),
    }
}

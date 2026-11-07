//! Timelock encryption on drand quicknet (tlock): seal now, anyone opens at time T.
//!
//! Encryption is `tlock_age` (identity-based encryption to a future round's BLS signature,
//! wrapped in an age file, interoperable with drand's Go `tlock`).
//! No party holds a key: the round's threshold signature is the decryption key.

mod chain;
mod until;

pub use chain::{
    CHAIN_HASH, CHAIN_HASH_HEX, GENESIS_TIME, PERIOD, PUBLIC_KEY_HEX, RELAYS, fetch_beacon,
    round_at, unlock_time, verify_signature,
};
pub use until::{fmt_unix, parse_until};

use minicbor::{Decode, Encode};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimelockError {
    #[error("not before round {round} ({})", fmt_unix(*unlock_at))]
    TooEarly { round: u64, unlock_at: u64 },
    #[error("beacon rejected: {0}")]
    BadBeacon(String),
    #[error("blob is for a different drand chain than quicknet")]
    WrongChain,
    #[error("malformed sealed blob: {0}")]
    BadBlob(String),
    #[error("beacon fetch failed: {0}")]
    Fetch(String),
    #[error("{0}")]
    Usage(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl TimelockError {
    /// Stable error code printed by the CLI.
    pub fn code(&self) -> &'static str {
        match self {
            Self::TooEarly { .. } => "E_TOO_EARLY",
            Self::BadBeacon(_) => "E_BAD_BEACON",
            Self::WrongChain => "E_WRONG_CHAIN",
            Self::BadBlob(_) => "E_BAD_BLOB",
            Self::Fetch(_) => "E_BEACON_FETCH",
            Self::Usage(_) => "E_USAGE",
            Self::Internal(_) => "E_INTERNAL",
        }
    }
}

pub type Result<T, E = TimelockError> = std::result::Result<T, E>;

/// A timelocked payload: the `xyz.arkiv.timelock` op body.
/// `ciphertext` is an age file whose only stanza is `tlock <round> <chain_hash>`.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct SealedBlob {
    #[n(0)]
    pub round: u64,
    #[cbor(n(1), with = "minicbor::bytes")]
    pub chain_hash: [u8; 32],
    #[cbor(n(2), with = "minicbor::bytes")]
    pub ciphertext: Vec<u8>,
}

impl SealedBlob {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        minicbor::to_vec(self).map_err(|e| TimelockError::Internal(e.to_string()))
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        minicbor::decode(b).map_err(|e| TimelockError::BadBlob(e.to_string()))
    }

    /// Unix time at which the beacon that opens this blob is published.
    pub fn unlock_at(&self) -> u64 {
        unlock_time(self.round)
    }
}

/// Encrypts `plaintext` so it opens at the first quicknet round published at or after `unlock_at`.
pub fn seal(plaintext: &[u8], unlock_at: u64) -> Result<SealedBlob> {
    let round = round_at(unlock_at);
    let public_key =
        hex::decode(PUBLIC_KEY_HEX).map_err(|e| TimelockError::Internal(e.to_string()))?;
    let mut ciphertext = Vec::new();
    tlock_age::encrypt(&mut ciphertext, plaintext, &CHAIN_HASH, &public_key, round)
        .map_err(|e| TimelockError::Internal(format!("tlock encrypt: {e}")))?;
    Ok(SealedBlob {
        round,
        chain_hash: CHAIN_HASH,
        ciphertext,
    })
}

/// Decrypts with the round's beacon signature, which is verified against the pinned quicknet key.
pub fn open(blob: &SealedBlob, beacon_signature: &[u8]) -> Result<Vec<u8>> {
    if blob.chain_hash != CHAIN_HASH {
        return Err(TimelockError::WrongChain);
    }
    let header = tlock_age::decrypt_header(blob.ciphertext.as_slice())
        .map_err(|e| TimelockError::BadBlob(e.to_string()))?;
    if header.round() != blob.round || header.hash() != blob.chain_hash {
        return Err(TimelockError::BadBlob(
            "header disagrees with round or chain".into(),
        ));
    }
    verify_signature(blob.round, beacon_signature)?;
    let mut out = Vec::new();
    tlock_age::decrypt(
        &mut out,
        blob.ciphertext.as_slice(),
        &blob.chain_hash,
        beacon_signature,
    )
    .map_err(|e| TimelockError::BadBlob(format!("decrypt: {e}")))?;
    Ok(out)
}

/// Opens `blob` using `fetch` for the beacon, but only once `now` (unix seconds) has reached its
/// round; before that it returns `TooEarly` without touching the network.
pub fn open_when_due(
    blob: &SealedBlob,
    now: u64,
    fetch: impl FnOnce(u64) -> Result<Vec<u8>>,
) -> Result<Vec<u8>> {
    if now < blob.unlock_at() {
        return Err(TimelockError::TooEarly {
            round: blob.round,
            unlock_at: blob.unlock_at(),
        });
    }
    open(blob, &fetch(blob.round)?)
}

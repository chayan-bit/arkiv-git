//! Sealed-patch receipts (docs/p4/zk-sealed-patch.md).
//!
//! [`verify`] checks a RISC Zero succinct STARK receipt against the pinned guest [`IMAGE_ID`]
//! and an expected [`Public`] statement. It is cheap and always available.
//! Proving lives in `zk/prover`, outside the workspace, so workspace builds never link the
//! RISC Zero prover.

pub use arkiv_git_zk_core::{Private, Public};
use risc0_zkvm::{InnerReceipt, Receipt};

/// Image id of `zk/methods/guest` (the `sealed_patch` guest), pinned.
/// `zk/prover` refuses to prove when the built guest does not match it.
pub const IMAGE_ID: [u32; 8] = [
    3285259048, 1873335759, 3459761104, 162765425, 1108785410, 3000095769, 112750350, 86684526,
];

#[derive(Debug, thiserror::Error)]
pub enum ZkError {
    #[error("receipt does not decode: {0}")]
    Decode(String),
    #[error("receipt is not a succinct STARK receipt")]
    NotSuccinct,
    #[error("proof does not verify against the pinned image id: {0}")]
    Proof(String),
    #[error("journal does not decode: {0}")]
    Journal(String),
    #[error("journal does not match the expected public statement")]
    Mismatch,
}

/// Verifies `receipt_bytes` (bincode `Receipt`) proves the relation for exactly `expected`.
pub fn verify(receipt_bytes: &[u8], expected: &Public) -> Result<(), ZkError> {
    let receipt: Receipt =
        bincode::deserialize(receipt_bytes).map_err(|e| ZkError::Decode(e.to_string()))?;
    if !matches!(receipt.inner, InnerReceipt::Succinct(_)) {
        return Err(ZkError::NotSuccinct);
    }
    receipt
        .verify(IMAGE_ID)
        .map_err(|e| ZkError::Proof(e.to_string()))?;
    let got: Public = receipt
        .journal
        .decode()
        .map_err(|e| ZkError::Journal(e.to_string()))?;
    if &got != expected {
        return Err(ZkError::Mismatch);
    }
    Ok(())
}

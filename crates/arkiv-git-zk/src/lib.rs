//! Sealed-patch receipts (docs/p4/zk-sealed-patch.md).
//!
//! [`verify`] checks a RISC Zero succinct STARK receipt against the pinned guest [`IMAGE_ID`]
//! and an expected [`Public`] statement. It is cheap and always available.
//! [`prove`] (feature `prove`) runs the guest and proves it; it is CPU heavy.

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
    #[error("prover: {0}")]
    Prove(String),
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

/// Segment size 2^18 cycles keeps prover memory to a few GB.
#[cfg(feature = "prove")]
pub const SEGMENT_PO2: u32 = 18;

/// Proves the relation for `private` and `spec` with the guest `elf`; returns the bincode receipt
/// and the total cycle count.
#[cfg(feature = "prove")]
pub fn prove(elf: &[u8], private: &Private, spec: &str) -> Result<(Vec<u8>, u64), ZkError> {
    use risc0_zkvm::{ExecutorEnv, LocalProver, Prover, ProverOpts, compute_image_id};
    let err = |e: &dyn std::fmt::Display| ZkError::Prove(e.to_string());
    let id = compute_image_id(elf).map_err(|e| err(&e))?;
    if id != IMAGE_ID.into() {
        return Err(ZkError::Prove(format!(
            "guest image id {id} is not the pinned IMAGE_ID"
        )));
    }
    let env = ExecutorEnv::builder()
        .write(private)
        .and_then(|b| b.write(&spec))
        .map_err(|e| err(&e))?
        .segment_limit_po2(SEGMENT_PO2)
        .build()
        .map_err(|e| err(&e))?;
    let info = LocalProver::new("local")
        .prove_with_opts(env, elf, &ProverOpts::succinct())
        .map_err(|e| err(&e))?;
    let bytes = bincode::serialize(&info.receipt).map_err(|e| err(&e))?;
    Ok((bytes, info.stats.total_cycles))
}

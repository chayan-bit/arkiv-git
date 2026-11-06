//! Proving the sealed-patch relation (moved out of `arkiv-git-zk` so the root workspace
//! never links the RISC Zero prover).

use arkiv_git_zk::IMAGE_ID;
use arkiv_git_zk_core::Private;
use risc0_zkvm::{ExecutorEnv, LocalProver, Prover, ProverOpts, compute_image_id};

/// Segment size 2^18 cycles keeps prover memory to a few GB.
pub const SEGMENT_PO2: u32 = 18;

/// Proves the relation for `private` and `spec` with the guest `elf`; returns the bincode
/// receipt and the total cycle count.
pub fn prove(elf: &[u8], private: &Private, spec: &str) -> Result<(Vec<u8>, u64), String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let id = compute_image_id(elf).map_err(|e| err(&e))?;
    if id != IMAGE_ID.into() {
        return Err(format!("guest image id {id} is not the pinned IMAGE_ID"));
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

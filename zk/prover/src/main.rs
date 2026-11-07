//! Demo prover: seals a patch under a fresh random key and proves the relation.
//!
//! Usage: zk-prover <base> <patch> <spec> <out-dir>
//! Writes receipt.bin, public.json and sealed.bin to out-dir. The key lives only in memory.
mod prove;

use std::time::Instant;
use std::{env, fs, path::Path, process::ExitCode};

use arkiv_git_zk_core::{NONCE_LEN, Private, check, seal};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("zk-prover: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    if env::var_os("RISC0_DEV_MODE").is_some() {
        return Err("RISC0_DEV_MODE is set; refusing to make a fake proof".into());
    }
    let args: Vec<String> = env::args().skip(1).collect();
    let [base, patch, spec, out] = args.as_slice() else {
        return Err("usage: zk-prover <base> <patch> <spec> <out-dir>".into());
    };
    let read = |p: &str| fs::read_to_string(p).map_err(|e| format!("{p}: {e}"));
    let (base, patch, spec) = (read(base)?, read(patch)?, read(spec)?);

    let mut key = [0u8; 32];
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut key).map_err(|e| e.to_string())?;
    getrandom::fill(&mut nonce).map_err(|e| e.to_string())?;
    let private = Private {
        key,
        sealed: seal(&key, &nonce, patch.as_bytes()),
        base,
    };
    // Fail fast natively before spending minutes proving a false statement.
    let public = check(&private, &spec).map_err(|e| format!("relation does not hold: {e:?}"))?;

    let start = Instant::now();
    let (receipt, cycles) = prove::prove(methods::SEALED_PATCH_ELF, &private, &spec)?;
    let prove_s = start.elapsed().as_secs_f64();
    let start = Instant::now();
    charter_zk::verify(&receipt, &public).map_err(|e| e.to_string())?;
    let verify_ms = start.elapsed().as_secs_f64() * 1e3;

    let out = Path::new(out);
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&public).map_err(|e| e.to_string())?;
    let write =
        |name: &str, data: &[u8]| fs::write(out.join(name), data).map_err(|e| e.to_string());
    write("receipt.bin", &receipt)?;
    write("public.json", format!("{json}\n").as_bytes())?;
    write("sealed.bin", &private.sealed)?;
    println!(
        "cycles={cycles} prove_s={prove_s:.1} receipt_bytes={} verify_ms={verify_ms:.1}",
        receipt.len()
    );
    Ok(())
}

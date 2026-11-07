# P4: zero-knowledge proofs for sealed patches

Status: demo-scale implementation, real proofs (no dev mode).
Date: 2026-11-07.
Code: `crates/arkiv-git-zk-core` (relation), `crates/charter-zk` (verifier, prover behind feature `prove`), `zk/` (guest and demo prover), `scripts/zk-prove.sh`, `chart zk verify`.

## 1. The relation

A sealed patch is a patch `P` encrypted under a key `k` that the author does not reveal yet.
The author proves, without revealing `k` or `P`, that they know `k` and a ciphertext `C` such that:

1. `sha256(k) = H`.
2. `C = nonce || XChaCha20-Poly1305(k, nonce, P)` decrypts and authenticates under `k`, giving `P`.
3. `P`, applied to the public base file `F`, gives a file `R`.
4. `R` satisfies the public spec `S`.

The public statement (the journal, `arkiv_git_zk_core::Public`) is `H`, `sha256(C)`, `sha256(F)`, `sha256(S)` and `S` itself.
The guest computes all four hashes from the witness and commits them, so a receipt for a statement is a receipt that the hashes match.
`k`, `P` and `R` never leave the prover.
`C` and `F` are private inputs to the guest but are public in practice: the op publishes `C`, and `F` is in the repo, so anyone can check them against the hashes.

Patch format (`apply_patch`): one op per line, applied in order with a cursor over the base lines.
`= N` copies the next N base lines, `- TEXT` drops the next base line and requires it to equal TEXT, and `+ TEXT` emits TEXT.
Base lines left after the last op are copied unchanged.
Lines are split on `\n` and rejoined with `\n`, so trailing newlines survive.

Spec format (`spec_holds`): one check per line.
`+ TEXT` requires `R` to contain TEXT, `- TEXT` requires `R` not to contain TEXT, and any other non-empty line fails the spec, so a typo can never pass silently.
A richer spec (a Rhai predicate or a pure function test) slots into `spec_holds` without changing anything else.

## 2. Why RISC Zero

RISC Zero (`risc0-zkvm` 3.0.6, the latest stable) and SP1 (`sp1-sdk` 6.10) were compared for this relation.
SP1's light verifier crate, `sp1-verifier`, only verifies Groth16 and Plonk wraps, which need a trusted setup and were ruled out.
Verifying a plain SP1 STARK proof needs the SP1 prover stack on the verifying machine.
RISC Zero verifies its succinct STARK receipts with `default-features = false`, so the CLI links a verifier without the prover.
Its toolchain installs without root (`rzup` from crates.io, components into `RISC0_HOME`), and it proves on CPU, with Metal on Apple Silicon.

## 3. Trust assumptions

- STARK soundness: the FRI-based STARK and the recursion circuit that compresses segment proofs into one succinct receipt are sound at RISC Zero's stated security level.
- zkVM circuit correctness: the rv32im circuit faithfully executes the guest ELF, and the guest code (`zk/methods/guest`, which calls `arkiv_git_zk_core::check`) is the relation above.
- Pinned image id: `charter_zk::IMAGE_ID` names the exact guest ELF.
  `verify` checks the receipt against that constant, so a receipt for any other program fails.
  `zk/prover` refuses to prove when the built guest does not hash to the pinned id.
  The guest is built with the local RISC Zero toolchain, not the reproducible Docker build, so a third party that wants to rebuild the ELF and match the id must use the same toolchain (`rzup` rust 1.97.0) and sources.
- No trusted setup: receipts stay STARKs; there is no Groth16 wrap.
- No dev mode: the verifier enables `risc0-zkvm/disable-dev-mode`, and `verify` also rejects any receipt that is not a succinct STARK receipt.

The proof says nothing about whether `S` is a good spec; a weak spec makes a weak claim.
`H = sha256(k)` lets the author later reveal `k` and anyone check it is the committed key.

## 4. Measured numbers

One real proof of the demo in `zk/demo/` (a one-line fix to a Rust `add` function), on an Apple Silicon MacBook Air (10 cores, 16 GB), 2026-11-07, while other cargo builds were running (load average about 10).

| What | Value |
|---|---|
| Guest execution | one segment, 2^17 cycles padded (`SEGMENT_PO2 = 18`) |
| Prove (segment proof, lift to succinct) | 53.1 s wall, 272 s CPU |
| Peak memory | 1.49 GB max RSS, 2.0 GB peak footprint |
| Receipt (bincode succinct `Receipt`) | 223,898 bytes |
| Verify, release build (in `zk-prover`) | 18.5 ms |
| `chart zk verify`, debug build, whole process | 0.64 s |
| Image id | `IMAGE_ID` in `crates/charter-zk/src/lib.rs` |

The fixture receipt is `crates/charter-zk/tests/fixtures/demo.receipt` (sha256 `040fb261c33e9462c06067b8cc97b9fe903021aa39388504e388a22adf068504`).
Building the prover on macOS compiles RISC Zero's Metal kernels; that needs `scripts/macos-shims/xcrun` (nix xcrun cannot see the Metal toolchain, and risc0 kernels need a macOS 14 Metal language target), which `scripts/zk-prove.sh` puts on `PATH`.
The verifier-only build links about 180 crates (`cargo tree -p charter-zk -e normal`), mostly the RISC Zero verifier and its field and hash code.

## 5. How repo programs use it

A sealed patch travels as an `xyz.arkiv.sealed` `commit` op whose `commitment` is `H` and whose payload carries a proof attachment (docs/p3/programs.md, "Proofs").
The receipt bytes travel in `xyz.arkiv.blob` ops; the attachment names them by hash and carries the public statement (`H`, `sha256(C)`, `sha256(F)`, `sha256(S)`, `S`).
Before every fold the client resolves the receipt, runs `charter_zk::journal` and compares the journal with the claimed statement exactly.
The program sees the result as `op.proofs[i]` with `ok`, the verified `image_id` and the statement, never the receipt.
The program never verifies proofs itself; it decides policy.
`programs/sealed-patch.rhai` lets an issue require, allow or ignore proofs, and counts a required-proof commit only when `ok` holds, the image id is the pinned guest, `spec_hash` is the issue's and `key_hash` is the commitment.
Results are cached by receipt hash, so re-folds do not verify again.
Checking `sha256(C)` against a published blob and `sha256(F)` against the file at a commit is left to the program or a later client step; the receipt already binds both hashes.
When the author later reveals `k`, clients check `sha256(k) = H`, decrypt `C` and apply `P`, and get the file the proof promised.

## 6. Running it

Verify (cheap, any machine): `chart zk verify --receipt demo.receipt --public demo.public.json`.
Prove (heavy): `mac-heavy scripts/zk-prove.sh`, which installs the toolchain into `.zk-toolchain/` on first run, builds the guest and prover, and writes `receipt.bin`, `public.json` and `sealed.bin` to `zk/out/`.
The prover generates the demo key in memory and never writes it anywhere.
`ZK_PROVE_TIMEOUT_S` bounds the proving run (default 1800 s).
The root workspace links only the verifier, so `cargo test --workspace --all-features` needs no shim; proving code lives in `zk/prover` (outside the workspace) and only `scripts/zk-prove.sh` puts `scripts/macos-shims` on `PATH`.
After any change to the guest or `arkiv-git-zk-core`, the image id changes: rebuild, copy the new `SEALED_PATCH_ID` into `IMAGE_ID`, re-prove and replace the fixtures.

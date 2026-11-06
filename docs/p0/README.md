# P0 spike: summary and decision points

Date: 2026-11-06.
Detail lives in the three docs next to this one: `arkiv-limits.md`, `openmls-xwing.md`, `heartwood-reuse.md`.

## Gate status

P0 gate is "numbers in hand".
Read-only numbers are in hand (measured on the Rabdi testnet or read from the node source, each row labelled).
Funded numbers (real receipts, largest create that mines, throughput burst, expiry timing) are not, because the throwaway testnet address is unfunded and the faucet needs a wallet sign-in plus a captcha.

## Headline numbers

| Item | Value | Label |
| --- | --- | --- |
| Testnet | Rabdi, chain id 7794095, 2 s blocks | MEASURED |
| Largest payload end to end | 130,336 B (RPC edge caps the body at 256 KiB) | MEASURED |
| Chosen chunk size | 64 KiB (a 120 KiB chunk at max lifetime exceeds the 16.78M per-tx gas cap) | DERIVED |
| Attributes per entity | 32; `str` values at most 128 B; `bytes` never indexed | SOURCE-VERIFIED, MEASURED |
| Max lifetime | 7,884,000 blocks (182.5 days), extendable without limit; no permanent entities | SOURCE-VERIFIED |
| Who extends | Owner only, unless created with the immutable `permissionlessExtension` flag | DOCS-CLAIMED, simulated |
| Storage price (testnet weights) | about 0.0196 tGLM per MiB-month, plus 0.0428 tGLM per MiB written once | MEASURED |
| 100 MB repo for one year | about 28 GLM (about $3 at spot); mainnet pricing unknown | DERIVED |
| Throughput | about 352 KiB/s theoretical; per-account limit unknown until funded run | DERIVED |
| Queries | `=`, ranges, `STARTSWITH`, `AND`/`OR`, `ORDER BY`, `$owner`; no `NOT`, no `!=`, no `atBlock`, limit 100 | MEASURED |
| Contracts | Contract creation disabled on Arkiv | MEASURED, SOURCE-VERIFIED |
| Sequencer | Single sequencer, no forced-inclusion path | SOURCE-VERIFIED by absence |
| OpenMLS X-Wing | Works: `MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519` (OpenMLS 0x004E, draft-07 0x0011), libcrux provider; spike test passes | MEASURED |
| heartwood | Depend on pinned `radicle`, `radicle-cob`, `radicle-crypto`; write the Arkiv sync layer and `git-remote-arkiv` ourselves | Recommendation |

## Recommendations taken (reversible, flag if you disagree)

- Arkiv access from Rust: the published `arkiv-sdk` crate (0.4.1) targets the retired GolemDB ABI and does not work on Rabdi.
  P1 uses `arkiv-bindings` from the node repo (MIT) as a git dependency pinned by commit, plus alloy.
- Every public-repo entity is created with `permissionlessExtension`, because the plan says lifetimes are extended by the owner or sponsors, and the flag cannot be added later.
- Version history is modelled as append-only entities (no `atBlock` queries exist).
- Anchors: a per-peer signed chain `{seq, prev = SHA-256(previous anchor), sigrefs_oid, ...}`, first-in-Arkiv-order wins, plus a local high-water mark; the client fails closed when the head anchor is missing.
- OpenMLS: pin a `main` commit (the 0.9.2 release carries RUSTSEC-2026-0331 and has a pending X-Wing key-derivation change), run `cargo audit` in CI, create no persistent groups until a release with the new derivation lands, and plan for suite migration via MLS ReInit.
- Censorship: accept the single sequencer for P1, consistent with the locked trust model (Arkiv trusted for ordering and availability).

## Open decisions (yours, not made here)

1. Post-quantum signatures.
   X-Wing covers confidentiality only; MLS credentials and Radicle signatures are Ed25519, forgeable by a future quantum attacker.
   Options: keep Ed25519 now and migrate later, or adopt ML-DSA (an X-Wing + ML-DSA-44 MLS suite exists; heartwood identity is Ed25519-only, which would push toward forking its crypto layer).
   This decides P1's identity-document and signed-refs signature scheme.
2. History access for new members (P2).
   OpenMLS discards exporter secrets per epoch, so a member joining at epoch e cannot read earlier pushes unless we keep a key lockbox that re-wraps old epoch keys under the current one.
   The lockbox matches GitHub-style roles but gives up forward secrecy for old content against current members.
3. Rotation timer values (30 days or 500 pushes) remain open.
4. Owner-key deletion: the EVM key that owns an anchor entity can delete it (`readonly` does not stop deletes).
   The high-water mark and delegate quorum mitigate it; how that key is held (derived from the device key or separate) needs a P1 call.
5. Repo contracts (P4) cannot run on Arkiv; where they run is out of scope now but no longer "decide in P0", the answer is "not on Arkiv".

## Next steps

- Fund `0x201b7FB60f8f4B221afCEcEcAeeB6d09Af42f15A` (throwaway, testnet only) with about 0.05 tGLM from the Rabdi faucet, then run `node src/measure.mjs funded` in `spike/arkiv-measure` to close the gate.
- After decision 1, start P1: entity schema, anchor chain, `git-remote-arkiv` fetch/push.

# P0 spike: summary and decision points

Date: 2026-11-06.
Detail lives in the docs next to this one: `arkiv-limits.md`, `openmls-xwing.md`, `heartwood-reuse.md`, `pq-signatures.md`.
The P1 design that builds on them is `../p1/design.md`.

## Gate status

P0 gate is "numbers in hand".
Read-only numbers are in hand (measured on the Rabdi testnet or read from the node source, each row labelled).
Funded numbers are in hand too (real receipts, largest create that mines, sponsor extension, patch semantics, expiry timing, throughput bursts), measured on 2026-11-06 for 0.0674 tGLM; the gate is closed.
Open items from the funded run: no per-account pending limit was hit (200 accepted at once), the cause of the 16-per-block ceiling is unknown, and anonymous reads are metered, so P1 needs an Arkiv access key.

## Headline numbers

| Item | Value | Label |
| --- | --- | --- |
| Testnet | Rabdi, chain id 7794095, 2 s blocks | MEASURED |
| Largest payload end to end | 130,336 B accepted by the edge (caps the body at 256 KiB); 130,272 B mined (5.30M gas) | MEASURED |
| Chosen chunk size | 64 KiB (a 120 KiB chunk at max lifetime exceeds the 16.78M per-tx gas cap); a 64 KiB create mined at 2,690,005 gas, 0.6 percent under the estimate | DERIVED, receipt-confirmed |
| Attributes per entity | 32; `str` values at most 128 B; `bytes` never indexed | SOURCE-VERIFIED, MEASURED |
| Max lifetime | 7,884,000 blocks (182.5 days), extendable without limit; no permanent entities | SOURCE-VERIFIED |
| Who extends | Owner only, unless created with the immutable `permissionlessExtension` flag; a second key extended a flagged entity for 46,469 gas, owner unchanged | MEASURED |
| Storage price (testnet weights) | about 0.0196 tGLM per MiB-month, plus 0.0428 tGLM per MiB written once | MEASURED |
| 100 MB repo for one year | about 28 GLM (about $3 at spot); mainnet pricing unknown; unchanged by the receipts | DERIVED |
| Throughput | one account: 200 pending txs accepted at once, at most 16 per block; 393,216 B in one block with 24 KiB creates; 100 MiB at 64 KiB chunks about 12 minutes at the observed average; 352 KiB/s theoretical | MEASURED, DERIVED |
| Queries | `=`, ranges, `STARTSWITH`, `AND`/`OR`, `ORDER BY`, `$owner`; no `NOT`, no `!=`, no `atBlock`, limit 100 | MEASURED |
| Contracts | Contract creation disabled on Arkiv | MEASURED, SOURCE-VERIFIED |
| Sequencer | Single sequencer, no forced-inclusion path | SOURCE-VERIFIED by absence |
| OpenMLS X-Wing | Works: `MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519` (OpenMLS 0x004E, draft-07 0x0011), libcrux provider; spike test passes | MEASURED |
| ML-DSA | RustCrypto `ml-dsa =0.1.2`, ML-DSA-65 (pk 1,952 B, sig 3,309 B); MLS suite X-Wing + ML-DSA-44 (OpenMLS 0x0052) spike test passes | MEASURED, Recommendation |
| heartwood | Superseded by ML-DSA: its keys are Ed25519 throughout (about 900 uses in 84 files), so port its rules (sigrefs, identity, quorum, COB format) instead of depending on its crates | Recommendation |

## Recommendations taken (reversible, flag if you disagree)

- Arkiv access from Rust: the published `arkiv-sdk` crate (0.4.1) targets the retired GolemDB ABI and does not work on Rabdi.
  P1 uses `arkiv-bindings` from the node repo (MIT) as a git dependency pinned by commit, plus alloy.
- Every public-repo entity is created with `permissionlessExtension`, because the plan says lifetimes are extended by the owner or sponsors, and the flag cannot be added later.
- Version history is modelled as append-only entities (no `atBlock` queries exist).
- Anchors: a per-peer signed chain `{seq, prev = SHA-256(previous anchor), sigrefs_oid, ...}`, first-in-Arkiv-order wins, plus a local high-water mark; the client fails closed when the head anchor is missing.
- OpenMLS: pin a `main` commit (the 0.9.2 release carries RUSTSEC-2026-0331 and has a pending X-Wing key-derivation change), run `cargo audit` in CI, create no persistent groups until a release with the new derivation lands, and plan for suite migration via MLS ReInit.
- Censorship: accept the single sequencer for P1, consistent with the locked trust model (Arkiv trusted for ordering and availability).

## Decided by the user

- 2026-11-06: ML-DSA signatures from day one (identities, signed refs, anchors, COBs, MLS credentials).

## Open decisions (yours, not made here)

1. Pure ML-DSA-65 or hybrid ML-DSA-65 + Ed25519 composite for our own signatures (`pq-signatures.md`).
   Recommendation: hybrid, so a forger must break both, which covers bugs in young ML-DSA code as well as quantum attacks.
   Either half can be held in hardware (Secure Enclave on Apple 26+ holds ML-DSA-65; a YubiKey holds Ed25519).
   Cost: about 3.4 KB per signature instead of 3.3 KB, and the composite draft is not yet an RFC.
2. MLS signing at ML-DSA-44 (category 2), one level below X-Wing's category 3.
   It is the only standard suite that keeps X-Wing; the only ML-DSA-65 suite drops X-Wing, and a custom suite would be non-standard.
   Recommendation: accept ML-DSA-44 inside MLS; MLS leaf keys become software keys bound to the hardware device key.
3. Testnet funding for the P1 gate: about 4.4 tGLM per full 100 MB run, about 15 tGLM with debugging; the faucet has a 24-hour cooldown.
   Recommendation: run the full gate on a local `arkiv-reth --dev` node (no funds) and a 10 MiB run on Rabdi, and ask Arkiv for a testnet grant for the full Rabdi run.
4. EVM owner keys derived from the same seed as the ML-DSA identity key (P1 design Q3).
   Recommendation: yes; a quantum break of the EVM key then makes clients fail closed instead of accepting a rollback.
5. History access for new members (P2): a key lockbox re-wraps old epoch keys under the current one, which gives up forward secrecy for old content against current members.
6. Rotation timer values (30 days or 500 pushes) remain open.
7. Repo contracts (P4) cannot run on Arkiv; where they run is out of scope now.

The P1 design lists further recommendations it resolved itself (canonical CBOR, control and bulk addresses per repo, self-contained packs, lifetime defaults); see `../p1/design.md` section on open questions.

## Next steps

- Get an Arkiv access key from hub.arkiv.network (wallet sign-in plus captcha, a human step) before P1 integration tests, since anonymous reads run out after about 1,000 queries.
- After decisions 1 to 4, start P1 implementation with TDD against the in-memory Arkiv fake, following `../p1/design.md`.

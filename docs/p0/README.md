# P0 spike: summary and decision points

Date: 2026-11-06.
Detail lives in the three docs next to this one: `arkiv-limits.md`, `openmls-xwing.md`, `heartwood-reuse.md`.

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

1. Post-quantum signatures: DECIDED by the user on 2026-11-06, ML-DSA from day one.
   X-Wing covers confidentiality only; MLS credentials and Radicle signatures were Ed25519, forgeable by a future quantum attacker.
   Consequence: P1 identity documents, signed refs, anchors and COB signatures use ML-DSA, and MLS uses an X-Wing + ML-DSA suite.
   heartwood identity is Ed25519-only, so the reuse recommendation is re-checked in `pq-signatures.md`.
2. History access for new members (P2).
   OpenMLS discards exporter secrets per epoch, so a member joining at epoch e cannot read earlier pushes unless we keep a key lockbox that re-wraps old epoch keys under the current one.
   The lockbox matches GitHub-style roles but gives up forward secrecy for old content against current members.
3. Rotation timer values (30 days or 500 pushes) remain open.
4. Owner-key deletion: the EVM key that owns an anchor entity can delete it (`readonly` does not stop deletes).
   The high-water mark and delegate quorum mitigate it; how that key is held (derived from the device key or separate) needs a P1 call.
5. Repo contracts (P4) cannot run on Arkiv; where they run is out of scope now but no longer "decide in P0", the answer is "not on Arkiv".

## Next steps

- Get an Arkiv access key from hub.arkiv.network (wallet sign-in plus captcha, a human step) before P1 integration tests, since anonymous reads run out after about 1,000 queries.
- After decision 1, start P1: entity schema, anchor chain, `git-remote-arkiv` fetch/push.

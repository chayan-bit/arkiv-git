# P0 gate: OpenMLS with X-Wing (ML-KEM-768 + X25519)

Commands below were recorded before the rename to Charter; `arkiv-git` is now `chart`.

Date: 2026-11-06.

## Verdict

GO, with conditions: OpenMLS supports X-Wing today behind the `draft-ietf-mls-pq-ciphersuites` feature, and the spike passes on both the 0.9.2 release and `main`, but no released version is wire-stable or advisory-free yet, so we build against a pinned `main` rev (or the next release) and target the draft-ietf X-Wing suite, never the legacy 0x004D one.

## Recommended suite

`MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519`.
OpenMLS code point 0x004E (provisional), which draft-ietf-mls-pq-ciphersuites-07 now lists as 0x0011.
KEM X-Wing (HPKE KEM id 0x647A), HKDF-SHA384, AES-256-GCM inside MLS, SHA-384 transcript hash, Ed25519 signatures.
Provider: `openmls_libcrux_crypto` (ML-KEM from the formally verified `libcrux-ml-kem`).
The legacy `MLS_256_XWING_CHACHA20POLY1305_SHA256_Ed25519` (0x004D) also works but is OpenMLS-only, not in any IETF draft, and is already being deprecated upstream (see evidence row 6).
The AEAD inside MLS does not matter for our content, which is sealed with XChaCha20-Poly1305 outside MLS.

## Evidence

Release sources are the crates.io tarballs of `openmls 0.9.2` (2026-10-22), `openmls_traits 0.6.0`, `openmls_libcrux_crypto 0.4.0` and `openmls_rust_crypto 0.6.0` (all 2026-08-25).
`main` refers to openmls/openmls commit `28369d5b48608bef4ddf8371d91503b0f3852bb3` (2026-10-28).

| # | Question | Finding | Source |
|---|----------|---------|--------|
| 1 | X-Wing suite defined in release? | Yes, two kinds: legacy `MLS_256_XWING_CHACHA20POLY1305_SHA256_Ed25519 = 0x004D`, and draft-ietf suites `MLS_128_MLKEM768X25519_AES128GCM_SHA256_Ed25519 = 0x004F`, `MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519 = 0x004E`, `MLS_128_MLKEM768X25519_CHACHA20POLY1305_SHA384_MLDSA44 = 0x0052`. All gated by `#[cfg(feature = "draft-ietf-mls-pq-ciphersuites")]`. | `openmls_traits-0.6.0/src/types.rs:451`, `:463`, `:471`, `:511` |
| 2 | Same on `main`? | Same names and code points; the internal `HpkeKemType::XWingKemDraft6` value changed from 0x004D to the IANA id 0x647A. | `traits/src/types.rs:235`, `:476`, `:488`, `:496`, `:557` at `28369d5` |
| 3 | Which drafts? | KEM is X-Wing draft-06 (`XWingKemDraft6`, `libcrux_kem::Algorithm::XWingKemDraft06`); HPKE suite id uses IANA KEM 0x647A (`hpke_rs_crypto::KemAlgorithm::XWingDraft06 = 0x647a`). Suites follow draft-ietf-mls-pq-ciphersuites (comments cite TBD1/TBD2/TBD9). Latest drafts: draft-connolly-cfrg-xwing-kem-11 (2026-09-23), draft-ietf-mls-pq-ciphersuites-07 (2026-11-01). | `openmls_traits-0.6.0/src/types.rs:210`; `hpke-rs-crypto-0.7.0/src/types.rs:41,46`; `libcrux-kem-0.0.9/src/kem.rs:14`; datatracker API |
| 4 | Code point final? | No. IANA MLS registry has only 0x0001 to 0x0007 assigned. Draft-07 requests 0x0010 (AES128/SHA256), 0x0011 (AES256/SHA384), 0x0020 (ChaCha/SHA384/ML-DSA-44) for X-Wing suites, which differ from OpenMLS's 0x004F/0x004E/0x0052. The comment on 0x004D even says "draft-01" while the KEM is draft-06. | https://www.iana.org/assignments/mls/mls.xml; draft-ietf-mls-pq-ciphersuites-07 Section 3.1 Tables 1-2; `types.rs:449` |
| 5 | Provider and flag | `openmls_libcrux_crypto 0.4.0` advertises only 0x004D among PQ suites in `supported_ciphersuites()`, but its `supports()` only checks AEAD, signature and hash, so 0x004E/0x004F also work (spike proves 0x004E). `openmls_rust_crypto 0.6.0` supports 0x004E, 0x004F, 0x0052 and the ML-DSA suites, but not 0x004D. Needs `openmls/draft-ietf-mls-pq-ciphersuites` (plus `libcrux-provider` or a direct dep). On `main` libcrux advertises 0x004E/0x004F/0x0052 too (#2274, #2170). | `openmls_libcrux_crypto-0.4.0/src/crypto.rs:64-97`; `openmls_rust_crypto-0.6.0/src/provider.rs:92-135`; `openmls-0.9.2/Cargo.toml` `[features]`; `libcrux_crypto/CHANGELOG.md` Unreleased at `28369d5` |
| 6 | Wire stability across releases | Breaking: `main` moves to hpke-rs 0.8, whose `DeriveKeyPair` for ML-KEM and X-Wing follows draft-ietf-hpke-pq (`SHAKE256.LabeledDerive`) instead of the unlabeled SHAKE256 used by every release. Mixed 0.9.x / next-release groups fail on any commit with an update path. A `legacy-pq-key-derivation` flag exists only for 0x004D on libcrux; RustCrypto has no opt-out (open issue #2282). Upstream advice is to migrate 0x004D groups via ReInit to a draft-ietf suite. | `libcrux_crypto/CHANGELOG.md` Unreleased "Changed" at `28369d5`; https://github.com/openmls/openmls/issues/2282 |
| 7 | libcrux ML-KEM verified? | Partially. README: portable and AVX2 field arithmetic, NTT, serialization and generic high-level code "is formally verified using hax and F*". The status table shows gaps (e.g. `ind_cpa` 0/21, `sampling` 0/5, `matrix` 0/5 correct). The X-Wing combiner crate `libcrux-kem` is marked "pre-verification"; X25519 (`libcrux-curve25519`) is HACL*-derived verified code. | `libcrux-ml-kem-0.0.10/README.md:166-172`; `libcrux-ml-kem/proofs/verification_status.md` at cryspen/libcrux `1f9c898`; `libcrux-kem-0.0.9/README.md:3`; `libcrux-curve25519-0.0.8/README.md:7-14` |
| 8 | AEAD / hash / signature of recommended suite | AES-256-GCM, SHA-384 (HKDF-SHA384), Ed25519. Signature is classical in all Ed25519 suites; draft-07 Section 4 says these give PQ confidentiality against passive (harvest-now-decrypt-later) attackers only, not PQ authenticity. | `traits/src/types.rs:705-715` at `28369d5`; draft-ietf-mls-pq-ciphersuites-07 Section 4 |
| 9 | ML-DSA / hybrid signatures | Pure ML-DSA-44/65/87 suites exist (e.g. X-Wing + ChaCha20 + SHA384 + ML-DSA-44 = 0x0052, draft 0x0020). RustCrypto provider in 0.6.0, libcrux provider on `main` (#2170). No hybrid (Ed25519 + ML-DSA composite) signature support anywhere in OpenMLS. | `types.rs:511`; `openmls_rust_crypto-0.6.0/src/provider.rs:127`; https://github.com/openmls/openmls/issues/2169 |
| 10 | Exporter API | `MlsGroup::export_secret(&self, crypto, label: &str, context: &[u8], key_length: usize)` implements RFC 9420 Section 8.5 MLS-Exporter. Returns `Vec<u8>` in 0.9.2; on `main` it returns `ExportedSecret<T>` (zeroize-on-drop, use `.as_slice()`, #2202). Fails with `UseAfterEviction` once the member is removed. | `openmls-0.9.2/src/group/mls_group/exporting.rs:35-58`; root `CHANGELOG.md` Unreleased "Changed" at `28369d5` |
| 11 | Advisories on the released X-Wing stack | `cargo audit` on the 0.9.2 + libcrux 0.4.0 lockfile: RUSTSEC-2026-0331 (high, `libcrux-kem 0.0.9` panic on decoding short hybrid keys, i.e. a crafted KeyPackage can crash a member), RUSTSEC-2026-0330 (medium, same crate), RUSTSEC-2026-0329 (high, `libcrux-hmac-drbg 0.0.1`), RUSTSEC-2026-0387 (`crypto-bigint 0.5.5`). `hpke-rs-libcrux 0.7.0` pins `libcrux-kem =0.0.9`, so `cargo update` cannot fix it. The same audit on `main` (`hpke-rs-libcrux 0.8.0`, `libcrux-kem 0.0.10`, `libcrux-ml-kem 0.0.11`) is clean apart from one unmaintained-crate warning. | `cargo audit` (cargo-audit 0.22.2) on both spike lockfiles |
| 12 | Maturity and audit | Releases: PQ suites first shipped in `openmls_libcrux_crypto 0.4.0` / openmls 0.9.0 (2026-08-25, #2046); latest openmls 0.9.2 (2026-10-22). No public third-party audit of OpenMLS or of its PQ suites found in the repo or issues. Seven GHSA advisories since 2025-09, three published in Oct 2026. No interop results for PQ suites against other MLS stacks found (issue #1940 plans ts-mls testing for RustCrypto). | crates.io API; `gh api repos/openmls/openmls/security-advisories`; https://github.com/openmls/openmls/issues/1940 |
| 13 | mls-rs (AWS) for comparison | `mls-rs-crypto-awslc 0.27.0` has an X-Wing-style combined KEM behind `post-quantum` (which enables `aws-lc-rs/unstable`), on private code point `ML_KEM_768_X25519 = 65100` (0xFE4C). Not interoperable with OpenMLS or the draft. | `mls-rs-core-0.27.0/src/crypto/cipher_suite.rs:79`; `mls-rs-crypto-awslc-0.27.0/src/lib.rs:352` |

## Spike result

PASS, both suites, on the release and on `main`.
Crate: `spike/openmls-xwing/` (test in `tests/xwing_group.rs`, exact pins in `Cargo.toml`, `Cargo.lock` kept).
Pins: `openmls =0.9.2`, `openmls_traits =0.6.0`, `openmls_libcrux_crypto =0.4.0`, `openmls_basic_credential =0.6.0`, all with `draft-ietf-mls-pq-ciphersuites`.

What the test does, per suite, with every message round-tripped through TLS wire bytes:
1. Asserts the suite's KEM is `XWingKemDraft6` and that a KeyPackage init key is 1216 bytes (X-Wing public key size, IANA HPKE 0x647A).
2. Alice creates the group, adds Bob and Carol in one commit, both join from the one Welcome.
3. All three derive the same 32-byte exporter secret.
4. Alice removes Carol, Bob processes and merges the commit.
5. Asserts epoch advanced by exactly one, Alice and Bob agree on the new exporter secret, and it differs from the old one.

Where it ran: locally on the Mac via `mac-heavy cargo test`, because `ssh staging-entity` failed with `Permission denied (publickey)`.
Cost: about 100 to 130 s wall for a cold build, peak RSS reported by `/usr/bin/time -l` about 370 MB, far under the 12 GiB / 30 min limits.

RED (written first, built without the feature flag):

```
error[E0599]: no variant or associated item named `MLS_256_XWING_CHACHA20POLY1305_SHA256_Ed25519` found for enum `openmls::prelude::Ciphersuite`
error[E0599]: no variant or associated item named `MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519` found for enum `openmls::prelude::Ciphersuite`
```

GREEN (0.9.2 release):

```
MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519 ok: epoch 1 -> 2, exporter rotated, welcome 9522 B (2 joiners), remove commit 4023 B
test xwing_draft_ietf_aes256_sha384 ... ok
MLS_256_XWING_CHACHA20POLY1305_SHA256_Ed25519 ok: epoch 1 -> 2, exporter rotated, welcome 9346 B (2 joiners), remove commit 3974 B
test xwing_legacy_0x004d ... ok
test result: ok. 2 passed; 0 failed
```

GREEN (same test against `main` at `28369d5` via git deps, in a scratch copy; the only change was `.as_slice().to_vec()` on the `ExportedSecret` return):

```
MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519 ok: epoch 1 -> 2, exporter rotated, welcome 9450 B (2 joiners), remove commit 3999 B
MLS_256_XWING_CHACHA20POLY1305_SHA256_Ed25519 ok: epoch 1 -> 2, exporter rotated, welcome 9274 B (2 joiners), remove commit 3950 B
test result: ok. 2 passed; 0 failed
```

Size note: X-Wing makes a Welcome cost roughly 4.7 KB per joiner and a remove commit roughly 4 KB for a 3-member group, growing with log2(group size) path nodes.

## Content-key derivation sketch (XChaCha20-Poly1305 outside MLS)

The MLS exporter (RFC 9420 Section 8.5) already binds the group id, epoch, tree hash and transcript, so the context only needs to separate repos and purposes.

```rust
const LABEL: &str = "arkiv-git/content-key/v1";

// On every epoch change (after merge_pending_commit / merge_staged_commit / joining from a Welcome):
let epoch = group.epoch().as_u64();
let ctx = [repo_id.as_bytes(), &epoch.to_be_bytes()].concat(); // epoch is redundant with the exporter, kept for explicitness
let k_epoch = group.export_secret(provider.crypto(), LABEL, &ctx, 32)?; // ExportedSecret on main: .as_slice()
keystore.put(group_id, epoch, k_epoch); // must persist: OpenMLS discards the exporter secret when the epoch ends

// Seal one object (blob/tree/commit/packfile chunk):
let nonce: [u8; 24] = random();                     // XChaCha's 192-bit nonce makes random nonces safe
let aad = encode(b"arkiv-git/v1", group_id, epoch, repo_id, object_id);
let ct = XChaCha20Poly1305::new(&k_epoch).encrypt(&nonce, Payload { msg: plaintext, aad: &aad })?;
store(Header { v: 1, suite: 0x004E, epoch, nonce }, ct);  // header names the epoch so readers pick the key

// Open: look up k_epoch by (group_id, header.epoch), rebuild aad, decrypt.
```

Rotation policy maps directly onto MLS:
- Member or device add: `add_members` commit, epoch +1, new `k_epoch`.
- Member or device removal: `remove_members` commit (always carries an update path), epoch +1, removed member cannot compute the new `k_epoch`.
- Timer: `self_update` (commit with path) on a schedule, epoch +1, which gives post-compromise security.
- Writers must always seal with the newest `k_epoch` they hold.

Design consequence to settle in the key-management doc: a member who joins at epoch e cannot derive any `k_epoch` for epochs before e.
Git history must stay readable, so we need a key-history mechanism, for example a "key lockbox" object that encrypts the list of past epoch keys under the current `k_epoch` and is rewritten on each rotation.
That deliberately trades forward secrecy of old content for history access, and a removed member keeps every key up to their removal, which is unavoidable without re-encrypting history.

## Remaining risks

1. Wire instability before production: every PQ group created on released OpenMLS (0.9.x, hpke-rs 0.7) breaks against the next release because of the hpke-rs 0.8 `DeriveKeyPair` change, and OpenMLS's provisional code points (0x004E) already disagree with draft-07 (0x0011), so another renumbering is likely.
   Mitigation: no persistent groups until we pin a release (or a `main` rev) with the draft-ietf-hpke-pq derivation, record the ciphersuite in repo metadata, and design ReInit-based suite migration from day one.
2. The released libcrux X-Wing stack carries RUSTSEC-2026-0331 (high, remote crash via a malformed hybrid key in a KeyPackage) plus RUSTSEC-2026-0329/0330, fixable only by moving to `main` (hpke-rs 0.8, libcrux-kem 0.0.10).
   Mitigation: depend on a pinned `main` rev until the next release, and run `cargo audit` in CI.
3. Authentication stays classical: Ed25519 suites give PQ confidentiality against harvest-now-decrypt-later only, so a future quantum attacker could forge members or commits.
   Mitigation path: the X-Wing + ML-DSA-44 suite (0x0052, draft 0x0020) exists in both providers on `main`; no hybrid Ed25519+ML-DSA option exists.
4. Verification is partial: the X-Wing combiner (`libcrux-kem`) is pre-verification and parts of ML-KEM (`ind_cpa`, `sampling`, `matrix`) are not yet proven, so "formally verified" must not be claimed for the whole KEM.
5. No public audit and no cross-implementation interop data for OpenMLS PQ suites; several security advisories landed in Oct 2026.
6. Not checked in this spike: that X-Wing draft-06 (implemented) is byte-identical to draft-11 (latest), cross-provider interop (libcrux vs RustCrypto) on 0x004E, and behaviour with large groups.
7. Exporter-derived keys must be persisted by us (encrypted at rest), since OpenMLS drops the exporter secret at epoch end; losing the keystore loses content access for that epoch unless the key lockbox covers it.

## Options if a gate fails later (ranked)

The suite exists, so these are fallbacks only.
1. Pin a `main` rev of OpenMLS now and move to the next tagged release when it ships (small effort, about 0.5 day plus a rebase per upstream API break such as `ExportedSecret`).
2. Stay on 0.9.2 and fork `openmls_libcrux_crypto` to bump hpke-rs to 0.8 (a plain `[patch]` cannot work because `hpke-rs-libcrux 0.7` pins `libcrux-kem =0.0.9`); about 1 to 2 days, and it inherits the derivation break anyway.
3. Custom ciphersuite via provider traits: blocked in practice because `Ciphersuite` is a closed enum in `openmls_traits` (issue #1915, open), so it means forking OpenMLS (large, ongoing maintenance).
4. Switch to mls-rs with its awslc `post-quantum` feature: violates the locked "OpenMLS" decision and uses a private code point; listed only for completeness.

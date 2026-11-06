# P0: Post-quantum signature layer (ML-DSA)

Status: P0 design record.
Date: 2026-11-06.
Decided by the user, not reopened here: ML-DSA signatures from day one for identities, signed refs, ref anchors, collaborative objects and MLS credentials.
Locked context: Arkiv is trusted only for ordering, availability and timestamps; clients verify every signature; MLS (OpenMLS) with the X-Wing KEM; XChaCha20-Poly1305 for content; mandatory rotation; device keys in the Secure Enclave or a hardware key where the platform allows.

Labels: SOURCE-VERIFIED (read in code at the stated version), MEASURED (run here), DOCS-CLAIMED (vendor or spec text only), DERIVED (computed), RECOMMENDATION (needs the user's call where marked).

## TLDR

- Crate: RustCrypto `ml-dsa =0.1.2`, already in our lockfile through OpenMLS, pure Rust, `no_std`, FIPS 204 final, hedged signing available.
- Parameter set for our own signatures: ML-DSA-65 (NIST category 3, same as the ML-KEM-768 inside X-Wing, and the only level that Apple's Secure Enclave and Android 17 Keystore share with the 87 set).
- MLS: `MLS_128_MLKEM768X25519_CHACHA20POLY1305_SHA384_MLDSA44` (OpenMLS 0x0052, draft-07 0x0020), the only suite that combines X-Wing with ML-DSA; it forces ML-DSA-44 for MLS leaf keys.
- Spike: the existing create/add/remove/exporter test passes on 0x0052 on the pinned 0.9.2 release (RustCrypto provider) and on OpenMLS `main` at `28369d5` (RustCrypto and libcrux providers).
- Pure vs hybrid: RECOMMENDATION pending your call, composite ML-DSA-65 + Ed25519 (draft-ietf-lamps-pq-composite-sigs-19) for identity, device, sigrefs, anchor and COB signatures, pure ML-DSA-44 inside MLS because no hybrid MLS suite exists.
- Identity: node and identity IDs become a SHA-256 multihash of the multicodec-tagged public key, because a 1,952 B key fits neither a 128 B Arkiv `str` attribute nor a 255 B git ref path component.
- Git commit signing with a PQ algorithm works today through OpenSSH 10.6 `ssh-mldsa44-ed25519` (tested below), but stays optional; the ML-DSA-signed sigrefs plus the anchor remain the authority.
- heartwood reuse flips from "depend" to "vendor a subset": its Ed25519 key is a `Copy` 32-byte array threaded through about 900 type uses, so a `[patch]` cannot generalise it, and byte compatibility with the Radicle network is lost anyway once keys change.

## 1. Rust ML-DSA implementation

| Criterion | RustCrypto `ml-dsa` 0.1.2 | `libcrux-ml-dsa` 0.0.11 | `aws-lc-rs` 1.19.0 | `pqcrypto-mldsa` 0.1.2 | `fips204` 0.5.0 |
|---|---|---|---|---|---|
| FIPS 204 final | Yes, "as described in the FIPS 204 (final)" (`ml-dsa-0.1.2/README.md:11`); ACVP vectors (`tests/README.md:6`) and Wycheproof v1 (`tests/wycheproof.rs:1`) | Yes, links FIPS 204 final (`README.md:3`) | Yes; ML-DSA APIs stable since 1.18.0 (release notes, 2026-08-07) | PQClean ML-DSA | Claims FIPS 204 (crate docs, not read in depth) |
| Constant time | Branch-free hint code and Barrett division by constants (`src/hint.rs:22`, `src/algebra.rs:71-84`), after the decompose timing leak RUSTSEC-2025-0144 (fixed in 0.1.0-rc.3) | Claims via verified arithmetic; optional valgrind secret tracking (CHANGELOG 0.0.10) | AWS-LC C/asm, FIPS-lab tested | PQClean C | Claims constant time (not checked) |
| Verification / audit | No audit ("never been independently audited", `README.md:29`) | Field arithmetic, NTT and serialization verified with hax/F* (`README.md:7-10`); high-level sign/verify not verified | FIPS 4.0 module including ML-DSA submitted to NIST, certificate not issued yet; validated 3.x module has no ML-DSA | None | None found |
| Advisories (RustSec db `26fd2d43`, 2026-11-05) | 1, fixed (RUSTSEC-2025-0144) | 4 in 2026, all fixed by 0.0.9: wrong signer-response norm check in verify (RUSTSEC-2026-0077), hint-decoding panic (-0076), AVX2 `use_hint` edge case accepting what should be rejected (-0125), AVX2 reduction (-0126) | 5 `aws-lc-sys` in 2026, X.509/PKCS7/AES-CCM, none in ML-DSA | Unmaintained, PQClean archiving (RUSTSEC-2026-0166) | None |
| Signing mode | `Signer` trait is deterministic with empty context (`src/signing.rs:184-199`); hedged via `sign_randomized(msg, ctx, rng)` (`src/signing.rs:386`) | Caller supplies 32 B randomness (hedged) | Not checked | Randomized | Both (claimed) |
| no_std / wasm | `#![no_std]` (`src/lib.rs:1`), pure Rust, builds for wasm | `#![no_std]`, portable + AVX2 only, no NEON | No: C build (cmake/clang), no `wasm32-unknown-unknown` | No | Yes (claimed) |
| Deps | `module-lattice`, `shake`, `ctutils`, `hybrid-array`, `signature 3` | pinned `=0.0.x` libcrux crates | AWS-LC C library | PQClean C | Small |
| Maintenance | RustCrypto org; released 2026-10-31; 5.15M downloads | Cryspen; released 2026-10-07; 0.0.x API | AWS; released 2026-10-28 | Last release 2025-08-05 | Released 2026-10-23; 1.37M downloads |
| Already in our tree | Yes: OpenMLS `openmls_basic_credential` signs every ML-DSA credential with it, on 0.9.2 (`src/lib.rs:89-99`) and on `main` (`basic_credential/Cargo.toml:22`) | Only on OpenMLS `main` (libcrux provider verification) | No | No | No |

Pick: RustCrypto `ml-dsa`, exact-pinned `=0.1.2`.
Reasons: it is the dependency OpenMLS already uses to sign MLS credentials, so one ML-DSA implementation covers MLS and our own layer; it is pure Rust and `no_std`, so it builds for wasm and every target we ship; it runs the NIST ACVP and Wycheproof vectors in CI; and its one advisory is fixed.
Why not the others: libcrux's proofs do not cover the code where its 2026 verification bugs were (norm check, hint handling), it has no NEON path on Apple Silicon, and its API is 0.0.x; aws-lc-rs is the right answer only if FIPS validation becomes a requirement, and it costs a C toolchain and loses wasm; `pqcrypto-mldsa` is unmaintained; `fips204` has no edge over a crate we already carry.
Usage rules: always sign hedged (`sign_randomized` with the OS RNG), never through the deterministic `Signer` trait, because hedged signing is the FIPS 204 default and resists fault attacks; always pass a per-object ML-DSA context string (`arkiv-git/anchor/v1`, `arkiv-git/sigrefs/v1`, `arkiv-git/cob/v1`, `arkiv-git/device-cert/v1`, `arkiv-git/id/v1`); store the 32 B seed, not the expanded key.
Keep `libcrux-ml-dsa` as a dev-dependency for differential tests (sign with one, verify with the other); the OpenMLS `main` run below already does this for MLS.

## 2. Parameter set

| Set | NIST category | Public key | Signature | Seed | Arkiv gas per signature (calldata once + 1 year storage) | tGLM at 1 gwei | USD at 0.109 |
|---|---|---|---|---|---|---|---|
| Ed25519 (today) | classical | 32 B | 64 B | 32 B | 2,611 + 14,128 = 16,739 | 0.000017 | 0.000002 |
| ML-DSA-44 | 2 | 1,312 B | 2,420 B | 32 B | 98,736 + 534,220 = 632,956 | 0.00063 | 0.00007 |
| ML-DSA-65 | 3 | 1,952 B | 3,309 B | 32 B | 135,007 + 730,468 = 865,476 | 0.00087 | 0.00009 |
| ML-DSA-87 | 5 | 2,592 B | 4,627 B | 32 B | 188,782 + 1,021,420 = 1,210,201 | 0.00121 | 0.00013 |
| Composite ML-DSA-65 + Ed25519 | 3 | 1,984 B | 3,373 B | 64 B | 137,618 + 744,596 = 882,215 | 0.00088 | 0.00010 |

Sources: sizes from FIPS 204 Table 2 and draft-ietf-lamps-pq-composite-sigs-19 Appendix A; ML-DSA-44 sizes MEASURED by the spike (1,312 B key, 2,420 B signature).
Cost (DERIVED from `arkiv-limits.md` section 2): calldata 40.8 gas per random byte; storage `bytes x 15,768,000 blocks/year x 14e-6`, i.e. about 220.8 gas per byte-year; per-entity fixed cost excluded because the signature shares the entity with its payload.
At two signatures per push (sigrefs and anchor), 1,000 pushes kept for a year cost about 1.73 GLM (about 0.19 USD) with ML-DSA-65 against 0.03 GLM with Ed25519, which is noise next to the 28 GLM per 100 MB-year of pack storage.
Publishing an ML-DSA-65 public key once costs about 0.51M gas per year kept.

Pick for identity, device, sigrefs, anchor and COB signatures: ML-DSA-65.

- Category 3 matches the ML-KEM-768 inside X-Wing, so authenticity is not the weaker link.
- Apple CryptoKit's Secure Enclave exposes only `MLDSA65` and `MLDSA87` (https://developer.apple.com/documentation/cryptokit/secureenclave, iOS/macOS 26.0+), and Android 17 Keystore exposes ML-DSA-65 and ML-DSA-87 (Google security blog, 2026-03-25); ML-DSA-44 cannot be hardware-held on either.
- RFC 9980 (OpenPGP PQC) makes ML-DSA-65 + Ed25519 its one mandatory signature algorithm, so the same pairing is the most interoperable.
- The cost gap to ML-DSA-44 is about 0.05 USD per 1,000 pushes per year; ML-DSA-87 buys margin we do not need for 40 percent more bytes.

MLS is the exception: ML-DSA-44 is forced by the locked X-Wing KEM (section 4).

## 3. Pure ML-DSA vs hybrid (composite) signatures

### Hardware-key reality (2026-11-06)

| Hardware | ML-DSA today | Classical algorithms it holds | Source |
|---|---|---|---|
| Apple Secure Enclave | Yes: `SecureEnclave.MLDSA65` and `.MLDSA87`, iOS/iPadOS/macOS/watchOS/tvOS/visionOS 26.0+, `signature(for:context:)` supports a FIPS 204 context; no ML-DSA-44 | P-256 only (no Ed25519) | developer.apple.com CryptoKit docs, `SecureEnclave/MLDSA65/PrivateKey` (DOCS-CLAIMED) |
| Android Keystore (Android 17) | ML-DSA-65 and -87 via `KeyPairGenerator`; StrongBox coverage not stated | EC P-256, RSA, Ed25519 (KeyMint 2+, not re-checked) | https://blog.google/security/security-for-the-quantum-era-implementing-post-quantum-cryptography-in-android/ (DOCS-CLAIMED) |
| YubiKey | No shipping firmware; a next-generation PQC YubiKey with ML-DSA is in a gated beta from 2026-11-03 | Ed25519 (PIV from firmware 5.7, OpenPGP, FIDO2), P-256 | https://www.yubico.com/pqc-beta-program (DOCS-CLAIMED) |
| TPM 2.0 | TCG Library spec v1.85 adds ML-DSA-44/65/87 and ML-KEM; only software TPMs (wolfTPM fwTPM) implement it, no shipping chip found | RSA, ECDSA P-256 | wolfSSL wolfTPM v1.85 posts (DOCS-CLAIMED) |
| Windows CNG, .NET | ML-DSA and composite ML-DSA in software | n/a | Microsoft .NET `CompositeMLDsaAlgorithm` docs |

So "device keys in hardware" under ML-DSA means: on Apple 26+ (and likely Android 17 TEE) the ML-DSA-65 key itself is non-exportable hardware; on Linux, Windows and older Apple devices there is no PQ hardware key, and the only hardware option is a classical key (YubiKey Ed25519, TPM P-256) whose protection a quantum attacker who also steals the software ML-DSA half can bypass.

### Option A: pure ML-DSA-65

- One algorithm, one code path, smallest bytes, matches TLS (draft-ietf-tls-mldsa-06, in the RFC Editor queue) and JOSE/COSE (RFC 9964).
- Hardware-held on Apple 26+ and Android 17 without any extra work.
- Single point of failure: an implementation bug in young ML-DSA code (four libcrux advisories and one RustCrypto advisory in the last year, one of them in verification) or a cryptanalytic surprise makes every signature forgeable, with no classical fallback.
- On platforms without PQ hardware the device key is a software key protected only by the OS keychain.

### Option B: composite ML-DSA-65 + Ed25519 (`id-MLDSA65-Ed25519-SHA512`, OID 1.3.6.1.5.5.7.6.48)

- Construction (draft-ietf-lamps-pq-composite-sigs-19 section 3.2): `M' = "CompositeAlgorithmSignatures2025" || Label || len(ctx) || ctx || SHA-512(M)`; the ML-DSA half signs `M'` with `mldsa_ctx = Label`, the Ed25519 half signs `M'`; both must verify.
- A forger must break both halves, which covers the realistic near-term risk (an ML-DSA implementation bug) as well as the quantum one.
- Hardware: holding either half in hardware makes the composite key non-exportable for as long as that half's algorithm stands; Apple SE can produce the ML-DSA half (it supports the `Label` context), a YubiKey can produce the Ed25519 half, and that is the honest meaning of "device keys in hardware" off Apple: classical hardware protection now, degrading to software-ML-DSA protection once a quantum computer exists.
- Matches the ecosystem's caution for long-lived signatures: RFC 9980 defines ML-DSA only in composite form, OpenSSH chose `ssh-mldsa44-ed25519`, Sigstore proposes dual-signed Rekor checkpoints.
- Costs: 64 extra bytes per signature and 32 per key, a second (already vendored) primitive (`ed25519-dalek`), about 100 lines of composite glue tested against the draft's JSON vectors, a draft that is in the RFC Editor queue but not yet an RFC, and no multicodec code for composite keys (private-use code or a registration needed).
- Apple wrinkle: the SE has no Ed25519, so on Apple the Ed25519 half is a keychain software key next to the SE-held ML-DSA half; `id-MLDSA65-ECDSA-P256-SHA512` (OID .45) would put both halves in the SE but makes key types differ per platform, which we should avoid.
- MLS cannot be composite: OpenMLS and draft-ietf-mls-pq-ciphersuites-07 define no hybrid signature suite, so MLS leaf signatures are pure ML-DSA-44 under either option.

### RECOMMENDATION (pending your call: you said "ML-DSA", not "pure" or "hybrid")

Option B, composite ML-DSA-65 + Ed25519, for identity keys, device keys, sigrefs, anchors and COB changes; pure ML-DSA-44 inside MLS because nothing else exists there.
Reason: these signatures are long-lived evidence on a public chain, the main risk in 2026 is a bug in young ML-DSA code rather than a quantum computer, X-Wing already applies the same belt-and-braces logic to confidentiality, and the cost is 64 bytes plus a small amount of glue.
Either way, every key and signature carries an algorithm tag (multicodec) so we can move between A and B, or to ML-DSA-87, without a format break.
What flips it to A: you prefer one primitive over defence in depth, or the composite draft changes its encoding before RFC publication in a way we do not want to track.

## 4. MLS: X-Wing + ML-DSA

| Suite | OpenMLS code point | draft-07 code point | KEM | Signature | Release 0.9.2 providers | `main` `28369d5` providers |
|---|---|---|---|---|---|---|
| `MLS_128_MLKEM768X25519_CHACHA20POLY1305_SHA384_MLDSA44` | 0x0052 (`openmls_traits-0.6.0/src/types.rs:511`) | 0x0020 (draft-07 section 3.1) | X-Wing, HPKE 0x647A | ML-DSA-44 | RustCrypto only (`openmls_rust_crypto-0.6.0/src/provider.rs:102`); libcrux 0.4.0 has no ML-DSA | RustCrypto and libcrux (`libcrux_crypto/src/crypto.rs:122`, libcrux-ml-dsa 0.0.11, hedged signing at `:405-411`) |
| `MLS_192_MLKEM768_AES256GCM_SHA384_MLDSA65` | 0x0051 (`types.rs:519`) | 0x0021 | pure ML-KEM-768, not X-Wing | ML-DSA-65 | RustCrypto | both |
| `MLS_256_MLKEM1024_AES256GCM_SHA384_MLDSA87` | 0x0907 (`types.rs:527`) | 0x0022 | pure ML-KEM-1024 | ML-DSA-87 | RustCrypto | both |

- Feature flag: `draft-ietf-mls-pq-ciphersuites` on `openmls`, `openmls_traits`, `openmls_basic_credential` and the provider crate.
- Draft status: draft-ietf-mls-pq-ciphersuites-07 (2026-11-01), Standards Track, not yet an RFC; IANA has assigned none of these code points, and OpenMLS's provisional values differ from the draft's, so a renumbering is expected (see `openmls-xwing.md` risk 1).
- 0x0052 is the only X-Wing suite with a PQ signature; the draft pairs X-Wing with ML-DSA-44 on purpose, at its "128" level, choosing ChaCha20-Poly1305 for software speed (draft-07 section 2).
- Draft-07 section 2 specifies ML-DSA.Sign with an empty context and the raw `pkEncode` key, which OpenMLS follows; OpenMLS's basic credential signs deterministically (`openmls_basic_credential-0.6.0/src/lib.rs:89-99`), which FIPS 204 allows and verifiers cannot tell apart.
- Pick: 0x0052, the only suite consistent with the locked X-Wing decision.

### Spike result: PASS on release and on `main`

Test: `spike/openmls-xwing/tests/xwing_group.rs`, `xwing_mldsa44_0x0052`; the lifecycle is now generic over the provider, and the two Ed25519 tests still run on libcrux unchanged in behaviour.
New pin: `openmls_rust_crypto = "=0.6.0"` with `draft-ietf-mls-pq-ciphersuites`; `ml-dsa 0.1.2` and the provider were already locked transitively, so `Cargo.lock` only gained one direct-dependency line and no new package.
What the new test adds before the shared lifecycle: asserts the suite's signature scheme is ML-DSA-44 and its code point 0x0052, checks a fresh ML-DSA-44 key is 1,312 B and a signature 2,420 B, and verifies a `basic_credential` signature through the provider's verifier.
The shared lifecycle then runs create, add two members via one Welcome, equal exporter secrets, remove, epoch +1, exporter rotated, with every message round-tripped through TLS wire bytes.

RED (test written first, `openmls_rust_crypto` not yet a dependency):

```
error[E0432]: unresolved import `openmls_rust_crypto`
error: could not compile `openmls-xwing-spike` (test "xwing_group") due to 1 previous error
```

GREEN on the pinned release (`mac-heavy cargo test --offline -- --nocapture`, about 9 s warm, peak RSS about 420 MB):

```
MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519 ok: epoch 1 -> 2, exporter rotated, key package 2665 B, welcome 9522 B (2 joiners), remove commit 4023 B
MLS_256_XWING_CHACHA20POLY1305_SHA256_Ed25519 ok: epoch 1 -> 2, exporter rotated, key package 2665 B, welcome 9346 B (2 joiners), remove commit 3974 B
MLS_128_MLKEM768X25519_CHACHA20POLY1305_SHA384_MLDSA44 ok: epoch 1 -> 2, exporter rotated, key package 8658 B, welcome 22797 B (2 joiners), remove commit 10016 B
test result: ok. 3 passed; 0 failed
```

GREEN on OpenMLS `main` at `28369d5b48608bef4ddf8371d91503b0f3852bb3` (scratch copy with git deps, only change `ExportedSecret::as_slice()`), with an extra test running 0x0052 on the libcrux provider, where RustCrypto `ml-dsa` signs the credentials and `libcrux-ml-dsa` verifies them, a cross-implementation check:

```
MLS_128_MLKEM768X25519_CHACHA20POLY1305_SHA384_MLDSA44 ok: epoch 1 -> 2, exporter rotated, key package 8634 B, welcome 22725 B (2 joiners), remove commit 9992 B
test xwing_mldsa44_0x0052_libcrux ... ok
test xwing_mldsa44_0x0052 ... ok
test result: ok. 4 passed; 0 failed
```

`cargo audit` (cargo-audit 0.22.2, db 2026-11-05): adding the RustCrypto provider introduces no new advisory; the release lockfile still carries the four known libcrux/crypto-bigint advisories from `openmls-xwing.md`, and the `main` lockfile is clean apart from the `proc-macro-error2` unmaintained warning.

Consequences:

- ML-DSA roughly triples MLS message sizes: KeyPackage 8.7 KB (was 2.7 KB), Welcome 22.8 KB for a 3-member group (was 9.5 KB), remove commit 10 KB (was 4 KB).
- Each member adds about 5 KB to the ratchet tree (1,216 B X-Wing key, 1,312 B ML-DSA key, 2,420 B leaf signature), so a Welcome carrying the ratchet tree outgrows one 64 KiB chunk at roughly a dozen members; MLS messages need the same chunking as packs, or the tree must travel separately.
- ML-DSA-44 MLS leaf keys cannot live in the Apple Secure Enclave or Android Keystore (both offer 65/87 only); they are software keys per device, bound to the hardware-held device key by a device-signed statement carried in the MLS `BasicCredential` identity and checked by our credential validation (OpenMLS leaves credential validation to the application).
- Build on the pinned `main` rev for both providers (as `openmls-xwing.md` already recommends); on the 0.9.2 release only the RustCrypto provider can run 0x0052.

## 5. Identity encoding

Constraints that force a hash-based ID:

- An ML-DSA-65 public key is 1,952 B, about 2,670 base58 characters as a `did:key`.
- Arkiv indexed `str` attributes hold at most 128 B (`arkiv-limits.md`), and `bytes` attributes are never indexed.
- heartwood puts the node ID in ref paths (`refs/namespaces/<nid>/...`), and a loose ref path component must fit the 255 B filesystem name limit.

Proposed format:

- Key ID (KID) = multihash `sha2-256` (`0x12 0x20`) of `varint(multicodec) || public key`.
- Multicodec: `mldsa-65-pub` = 0x1211 (`multiformats/multicodec` `table.csv:184` at `fbd77eb`, 2026-10-23, status draft); `mldsa-44-pub` 0x1210 and `mldsa-87-pub` 0x1212 also exist; composite keys have no code, so option B needs a private-use code (0x300000 to 0x3FFFFF) or a registration.
- Text form: multibase base32 lower-case (prefix `b`, about 59 characters), because macOS file systems are case-insensitive and base58 IDs in loose ref paths could collide by case; heartwood's `z6Mk...` base58 form (`radicle-crypto/src/lib.rs:99`) has the same latent issue.
- This is libp2p's PeerId rule (hash keys larger than 42 bytes) with multicodec framing instead of protobuf.
- The public key travels next to the ID wherever a verifier first needs it (the identity document's delegate entries and the device certificate); every verifier recomputes the KID from the key before trusting it.
- RID: heartwood's RID is the SHA-1 blob ID of the root document (`heartwood-reuse.md` section 3); since byte compatibility with Radicle is gone anyway (section 7), use SHA-256 of the canonical root document and keep SHA-1 only for local git objects.

Key hierarchy (RECOMMENDATION):

| Key | Algorithm | Held where | Signs | Rotation |
|---|---|---|---|---|
| Identity key (one per person; a delegate in identity documents) | ML-DSA-65 or composite (section 3) | Offline 32 B seed or hardware key, used rarely; recovery via a second delegate | Device certificates, identity document revisions | Identity document revision, accepted by a majority of the parent's delegates (heartwood's `n/2 + 1` rule) and co-signed by the new key as proof of possession |
| Device key (one per device) | Same as identity key | Secure Enclave (Apple 26+), Android 17 Keystore, else OS keychain (plus YubiKey Ed25519 half under option B) | sigrefs, anchors, COB changes, binding entities, MLS credential bindings | Mandatory: a new device certificate with `not_after`; the old device's last anchor names the successor device |
| MLS leaf key (one per device per group) | ML-DSA-44 (suite 0x0052) | Software, encrypted at rest | MLS KeyPackages, commits, GroupInfo | Every MLS update path, plus a key bound to each new device certificate |
| Arkiv EVM owner key | secp256k1 (Arkiv requirement) | Separate, bound to the device key by a signed binding entity | Arkiv transactions only, no authority | With the device key |

- Device certificate: `{v, identity_kid, device_kid, device_pk, mls_binding?, not_before, not_after, seq}` signed by the identity key with context `arkiv-git/device-cert/v1`, published as an Arkiv entity; Arkiv's trusted order gives revocation and rotation a timestamp.
- Namespaces and anchor chains stay per device, as in Radicle, so one key keeps one chain and a seq conflict still means equivocation; canonical-ref quorum counts each identity once, using the latest anchored head among its currently certified devices.
- Verification of an old anchor uses the identity document revision named by its `id_head` and the device certificates valid at the anchor's Arkiv position, so rotating or revoking a key never invalidates history, and a stolen old key cannot back-date anything because Arkiv orders it.
- P2 fit: KIDs are opaque 32 B hashes, so blinding at the Arkiv boundary is one HMAC (`ag.nid = HMAC(epoch key, KID)`) and signatures are computed over plaintext before encryption, which leaves the "encrypt and key-hash only at the Arkiv boundary" design unchanged.

## 6. Git commit and tag signing

| Path | PQ signing today? | Evidence |
|---|---|---|
| SSH signatures (`gpg.format = ssh`) | Yes, hybrid ML-DSA-44 + Ed25519: OpenSSH 10.4 (2026-07-06) added experimental `ssh-mldsa44-ed25519@openssh.com`, 10.6 (2026-10-06) enabled `ssh-mldsa44-ed25519` without the vendor suffix; draft-ietf-sshm-composite-sigs-00 also defines ML-DSA-87 + P-384 (DOCS-CLAIMED, search summary) | https://www.openssh.com/txt/release-10.4, release-10.6; MEASURED below |
| OpenPGP | Standard exists: RFC 9980 (2026-06-30) defines ML-DSA-65 + Ed25519 (algorithm 30, MUST) and ML-DSA-87 + Ed448; GnuPG release support not confirmed; Sequoia 2.4 recognises the algorithm but backend support varies; rPGP 0.19 implements it | RFC 9980; docs.rs sources (DOCS-CLAIMED) |
| X.509 / gitsign | No: Fulcio issues no ML-DSA certificates; Sigstore TSA 2.2.0 has experimental ML-DSA timestamps, Rekor dual-signing is a proposal | Sigstore blog and issues (DOCS-CLAIMED) |

MEASURED (scratch repo, `nix-shell -p openssh git`, OpenSSH 10.6p1, git 2.56.0):

```
$ ssh-keygen -t mldsa44-ed25519 ...    # public key line: ssh-mldsa44-ed25519, 1828 base64 chars
$ git -c gpg.format=ssh -c commit.gpgsign=true commit -m "test: signed with mldsa44-ed25519"
$ git verify-commit -v HEAD
Good "git" signature for t@example.invalid with MLDSA44-ED25519 key SHA256:hp8i...
commit object bytes: 5620 (armored SSH signature 5444 B)
```

Decision, as in Radicle: commit and tag signatures stay optional and carry no authority; the ML-DSA-signed sigrefs plus the on-chain anchor decide which refs are valid.
Users who want PQ commit signatures can use `ssh-mldsa44-ed25519` now, with two caveats: macOS ships OpenSSH 10.3 (no ML-DSA), so `gpg.ssh.program` must point at a 10.6 `ssh-keygen`, and the SSH key is a separate key from the device key because no SSH agent fronts Secure Enclave ML-DSA keys.

## 7. heartwood reuse under ML-DSA

### How deep Ed25519 goes (heartwood `e6fe5207`, paths under `crates/`)

- `radicle-crypto`: `PublicKey` is `#[derive(Copy, ...)] struct PublicKey([u8; 32])` (`radicle-crypto/src/lib.rs:101,110,131`); `Signature` is a re-export of `ed25519::Signature`, a fixed 64 B array (`lib.rs:26`); the `Signer` trait requires `signature::Signer<Signature>` and `Keypair<VerifyingKey = VerifyingKey>` (`lib.rs:47-60`); the multicodec prefix is hard-coded `[0xED, 0x01]` (`lib.rs:99,215,282`); `ed25519-dalek` and `ssh-key` with `ed25519` are direct dependencies (`radicle-crypto/Cargo.toml:35,41`).
- Identities are that key: `NodeId = PublicKey` (`radicle-core/src/node.rs:24`), `Did(crypto::PublicKey)` (`radicle/src/identity/did.rs:19`).
- sigrefs parse the signature blob as a 64 B Ed25519 signature (`radicle/src/storage/refs/sigrefs/read.rs:530`).
- COBs: the change-storage trait is generic over `PublicKey`/`Signature`, but `Store` and `ChangeGraph` pin them to the Ed25519 types (`radicle-cob/src/lib.rs:115-123`, `radicle-cob/src/change_graph.rs:40-45`), and `Signatures` is a `BTreeMap<PublicKey, Signature>` (`radicle-cob/src/signatures.rs:23`).
- `ExtendedSignature<PublicKey = crate::PublicKey, Signature = crate::Signature>` is already generic in its parameters (`radicle-crypto/src/lib.rs:745`), the one seam that helps.
- Counts (`grep -w` over `src/`, test code included): `radicle` has `NodeId` 423, `Did` 183, `Signer` 160, `PublicKey` 98, `Signature` 97 uses; `radicle-cob` `PublicKey` 37, `Signature` 37, `ExtendedSignature` 22; `radicle-crypto` `PublicKey` 80; 84 files across `radicle`, `radicle-cob`, `radicle-core`, `radicle-crypto` touch these types, and 56 `derive(... Copy ...)` lines sit in files that use them, so `Copy` and the 32 B size are load-bearing.
- `radicle-dag` (0.10.0) is crypto-free; its only dependency is `fastrand` (`radicle-dag/Cargo.toml:12`).
- No post-quantum or algorithm-agility work exists upstream (no hits for "quantum", "ml-dsa", "dilithium" in the tree or the commit log).

### Options

| Option | What it means | Initial effort | Upgrade cost | Radicle compatibility | Risk |
|---|---|---|---|---|---|
| (a) depend + `[patch]` / fork `radicle-crypto` | A `[patch]` alone cannot work: a non-`Copy`, variable-size key and a non-64 B signature change the API of `radicle`, `radicle-cob` and `radicle-core`, so this is a fork of four crates | 3 to 5 weeks | Rebase across 84 files every heartwood minor (one every 1 to 2 months), about 2 to 5 days each | Lost anyway (DIDs, RIDs and signatures differ), so the main reason to depend disappears | Permanent fork of a fast-moving pre-1.0 codebase, still dragging libgit2 and bundled sqlite |
| (b) vendor a subset and adapt | Copy, with attribution and a recorded upstream commit, the sigrefs format and reader/writer, the identity `Doc` and identity-COB evaluation, canonical-ref quorum and the COB change format (about 12k lines including tests, roughly half of them non-test); depend on `radicle-dag` as is; replace the key types with our algorithm-tagged KID and key types | 2 to 3 weeks | A monthly upstream watch on those paths (`git log <pinned>..upstream -- <paths>`) and hand-porting of security fixes, about 0.5 to 1 day per month | Lost, but formats stay structurally identical, so a bridge needs only re-signing | We own the security-critical code; upstream fixes (such as 1.7.0 replay, 1.10.0 identity evaluation) arrive only when we port them |
| (c) own layer, heartwood as reference | Write sigrefs, identity and COB logic from the spec and heartwood's code, keep plain git2/gix for objects | 3 to 5 weeks | None from upstream | Lost | Highest chance of re-introducing bugs heartwood already fixed, mostly in identity evaluation |

RECOMMENDATION: option (b), vendor the format and verification subset and adapt it to ML-DSA keys; depend unchanged only on crypto-free leaf crates such as `radicle-dag`.
What flips it: back to (a) or plain depend if upstream heartwood adopts algorithm-agile or PQ keys; to (c) for a piece we would rewrite anyway, for example if P1 needs only sigrefs and identity (both small) and no COB engine.
P2 compatibility: unchanged; the vendored code works on plaintext SHA-1 git in the local cache, and encryption plus keyed hashing happen only at the Arkiv boundary, as `heartwood-reuse.md` section 7 already argues.
Side effect: (b) drops libgit2-only storage traits and bundled sqlite from the dependency tree, which was one of the "vendor" triggers in `heartwood-reuse.md`.

## Conflicts with the plan and earlier P0 docs

1. MLS authenticity is ML-DSA-44 (NIST category 2), one level below the category 3 X-Wing KEM, because 0x0052 is the only X-Wing + ML-DSA suite; the alternative 0x0051 (ML-DSA-65) would mean dropping X-Wing for pure ML-KEM-768, which contradicts the locked KEM choice.
2. "Device keys in the Secure Enclave" cannot cover MLS leaf keys: neither Apple's SE nor Android Keystore offers ML-DSA-44, so MLS signing keys are software keys bound to the hardware device key.
3. Hardware-held ML-DSA keys exist only on Apple OS 26+ (and Android 17); YubiKey PQC is a beta, no TPM ships ML-DSA, so on most desktops and Linux the device key is software-held unless we take the composite option with a YubiKey Ed25519 half.
4. `heartwood-reuse.md` recommended depending on pinned `radicle`, `radicle-cob` and `radicle-crypto`; under ML-DSA this flips to vendoring a subset, and byte compatibility with the Radicle network (same RID, DIDs and sigrefs) is no longer possible.
5. `heartwood-reuse.md` section 8 assumes `ag.nid` holds a `did:key` / `z6Mk...` string; under ML-DSA it holds the 59-character base32 KID (or its HMAC for private repos).
6. On the 0.9.2 release, 0x0052 runs only on the RustCrypto provider; the libcrux provider needs OpenMLS `main`, consistent with the existing "pin a `main` rev" recommendation.
7. ML-DSA MLS messages are about 2.5 to 3 times larger, so a Welcome with the ratchet tree passes one 64 KiB chunk at about a dozen members; MLS payloads need chunking or an out-of-band tree.
8. Composite keys (option B) have no multicodec code yet and depend on a draft in the RFC Editor queue.
9. `openmls-xwing.md` still lists only the Ed25519 suites in its spike result; this document's spike section supersedes that for MLS signatures (that file was not edited, per the brief).

## Sources

- FIPS 204: https://csrc.nist.gov/pubs/fips/204/final.
- RustCrypto `ml-dsa` 0.1.2 crates.io tarball (paths above); crates.io API, 2026-11-06, for versions, dates and downloads.
- `libcrux-ml-dsa` 0.0.11 tarball; cryspen/libcrux `1f9c898` (2026-11-03) `libcrux-ml-dsa/README.md`, `CHANGELOG.md`.
- aws-lc-rs v1.18.0 release notes: https://github.com/aws/aws-lc-rs/releases/tag/v1.18.0.
- RustSec advisory db at `26fd2d43` (2026-11-05): RUSTSEC-2025-0144, RUSTSEC-2026-0076, -0077, -0125, -0126, -0166, -0044 to -0048.
- OpenMLS 0.9.2 crate set tarballs and openmls/openmls `28369d5b48608bef4ddf8371d91503b0f3852bb3` (paths above).
- draft-ietf-mls-pq-ciphersuites-07 (2026-11-01), sections 2 and 3.1.
- draft-ietf-lamps-pq-composite-sigs-19 (datatracker: RFC Editor queue), sections 2.2, 3.2, 7 and Appendix A.
- RFC 9980 (OpenPGP PQC), RFC 9964 (ML-DSA for JOSE/COSE), RFC 9881 (ML-DSA in X.509), draft-ietf-tls-mldsa-06; datatracker API, 2026-11-06.
- multicodec `table.csv` at `fbd77ebf05` (2026-10-23).
- Apple CryptoKit `SecureEnclave`, `SecureEnclave.MLDSA65.PrivateKey`: https://developer.apple.com/documentation/cryptokit/secureenclave.
- Android: https://blog.google/security/security-for-the-quantum-era-implementing-post-quantum-cryptography-in-android/.
- Yubico: https://www.yubico.com/pqc-beta-program.
- TPM v1.85: https://www.wolfssl.com/wolftpm-post-quantum-cryptography-release-ml-dsa-and-ml-kem-support-via-tcg-tpm-2-0-library-specification-v1-85/.
- OpenSSH: https://www.openssh.com/txt/release-10.4, https://www.openssh.com/txt/release-10.6.
- Sigstore: https://blog.sigstore.dev/post-quantum-2025/, https://github.com/sigstore/timestamp-authority/releases/tag/v2.2.0.
- heartwood `e6fe5207` clone (paths above).

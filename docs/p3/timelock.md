# Timelock encryption (drand tlock)

Data can be encrypted now so that anyone can decrypt it from time T, with no trusted party and no agent that has to stay online.
This covers sponsorware that opens on a date, embargoed security releases, sealed patches that reveal themselves, and a private repo's epoch key released on schedule.

## Crate pick

The implementation reuses `tlock_age` 0.0.10 (MIT, thibmeu/tlock-rs) for encryption and `drand_core` 0.0.19 (MIT, thibmeu/drand-rs) for beacon verification and HTTP fetching.
Nothing of the identity-based encryption is written here.

Candidates surveyed:

- `tlock` 0.0.10 (released 2026-07-12) is the primitive only.
- It encrypts exactly 16 bytes and strips trailing zeros on decrypt, so it is used only underneath `tlock_age`.
- `tlock_age` 0.0.10 (2026-07-12) is chosen.
- It encrypts arbitrary-length plaintext as an age file with a `tlock <round> <chain-hash>` stanza, interoperable with drand's Go `tlock` CLI.
- Its age file key is the 16-byte tlock payload and it pads that back to 16 bytes, so the trailing-zero issue does not apply.
- `drand_core` 0.0.19 (2026-07-12) is chosen for `ApiBeacon::verify`, the BLS check of a beacon against a chain's public key.
- It also supplies `HttpClient`, used with a pinned chain hash and public key.

All three come from one maintainer and are MIT licensed.
They are actively developed (tlock-rs pushed 2026-07-13, drand-rs 2026-10-28) and pure Rust (arkworks pairing code, no C).
The single-maintainer risk is accepted because the format is the drand and age standard.
A blob therefore stays openable by drand's Go `tlock` if the crate is ever replaced.
Quicknet support was confirmed in the source, where the default `rfc9380` feature selects the hash-to-curve that `bls-unchained-g1-rfc9380` requires.
A real round-1000 beacon decrypts in the test suite.

Pinned chain (quicknet):

- Chain hash `52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971`.
- Scheme `bls-unchained-g1-rfc9380`, period 3 seconds, genesis 1692803367, signatures on G1 (48 bytes), public key on G2.
- Public key and genesis are constants in `crates/charter-timelock/src/chain.rs`.

## Trust assumption

Decryption needs the BLS threshold signature on the round number, which the drand network publishes at that round's time.
Security therefore rests on the League of Entropy: fewer than a threshold of its operators colluding cannot produce a round's signature early.
If a threshold of operators colluded they could unlock data early.
If the network halted, data would not open until it resumes.
Nothing else is trusted, because relays only carry beacons.
Every beacon is verified against the pinned public key before use, so a lying relay cannot cause a wrong or forged open.
Timelock gives a lower bound on secrecy, not an upper bound, so never rely on it to keep something secret forever.

## API (`charter-timelock`)

- `seal(plaintext, unlock_at_unix) -> SealedBlob { round, chain_hash, ciphertext }` picks the first round published at or after `unlock_at`, so it never opens early.
- `open(blob, beacon_signature) -> plaintext` verifies the signature against the pinned key and the blob's round, then decrypts.
- `open_when_due(blob, now, fetch)` returns `E_TOO_EARLY` with the unlock time before the round, without any network call.
- `fetch_beacon(round)` asks `api.drand.sh`, `api2.drand.sh` and `drand.cloudflare.com` in turn and returns only a verified signature.
- `SealedBlob` encodes to CBOR (`to_bytes` and `from_bytes`).
- Round arithmetic: round `r` is published at `genesis + (r - 1) * 3`, and `round_at` is the ceiling of that inverse.

## CLI

```
chart timelock seal --until <RFC3339|+duration> --in <file> --out <file>
chart timelock open --in <file> --out <file>
```

`--until` takes an instant such as `2027-01-01T00:00:00Z` or a relative `+90s`, `+30m`, `+12h`, `+7d`.
`open` fails with `E_TOO_EARLY` and the unlock time when the round has not been reached, and writes nothing.
Exit code is 1 for verification errors including `E_TOO_EARLY`, 2 for usage errors, and 3 when no relay could be reached.

## How repo programs use it

A repo program sees an op of type `xyz.arkiv.timelock` whose body is a CBOR `SealedBlob`.
The blob names its own round and chain, so any client can compute the unlock time without asking anyone.
Once that time has passed, any client fetches the round's beacon from any relay, verifies it, and opens the blob.
No agent, key holder or server has to act at reveal time.

- Sealed patch: the author publishes the patch encrypted under a random key and seals that key in an `xyz.arkiv.timelock` op.
- Clients show "opens at T" until the round passes, then reveal the patch.
- Sponsorware by date: the release key is sealed to the public-release date.
- Supporters get the key earlier from the author, and everyone else gets it automatically on the date.
- Embargoed security fix: the fix and its advisory are sealed to the embargo end.
- Disclosure then cannot be skipped or delayed by a lost key.
- Private repo epoch key: an epoch key is sealed to its release time.
- Scheduled opening of that history then does not depend on the owner being around.

The op carries ciphertext only and no signing material.
This crate sends no Arkiv transactions.

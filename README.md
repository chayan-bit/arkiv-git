# arkiv-git

Decentralized git hosting on Arkiv: a redesign of the Radicle (heartwood) protocol where Arkiv replaces gossip and seed nodes.
Arkiv is trusted for ordering, availability and timestamps only; clients verify every signature.

Plan of record: the "Arkiv Git: decentralized code hosting plan" doc (private Claude Doc).

## Phases in this repo

- P0 spike: measure Arkiv limits, confirm OpenMLS X-Wing support, decide fork vs depend on heartwood. Findings in `docs/p0/`.
- P1 core: `git-remote-arkiv`, identity documents, signed refs, ref anchoring, search.
- P2 private: encrypted repos with MLS groups (X-Wing), roles, rotation, blinded annotations.

Testnet only. Never use real funds or mainnet keys.

## Status (2026-11-06)

- P0 closed: limits and costs measured on the Rabdi testnet; see `docs/p0/README.md` for numbers, decisions and open questions.
- P1 implemented: `git push arkiv::<rid>` and `git clone arkiv::<rid>` work end to end.
- P1 gate passes against the in-memory Arkiv fake: a 100 MB repo pushes in about 18 s and clones in about 2 s, and a replayed old anchor or refs entity is rejected.
- On Rabdi a 10 MiB gate has passed live (below); the full 100 MB Rabdi run waits on testnet funding (open decision 2).
- Push funding uses rolling top-ups (design 4.2 step 6): the wallet needs the predicted spend plus one window of in-flight max fees, not 4.4 times the spend.

### Live Rabdi gate (2026-11-07)

All numbers MEASURED on Rabdi through the real `git push arkiv::` and `git clone arkiv::` binaries, every entity at a 1 day lifetime.

- Size: 10 MiB of incompressible blobs in 161 chunks, all 161 live after the push.
- Push of 10 MiB: 519.7 s over 261 blocks and 171 txs (161 chunks, manifest, refs, anchor and about 7 wallet top-ups).
- Second small push: 30.0 s over 16 blocks and 4 txs.
- Fresh clone in a second home: 13.2 s; HEAD, tree and blob bytes match and `git fsck --full --strict` is clean.
- Replay: the old anchor and refs re-posted from a foreign address and the old anchor re-posted from the peer's own control address; a fresh clone still gets the newest head.
- Stale head: a client holding the newest mark, served a view that hides the newest anchor with `entityNonce` lowered to match, gets `E_STALE_HEAD` (confirmed by a read-only recheck of the same repo after fixing a bug in the test's hiding proxy, which first made the check fail closed with `E_INCONSISTENT_VIEW`).
- Base fee: 1 gwei at start and at peak during both pushes.
- Rate limits: no 429 waits with the access key.
- Net spend: 0.4352 tGLM for the whole run (preflight predicted 0.4437), funded with rolling top-ups from a 0.59 tGLM wallet, all leftovers swept back.
- The push ran at about 0.6 chunks per block, well under the 4 per block pacing allows; per-tx RPC round trips (estimate, send, receipt polls) are the likely cause (not yet profiled).

## Development

```sh
nix develop                                   # Rust 1.97 toolchain, git, cargo-audit
cargo test --workspace --all-features         # unit, property, fake-Arkiv and git E2E tests
cargo test --release -p arkiv-git-sync --all-features --test gate -- --ignored --nocapture   # 100 MB gate on the fake
ARKIV_GIT_RABDI=1 cargo test -p arkiv-git-cli --test rabdi_smoke -- --ignored              # live smoke, needs the funded throwaway key
ARKIV_GIT_RABDI=1 cargo test -p arkiv-git-cli --test rabdi_gate live_rabdi_gate -- --ignored --nocapture   # live 10 MiB gate, about 0.44 tGLM
```

Try it locally without a chain: `export ARKIV_GIT_BACKEND=mem:/tmp/arkiv.log`, then `arkiv-git keys init`, `arkiv-git init --name demo`, and `git push` to the printed `arkiv::` URL.

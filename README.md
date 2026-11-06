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
- On Rabdi only a small live smoke test has run (a 3 KB repo); the full 100 MB Rabdi run waits on testnet funding (open decision 2).

## Development

```sh
nix develop                                   # Rust 1.97 toolchain, git, cargo-audit
cargo test --workspace --all-features         # unit, property, fake-Arkiv and git E2E tests
cargo test --release -p arkiv-git-sync --all-features --test gate -- --ignored --nocapture   # 100 MB gate on the fake
ARKIV_GIT_RABDI=1 cargo test -p arkiv-git-cli --test rabdi_smoke -- --ignored              # live smoke, needs the funded throwaway key
```

Try it locally without a chain: `export ARKIV_GIT_BACKEND=mem:/tmp/arkiv.log`, then `arkiv-git keys init`, `arkiv-git init --name demo`, and `git push` to the printed `arkiv::` URL.

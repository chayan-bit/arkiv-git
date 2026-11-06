# arkiv-git

## ETHGlobal Mumbai 2026

arkiv-git is decentralized git hosting on Arkiv: a stock `git push arkiv::<rid>` and `git clone arkiv::<rid>` work end to end on the Rabdi testnet with no server of ours in between.
It is a redesign of the Radicle (heartwood) protocol in which Arkiv replaces gossip and seed nodes, and every client verifies every signature and fails closed on rollback, expiry or missing data.
A live 10 MiB push and clone have passed on Rabdi through the real binaries (numbers below), and the 100 MB gate passes against an in-memory Arkiv.

Submitted to:

- Arkiv: "Build on Arkiv (from scratch)".
- Ethereum Foundation: "Best CROPS App" (see the `CROPS` section below).
- ETHGlobal finalist judging.

Demo video: <!-- FILL: demo video URL --> TODO-DEMO-VIDEO-URL (placeholder, replaced before submission).

Notes for judges, in `arkiv/`:

- [`arkiv/schema.md`](arkiv/schema.md): every entity kind, its indexed attributes, owner, creation flags, lifetime and queries.
- [`arkiv/friction.md`](arkiv/friction.md): reproducible feedback on building against Arkiv, with a list of what would make it easier.
- [`arkiv/reach.md`](arkiv/reach.md): who this is for.
- [`arkiv/why-arkiv.md`](arkiv/why-arkiv.md): what users would lose if the backend were Postgres.
- [`arkiv/how-you-use-arkiv.md`](arkiv/how-you-use-arkiv.md): how filters, ownership, expiry and permissionless extension change what users see.

## What it is

Decentralized git hosting on Arkiv: a redesign of the Radicle (heartwood) protocol where Arkiv replaces gossip and seed nodes.
Arkiv is trusted for ordering, availability and timestamps only; clients verify every signature.

Plan of record: the "Arkiv Git: decentralized code hosting plan" doc (private Claude Doc).

## Phases in this repo

- P0 spike: measure Arkiv limits, confirm OpenMLS X-Wing support, decide fork vs depend on heartwood. Findings in `docs/p0/`.
- P1 core: `git-remote-arkiv`, identity documents, signed refs, ref anchoring, search.
- P2 private: encrypted repos with MLS groups (X-Wing), roles, rotation, blinded annotations.

- P3 collaborative objects and repo programs: on feature branches, not merged to `main` (see "In progress" below).

Testnet only. Never use real funds or mainnet keys.

## Status on main (2026-11-07)

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

### In progress, not merged to main

As of this commit none of the following is merged; each lives on its own branch and is described by what its commits and docs say, not by a release.

- `feat/cob-ops`: signed collaborative-object ops (issues, patches, reviews, attestations) as a `cob` entity kind, with `arkiv-git op post` and `arkiv-git op list`; core, sync and CLI are implemented there, and the op format is in `docs/p3/cob-ops.md` on that branch.
  Keepalive does not extend `cob` entities yet and there is no completeness check for them.
- `feat/repo-programs` and `feat/programs-wired`: a deterministic Rhai repo-program engine pinned by hash in the identity document, with four demo templates (branch protection, CI-attested, bounty, sealed patch) and, on the second branch, enforcement of the pinned program on fetch.
  The programs compute what is owed; settlement is not part of it.
- `feat/timelock`: timelock seal and open (`arkiv-git timelock seal`, `arkiv-git timelock open`) using drand quicknet.
- `feat/private-repos`: private repos with MLS X-Wing groups and sealed entities that store only ciphertext; work in progress.
- `feat/escrow`: early work on top of the collaborative-object branch.
- `feat/zk-sealed`: a design note only, no implementation commits.

## How to try it

Everything below needs the repo's dev shell, which provides the pinned Rust 1.97 toolchain, git, cargo-audit and Node 22.

```sh
nix develop
cargo build --release
export PATH="$PWD/target/release:$PATH"   # git finds git-remote-arkiv through PATH
```

### Locally, with no chain

The `mem:` backend is an Arkiv fake that persists to a file.
The end-to-end test drives the real `arkiv-git` and `git-remote-arkiv` binaries against it, with two users, `git push`, `git clone` and `git fsck --full --strict`:

```sh
cargo test -p arkiv-git-cli --test e2e
```

To do the same by hand, point the CLI at a log file:

```sh
export ARKIV_GIT_BACKEND=mem:/tmp/arkiv.log
export ARKIV_GIT_HOME=/tmp/arkiv-git-demo   # optional, keeps the seed out of ~/.arkiv-git
arkiv-git keys init                         # prints your nid and wallet address
arkiv-git init --name demo                  # prints rid and url
git push arkiv::<rid> main                  # from any git repo with a commit on main
git clone arkiv::<rid> demo-clone
```

Known gap: the `mem:` wallet starts with a zero balance and the CLI has no command to fund it, so `arkiv-git init` stops with `E_INSUFFICIENT_FUNDS` unless the wallet was funded through the library.
The e2e test does that funding; for a hand run, fund the printed wallet address the way `fund()` in `crates/arkiv-git-cli/tests/e2e.rs` does.

### On the Rabdi testnet

Testnet funds only.
Never use real funds or mainnet keys.

1. Get an Arkiv access key at hub.arkiv.network (wallet sign-in and a captcha, so a person has to do it).
   Anonymous reads run out after about 1,000 queries.
   `export ARKIV_ACCESS_KEY=<your key>` and never commit it.
2. `arkiv-git keys init` creates a seed under `$ARKIV_GIT_HOME` (default `~/.arkiv-git`) and prints your nid and the wallet address.
3. `arkiv-git fund --bytes 10485760` prints the wallet, its balance and an estimate of what an init plus a push of that size needs.
   It makes no transfer: send tGLM to the wallet from the faucet at hub.arkiv.network/faucet.
4. `arkiv-git init --name demo --lang rust --license MIT` creates the repo with you as its single delegate and prints `rid:` and `url: arkiv::<rid>`.
5. In any git repo: `git remote add arkiv arkiv::<rid>` and `git push arkiv main`.
6. Anywhere else: `git clone arkiv::<rid> demo-clone`.
7. `arkiv-git status arkiv::<rid>` shows each peer's head and the minimum expiry in days, and `arkiv-git keepalive arkiv::<rid> --dry-run` shows what it would extend.

`ARKIV_GIT_DATA_LIFETIME=1d` shortens the data lifetime for demos; the live gate used 1 day.
The default is 30 days for data and 182.5 days for the control plane.

## Wallets and a query you can run

<!-- FILL: demo wallet addresses and rid after the final live demo push -->

Anyone can read a repo's anchor chain straight from Arkiv, without arkiv-git.
The anchor kind fixes each peer's refs head; this query lists the anchors of one repo in sequence order (shape from `docs/p1/design.md` section 3.6):

```text
ag.kind = str('anchor') AND ag.rid = bytes32(0x<RID_HEX: 64 hex characters of the rid, FILL after the demo push>) ORDER BY ag.seq:u64 ASC
```

```sh
curl -s -X POST https://rpc.rabdi.db-chain.testnet.arkiv.network -H 'content-type: application/json' --data-binary @- <<EOF
{"jsonrpc":"2.0","id":1,"method":"arkiv_query","params":["ag.kind = str('anchor') AND ag.rid = bytes32(0x<RID_HEX>) ORDER BY ag.seq:u64 ASC",{"limit":10}]}
EOF
```

The rid in hex is the revision hash that `arkiv-git id show arkiv::<rid>` prints for revision 0.
Replace `<RID_HEX>` before running; with an all-zero rid the query is valid and returns no rows.
Add `"select":{"count":true}` to the options to get only a count.
Each row is a hint, not proof: a client checks the signature, the creator address and the chain before trusting it, which `git clone` does for you.

## Development

```sh
nix develop                                   # Rust 1.97 toolchain, git, cargo-audit
cargo test --workspace --all-features         # unit, property, fake-Arkiv and git E2E tests
cargo test --release -p arkiv-git-sync --all-features --test gate -- --ignored --nocapture   # 100 MB gate on the fake
ARKIV_GIT_RABDI=1 cargo test -p arkiv-git-cli --test rabdi_smoke -- --ignored              # live smoke, needs the funded throwaway key
ARKIV_GIT_RABDI=1 cargo test -p arkiv-git-cli --test rabdi_gate live_rabdi_gate -- --ignored --nocapture   # live 10 MiB gate, about 0.44 tGLM
```

Try it locally without a chain: `export ARKIV_GIT_BACKEND=mem:/tmp/arkiv.log`, then `arkiv-git keys init`, `arkiv-git init --name demo`, and `git push` to the printed `arkiv::` URL.

## How AI was used

This project was built with Claude Code (Anthropic) as a coding agent, orchestrating parallel worker agents under the author's direction.
The author made the architecture and security decisions; the agents did the research, implementation and measurement within them.
The decision log, from `docs/p0/README.md` ("Decided by the user"):

- 2026-11-06: ML-DSA signatures from day one (identities, signed refs, anchors, COBs, MLS credentials).
- 2026-11-06: hybrid composite ML-DSA-65 + Ed25519 for our own signatures (draft-ietf-lamps-pq-composite-sigs); a signature is valid only if both halves verify.
  MLS credentials stay pure ML-DSA because no hybrid MLS suite exists.

`docs/p0/README.md` also lists the recommendations an agent took and flagged as reversible, and the open decisions still waiting for the author.
Every change went through tests, clippy and CI: CI runs `cargo fmt --check`, `cargo clippy -D warnings`, the workspace tests, `cargo audit`, and a check that no tracked secret files or em dashes are in the tree.
Live numbers in this README were measured on Rabdi through the real binaries, and each is labelled as measured, derived or read from source in the docs it comes from.

## CROPS

Censorship resistance, open source, privacy and security, as this project stands today.

- **C (censorship resistance):** we run no server, the data lives on Arkiv, and clients verify everything they read.
  Arkiv runs a single sequencer that can refuse writes, which is censorship; that is outside the P1 trust model, which trusts Arkiv for ordering and availability only.
  A refused push fails visibly (it ends with an error such as `E_UPLOAD_INCOMPLETE` and nothing is anchored) rather than silently.
- **O (open source):** MIT OR Apache-2.0, and anyone can build and run it from the repo alone with `nix develop`.
- **P (privacy):** public repos are public by design.
  Private repos are in progress and store only ciphertext with keyed-hash ids.
  The RPC provider sees which entities a client queries, so use your own Arkiv node or a privacy RPC.
- **S (security):** composite ML-DSA-65 + Ed25519 signatures (both halves must verify), and the seed is stored locally with mode 600.
  Trust assumptions: Arkiv is trusted for ordering and availability only, and nothing else; the client checks every signature, hash and chain link, and keeps its own high-water marks so it refuses to go backwards.
  The EVM keys that own entities are secp256k1 and not post-quantum, which is why the signatures that matter are separate.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

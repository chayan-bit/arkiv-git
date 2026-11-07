# Live Rabdi: multi-peer collaboration, quorum, rollback, programs and escrow

Commands below were recorded before the rename to Charter; `arkiv-git` is now `chart`.

Date: 2026-11-07.
Network: Rabdi testnet, chain id 7794095, RPC https://rpc.rabdi.db-chain.testnet.arkiv.network, explorer https://rabdi.explorer.arkiv.network.
Code: `crates/arkiv-git-cli/tests/rabdi_collab.rs` and `crates/arkiv-git-cli/tests/common/collab.rs` on `test/live-collab`.
Every live entity was written with a 1 day lifetime (43,200 blocks), so the repos below expire around block 353,000.

## How it was run

Each flow is one ignored Rust test that drives the real `arkiv-git` and `git-remote-arkiv` binaries as separate users (alice, bob, carol, dave, erin, an escrow agent and an auditor), each with its own `$HOME` and `$ARKIV_GIT_HOME`.
The tests run only with `ARKIV_GIT_RABDI=1` and `--ignored`, so `cargo test --workspace` never touches the network.
A bank wallet (`0x32cE8132B0edE30Eda5fAbec73fb41E2C340e868`) was filled once with 0.05 tGLM from the throwaway key under the shared `lockf` lock, and a ledger file caps the bank at that amount.
Peers are lent a small float per write and swept back after it; a push refused with `E_INSUFFICIENT_FUNDS` is retried once with the amount the error names.
`sweep_bank` collects every keyed home, escrow identity and payout key under the run root and returns the bank's balance to the throwaway key.
The access key and every seed stayed in local files and were never printed.

```sh
lockf <lock> cargo test -p arkiv-git-cli --test rabdi_collab fund_bank -- --ignored
ARKIV_GIT_RABDI=1 ARKIV_GIT_COLLAB_ROOT=<dir> \
  cargo test -p arkiv-git-cli --release --test rabdi_collab live_ -- --ignored --nocapture --test-threads 1
ARKIV_GIT_RABDI=1 ARKIV_GIT_COLLAB_ROOT=<dir> cargo test -p arkiv-git-cli --test rabdi_collab sweep_bank -- --ignored
```

## Results

| Feature | Result | Evidence |
|---|---|---|
| 1. bob (non-delegate) clones with no keys | PASS | repo `arkiv::zfhskZGzf6n72AJkfAFqr591Lcs2fBXwmfQGjk7j4RiG`, alice control [0x6C8A...8EE2](https://rabdi.explorer.arkiv.network/address/0x6C8AF8e24b9196256bbe635B62060ac815928EE2); bob's clone HEAD is c0 `ae81a21` |
| 1. Issues and comments between peers | PASS | bob's issue `7e78154f...6e7dbb`, alice's comment on it, alice's close after the merge |
| 1. bob pushes to his namespace and posts a patch op | PASS | bob's anchor seq 0 from control `0xC4D8e9a6EBa60d20d981e7CaC28286EB9d7e8d12`; patch `4ffb4d40...7eebd6` at cb `2374bf8` |
| 1. alice sees it with `op list`, reviews and approves | PASS | `op list --json` shows the patch authored by bob; review `d1c74a4e...` plus approve op |
| 1. alice fetches `arkiv::<rid>/<bob nid>`, merges, pushes | FIXED | `FETCH_HEAD 2374bf8`; merge `0c81929`; failed with `couldn't find remote ref refs/heads/fix` until the peer-url fix (see Bugs) |
| 1. carol clones fresh, gets the merge, fsck clean, code builds | PASS | carol HEAD `0c81929` with bob's `2374bf8` as ancestor, `git fsck` clean, checkout compiles and prints hello world |
| 1. Keyless `op list` verifies authors | PASS | carol (no keys) verifies 7 ops with the right authors |
| 2. Second delegate via `id propose`, threshold 2 | PASS | repo `arkiv::z5P6MbWuHid8bNJQBf2RTz4snULdp5zpC2JbA2W2xigJe`, alice control [0x026E...fB38](https://rabdi.explorer.arkiv.network/address/0x026E1492E3322C0C826464B984F79ae43Fb0fB38), bob control [0x13b1...8E1b](https://rabdi.explorer.arkiv.network/address/0x13b1D89e4023C2Ea8636ab0944B553A82A748E1b); revision 1 effective on alice alone (maj of the old set) |
| 2. One of two delegates at a commit gives no quorum | PASS | main omitted with `warning: no quorum for refs/heads/main` |
| 2. Both delegates at the same OID give quorum | PASS | both at c0 `392f35e`, then both at c1 `3374d66`; fresh clone and dave's pull follow |
| 2. A single delegate's push does not move the canonical head | PASS | dave stays at `392f35e`; erin's fresh clone gets no main while the delegates disagree |
| 2. Identity change needs both delegates | PASS | revision `b640292b...6b3272` pending on alice alone, effective after bob's `id accept` |
| 3a. Revert | PASS | revert `389ab08` pushed; carol pulled it and dave cloned it |
| 3b. Plain non-ff push without a program | PASS | git refuses it before the helper sends anything |
| 3b. Forced rewind without a program | PASS | a signed rewind is a new anchor seq and is accepted; carol's fetch and erin's fresh clone follow main back to `ae81a21` |
| 3b. High-water mark, withheld anchor | PASS | a view hiding alice's anchor seq 4 (`0x03323725...a334f70f91`) gives `E_STALE_HEAD` |
| 3b. Forced rewind with branch-protection | PASS | `E_PROGRAM_REJECTED: not a fast-forward of the accepted head`, even with an approved patch at the old commit |
| 3c. Fresh clone builds every earlier commit | PASS | c0, cb, c1, m and r all checked out and compiled |
| 4. Pin branch-protection | PASS | repo `arkiv::zC8649QSBw1wB7q4kwLcFLiGFCBPTcQSLZwGj82dfRyre`, control [0x7d00...b2a4](https://rabdi.explorer.arkiv.network/address/0x7d009B45e905893FD4D7dbFaef01D5D37A61b2a4); program pinned by hash in identity revision 1 |
| 4. Direct push by the delegate | PASS | `E_PROGRAM_REJECTED: main needs a patch at ... with 1 delegate approvals`, refused locally, nothing anchored |
| 4. bob cannot bypass | PASS | bob's own main `e1f96c8` (anchor seq 0 from `0xe32aB8AE6177B382fAE6Ba624466426D62D947bE`) is not an event; his approval of his own patch does not count |
| 4. Approved patch accepted | PASS | patch `415000ce...73e17d` at `e1f96c8` approved by alice, push accepted |
| 4. Modified client anchoring a rejected head | PASS | alice's anchor seq 2 (`0x62bf137a...b37970c6`) rewinds to `cbffdea` with the local check skipped; the fold rejects it and a fresh clone stays at `e1f96c8` |
| 5. Pin bounty.rhai, appoint an agent with its own home | PASS | repo `arkiv::zDjiBvA7dGRLDnhvx5U6thCKe3hmKLE5nQUEzwzdwfP4X`, control [0x9F38...C059](https://rabdi.explorer.arkiv.network/address/0x9F38eb6506C5e3Fb2F4C8C54b941a24Fb146C059); escrow `zHvtKLDhswzgNqytdGVs8u6jq8QcAJAUR6Jv7yZmgwezC` at [0x9B85...2C49](https://rabdi.explorer.arkiv.network/address/0x9B85728a009dA059dfCDAe26Dd5c810186882C49) |
| 5. Merge makes the program owe the bounty | PASS | issue `03575d15...ca85217` with bounty 0.002 tGLM; bob's patch `ecc8a5d1...f4cb8b8` merged; `program status` shows the `bounty:<issue>` obligation |
| 5. Negative case: no pay while unfunded | PASS | agent round before the deposit: `skipped bounty:03575d15...: ... underfunded` |
| 5. Deposit 0.002 tGLM verified by tx | PASS | [0x7a1f50b6...e24018c6e](https://rabdi.explorer.arkiv.network/tx/0x7a1f50b67e42e20065e118d748e57886e09f76ba4bbd84344326db2e24018c6e), 2,000,000,000,000,000 wei alice to escrow |
| 5. `escrow run` pays bob's EVM address | PASS | [0x017d4957...df8f3364898](https://rabdi.explorer.arkiv.network/tx/0x017d4957756577cb92b520e1b78172ff980cf3a497567500ef9e95f8d3364898); bob's control address [0xFB40...8755](https://rabdi.explorer.arkiv.network/address/0xFB409cFdd556F403778A89E349926D44877c8755) gained exactly 2,000,000,000,000,000 wei |
| 5. No double pay | PASS | a second agent round pays nothing |
| 5. `escrow audit` clean | PASS | `credited 2000000000000000 wei in 1 deposit(s) | settled: 1 op(s) | balance 729000000000000 wei, owes 0`, exit 0 |
| 5. Negative case: audit flags an unbooked transfer | PASS | after a payout-key transfer with no settlement op: `FLAG unauthorized payout: 1 tx(s) from 0x9B85... have no settlement op`, exit 1 |
| Push funding estimate | FIXED | a 300 byte push needed 0.0275 tGLM up front (see Bugs) |

A first escrow run on repo `arkiv::zFt2WrgbAzzHPYZu97C19fQ1FcymdNP9LMwM8nkzLToig` verified a deposit ([0xc9da14c4...4b3f](https://rabdi.explorer.arkiv.network/tx/0xc9da14c46265769836eecec5c801d9764156dcbd64004f1210768fd1b3e60b3f)), then stopped on budget; the run above is the one that completed.

## Bugs found and fixed

1. A peer url fetch of a non-delegate listed nothing.
   `git fetch arkiv::<rid>/<bob nid>` failed with `couldn't find remote ref`, because `list` only synced delegates and extra peers, never the peer named in the url.
   Fix 138755f `fix(cli): verify the named peer when listing a peer url` adds the e2e test `a_contributor_push_is_fetched_through_its_peer_url` and a line in `docs/p1/design.md`.
   main carried the same helper change as 673a588, so the merge took main's version; the test and doc line stay.
2. The push funding check priced every chunk as a full 64 KiB chunk.
   A tiny push needed about 0.0275 tGLM up front while spending about 0.001.
   Fix 9c17dc9 `fix(sync): price a push's chunks at their real sizes` with a unit test; main carried the same fix as bee5217, so the merge took main's version.
   After the fix a tiny push needs about 0.0065 tGLM up front (0.0105 on a repo's first write).

## Spend

| Run | Net spend (tGLM) |
|---|---|
| Collab run 3 (features 1 and 3) | 0.01083 |
| Quorum (feature 2) | 0.00587 |
| Program (features 4 and 3b with program) | 0.00823 |
| Program resume (modified client) | 0.00061 |
| Escrow run 1 (stopped on budget) | 0.00629 |
| Escrow run 2 plus resume stages (feature 5) | 0.01011 |
| Earlier aborted collab runs 1 and 2 (bugs above) | 0.00728 |
| Recovered by `collect_leftovers` (stranded floats and bob's 0.002 bounty) | -0.00409 |

Per-run figures are bank balance deltas, so they include floats left on peer addresses that the leftover collection later returned to the bank.
Total sent from the throwaway: 0.05 tGLM, in one funding transfer to the bank.
Returned to the throwaway: 0.004879878 tGLM in [0xd36f904a...dc97bacc42](https://rabdi.explorer.arkiv.network/tx/0xd36f904a77847ef76dd0aae1106ae691959e5795c86f1b85632962dc97bacc42) at block 310,951.
Net spend: about 0.0451 tGLM, all of it gas and storage.
Dust left: 0.000039 tGLM in the bank, about 0.000039 tGLM at the escrow address, and sweep gas reserves of a few hundred microGLM spread over the swept addresses.

## Gaps

- Refund after expiry and the unmerged-patch refund were not run live; the budget ran out, and both stay covered only by the MemArkiv tests.
- In the escrow flow bob's commit reached alice out of band; the on-Arkiv contributor path (namespace push, peer url fetch) is proven in feature 1.
- The escrow run 1 repo's audit now flags, because the run's sweep moved funds from its payout key without a settlement op; this is the audit working as designed, not a bug.
- `arkiv-git fund --bytes` still prices a full 16 tx window of full chunks (about 0.0997 tGLM estimate); left unchanged as outside scope.
- With a 1 day lifetime every push prints `an entity this repo needs expires in 0 days`; the day count rounds down.
- During a threshold 2 disagreement a fresh clone gets an empty repo with a warning; this is the documented rule (design 3.8), but it is a sharp edge for users.
- The repos expire with their 1 day lifetime unless someone runs `arkiv-git keepalive`.

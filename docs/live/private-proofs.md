# Live Rabdi: private repos, programs on private repos, proofs and timelock

Commands below were recorded before the rename to Charter; `arkiv-git` is now `chart`.

Date: 2026-11-07 (UTC clock 2026-11-06 22:xx).
Network: Rabdi testnet, chain id 7794095, RPC https://rpc.rabdi.db-chain.testnet.arkiv.network, explorer https://rabdi.explorer.arkiv.network.
Code: `crates/arkiv-git-cli/tests/rabdi_private.rs` on `test/live-private`.
Every live entity was written with a 1 day lifetime (43,200 blocks), so the entities below expire around block 352,600.

## How it was run

Each flow is one Rust function that drives the real `arkiv-git` and `git-remote-arkiv` binaries as separate users, each with its own `$HOME` and `$ARKIV_GIT_HOME`.
The same function runs on the file-persisted MemArkiv as a normal test (`mem_*`, part of `cargo test --workspace`) and on Rabdi as an ignored test (`live_*`, needs `ARKIV_GIT_RABDI=1`).
Live peers are funded from a bank wallet (`0xCAAc3892D1df8f05Dc5ff613baeD0f7A7148F313`), which `bank_fund` filled once with 0.05 tGLM from the throwaway key under the shared lock.
Every flow sweeps its peers' wallet, control and bulk addresses back to the bank, and `bank_sweep` returned the rest to the throwaway key.
The access key and every seed stayed in local files and were never printed.

```sh
lockf <lock> cargo test -p arkiv-git-cli --test rabdi_private bank_fund -- --ignored
ARKIV_GIT_RABDI=1 ARKIV_GIT_LIVE_BANK=<bank home> ARKIV_GIT_LIVE_ROOT=<dir> \
  cargo test -p arkiv-git-cli --test rabdi_private live_ -- --ignored --nocapture --test-threads 1
```

## Results

| Feature | Result | Evidence |
|---|---|---|
| Private repo create and push (MLS suite 0x0052) | PASS | rid `z423dk8C3GyfUuBtMXAjTCJ5D9VExtDfeQNsqk19UcMnA` (hex `2cd79e8a...fb75b3`); alice control [0x7E69...87A8](https://rabdi.explorer.arkiv.network/address/0x7E69995eB0525642D123b03D7f6ACF9dEC1887A8), first entities at block 309,161 |
| Key packages, add bob (write) and carol (read) | PASS | `member list` shows epoch 2 with Admin, Write, Read; bob kp entity `a003106b...31a1` from wallet 0x23b8...68D3 at block 309,192, carol kp `aeddb1bd...b121` at 309,195 |
| bob joins from the Welcome and clones | PASS | bob's clone HEAD equals alice's v0 and README matches |
| carol clones (read) | PASS | carol's clone HEAD equals v0 |
| carol's push refused | PASS | `E_ROLE_DENIED: Write role needed in this private repo, you have Some(Read)`, nothing sent |
| carol's `member add` refused | PASS | `E_ROLE_DENIED: Admin role needed ...` |
| dave (non-member) clone, `op list`, `program status` | PASS | `E_NOT_MEMBER` for all three; `search --name` does not find the repo |
| Raw Arkiv view is unlinkable | PASS | 38 entities created by the three members' wallet, control and bulk addresses; none carries the rid (bytes32, hex or z-text), the repo name or the README markers in any attribute or body; `ag.rid = <rid>` and `ag.name = <name>` queries return 0 rows |
| Branch protection pinned on the private repo | PASS | `program pin` through the sealed view, program `rhai/1` |
| Direct push by the delegate refused | PASS | `E_PROGRAM_REJECTED: main needs a patch at 9cd02ff... with 1 delegate approvals`, refused before any tx |
| bob's patch plus alice's approval accepted | PASS | bob pushed to his namespace (bulk 0x1b18...6a85b, blocks 309,250 to 309,261), posted the patch op, alice approved, fetched `arkiv::<rid>/<bob nid>` and pushed |
| Member clone gets the program's head | PASS | carol's fresh clone HEAD is bob's commit and `program status` prints `head: refs/heads/main <c1>` |
| Remove bob, new epoch | PASS | `member list` shows epoch 3 with Admin and Read only |
| bob cannot read content pushed after removal | PASS | alice's v2 went in through patch and approval; bob's fresh clone succeeds but stops at `7307fb1` (c1), README has no v2 marker, `op list` lacks the v2 patch op and still shows the ops he could read |
| carol still reads after removal | PASS | `git pull` in carol's clone reaches v2 with the v2 README |
| Proof blob ops (`op post` of receipt chunks) | FIXED | `op post` refused every 32,000-byte chunk with `E_BAD_FORMAT: body of 64,120 B exceeds the 62,119 B cap`; fixed by `BLOB_CHUNK = 30,000` (landed on main as e2dba59 in parallel; regression test dbfd900) |
| Fetch of a non-delegate peer URL | FIXED | `git fetch arkiv::<rid>/<nid>` for bob failed with "couldn't find remote ref main"; fixed by syncing the named peer (main 673a588 in parallel; regression test e589d9e) |
| Up-front funding check for small pushes | FIXED | a few-byte private push demanded 0.029 tGLM up front and failed `E_INSUFFICIENT_FUNDS` on a 0.015 wallet, because every chunk was priced and reserved as a full 64 KiB chunk; fixed in bee5217, after which the same push passed from a 0.016 wallet |
| Sealed-patch proof accepted on a fresh fold (public repo) | PASS | rid `z82EghwWCiLAGwsgd7W7grLwN9fXNgYV2Yj1wP9dus8HN`; 8 receipt blob ops plus one tampered tail blob (last three: `0a6ef5e4...`, `aaedd909...`, `86b90a41...`); bob's `program status` shows sealed object `3d547445...bdd` with `proven: true` and the demo issue, plus its `release_key` obligation |
| Missing proof rejected | PASS | sealed object `d251495e...459f` absent from state |
| Tampered receipt (one byte) rejected | PASS | sealed object `2c37bcb6...aa66` absent from state |
| Valid proof for a different spec rejected | PASS | sealed object `cf492570...0a27` (issue with another `spec_hash`) absent from state |
| Fresh clone of the proof repo | PASS | `git clone` in bob's home gets `809d23e feat: base` |
| Timelock seal, early open, open after the round (drand quicknet, live) | PASS | sealed until round 33,733,964; open before it gave `E_TOO_EARLY: not before round 33733964`, and a fresh home opened it after the round with the same plaintext hash |
| Timelock blob carried as an `xyz.arkiv.timelock` op, then sealed-patch reveal | NOT LIVE | budget ran out at the timelock commit op (`E_INSUFFICIENT_FUNDS`); the whole path (op post, reader fetches the op, `E_TOO_EARLY`) passes on MemArkiv, and the reveal after the round is covered only by the live test code, not by a completed live run |

## Timings (private flow, run 3, 494 s end to end)

- `init --private`: 13 s; first push of v0: 56 s; two key packages: 12 s; two `member add`: 27 s.
- bob's first clone (join from Welcome): 7 s; carol's clone: 6 s; refusals for carol and dave: under 2 s each, with no tx.
- `program pin`: 26 s; bob's push to his namespace plus patch op: 71 s; review and approve: 16 s; alice's accepted push: 55 s.
- `member remove`: 12 s; v2 patch, approval and push: 83 s.
- No rate-limit waits with the access key in any run.

The proof flow spent about 4 minutes posting ops; each 30,000-byte blob op took about 4 to 6 s.

## Spend

- Left the throwaway key: 0.05 tGLM in one transfer, the only send from it.
- Returned to the throwaway key: 0.000479 tGLM; 0.000039 tGLM stays on the bank wallet as the sweep's own gas reserve.
- Private flow: 0.00124 (run 1, stopped by the funding check), 0.00742 (run 2, stopped by bob's funding), 0.01349 (run 3, PASS).
- Proof flow: 0.01938 (ran out of funds after 6 of 8 blob ops), then 0.0079 to finish the blobs and sealed commits from the same repo.
- One 30,000-byte blob op costs about 0.0027 tGLM, because the receipt travels as hex, so attaching one 224 KB receipt costs about 0.02 tGLM.

## Honest gaps

- The proof flow ran live on a public repo to fit the budget; the same flow on a private repo (sealed blob ops near the 64 KiB bucket) passes only on MemArkiv.
- The live proof flow was finished by a scratch driver that repeats the remaining steps of `zk_flow` against the same repo after a top-up (`bank_pay`), not by one uninterrupted run of `live_zk_sealed_patch_proofs_and_timelock`.
- The timelock op on Arkiv and the reveal after the round were not run live (see the table); the live timelock check exercised drand only.
- Roles are enforced client-side, as the docs say: carol's refused push sends nothing, but a modified client could still post anchors that readers do not yet ignore (docs/p2 TODO).
- A removed member keeps the epochs he held, so bob still reads everything sealed before his removal, as documented.
- `ag.kind`, `ag.nid`, `ag.seq` and entity sizes stay plaintext (docs/p2 TODO), so the raw view shows structure, though not which repo.
- With 1 day lifetimes `program status` warns "an entity this repo needs expires in 0 days", which is the day count rounding down, not a fault.

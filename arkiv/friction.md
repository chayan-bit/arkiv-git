# Building arkiv-git on Arkiv: friction log

This is feedback from building a real client (Rust, git remote helper, 100 MiB packs) on the Rabdi testnet, chain id 7794095.
Every item says what we expected, what happened, and how to reproduce it.
The items marked "re-run 2026-11-07" were reproduced live against the public RPC on the day this file was written (head block about 306,335).
The others were measured on 2026-11-06 during the P0 spike or on 2026-11-07 during the live 10 MiB gate, and are described in `docs/p0/arkiv-limits.md` and the README; they were not re-run for this file.
Error messages are quoted with the node's long dash replaced by a plain hyphen.

Arkiv gave us the thing we needed, a shared, ordered, queryable store with per-entity expiry, and most of the rough edges below have a workaround that is now in the code.
We list them in order of how much design work each one cost us.

## 1. The published Rust SDK does not work on Rabdi

- Expected: `cargo add arkiv-sdk` gives a working Rust client for the current network.
- Actual: `arkiv-sdk` 0.4.1 on crates.io (published 2025-09-30) targets the retired GolemDB contract ABI.
  Its entity registry address is `0x00000000000000000000000000000061726B6976` and it extends entities by a relative `number_of_blocks`, while Rabdi's registry is `0x4400000000000000000000000000000000000044` with `execute(Operation[])` and absolute `expiresAt`.
  The git `main` of the crate is 0.5.0 (last commit 2026-01-31) and is licensed GPL-3.0-or-later.
  The Python `arkiv-sdk` (1.0.0b2 and a b3 dev build) has the same old address.
- Evidence level: SOURCE-VERIFIED by reading `src/contract.rs` and `src/tx/ops/extend.rs`; we did not run the Rust or Python SDKs against Rabdi, because the address and ABI mismatch is visible in the source.
  The TypeScript SDK (`@arkiv-network/sdk` 1.0.1) uses the current address and does work.
- Reproduce: `cargo add arkiv-sdk`, open `src/contract.rs` in the downloaded crate, and compare the registry address with the one in the TypeScript SDK's `src/consts.ts`.
- What we did: a Rust workspace that depends on `arkiv-bindings` from the Arkiv node repository (MIT, not on crates.io) as a git dependency pinned by commit, plus alloy, and our own raw JSON-RPC client (`crates/arkiv-git-store/src/rpc.rs`).
  That works, but it means a Rust developer has to know to ignore the crate with the obvious name.

## 2. `eth_getLogs` over block ranges does not work (re-run 2026-11-07)

- Expected: logs for a range of blocks, which is how an indexer or auditor would read entity creation order.
- Actual: a 1,000-block range ending at the head returns JSON-RPC error `-32603` with message `golemdb segment_range_of(transactions): NotImplemented`.
  During the P0 spike, single recent blocks and some windows under about 500 blocks near the head worked, and some single older blocks failed with the same error.
  `eth_getBlockReceipts` is answered with "the method eth_getBlockReceipts does not exist/is not available".
- Reproduce:

```sh
RPC=https://rpc.rabdi.db-chain.testnet.arkiv.network
HEAD=$(( $(curl -s -X POST $RPC -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' | jq -r .result) ))
curl -s -X POST $RPC -H 'content-type: application/json' -d \
  "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getLogs\",\"params\":[{\"fromBlock\":\"$(printf '0x%x' $((HEAD-1000)))\",\"toBlock\":\"$(printf '0x%x' $HEAD)\"}]}"
```

- Impact: no rule in arkiv-git may depend on scanning logs.
  The order of two entities (which of two anchors for the same sequence number came first) is taken from query result order, which Arkiv documents as creation order: by transaction (block, then index), then by entity key inside one transaction.
- Workaround: ordering from query results (`docs/p1/design.md` section 3.2).

## 3. No historical reads, no `NOT`, no `!=` (re-run 2026-11-07)

- Expected: `atBlock` to read old state, and a way to exclude a value.
- Actual:
  - A query with an `atBlock` of head minus 2 is refused with error `-32006`, "block ... is unavailable: state for this block is not retained".
    The node runs `head-only`.
  - `NOT (attr = value)` parses and then fails the whole query with `-32603`, `count: store: NotImplemented` (we ran it as a count query).
  - `attr != value` is rejected at parse time with `-32002`: "!= is not part of the query language - write NOT (attr = value) for the complement".
    The suggested replacement is the operator that fails, so a developer who follows the error message ends up at the second error.
- Reproduce:

```sh
q() { curl -s -X POST $RPC -H 'content-type: application/json' -d "$1"; echo; }
q '{"jsonrpc":"2.0","id":1,"method":"arkiv_query","params":["NOT ($owner = addr(0x0000000000000000000000000000000000000001))",{"select":{"count":true}}]}'
q '{"jsonrpc":"2.0","id":1,"method":"arkiv_query","params":["$owner != addr(0x0000000000000000000000000000000000000001)",{"select":{"count":true}}]}'
```

- Impact: version history has to be modelled as append-only entities with sequence numbers and hash links, and every "everything except X" filter has to run client side.
  This shaped the whole design: anchors are a signed chain, and rollback protection is a local high-water mark plus `entityNonce` (section 12) instead of "read the state at block N".

## 4. Results are capped at 100 rows, with no cursors in the Rabdi SDK (re-run 2026-11-07)

- Expected: pagination that stays correct while other people write.
- Actual: `limit` above 100 is refused ("limit 101 exceeds the node maximum of 100").
  The Rabdi SDK exposes `limit` and `offset` but no cursors, and offset pages can shift when entities are created or expire between calls (not measured, but it follows from how offset paging works).
- Impact: we page by key where an ordering attribute exists (`ag.seq`, `ag.i`, request `attr >= last`, drop keys already seen), and use offset paging with a count check only for the identity query.
  A 100 MiB clone needs about 110 queries.
- Reproduce: `q '{"jsonrpc":"2.0","id":1,"method":"arkiv_query","params":["*",{"limit":101}]}'`.

## 5. `$createdAt` is returned but cannot be queried or sorted on (re-run 2026-11-07)

- Expected: "pushed in the last 7 days, newest first" is a one-line query on creation time.
- Actual: `$createdAt > u64(1)` fails with `-32002`, "$createdAt is not queryable - it is returned by projections only".
  `$updatedAt` behaves the same, and `ORDER BY` accepts one attribute with an explicit type, not a system attribute.
- Workaround: every anchor carries its own `ag.at` (`u64`, indexed), the head block number the writer saw when signing.
  Clients enforce `ag.at <= $createdAt`, so a writer can only understate recency.
- Reproduce: `q '{"jsonrpc":"2.0","id":1,"method":"arkiv_query","params":["$createdAt > u64(1)",{"select":{"count":true}}]}'`.

## 6. Anonymous reads run out after about 1,000 queries, and the TypeScript SDK hangs instead of erroring

- Expected: a 429 surfaces as an error the caller can back off on.
- Actual, measured 2026-11-06:
  - The public RPC answered `arkiv_query` with HTTP 429 and error code `ANON_COST_LIMITED` ("unauthenticated query budget exhausted") with `RateLimit: limit=10000, remaining=0, reset=391` and `Retry-After` of 391 s, then 243 s and 121 s on later polls.
  - A query is billed 10 cost units (header `Arkiv-Cost`, the same for a count and a 100-row page), so the anonymous budget is about 1,000 queries per window, and reads came back by themselves after about 6.5 minutes.
  - Writes, receipts and `eth_*` calls kept working during the lockout.
  - The TypeScript SDK's read calls then hung rather than failed.
    Three sections of our funded probe stalled for 240 s each, which was our own timeout and not an SDK timeout.
  - The same SDK call that hung (a sponsor extension) succeeded in the next pass once the budget had recovered, which points at the 429 and not at the SDK or the node.
    We did not isolate this further, and we did not isolate who spent the budget (our probes, an aborted earlier run, or other users behind the same IP).
- Also true on a healthy anonymous call (re-run 2026-11-07): the response carries `RateLimit: limit=2000, remaining=1980, reset=2`, a separate per-IP request window.
- Impact: a real client needs an access key from hub.arkiv.network, which is wallet sign-in plus a captcha, so it is a human step that cannot be scripted in CI.
  arkiv-git reads the key from `ARKIV_ACCESS_KEY`, sends it as `X-API-KEY`, waits out `Retry-After` for a bounded time on a 429, and then fails with `E_RATE_LIMITED`.
  With a key, the live 10 MiB gate on 2026-11-07 saw no 429 waits.
- Reproduce (do not run this against a shared IP casually): loop `arkiv_query` anonymously, for example a count query, about 1,100 times without waiting, and read the `RateLimit`, `Retry-After` and `Arkiv-Cost` headers on the failing call.
  Then call the TypeScript SDK's `select().from()...` read during the lockout and watch it not return.

## 7. The response size cap is unpublished

- Expected: a documented maximum response size, because a page of 100 entities can carry up to 100 x 128 KiB of `bytes`.
- Actual: the docs say the RPC edge "may cap" response size and publish no number.
- Impact: arkiv-git cannot compute a safe page size.
  It starts at 16 chunk bodies per page (about 1 MiB of payload) and halves down to 1 on HTTP 413, a 5xx or a timeout.
  We did not measure where the cap is, so the starting size is a guess.

## 8. Contract creation is disabled, and two documents disagree about calls to other addresses (re-run 2026-11-07)

- Expected: either an EVM, or a clear statement that there is none.
- Actual: `eth_estimateGas` for a contract-creation transaction returns `-32603`, `contract creation is disabled (no-EVM Arkiv executor)`.
  The node's api-spec says a transaction to a non-Arkiv address reverts, while its tokenomics doc and the live node treat it as a plain value transfer (21,326 gas for a call with data).
- Reproduce: `q '{"jsonrpc":"2.0","id":1,"method":"eth_estimateGas","params":[{"from":"0x0000000000000000000000000000000000000001","data":"0x6080"}]}'`.
- Impact: repo programs, bounties and escrow cannot be contracts on this chain.
  arkiv-git keeps every rule client verified, and treats any on-chain settlement as a separate EVM chain that stores only hashes pointing at Arkiv entity keys.
  The Arkiv docs we read do not say how contracts are intended to reach Arkiv data.

## 9. At most 16 transactions per account per block, cause unknown, and a slow ramp

- Expected: the documented per-account pending limit, or a way to learn it.
- Actual, measured 2026-11-06:
  - 200 pending transactions from one account were accepted at once with no rejection, so the reth default of 16 pending slots per sender is not enforced at submission.
  - No block ever held more than 16 transactions from that account, and 16 was reached in three separate bursts (24 KiB creates, 1 KiB creates, plain transfers).
  - A flood of 200 plain transfers took 47 consecutive blocks, about 2 per block for the first 20 blocks and ramping up to 16 per block at the end.
  - We could not find a cause in the node source or the docs, and we did not test more than 200 pending.
- Impact: arkiv-git's pipeline allows 16 unconfirmed transactions per sender and treats that as the ceiling.

## 10. Push throughput in practice is about 0.6 chunk transactions per block

- Expected: with 64 KiB chunks and 30M gas per block, up to about 11 chunks per block at a 1-day lifetime, so a 100 MiB push in about 5 minutes at the gas-bound best case and about 12 minutes at a 4-per-block average.
- Actual, live gate 2026-11-07: 10 MiB in 161 chunks took 519.7 s over 261 blocks and 171 transactions, about 0.6 chunk transactions per block, against 4 per block allowed by arkiv-git's own pacing (which keeps blocks under half full so the base fee stays at its 1 gwei floor).
  The base fee stayed at 1 gwei at start and at peak.
- We have not profiled the cause.
  Per-transaction RPC round trips (estimate gas, send, receipt polls) are our leading suspect, and some of it may be in our client rather than Arkiv, so this is a measurement and not a bug report.
- It is the main reason the full 100 MB run on Rabdi has not been done; the 100 MB gate passes against the in-memory fake in about 18 s.

## 11. Size ceilings are lower than documented, and the gas cap bites at long lifetimes

- Expected: a 128 KiB `bytes` attribute can be created.
- Actual, measured 2026-11-06:
  - The node's documented limit is 131,072 B per `bytes` attribute, but the largest payload that reached the node through the public RPC was 130,336 B, because the edge answers HTTP 413 "request body too large" above a 256 KiB JSON-RPC body.
  - The largest create that mined was a 130,272 B payload (5,303,300 gas); sizes between that and 130,336 B were not tried.
  - The largest raw signed transaction the public RPC accepts is 131,030 B, and the node's own transaction pool limit is 131,072 B encoded.
    Two 64 KiB payloads are 131,072 B, so one chunk fits per transaction and batching chunks is impossible.
  - The per-transaction gas cap is 16,777,216 (EIP-7825).
    A 126 KiB create at the maximum lifetime costs 19,729,340 gas and cannot be sent; a 64 KiB create at the maximum lifetime costs 10.1M gas and can.
  - Genesis configures a 200,000,000 block gas limit, but the builder stays at 30,000,000 on every block we looked at (a known issue per the node docs).
- Impact: 64 KiB chunks, one per transaction, with the 30-day data lifetime chosen partly so a chunk costs 3.92M gas instead of 10.1M.

## 12. `entityNonce` is the most useful call we found, and its documentation says the name is "open"

- What it does: `entityNonce(address)`, an `eth_call` to the Arkiv registry `0x4400000000000000000000000000000000000044` with selector `0x36917bfd`, returns how many entities an address has ever created, live or not.
- Why we depend on it: it gives an exact completeness check without logs or history.
  If an address's live entity count (`$creator = addr(...)` with `select: {count: true}`) is below its `entityNonce`, something it created has expired or been deleted, and arkiv-git fails closed instead of showing a silently older repo head.
- Actual: the node's api-spec lists `entityNonce(owner)` under "Account & nonce reads" and marks the exact method names as open.
  It works on Rabdi (re-run 2026-11-07: an address that has created entities returned 103 in the 32-byte result, while its transaction count was 317, so the two differ as expected).
- Ask: please document `entityNonce` as a supported call with the selector and registry address above, and promise it keeps counting creations and not live entities.
  A whole class of "has anything I created disappeared?" features rests on it.
- Reproduce: `eth_call` with `{"to":"0x4400000000000000000000000000000000000044","data":"0x36917bfd" + 12 zero bytes + <20-byte address>}` at `latest`, then compare with `eth_getTransactionCount` for the same address.

## 13. Smaller items

- Expiry is silent: an entity leaves the query surface with no event, and the purge removes at most 10 entities per block.
  arkiv-git compensates with `status` and `keepalive`, and warns when the minimum expiry is under 7 days.
- Deleting refunds nothing, so "create long then delete early" is never right; arkiv-git creates for 30 days and renews.
- Permanent entities are described in the api-spec but refused by the node today (the tokenomics doc says so).
- Pricing and limits are fixed at genesis and called placeholders; mainnet pricing and whether a mainnet exists are not stated in the docs we read, so every cost figure in this repo is testnet-calibrated.
- Testnets are replaced (Tiramisu, Braga and Kaolin were retired, and Malpua was created on 2026-11-03), so integration tests cannot rely on persistent testnet state.
- The faucet and access keys both need a wallet sign-in plus a captcha, which suits people and blocks CI and agents; we funded the test wallet by hand.

## What would make this easier

1. A Rust SDK that targets the current ABI, published under MIT or Apache-2.0, or a clear "use `arkiv-bindings`" note and a crates.io release of that crate.
2. Working `eth_getLogs` ranges and `eth_getBlockReceipts`, or an explicit statement that they will not exist.
3. A documented, stable `entityNonce` call (section 12).
4. Make `$createdAt` and `$updatedAt` queryable and usable in `ORDER BY`.
5. Either implement `NOT` or make the `!=` error stop pointing at it.
6. Cursor pagination in the Rabdi SDK, or a stable sort so offset pages are safe.
7. A published response size cap and a published per-account transactions-per-block rule, with the reason.
8. SDK read calls that raise a typed error on 429 with the `Retry-After` value instead of hanging.
9. A scriptable way to obtain testnet funds and an access key for CI, such as a signed-request flow without a captcha, even if rate limited.
10. One consistent statement across api-spec and tokenomics on permanent entities and on calls to non-Arkiv addresses.
11. A statement of how contracts are meant to reach Arkiv data, even if the answer is "they are not".

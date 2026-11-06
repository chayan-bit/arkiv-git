# arkiv-measure

P0 spike for arkiv-git: measures the real limits and fees of Arkiv on the Rabdi testnet (chain id 7794095).
Results feed `docs/p0/arkiv-limits.md`.

Testnet only.
The script aborts unless the RPC reports chain id 7794095, and it holds no real funds.

## Why the TypeScript SDK

The official Rust SDK (`arkiv-sdk` 0.4.1 on crates.io, 0.5.0 on git main) targets the retired GolemDB/OP-stack ABI (`ArkivEntityBTLExtended`, relative BTL, address `0x...61726B6976`).
Rabdi speaks `execute(Operation[])` at `0x4400000000000000000000000000000000000044`, which only `@arkiv-network/sdk` 1.0.1 (npm, 2026-11-02) implements.
Where the SDK refuses before the node does, or for estimate-only probes, the script encodes the same ABI by hand in `src/lib.mjs`.

## Run

Nix only, nothing is installed globally:

```sh
cd spike/arkiv-measure
nix-shell -p nodejs_22 --run 'npm ci'
nix-shell -p nodejs_22 --run 'node src/measure.mjs all'      # or: readonly | funded
```

First run creates `.env` (mode 600, gitignored) with a fresh throwaway key and prints only the public address.
`ARKIV_ACCESS_KEY` in `.env` is optional; without it the anonymous per-IP rate limit applies, which these probes stay well under.
Each run writes `results/<timestamp>-<mode>.json`.

## Funding

Read-only probes need no funds.
Funded probes run automatically when the address holds at least 0.04 tGLM, otherwise the script prints the address and skips them.
The Rabdi faucet (https://hub.arkiv.network/faucet) needs a wallet sign-in (SIWE) and a captcha, so fund by hand.
Expected spend is about 0.05 tGLM at the 1 gwei base-fee floor, hard-capped at 0.09 tGLM.

## Probes

Read-only (`src/readonly.mjs`), no transaction is mined:

| Probe | Measures |
| --- | --- |
| `chainProbes` | chain id, client version, base fee, block time and gas limit over the last 100 blocks, busiest block, largest tx seen on chain |
| `edgeSizeProbe` | largest raw tx the public RPC accepts (binary search, signed by an ephemeral always-unfunded key, never mined) |
| `gasModelProbes` | `eth_estimateGas` of creates by size (1 to 126 KiB) and lifetime (1 d to max), derived gas per byte of calldata and per byte-block of storage, cost per MiB-month |
| `annotationLimitProbes` | max attributes (32/33), max `str` (128/129 B), max name (32 B), indexed vs unindexed cost |
| `extendProbes` | extension gas (single and batched up to 100 entities), non-owner extension of a `permissionlessExtension` entity, non-owner on a plain entity, shortening |
| `contractAndHistoryProbes` | contract creation, calls to non-Arkiv addresses, old block availability |
| `queryProbes` | latency (median of 5) for count, owner, range, OR, ORDER BY, annotation eq/prefix; error behaviour for limit 101, NOT, `!=`, `atBlock`, offset past end |

Funded (`src/funded.mjs`, `src/throughput.mjs`), real writes with the throwaway key, entities deleted at the end:

| Probe | Measures |
| --- | --- |
| size ladder | fee paid for creates of 1, 4, 16, 64 KiB at a 1 day lifetime |
| max create | a single create at the largest payload the RPC edge lets through |
| extend | owner extension (cost, new expiry), non-owner extension of a permissionless entity (who pays, owner unchanged), non-owner on a plain entity, shortening |
| update | patch replaces attributes, key and expiry unchanged, index follows the patch, patch cost on grow and shrink |
| expiry | 20 block entity: how long until queries drop it, extend and get after expiry, whether the create calldata stays in block history |
| throughput | 32 pre-signed 24 KiB creates from one account sent at once: per-account txpool slots, txs and bytes per block, bytes per second |
| annotation queries | SDK builder and raw queries over the script's own entities |

Funded sections are independent: a failure in one is recorded as `error.<section>` and the rest still run.

## Status

Read-only probes and the funded probes were run on 2026-11-06 against Rabdi; results are in `docs/p0/arkiv-limits.md` and `results/*.json`.
`funded` (pass 1) covers the size ladder, estimate vs receipt, the largest create, owner extension and the 24 KiB burst.
`funded2` reruns the sections that stalled when the anonymous query budget ran out (sponsor extension, patch, expiry, queries) and adds a 48 x 1 KiB burst and a 200-transfer flood; `ONLY=sponsor,update,expiry,queries,poolLimit,burst` picks sections.
Reads can hit HTTP 429 `ANON_COST_LIMITED`; `funded2` waits for the budget to return before any SDK read, because the SDK hangs on it.
Funded runs spend real testnet tokens (the whole campaign used 0.0674 tGLM); stale entities simply expire after one day.

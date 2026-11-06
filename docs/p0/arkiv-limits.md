# Arkiv limits and costs (P0 spike)

Measured and read on 2026-11-06 against the Rabdi testnet (chain id 7794095) and the local Arkiv node source at v1.0.1 (`~/Code/projects/arkiv-contrib/arkiv`, commit `2a3526d`).
Labels: SOURCE-VERIFIED (read in code), DOCS-CLAIMED (read in docs only), MEASURED (observed on the live testnet by `spike/arkiv-measure`), DERIVED (computed from MEASURED or SOURCE-VERIFIED numbers), UNKNOWN.
Gas numbers marked MEASURED come from `eth_estimateGas`, which the node executes in full, so they equal what a funded transaction uses.
The funded probes (real writes, throughput burst, expiry timing) have not run yet, because the throwaway address is unfunded; see the end of this file.

## Which network and which version

- The public testnet to build on is Rabdi.
  It runs reth 2.7.0 with the Arkiv entity engine (GolemDB store), no EVM, 2 s blocks.
  (MEASURED `web3_clientVersion`; DOCS-CLAIMED https://github.com/Arkiv-Network/arkiv-starlight-docs `networks/rabdi.mdx`.)
- Tiramisu (reth 2.5.0, chain id 7738577) still answers but the docs call it replaced.
  Braga and Kaolin (OP-stack L3s on Hoodi) were retired on 2026-08-12 and 2026-05-15.
  (DOCS-CLAIMED `networks/older-networks.mdx`.)
- The local source at v1.0.1 matches Rabdi's ABI and error behaviour (the `execute(Operation[])` ABI, the `NOT` failure, the `contract creation is disabled` message), so the source citations below apply to the live chain.
- Pricing and limits are fixed at genesis and the docs call today's values placeholders.
  Treat every cost below as testnet-calibrated, not a mainnet price.
  (SOURCE-VERIFIED `docs/tokenomics_v1.md:18`.)

## 1. Sizes, counts, gas limits

| Item | Value | Source | Label |
| --- | --- | --- | --- |
| Max `bytes` attribute (node) | 128 KiB (131,072 B) per attribute | `docs/api-spec.md:172`, `crates/arkiv-interfaces/src/constants.rs:67-71` | SOURCE-VERIFIED |
| Max payload actually accepted end to end (one `bytes` attribute, public RPC) | 130,336 B (127.3 KiB), not 128 KiB | `spike/arkiv-measure` `edgeSizeProbe`; the RPC edge answers HTTP 413 "request body too large" above a 256 KiB JSON-RPC body (boundary body 262,134 B) | MEASURED |
| Max raw signed tx through the public RPC | 131,030 B | same probe | MEASURED |
| Node txpool tx size limit | 131,072 B encoded (reth default 128 KiB, not overridden in Arkiv's flags) | reth fork `crates/transaction-pool/src/validate/constants.rs:12`, `eth.rs:515-521`; Arkiv node flags in `docker/compose.yml:45`, `kurtosis/arkiv-chain.yaml:29` | SOURCE-VERIFIED |
| Per-entity total bytes cap | Open in the node (32 attributes x 128 KiB possible); SDK enforces 128 KiB total per create or patch as a provisional cap | `constants.rs:69-70` (issue #155), SDK `src/schema/attributes.ts` `MAX_ENTITY_BYTES` | SOURCE-VERIFIED |
| Attributes per entity | 32 user attributes; the SDK's named schema tag `arkiv.schema` uses one, leaving 31 | `constants.rs:74-77`, docs `reference/limits.mdx`; 32 accepted, 33 reverts | SOURCE-VERIFIED, MEASURED |
| Indexed attributes per entity | Any of the 32 may be indexed (genesis: 39 cells, 36 indexed cells, which are 32 user plus system cells) | Rabdi `genesis.json` `record_limits`, `entity_records.rs:55-71` | SOURCE-VERIFIED |
| Attribute name | `[A-Za-z][A-Za-z0-9_.-]*`, at most 32 B (ABI is `bytes32`) | `docs/api-spec.md` section 4 "Attribute names"; 32 B accepted | SOURCE-VERIFIED, MEASURED |
| `str` value | at most 128 B UTF-8; 128 B accepted, 129 B reverts | `constants.rs:60`, probe | SOURCE-VERIFIED, MEASURED |
| Numeric annotation types | `bool`, `i32`, `u64`, `u256`, `dec` (18 dp), `bytes32`, `addr`, `key`; each is one 32 B word on the wire | `docs/api-spec.md:36-47` | SOURCE-VERIFIED |
| `bytes` attribute | never indexed, so never queryable | `docs/api-spec.md:46` | SOURCE-VERIFIED |
| Max tx size (gas) | 16,777,216 gas (EIP-7825 cap, enforced by the executor) | `crates/arkiv-reth-executor/src/lib.rs:722-738`, SDK `src/consts.ts:21` | SOURCE-VERIFIED |
| Block gas limit | 30,000,000 on every one of the last 100 blocks | the 100 blocks ending at head 290,204 | MEASURED |
| Block gas limit configured in genesis | 200,000,000, but the builder ignores it and stays at 30M (known bug) | Rabdi `genesis.json` `gasLimit 0xbebc200`; `docs/tokenomics_v1.md:22` | SOURCE-VERIFIED |
| Largest tx seen on chain (last 100 blocks) | 85,796 B input, 8 txs and 7.06M gas in the busiest block | `chainProbes` | MEASURED (observed, not a capacity) |

## 2. BTL (expiry)

| Item | Value | Source | Label |
| --- | --- | --- | --- |
| Unit | Blocks, 2 s each (43,200 per day). There is no BTL field: the create carries an absolute `expiresAt` and/or a relative `minLifetime`, resolved as `max(expiresAt, landingBlock + minLifetime)` | `docs/api-spec.md:105-123`, SDK `src/entity/expiry.ts` | SOURCE-VERIFIED |
| Block time | 2 s (min 2, mean 2, max 2 over 100 blocks) | `arkiv_getBlockTiming`, headers | MEASURED |
| Max lifetime | 7,884,000 blocks (182.5 days) counted from the block the create or extension lands in; applies to the remaining lifetime, so repeated extension is unlimited | Rabdi `genesis.json` `host.arkiv.max_lifetime`, `docs/tokenomics_v1.md:247-250`, SDK `src/consts.ts:9` | SOURCE-VERIFIED |
| Permanent entities | Refused by the node today (`MAX_EXPIRES_AT` is rejected) although the api-spec still describes it | `docs/tokenomics_v1.md:250` vs `docs/api-spec.md:118` | SOURCE-VERIFIED |
| Extension op | Operation tag 3, `ExtendExpiry(entityKey, expiresAt, minLifetime)`; SDK `extendEntity`; a new expiry earlier than the current one reverts, equal is a no-op | `docs/api-spec.md:105-123,142-146`, SDK `src/actions/wallet/extendEntity.ts` | SOURCE-VERIFIED |
| Extension semantics | A duration counts from the landing block, not from the old expiry (10 days left plus `fromDays(30)` leaves 30 days) | `writing/expiration.mdx` | DOCS-CLAIMED |
| Who may extend | Owner only, unless the entity was created with the `permissionlessExtension` flag (bit 1, immutable after create); then anyone may extend and the owner is unchanged | `docs/api-spec.md:130`, `writing/create.mdx` | DOCS-CLAIMED |
| Non-owner extend, flagged entity | Succeeds in simulation from an unrelated address (gas 43,259) | `extendProbes`, 104 of the first 4,000 live entities carry the flag | MEASURED (estimate) |
| Non-owner extend, plain entity | Reverts in simulation (the revert reason is not returned by the node, so this is not proof of `NotOwner` alone) | `extendProbes` | MEASURED (estimate) |
| Shorten expiry | Reverts in simulation | `extendProbes` | MEASURED (estimate) |
| Cost of one extension | Single op on a 1 KB entity: about 43,300 gas for +10 blocks, 45,300 for +1 day; storage part is `units x addedBlocks x 14e-6` | `extendProbes`, formula `docs/tokenomics_v1.md:186,200` | MEASURED |
| Cost of batched extensions | One tx extending 100 entities: 2,161,257 gas, so about 21,400 gas per extra entity (50 entities: 1,094,987; 10: 235,917) | `extendProbes` | MEASURED |
| Cost of extending a 64 KiB chunk by max lifetime | About 7.5M gas (67,472 units x 7,884,000 blocks x 14e-6, plus about 21k) | formula, DERIVED from MEASURED units | DERIVED |
| At expiry | The entity leaves the query surface and every op on it fails; no event is emitted; the purge tx removes at most 10 entities per block (1M gas, unpaid) | `docs/api-spec.md:189`, `docs/tokenomics_v1.md:256-265`, `writing/expiration.mdx` | SOURCE-VERIFIED |
| Recoverable after expiry? | Not through `arkiv_query` or `getEntity` (no historical state: the store runs `head-only`). The create tx calldata stays in block history while the node keeps blocks; Rabdi still serves block 1 | `kurtosis/arkiv-chain.yaml:16-19`, probe `block_1_still_served` | SOURCE-VERIFIED, MEASURED |
| Delete refund | None: deleting an entity returns no storage fee | `docs/tokenomics_v1.md:188` | SOURCE-VERIFIED |

## 3. Pricing

Fee formula (EIP-1559 gas, paid in the native token, tGLM on Rabdi):

```
gasUsed       = compute_gas + storage_gas
compute_gas   = max(21000 + 4*tokens + golemdb_cost, 21000 + 10*tokens)   tokens = zero_bytes + 4*nonzero_bytes
storage_gas   = ceil(units * blocksOfLifetime * capacity_rate / 1e6)
units         = cell_bytes*1 + index_bytes*2 + cells*64 + indexed_cells*65
```

Source: `docs/tokenomics_v1.md:88,200` and `crates/arkiv-tokenomics`; the weights below are Rabdi's own genesis (`capacity_rate 14`, `cell_byte_units 1`, `index_byte_units 2`, `c_cell 64`, `c_idx 65`).
Label: SOURCE-VERIFIED for the formula and weights.

| Item | Value | Source | Label |
| --- | --- | --- | --- |
| Base fee | 1 gwei, which is also the floor (the node never lets it fall below the genesis `baseFeePerGas`); idle chain shows exactly 1 gwei | `docs/tokenomics_v1.md:224`, head block | SOURCE-VERIFIED, MEASURED |
| Calldata and compute cost per byte (random bytes, so nearly all non-zero) | 40.8 gas per byte, about 42.8M gas (0.0428 tGLM) per MiB, paid once, independent of lifetime | `gasModelProbes` (size sweep 16 to 126 KiB) | MEASURED |
| Create gas by payload, 1 day lifetime | 1 KiB 142,122; 4 KiB 228,644; 16 KiB 701,674; 32 KiB 1,369,743; 64 KiB 2,707,122; 96 KiB 4,044,349; 126 KiB 5,297,849 | `gasModelProbes` | MEASURED |
| Create gas, 16 KiB, by lifetime | 1 d 701,432; 30 d 1,015,634; 90 d 1,666,757; max 2,669,619 | `gasModelProbes` | MEASURED |
| Create gas, 64 KiB, by lifetime | 1 d 2,707,515; 30 d 3,890,917; max (7,884,000 blocks) 10,113,997 | `gasModelProbes` | MEASURED |
| Storage units of a 64 KiB entity | 67,472 (payload plus about 1,900 units of per-entity overhead; 16 KiB entity: about 17,930, overhead about 1,550) | slope of the lifetime sweep divided by 14e-6 | MEASURED |
| Storage price | 0.0000153 gas per byte-block at 16 KiB, 0.0000144 at 64 KiB; for 64 KiB chunks about 19.6M gas (0.0196 tGLM) per MiB-month and about 20.1 tGLM per GiB-month | derived from the two rows above | DERIVED |
| Cost per MB-month in native token | Storage only: 0.0196 tGLM per MiB-month. Including the one-time write cost spread over month one: about 0.062 tGLM per MiB | DERIVED | DERIVED |
| 126 KiB create at max lifetime | 19,729,340 gas, above the 16,777,216 tx cap, so it cannot be sent; the longest lifetime a 126 KiB entity can buy in one tx is about 145 days | `gasModelProbes` | MEASURED |
| 32 x u64 indexed attributes vs unindexed | 455,501 vs 217,981 gas (indexing about doubles the write cost of small entities) | `annotationLimitProbes` | MEASURED |
| Token price | Testnet tGLM has none. GLM spot is 0.108966 USD on 2026-11-06 (CoinGecko); the node docs assume 0.10 | https://api.coingecko.com/api/v3/simple/price?ids=golem&vs_currencies=usd, `docs/tokenomics_v1.md:287` | DOCS-CLAIMED (spot), MEASURED (HTTP read) |
| Mainnet exists and its fee schedule | Not listed in the docs or the network registry (only Rabdi, Tiramisu, Malpua, Sorbet testnets) | https://github.com/Arkiv-Network/db-chain-networks | UNKNOWN |
| Fee flow | Base fee of compute gas burns; base fee of storage gas goes to a pool account nothing pays out of yet; tips go to the block producer | `docs/tokenomics_v1.md:25-28,211-216` | SOURCE-VERIFIED |
| RPC metering | Public RPC is rate limited per IP (Rabdi anonymous: 2,000 requests per 10 s window; Tiramisu: 600 per 60 s); `arkiv_query` is billed 10 cost units per call (`Arkiv-Cost` header, same for a count and a 100-row page); a free access key from hub.arkiv.network raises limits | response headers, `networks/rabdi.mdx`, `start-here/access-keys.mdx` | MEASURED, DOCS-CLAIMED |

## 4. Write throughput and per-account limits

| Item | Value | Source | Label |
| --- | --- | --- | --- |
| Gas per block | 30M, of which up to 1M can go to the unpaid purge tx | `docs/tokenomics_v1.md:256-265` | SOURCE-VERIFIED |
| Theoretical bytes per block, 64 KiB chunks | 1 day lifetime: 11 chunks (704 KiB) per block, 352 KiB/s; 30 days: 7 chunks (448 KiB), 224 KiB/s; max lifetime: 2 chunks (128 KiB), 64 KiB/s | 30M / measured create gas | DERIVED |
| Observed busiest block, last 100 | 8 txs, 7.06M gas | `chainProbes` | MEASURED (observed) |
| Measured burst from one account | Not run (needs funds) | `throughputProbe` | UNKNOWN |
| Pending txs per account | 16 in the reth fork's default pool (`TXPOOL_MAX_ACCOUNT_SLOTS_PER_SENDER`); Rabdi's flags are not published, the burst probe measures it | reth fork `crates/transaction-pool/src/config.rs:12` | SOURCE-VERIFIED (default), UNKNOWN (live) |
| Sender nonce rule | v1.0.1 added `--txpool.enforce-tracked-nonce`: a tx below the sender's tracked nonce is rejected from the pool, to stop a stuck sender; the Kurtosis file notes it is safe because there is one sequencer and no reorgs | commit `4bf1630`, `kurtosis/arkiv-chain.yaml:20-29` | SOURCE-VERIFIED |
| Chain-wide storage capacity | If every block were full of storage buys, the chain could sell about 1.9 TiB-years of storage per year (30M gas x 15.77M blocks / 14.9M gas per 64 KiB-year) | DERIVED | DERIVED |
| Other limits | At max lifetime a 64 KiB create costs 10.1M gas, so 30M gas fits 2 per block; batching several creates in one tx saves only the 21k base gas each | `gasModelProbes` | DERIVED |

## 5. Query language

| Item | Value | Source | Label |
| --- | --- | --- | --- |
| Operators | `=`, `<`, `<=`, `>`, `>=`, `BETWEEN` (inclusive), `STARTSWITH` (`str` only), `EXISTS(attr)`, `TYPEOF(attr) = tag`, `AND`, `OR`, parentheses, `*` (match all) | `docs/api-spec.md` section 4, `reference/query-language.mdx` | SOURCE-VERIFIED |
| `NOT` | Parses but fails on Rabdi: `count: store: NotImplemented` | `docs/query-language-conformance.md` Q1 (line 40), live query | SOURCE-VERIFIED, MEASURED |
| `!=` | Rejected: "write NOT (attr = value)" | live query, conformance Q4 | MEASURED |
| Typed values | A predicate also checks the type: `x = i32(4)` never matches a `dec` `x`; untagged numbers are `i32` | `docs/api-spec.md` section 4 "Rules" | SOURCE-VERIFIED |
| Entity filters | `$key`, `$owner`, `$creator`, `$expiresAt` all take every comparison; `$createdAt` and `$updatedAt` are returned but not queryable | `docs/api-spec.md` "System attributes" | SOURCE-VERIFIED |
| Sort | `ORDER BY` one attribute with an explicit type (`rank:u256 DESC`); ties in creation order | `docs/api-spec.md` "Sorting" | SOURCE-VERIFIED |
| Pagination | `limit` at most 100 (101 is rejected), `offset`; offset past the end returns an empty page; Rabdi has no cursors in the SDK (older networks did) | `docs/api-spec.md:393`, `networks/older-networks.mdx`, live query | SOURCE-VERIFIED, MEASURED |
| Max result size | 100 rows, each up to 128 KiB of `bytes`; the RPC edge "may cap" response size and no limit is published | `querying/without-schema.mdx` | UNKNOWN |
| Count | Exact total via `select: { count: true }` (118,751 live entities on Rabdi at probe time) | live query | MEASURED |
| Historical queries (`atBlock`) | Not available on Rabdi: even head minus 1 fails with "state for this block is not retained" | live queries; `kurtosis/arkiv-chain.yaml:16-19` (`head-only`) | MEASURED |
| Expired entities | Not queryable | `writing/expiration.mdx` | DOCS-CLAIMED |
| Query latency (client in India, via Cloudflare, median of 5) | count 553 to 734 ms; page of 100 keys 492 to 632 ms; owner count 222 to 366 ms; `$expiresAt` range 311 to 409 ms; OR of two ranges 318 to 407 ms; ORDER BY 544 to 669 ms (three runs, ranges are run to run) | `queryProbes` | MEASURED |
| Annotation queries on the spike's own data (str eq, STARTSWITH, u64 range) | Not run (needs the funded run to create the entities) | `queryProbes` with a tag | UNKNOWN |

## 6. Smart contracts

- Contracts cannot run on the Arkiv chain.
  Contract creation is rejected by the executor, and `eth_estimateGas` for a CREATE tx returns `contract creation is disabled (no-EVM Arkiv executor)`.
  (SOURCE-VERIFIED `crates/arkiv-reth-executor/src/lib.rs:45,915-921`; MEASURED live.)
- A call to any non-Arkiv address is a plain value transfer (21,326 gas for a call with data).
  The api-spec says such txs revert; the tokenomics doc and the live node say transfer.
  (MEASURED; conflicting docs: `docs/api-spec.md:20` vs `docs/tokenomics_v1.md:53-54`.)
- No contract can read entity state, because there is no EVM. The only state proofs are `eth_getProof` against the block state root, verified client side.
  (SOURCE-VERIFIED `docs/api-spec.md` section 5, "There is no contract-deployment or bytecode-execution surface".)
- The intended path to contracts is not defined.
  The current stack states "There is no L1, L2, settlement chain, or cross-chain coordination" (`kurtosis/arkiv-chain.yaml:3-5`, SOURCE-VERIFIED).
  The earlier L2/L3 design (Braga, Kaolin with a Standard Bridge) was retired in 2026, and the old `arkiv-op-reth` precompile design let contracts `CALL` the entity address.
  Nothing in the docs says a settlement chain is planned.
  Label: UNKNOWN.

## 7. Sequencer and censorship resistance

- One sequencer proposes all blocks.
  The Kurtosis network says "Participant 1 is the sole sequencer"; Rabdi's public RPC fronts watcher nodes and the producer is in-cluster only; the 64 validator keys are managed by the deployment tooling.
  (SOURCE-VERIFIED `kurtosis/arkiv-chain.yaml:10`; DOCS-CLAIMED https://github.com/Arkiv-Network/db-chain-networks `networks/testnet/rabdi/README.md` and `README.md`.)
- Writes go to a metered public RPC that can rate limit or refuse a caller.
  There is no forced-inclusion path (no L1 deposit queue, no alternative mempool) in the node source or docs.
  (SOURCE-VERIFIED by absence in `docs/` and `crates/`: grep for "forced", "censor" finds nothing; UNKNOWN for any undocumented plan.)
- Consequence: Arkiv gives ordering, availability and timestamps only as long as one operator behaves; clients must verify every signature and treat liveness as an operator promise.

## 8. Testnet

| Item | Value | Source | Label |
| --- | --- | --- | --- |
| Name | Rabdi ("Arkiv's current public testnet") | `networks/rabdi.mdx`, https://github.com/Arkiv-Network/db-chain-networks | DOCS-CLAIMED |
| Chain id | 7794095 (0x76edaf) | `eth_chainId` | MEASURED |
| HTTP RPC | https://rpc.rabdi.db-chain.testnet.arkiv.network (optional access key as the last path segment, or `X-API-KEY` header) | live request | MEASURED |
| WS RPC | wss://rpc.rabdi.db-chain.testnet.arkiv.network | `networks/rabdi.mdx`, SDK `src/chains/rabdi.ts` | DOCS-CLAIMED |
| Explorer | https://rabdi.explorer.arkiv.network (HTTP 200) | live request | MEASURED |
| Native token | tGLM, 18 decimals | SDK `src/chains/rabdi.ts` | SOURCE-VERIFIED |
| Faucet | https://hub.arkiv.network/faucet. It needs a Sign-In with Ethereum session and a captcha token in the claim request (`POST /api/faucet {recipient, captchaToken}`; the unauthenticated `GET /api/faucet` returns 401); the page text states a 24 hour cooldown per wallet. Not attempted. Claim amount is only shown after sign-in | live HTTP, hub JS bundle | MEASURED |
| Access keys | https://hub.arkiv.network/access-keys: wallet sign-in plus captcha, one key per wallet per network, monthly cost-unit quota | `start-here/access-keys.mdx` | DOCS-CLAIMED |
| State persistence | State is not carried between networks (Tiramisu, Braga, Kaolin were replaced) and Malpua was created on 2026-11-03, so assume testnet state is disposable | `networks/older-networks.mdx`, registry git log | DOCS-CLAIMED |

## 9. SDKs

| Language | Package, version, last release | API | Works on Rabdi? | Label |
| --- | --- | --- | --- | --- |
| TypeScript | `@arkiv-network/sdk` 1.0.1, npm 2026-11-02 (repo `Arkiv-Network/arkiv-sdk-js`, MIT); built on viem | `createEntity`, `patchEntity` (update), `extendEntity`, `deleteEntity`, `changeOwnership`, `executeBatch`; reads `select().from().where().orderBy().limit().offset()`, `count()`, `getEntity()`, `query()`; schemas via `defineSchema` | Yes: its `ARKIV_ADDRESS` is `0x4400...0044` and it uses `execute(Operation[])` | SOURCE-VERIFIED (`src/consts.ts`, `src/entity/operations.ts`) |
| Rust | `arkiv-sdk` 0.4.1 on crates.io (2025-09-30); git main is 0.5.0, last commit 2026-01-31; GPL-3.0-or-later | `ArkivClient`, ops `create`, `update`, `extend`, `delete`, `chown` (`src/tx/ops/*.rs`), alloy based | No: it targets the old GolemDB ABI (address `0x00000000000000000000000000000061726B6976`, relative `number_of_blocks` BTL, `ArkivEntityBTLExtended` events) | SOURCE-VERIFIED (`src/contract.rs:12,60`, `src/tx/ops/extend.rs:12-23`); not run |
| Python | `arkiv-sdk` 1.0.0b2 (2026-03-04), 1.0.0b3.dev0 (2026-05-07) on PyPI; MIT | `extend_entity(entity_key, extend_by)`, batch ops, web3.py based | No: same old address `0x...61726B6976` and relative extension | SOURCE-VERIFIED (`src/arkiv/contract.py:12`); not run |
| Rust, practical path for P1 | `arkiv-bindings` and `arkiv-interfaces` inside the node workspace (MIT, not on crates.io): the sol! `IEntityRegistry` ABI, `Operation`/`Create`/`Patch`/`ExtendExpiry` structs, attribute encoders, the key derivation; use as a git dependency pinned to `v1.0.1`, or vendor | `crates/arkiv-bindings/src/lib.rs`, `encode.rs` | Yes, it is the node's own ABI | SOURCE-VERIFIED |

`spike/arkiv-measure` uses the TypeScript SDK for that reason and encodes the same ABI by hand (about 40 lines in `src/lib.mjs`) for raw-size and estimate-only probes.

## Implications for arkiv-git

### Pack chunk size

- Use 64 KiB of payload per chunk entity, with the last chunk of a pack smaller.
- The hard ceiling is 130,336 B for a single `bytes` attribute through the public RPC, not the documented 128 KiB, and that ceiling must also hold the annotations and ABI framing.
- At 64 KiB the per-entity storage overhead is 2.9 percent (16 KiB would pay 9.5 percent, 32 KiB 4.7 percent), and a create at the maximum lifetime costs 10.1M gas, well under the 16.78M per-tx cap.
  A 120 KiB chunk at maximum lifetime exceeds the cap (18.8M), forcing short creates plus extensions.
- Calldata dominates a short life: 0.0428 tGLM per MiB written once against 0.0196 per MiB-month stored, so compress before chunking (git packs already are) and never re-upload unchanged chunks; a content-addressed chunk key (`bytes32` annotation) makes reuse trivial.
- Keep annotations to a few indexed ones (`repo` id, pack hash, chunk index as `u64`, `kind`): each indexed attribute roughly doubles the write cost of small entities, and `bytes` payloads cannot be indexed, so anything that must be searchable (ref names, object ids) has to be a `str` of at most 128 B or a `bytes32`.

### Cost of a 100 MB repo for one year

- Assumption: 100 MiB of packed data, 1,600 chunks of 64 KiB, base fee at the 1 gwei floor, GLM at 0.109 USD, Rabdi's current placeholder pricing.
- Storage for 365 days: 1,600 x 67,472 units x 15,768,000 blocks x 14e-6 = 23.8G gas, about 23.8 GLM.
- One-time calldata and compute: 1,600 x 2.67M = 4.27G gas, about 4.27 GLM.
- Extension transactions (two per chunk in batches of 100): about 0.07 GLM.
- Total about 28.2 GLM, roughly 3.07 USD per year, or about 2.9 USD for 100 MB decimal (95.4 MiB).
- Storage alone is about 2.19 USD per GiB-month; the node docs' own estimate is 1.81 USD per GB-month at 0.10 USD per GLM, consistent once the 2 to 3 percent entity overhead and the 0.109 price are applied.
- Treat as order of magnitude: mainnet price and fee schedule are UNKNOWN, the base fee rises up to 12.5 percent per block whenever blocks are over half full, and the docs call today's weights placeholders.

### BTL-extension strategy

- Cost is linear in total lifetime, so a long first lifetime buys no discount and only front-loads gas (a max-lifetime 64 KiB chunk takes a third of a block).
  Create chunks for 30 to 90 days and renew.
- Renew in batches of up to 100 entities per tx (about 21.4k gas each plus the storage for added blocks).
  Extend when under one third of the lifetime remains, and always to `now + lifetime` since a duration counts from the landing block, not additively.
- Create every arkiv-git entity (chunks, refs, identity docs, issues) with `permissionlessExtension` set.
  The flag lets anyone extend but never edit, so any seed, CI job, or sponsor can keep a repo alive without holding the owner key.
  It cannot be added later, so it must be decided at P1 schema time.
- For immutable content-addressed chunks also set `readonly`; Rabdi already holds entities owned by the burn address `0x...dEaD` with both flags (104 of the first 4,000 entities carry `permissionlessExtension`), which is exactly this pattern.
- Expiry is silent (no event), so run a watcher that queries `$expiresAt < u64(head + window)` filtered by `$owner` or the repo tag, and alert well before expiry.
  A purged entity is gone from queries; its create calldata may still sit in block history, but nothing should depend on that.
- Delete returns nothing, so do not create long and delete early.
- Never set a lifetime assuming a year: the node caps each create or extension at 182.5 days of remaining life, so every repo needs at least two extensions a year.

### Can repo contracts live on Arkiv?

- No.
  The chain has no EVM, contract creation is disabled (measured live), and contracts elsewhere cannot read Arkiv state; there is no L1, L2 or settlement chain in the current stack, and none is announced.
- Keep every rule that matters (delegate signatures, ref updates, identity documents) client-verified, as the plan already assumes.
- If a contract-based feature is ever needed (bounties, on-chain anchoring of a ref root, name registry), it belongs on a separate EVM chain that only stores hashes pointing at Arkiv entity keys.
  That path is out of scope for P1 and should be a design option, not a dependency.

### Surprises that change the design

- Arkiv is not a transparent mirror: the documented 128 KiB payload cannot be sent (130,336 B is the real ceiling), and the 16.78M tx gas cap makes large long-lived creates impossible in one tx.
- There is no historical query (`atBlock` is unavailable, `NOT` fails), so version history must be modelled as append-only entities with sequence numbers, not as at-block reads.
- A single sequencer with no forced inclusion means no protocol-level censorship resistance; the README claim that Arkiv is trusted only for ordering, availability and timestamps needs a stated liveness assumption.
- The only compatible SDK is TypeScript.
  P1's Rust client should depend on `arkiv-bindings` from the node repo (MIT, git dependency) instead of the published `arkiv-sdk` crate, which targets a retired ABI and is GPL-3.0.
- Testnets are disposable (three replaced networks, a new one on 2026-11-03), so P1 integration tests must not depend on persistent testnet state.

## What is not measured yet

The following need a funded address and are implemented but have not run against the chain:

- Fee actually paid per size, taken from receipts (sizes 1, 4, 16, 64 KiB) to confirm the estimate-based model.
- Largest create that mines (payload 130,272 B), and whether the txpool or node adds a lower limit than the RPC edge.
- Owner extension cost, non-owner extension of a flagged entity (who pays, owner unchanged), non-owner on a plain entity, shortening.
- Update semantics: patch keeps the key and expiry, replaces attributes whole, the index follows, cost on grow and shrink.
- Expiry: blocks between `expiresAt` and disappearance from queries, behaviour of extend and get after expiry, whether create calldata stays on chain.
- Throughput burst: 32 pre-signed 24 KiB creates from one account (per-account pending limit, txs and bytes per block, bytes per second).
- Annotation queries over the spike's own data.

To run: fund the public address below with at least 0.04 tGLM (about 0.05 is spent), then `cd spike/arkiv-measure && nix-shell -p nodejs_22 --run 'node src/measure.mjs funded'`.
The faucet needs a wallet sign-in and a captcha, so it was not attempted.

Public address of the throwaway testnet key: `0x201b7FB60f8f4B221afCEcEcAeeB6d09Af42f15A` (the private key is only in the gitignored `spike/arkiv-measure/.env`).

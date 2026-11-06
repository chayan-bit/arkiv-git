// Read-only probes: no funds needed, nothing is mined. Gas numbers come from eth_estimateGas, which the node
// executes in full (calldata floor + GolemDB cost + storage gas), so they equal what a funded tx would use.
import { decodeErrorResult, parseAbi } from "viem"
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts"
import {
  BLOCKS_PER_DAY,
  BLOCKS_PER_MONTH,
  KIB,
  TX_GAS_CAP,
  attr,
  createCalldata,
  errText,
  extendCalldata,
  median,
  randomPayload,
  timed,
  to,
  extendBatchCalldata,
} from "./lib.mjs"

const ERRORS = parseAbi([
  "error NotOwner(bytes32 entityKey, address caller, address owner)",
  "error ExpiryBeyondMaxLifetime(uint64 target, uint64 currentBlock, uint64 maxLifetime)",
  "error TooManyAttributes(uint256 count, uint256 maxCount)",
  "error EntityExpired(bytes32 entityKey, uint64 expiresAt)",
  "error ExpiryNotExtended(bytes32 entityKey, uint64 newExpiresAt, uint64 currentExpiresAt)",
  "error ReadOnlyEntity(bytes32 entityKey)",
  "error InvalidValueType(bytes32 name, uint8 typeId)",
])
const hex = (n) => "0x" + n.toString(16)

/** Name of the typed revert in a viem error chain, else its first line. */
export function revertName(e) {
  const data = e?.walk?.((x) => typeof x?.data === "string" && x.data.startsWith("0x"))?.data
  try {
    if (data) return decodeErrorResult({ abi: ERRORS, data }).errorName
  } catch {}
  return errText(e)
}

const estimate = (c, from, data) => timed(() => c.estimateGas({ account: from, to, data }))

export async function chainProbes({ publicClient: c, report: r }) {
  const [clientVersion, head, gasPrice, timing] = await Promise.all([
    c.request({ method: "web3_clientVersion" }),
    c.getBlock(),
    c.getGasPrice(),
    c.request({ method: "arkiv_getBlockTiming" }),
  ])
  r.add("chain", "chainId", await c.getChainId(), "MEASURED")
  r.add("chain", "clientVersion", clientVersion, "MEASURED")
  r.add("chain", "head", Number(head.number), "MEASURED")
  r.add("chain", "gasPrice_wei", gasPrice, "MEASURED")
  r.add("chain", "baseFeePerGas_wei", head.baseFeePerGas, "MEASURED")
  r.add("chain", "arkiv_getBlockTiming", timing, "MEASURED")

  const nums = Array.from({ length: 100 }, (_, i) => head.number - BigInt(i))
  const blocks = await Promise.all(nums.map((n) => c.getBlock({ blockNumber: n, includeTransactions: true })))
  const dts = blocks.slice(0, -1).map((b, i) => Number(b.timestamp - blocks[i + 1].timestamp))
  const txs = blocks.flatMap((b) => b.transactions)
  r.add(
    "chain",
    "blockTime_s",
    { mean: dts.reduce((a, b) => a + b, 0) / dts.length, min: Math.min(...dts), max: Math.max(...dts) },
    "MEASURED",
  )
  r.add("chain", "gasLimit_last100", [...new Set(blocks.map((b) => Number(b.gasLimit)))], "MEASURED")
  r.add("chain", "maxGasUsed_last100", Math.max(...blocks.map((b) => Number(b.gasUsed))), "MEASURED")
  r.add("chain", "maxTxsPerBlock_last100", Math.max(...blocks.map((b) => b.transactions.length)), "MEASURED")
  r.add("chain", "maxTxInputBytes_last100", Math.max(0, ...txs.map((t) => (t.input.length - 2) / 2)), "MEASURED")
  return { head }
}

/** Largest raw tx the public RPC edge forwards. Signed with an always-unfunded ephemeral key: never mined. */
export async function edgeSizeProbe({ chain, report: r }) {
  const eph = privateKeyToAccount(generatePrivateKey())
  const url = chain.rpcUrls.default.http[0]
  const send = async (n) => {
    const data = createCalldata({ attributes: [attr.bytes("payload", randomPayload(n))], lifetime: BLOCKS_PER_DAY })
    const raw = await eph.signTransaction({
      type: "eip1559",
      chainId: chain.id,
      nonce: 0,
      to,
      data,
      gas: 16_000_000n,
      maxFeePerGas: 2_000_000_000n,
      maxPriorityFeePerGas: 1n,
      value: 0n,
    })
    const body = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "eth_sendRawTransaction", params: [raw] })
    const res = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body })
    const passed = res.status === 200 && (await res.text()).includes("insufficient funds") // node saw it, size check passed
    return { passed, body: body.length, encoded: (raw.length - 2) / 2 }
  }
  let lo = 100 * KIB,
    hi = 140 * KIB,
    best = await send(lo)
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1,
      t = await send(mid)
    if (t.passed) {
      lo = mid
      best = t
    } else hi = mid
  }
  r.add("limits", "maxSingleBytesAttr_throughRpc_bytes", lo, "MEASURED")
  r.add("limits", "maxRawTxEncoded_throughRpc_bytes", best.encoded, "MEASURED")
  r.add("limits", "jsonRpcBodyAtBoundary_bytes", best.body, "MEASURED")
  r.add("limits", "rejectedAbove", "HTTP 413 request body too large from the RPC edge (not the node)", "MEASURED")
}

export async function gasModelProbes({ publicClient: c, account, head, report: r }) {
  const from = account.address
  const create = (bytes, lifetime, extra = []) =>
    createCalldata({ attributes: [attr.str("tag", "spike", true), attr.bytes("payload", randomPayload(bytes)), ...extra], lifetime })
  const bySize = {}
  for (const kib of [1, 4, 16, 32, 64, 96, 126]) {
    const t = await estimate(c, from, create(kib * KIB, BLOCKS_PER_DAY))
    bySize[kib] = t.ok ? Number(t.value) : t.error
  }
  r.add("gas", "create_1day_gas_by_KiB", bySize, "MEASURED")
  const byLife = {}
  for (const [k, l] of [
    ["1d", BLOCKS_PER_DAY],
    ["30d", BLOCKS_PER_MONTH],
    ["90d", 3n * BLOCKS_PER_MONTH],
    ["max", 7_884_000n],
  ]) {
    const t = await estimate(c, from, create(16 * KIB, l))
    byLife[k] = t.ok ? Number(t.value) : t.error
  }
  r.add("gas", "create_16KiB_gas_by_lifetime", byLife, "MEASURED")
  const byLife64 = {}
  for (const [k, l] of [["1d", BLOCKS_PER_DAY], ["30d", BLOCKS_PER_MONTH], ["max", 7_884_000n]]) {
    const t = await estimate(c, from, create(64 * KIB, l))
    byLife64[k] = t.ok ? Number(t.value) : t.error
  }
  r.add("gas", "create_64KiB_gas_by_lifetime", byLife64, "MEASURED")
  const units64 = (byLife64.max - byLife64["1d"]) / ((7_884_000 - Number(BLOCKS_PER_DAY)) * 14e-6)
  r.add("gas", "derived_storage_units_per_64KiB_entity(capacity_rate=14)", { units: Math.round(units64), overheadVsPayload: Math.round(units64 - 64 * KIB) }, "MEASURED")
  const big = await estimate(c, from, create(126 * KIB, 7_884_000n))
  r.add("gas", "create_126KiB_maxLifetime_estimate", big.ok ? Number(big.value) : revertName({ details: big.error }), "MEASURED")
  r.add("gas", "create_126KiB_maxLifetime_exceeds_txGasCap", big.ok ? Number(big.value) > TX_GAS_CAP : null, "MEASURED")
  r.add("gas", "txGasCap", TX_GAS_CAP, "SOURCE-VERIFIED")

  // Derived coefficients: calldata part from the size sweep, storage part from the lifetime sweep.
  const calldataPerByte = (bySize[126] - bySize[16]) / (110 * KIB)
  const storagePerByteBlock = (byLife.max - byLife["1d"]) / ((7_884_000 - Number(BLOCKS_PER_DAY)) * 16 * KIB)
  const baseFee = Number(head.baseFeePerGas)
  const perMiB = { calldata_gas: calldataPerByte * 2 ** 20, storage_gas_per_30d: storagePerByteBlock * 2 ** 20 * Number(BLOCKS_PER_MONTH) }
  r.add("gas", "derived_gas_per_byte_calldata", calldataPerByte, "MEASURED")
  r.add("gas", "derived_storage_gas_per_byte_block", storagePerByteBlock, "MEASURED")
  r.add(
    "gas",
    "derived_perMiB_30d",
    {
      ...perMiB,
      tGLM_at_head_basefee: ((perMiB.calldata_gas + perMiB.storage_gas_per_30d) * baseFee) / 1e18,
      storageOnly_tGLM_per_MiB_month: (perMiB.storage_gas_per_30d * baseFee) / 1e18,
    },
    "MEASURED",
  )
}

export async function annotationLimitProbes({ publicClient: c, account, report: r }) {
  const from = account.address
  const many = (n) => Array.from({ length: n }, (_, i) => attr.u64(`a${String(i).padStart(2, "0")}`, i, true))
  const ok = async (data) => {
    const t = await estimate(c, from, data)
    return t.ok ? "ok" : revertName({ details: t.error })
  }
  const mk = (attributes) => createCalldata({ attributes, lifetime: BLOCKS_PER_DAY })
  r.add("limits", "attributes_32_indexed_u64", await ok(mk(many(32))), "MEASURED")
  r.add("limits", "attributes_33_indexed_u64", await ok(mk(many(33))), "MEASURED")
  r.add("limits", "str_128B_indexed", await ok(mk([attr.str("s", "x".repeat(128), true)])), "MEASURED")
  r.add("limits", "str_129B_indexed", await ok(mk([attr.str("s", "x".repeat(129), true)])), "MEASURED")
  r.add("limits", "name_32B", await ok(mk([attr.u64("n".repeat(32), 1, true)])), "MEASURED")
  r.add("limits", "name_33B", "not encodable: the ABI carries names as bytes32", "SOURCE-VERIFIED")
  const ctrl = await estimate(c, from, mk(many(32)))
  const noIdx = await estimate(c, from, mk(Array.from({ length: 32 }, (_, i) => attr.u64(`a${String(i).padStart(2, "0")}`, i, false))))
  r.add("gas", "32xu64_indexed_vs_unindexed_gas", { indexed: Number(ctrl.value), unindexed: Number(noIdx.value) }, "MEASURED")
}

/** Scans the first `pages` x 100 live entities; returns their system fields. */
export async function scanEntities(c, pages = 40) {
  const res = await Promise.all(
    Array.from({ length: pages }, (_, i) =>
      c
        .request({
          method: "arkiv_query",
          params: ["*", { select: { key: true, owner: true, expiresAt: true, creationFlags: true }, limit: "0x64", offset: hex(i * 100) }],
        })
        .then((x) => x.data)
        .catch(() => []),
    ),
  )
  return res.flat()
}

export async function extendProbes({ publicClient: c, account, report: r }) {
  const from = account.address
  const all = await scanEntities(c)
  const perm = all.filter((e) => e.creationFlags?.permissionlessExtension)
  const plain = all.find((e) => !e.creationFlags?.permissionlessExtension)
  r.add("extend", "scanned_entities", all.length, "MEASURED")
  r.add("extend", "permissionlessExtension_share", `${perm.length}/${all.length}`, "MEASURED")
  const ext = (list, plus) => extendBatchCalldata(list.map((e) => ({ entityKey: e.key, absolute: BigInt(e.expiresAt) + plus })))
  const gasOf = async (data) => {
    const t = await estimate(c, from, data)
    return t.ok ? Number(t.value) : revertName({ details: t.error })
  }
  const single = {}
  for (const plus of [10n, BLOCKS_PER_DAY]) single[`+${plus}blocks`] = await gasOf(ext([perm[0]], plus))
  r.add("extend", "single_permissionless_by_non_owner_gas", single, "MEASURED")
  const batch = {}
  for (const k of [1, 10, 50, Math.min(100, perm.length)]) batch[`${k}_entities`] = await gasOf(ext(perm.slice(0, k), 10n))
  r.add("extend", "batch_gas_by_entity_count", batch, "MEASURED")
  const t = await estimate(c, from, extendCalldata({ entityKey: plain.key, absolute: BigInt(plain.expiresAt) + 10n }))
  r.add("extend", "non_owner_on_plain_entity", t.ok ? "ok (unexpected)" : revertName({ details: t.error }), "MEASURED")
  r.add(
    "extend",
    "shorten_expiry",
    await (async () => {
      const x = await estimate(c, from, extendCalldata({ entityKey: perm[0].key, absolute: BigInt(perm[0].expiresAt) - 5n }))
      return x.ok ? "ok" : revertName({ details: x.error })
    })(),
    "MEASURED",
  )
}

export async function queryProbes({ publicClient: c, account, report: r }, { owner = account.address, tag } = {}) {
  const run = (query, opts) => timed(() => c.request({ method: "arkiv_query", params: [query, opts] }))
  const lat = async (name, query, opts) => {
    const ts = []
    let last
    for (let i = 0; i < 5; i++) {
      last = await run(query, opts)
      ts.push(last.ms)
    }
    r.add("query", `${name}_ms_median_of_5`, last.ok ? median(ts) : `ERR ${last.error}`, "MEASURED")
    return last
  }
  const total = await lat("count_all", "*", { select: { count: true } })
  if (total.ok) r.add("query", "entities_on_chain", Number(BigInt(total.value.count)), "MEASURED")
  await lat("page100_key_only", "*", { select: { key: true }, limit: "0x64" })
  await lat("owner_eq_count", `$owner = addr(${owner})`, { select: { count: true } })
  await lat("expiresAt_range", `$expiresAt BETWEEN u64(0) AND u64(99999999)`, { select: { count: true } })
  await lat("or_two_ranges", `$expiresAt < u64(100) OR $expiresAt > u64(7000000)`, { select: { count: true } })
  await lat("order_by_expiresAt_desc", "* ORDER BY $expiresAt DESC", { select: { key: true }, limit: "0xa" })
  if (tag) {
    await lat("annotation_eq_str", `tag = str('${tag}')`, { select: { count: true } })
    await lat("annotation_prefix", `tag STARTSWITH str('${tag.slice(0, 6)}')`, { select: { count: true } })
  }
  const expectErr = async (name, query, opts) => {
    const t = await run(query, opts)
    r.add("query", name, t.ok ? "accepted" : t.error, "MEASURED")
  }
  await expectErr("limit_101", "*", { select: { key: true }, limit: "0x65" })
  await expectErr("NOT_complement", "NOT ($expiresAt > u64(5))", { select: { count: true } })
  await expectErr("not_equal_operator", "$expiresAt != u64(5)", { select: { count: true } })
  await expectErr("atBlock_old", "*", { select: { count: true }, atBlock: "0x1" })
  await expectErr("atBlock_head_minus_1", "*", { select: { count: true }, atBlock: hex((await c.getBlockNumber()) - 1n) })
  await expectErr("offset_past_end", "*", { select: { key: true }, limit: "0x1", offset: hex(10n ** 12n) })
}

/** Smart-contract surface and block-history retention. */
export async function contractAndHistoryProbes({ publicClient: c, account, report: r }) {
  const from = account.address
  const initCode = "0x6080604052348015600f57600080fd5b50603f80601d6000396000f3fe6080604052600080fdfea164736f6c6343000800000a"
  const create = await timed(() => c.estimateGas({ account: from, data: initCode }))
  r.add("contracts", "contract_creation", create.ok ? "accepted (unexpected)" : create.error, "MEASURED")
  const call = await timed(() => c.estimateGas({ account: from, to: "0x000000000000000000000000000000000000dEaD", data: "0x12345678" }))
  r.add("contracts", "call_to_non_arkiv_address_with_data_gas", call.ok ? Number(call.value) : call.error, "MEASURED")
  const old = await timed(() => c.getBlock({ blockNumber: 1n, includeTransactions: true }))
  r.add("history", "block_1_still_served", old.ok ? { hash: old.value.hash, txs: old.value.transactions.length } : old.error, "MEASURED")
}

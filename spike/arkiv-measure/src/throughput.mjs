// Throughput burst: N pre-signed creates of `kib` KiB from ONE account, sent concurrently with explicit nonces.
// Rejections (per-account txpool slots, nonce gaps) are retried in later rounds; receipts give bytes per block.
import { BLOCKS_PER_DAY, KIB, attr, createCalldata, errText, median, randomPayload, sleep, tglm, to } from "./lib.mjs"

const PER_TX_WEI_24KIB = 1_300_000_000_000_000n // conservative fee per 24 KiB create at ~1 gwei

export async function throughputProbe({ publicClient: c, account, chain, report: r, run, spent, addSpend, budget, n: nWanted, kib = 24 }) {
  const balance = await c.getBalance({ address: account.address })
  const sample = createCalldata({
    attributes: [attr.str("tag", run, true), attr.bytes("payload", randomPayload(kib * KIB))],
    lifetime: BLOCKS_PER_DAY,
  })
  const est = await c.estimateGas({ account: account.address, to, data: sample })
  const gas = (est * 13n) / 10n
  const PER_TX_WEI = kib === 24 ? PER_TX_WEI_24KIB : gas * 1_000_000_000n // 1 gwei is the base-fee floor
  const n = nWanted ?? Math.max(4, Math.min(20, Number((balance - 15_000_000_000_000_000n) / PER_TX_WEI)))
  if (spent() + BigInt(n) * PER_TX_WEI > budget) {
    r.add("throughput", "skipped", "spend cap", "MEASURED")
    return
  }
  const nonce0 = await c.getTransactionCount({ address: account.address, blockTag: "pending" })
  const raws = await Promise.all(
    Array.from({ length: n }, async (_, i) => {
      const data = createCalldata({
        attributes: [attr.str("tag", run, true), attr.bytes("payload", randomPayload(kib * KIB))],
        lifetime: BLOCKS_PER_DAY,
      })
      return account.signTransaction({
        type: "eip1559",
        chainId: chain.id,
        nonce: nonce0 + i,
        to,
        data,
        gas,
        maxFeePerGas: 3_000_000_000n,
        maxPriorityFeePerGas: 1n,
        value: 0n,
      })
    }),
  )
  const hashes = new Array(n).fill(null)
  const accepted = []
  const errors = new Set()
  let pending = raws.map((_, i) => i)
  const t0 = Date.now()
  const headBefore = await c.getBlockNumber()
  for (let round = 0; round < 6 && pending.length; round++) {
    const res = await Promise.allSettled(pending.map((i) => c.sendRawTransaction({ serializedTransaction: raws[i] })))
    const next = []
    res.forEach((x, k) => {
      if (x.status === "fulfilled") hashes[pending[k]] = x.value
      else {
        errors.add(errText(x.reason))
        next.push(pending[k])
      }
    })
    accepted.push(pending.length - next.length)
    pending = next
    if (pending.length) await sleep(4000)
  }
  const sent = hashes.filter(Boolean)
  const receipts = await Promise.all(sent.map((hash) => c.waitForTransactionReceipt({ hash, timeout: 120_000 }).catch(() => null)))
  const wall = (Date.now() - t0) / 1000
  const ok = receipts.filter((x) => x?.status === "success")
  const byBlock = {}
  for (const x of ok) {
    const b = Number(x.blockNumber)
    byBlock[b] ??= { txs: 0, gas: 0 }
    byBlock[b].txs++
    byBlock[b].gas += Number(x.gasUsed)
  }
  const blocks = Object.keys(byBlock).map(Number)
  const span = blocks.length ? Math.max(...blocks) - Math.min(...blocks) + 1 : 0
  const fee = ok.reduce((a, x) => a + x.gasUsed * x.effectiveGasPrice, 0n)
  const bytesTotal = ok.length * kib * KIB
  addSpend(fee)
  r.add(
    "throughput",
    "burst",
    {
      requested: n,
      payloadKiB: kib,
      confirmed: ok.length,
      acceptedPerRound: accepted,
      rejectionMessages: [...errors],
      firstBlock: Math.min(...blocks),
      blocksSpanned: span,
      perBlock: byBlock,
      headBeforeSend: Number(headBefore),
      bytesConfirmed: bytesTotal,
      wallSeconds: wall,
      bytesPerSecondWall: Math.round(bytesTotal / wall),
      bytesPerSecondChain: span ? Math.round(bytesTotal / (span * 2)) : 0,
      maxBytesInOneBlock: Math.max(0, ...Object.values(byBlock).map((b) => b.txs * kib * KIB)),
      medianGasPerTx: median(ok.map((x) => Number(x.gasUsed))),
      feeTotal_tGLM: tglm(fee),
    },
    "MEASURED",
  )
  return ok.map((x) => x.transactionHash)
}

// Per-account pool limit: N self-transfers (21,000 gas, the cheapest tx) pre-signed with consecutive nonces and sent at once.
// Rejected ones are re-sent in later rounds so no nonce gap is left behind; the first-round accept count is the pool's limit.
export async function poolLimitProbe({ publicClient: c, account, chain, report: r, n, spent, addSpend, budget }) {
  const fee = 21_000n * 1_000_000_000n
  if (spent() + BigInt(n) * fee > budget) return r.add("throughput", "pool_limit_skipped", "spend cap", "MEASURED")
  const nonce0 = await c.getTransactionCount({ address: account.address, blockTag: "pending" })
  const raws = await Promise.all(
    Array.from({ length: n }, (_, i) =>
      account.signTransaction({ type: "eip1559", chainId: chain.id, nonce: nonce0 + i, to: account.address, value: 0n, gas: 21_000n, maxFeePerGas: 3_000_000_000n, maxPriorityFeePerGas: 1n }),
    ),
  )
  const hashes = new Array(n).fill(null)
  const errors = new Map()
  const accepted = []
  let pending = raws.map((_, i) => i)
  const head0 = await c.getBlockNumber()
  for (let round = 0; round < 14 && pending.length; round++) {
    const res = await Promise.allSettled(pending.map((i) => c.sendRawTransaction({ serializedTransaction: raws[i] })))
    const next = []
    res.forEach((x, k) => {
      if (x.status === "fulfilled") hashes[pending[k]] = x.value
      else {
        const m = errText(x.reason)
        errors.set(m, (errors.get(m) ?? 0) + 1)
        next.push(pending[k])
      }
    })
    accepted.push(pending.length - next.length)
    pending = next
    if (pending.length) await sleep(4000)
  }
  const receipts = await Promise.all(hashes.filter(Boolean).map((hash) => c.waitForTransactionReceipt({ hash, timeout: 90_000 }).catch(() => null)))
  const ok = receipts.filter((x) => x?.status === "success")
  const byBlock = {}
  for (const x of ok) byBlock[Number(x.blockNumber)] = (byBlock[Number(x.blockNumber)] ?? 0) + 1
  addSpend(ok.reduce((a, x) => a + x.gasUsed * x.effectiveGasPrice, 0n))
  r.add(
    "throughput",
    "pool_limit_self_transfers",
    {
      requested: n,
      acceptedPerRound: accepted,
      rejections: Object.fromEntries(errors),
      confirmed: ok.length,
      txsPerBlock: byBlock,
      maxTxsInOneBlock: Math.max(0, ...Object.values(byBlock)),
      headBeforeSend: Number(head0),
      fee_tGLM: tglm(ok.reduce((a, x) => a + x.gasUsed * x.effectiveGasPrice, 0n)),
    },
    "MEASURED",
  )
}

// Throughput burst: N pre-signed creates of `kib` KiB from ONE account, sent concurrently with explicit nonces.
// Rejections (per-account txpool slots, nonce gaps) are retried in later rounds; receipts give bytes per block.
import { BLOCKS_PER_DAY, KIB, attr, createCalldata, errText, median, randomPayload, sleep, tglm, to } from "./lib.mjs"

const PER_TX_WEI = 1_300_000_000_000_000n // conservative fee per 24 KiB create at ~1 gwei

export async function throughputProbe({ publicClient: c, account, chain, report: r, run, spent, addSpend, budget }) {
  const kib = 24
  const balance = await c.getBalance({ address: account.address })
  const n = Math.max(4, Math.min(32, Number((balance - 15_000_000_000_000_000n) / PER_TX_WEI)))
  if (spent() + BigInt(n) * PER_TX_WEI > budget) {
    r.add("throughput", "skipped", "spend cap", "MEASURED")
    return
  }
  const sample = createCalldata({
    attributes: [attr.str("tag", run, true), attr.bytes("payload", randomPayload(kib * KIB))],
    lifetime: BLOCKS_PER_DAY,
  })
  const gas = ((await c.estimateGas({ account: account.address, to, data: sample })) * 13n) / 10n
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

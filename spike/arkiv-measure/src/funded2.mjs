// Second funded pass: the sections that hung in the first pass (sponsor extend, patch semantics, expiry, queries).
// Root cause of the first-pass hangs: the anonymous query budget (10,000 cost units per window, 10 per arkiv_query) was
// exhausted, so every SDK read blocked on HTTP 429. This pass gates each read section on a raw budget check, logs every
// sub-call with its latency, and never deletes (no refund exists; entities just expire after their 1-day lifetime).
import { ExpirationTime } from "@arkiv-network/sdk"
import { eq } from "@arkiv-network/sdk/query"
import { bytes, defineSchema, key as keyVal, str, u64 } from "@arkiv-network/sdk/schema"
import { generatePrivateKey } from "viem/accounts"
import { bytesToHex, hexToBytes } from "viem"
import { BLOCKS_PER_DAY, KIB, errText, makeClients, randomPayload, sleep, timed, tglm, to, extendCalldata } from "./lib.mjs"
import { queryProbes } from "./readonly.mjs"
import { poolLimitProbe, throughputProbe } from "./throughput.mjs"

const MAX_SPEND_WEI = 24_000_000_000_000_000n // 0.024: coordinator total 0.08, 0.0533 already spent by pass one and the aborted run
const SPONSOR_FUND_WEI = 400_000_000_000_000n // 0.0004 tGLM, stranded afterwards (the ephemeral key is never saved)
const URL = process.env.ARKIV_RPC ?? "https://rpc.rabdi.db-chain.testnet.arkiv.network"

/** Raw arkiv_query that reports the status and the rate-limit headers instead of hiding a 429 behind retries. */
async function rawQuery(query, options) {
  const t0 = performance.now()
  const res = await fetch(URL, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "arkiv_query", params: [query, options] }),
  })
  const body = await res.json().catch(() => ({}))
  return {
    status: res.status,
    ms: Math.round(performance.now() - t0),
    cost: res.headers.get("arkiv-cost"),
    quotaUsedPct: res.headers.get("arkiv-quota-used-percent"),
    ratelimit: res.headers.get("ratelimit"),
    retryAfter: res.headers.get("retry-after"),
    error: body.error ?? null,
    result: body.result ?? null,
  }
}

/** Waits (bounded) until reads are served again; returns the last probe so the budget state is recorded. */
async function waitForReads(maxWaitMs = 12 * 60_000) {
  const t0 = Date.now()
  for (;;) {
    const p = await rawQuery("*", { select: { count: true } })
    if (p.status === 200) return p
    const wait = Math.min(Number(p.retryAfter ?? 30), 120) * 1000 + 2000
    console.log(`reads blocked (${p.status} ${JSON.stringify(p.error)}), retry-after ${p.retryAfter}s`)
    if (Date.now() - t0 + wait > maxWaitMs) return p
    await sleep(wait)
  }
}

export async function runFunded2(ctx) {
  const { publicClient: c, walletClient: w, account, report: r } = ctx
  const run = `p2-${Date.now().toString(36)}`
  const Spike = defineSchema("arkiv-git.spike.p0", { tag: str().indexed(), seq: u64().indexed(), payload: bytes() })
  let spent = 0n
  const track = async (txHash, label) => {
    const rc = await c.waitForTransactionReceipt({ hash: txHash, timeout: 60_000 })
    const fee = rc.gasUsed * rc.effectiveGasPrice
    spent += fee
    if (spent > MAX_SPEND_WEI) throw new Error(`spend cap hit: ${tglm(spent)} tGLM`)
    return { label, block: Number(rc.blockNumber), gasUsed: Number(rc.gasUsed), fee_tGLM: tglm(fee), status: rc.status }
  }
  const log = (m) => console.log(`   ${new Date().toISOString().slice(11, 19)} ${m}`)
  const write = async (label, fn) => {
    const x = await fn()
    log(`${label}: tx sent ${x.txHash}`)
    return { ...x, ...(await track(x.txHash, label)) }
  }
  const mkVals = (kib, seq = 1n) => ({ tag: run, seq: u64(seq), payload: hexToBytes(randomPayload(kib * KIB)) })
  const create = (label, kib, opts = { expiration: ExpirationTime.fromDays(1) }) => write(label, () => w.createEntity(Spike, mkVals(kib), opts))
  const step = async (name, fn, ms = 180_000) => {
    console.log(`-- step ${name}`)
    let timer
    try {
      await Promise.race([fn(), new Promise((_, rej) => (timer = setTimeout(() => rej(new Error(`step timed out after ${ms} ms`)), ms)))])
    } catch (e) {
      r.add("error", name, errText(e), "MEASURED")
    } finally {
      clearTimeout(timer)
    }
  }
  const first = await waitForReads()
  console.log(`reads: status ${first.status} cost ${first.cost} quotaUsedPct ${first.quotaUsedPct} ratelimit ${first.ratelimit}`)
  const only = process.env.ONLY?.split(",")
  const want = (n) => !only || only.includes(n)

  if (want("sponsor"))
    await step("sponsor", async () => {
      const perm = await create("create_4KiB_permissionless", 4, { expiration: ExpirationTime.fromDays(1), flags: { permissionlessExtension: true } })
      const plain = await create("create_4KiB_plain", 4)
      const sponsor = makeClients(generatePrivateKey()) // ephemeral, never persisted
      const fund = await track(await w.sendTransaction({ to: sponsor.account.address, value: SPONSOR_FUND_WEI }), "fund_sponsor")
      r.add("extend", "plain_transfer_gas", fund.gasUsed, "MEASURED")
      // Raw path so each sub-call is visible: estimate (unfunded-capable), send, receipt.
      const sponsorExtend = async (entityKey, label) => {
        const data = extendCalldata({ entityKey, lifetime: 2n * BLOCKS_PER_DAY })
        const est = await timed(() => sponsor.publicClient.estimateGas({ account: sponsor.account.address, to, data }))
        log(`${label}: estimate ${est.ok ? est.value : est.error} in ${est.ms} ms`)
        if (!est.ok) return { ok: false, stage: "estimate", error: est.error }
        const sent = await timed(() => sponsor.walletClient.sendTransaction({ to, data, gas: (est.value * 13n) / 10n }))
        log(`${label}: send ${sent.ok ? sent.value : sent.error} in ${sent.ms} ms`)
        if (!sent.ok) return { ok: false, stage: "send", error: sent.error }
        const rc = await track(sent.value, label)
        return { ok: rc.status === "success", stage: "receipt", ...rc }
      }
      const a = await sponsorExtend(perm.entityKey, "sponsor_extend_flagged")
      r.add("extend", "non_owner_extend_permissionless", a, "MEASURED")
      const b = await sponsorExtend(plain.entityKey, "sponsor_extend_plain")
      r.add("extend", "non_owner_extend_plain", b, "MEASURED")
      // Same non-owner extension through the SDK, bounded, to tell an SDK problem from a node or rate-limit problem.
      const perm2 = await create("create_1KiB_permissionless_sdk", 1, { expiration: ExpirationTime.fromDays(1), flags: { permissionlessExtension: true } })
      const viaSdk = await timed(() =>
        Promise.race([
          sponsor.walletClient.extendEntity({ entityKey: perm2.entityKey, expiration: ExpirationTime.fromDays(2) }),
          sleep(60_000).then(() => {
            throw new Error("sdk sponsor extend still pending after 60 s")
          }),
        ]),
      )
      r.add("extend", "non_owner_extend_via_sdk", viaSdk.ok ? { ok: true, ms: viaSdk.ms, newExpiresAt: Number(viaSdk.value.expiresAt) } : viaSdk.error, "MEASURED")
      const shorter = await timed(() => w.extendEntity({ entityKey: perm.entityKey, expiration: ExpirationTime.fromBlocks(10) }))
      r.add("extend", "owner_shorten", shorter.ok ? "accepted (unexpected)" : shorter.error, "MEASURED")
      const e16 = await create("create_16KiB_for_extend", 16)
      const ext = await write("extend_16KiB", () => w.extendEntity({ entityKey: e16.entityKey, expiration: ExpirationTime.fromDays(2) }))
      r.add("extend", "16KiB_1d_to_2d", { gasUsed: ext.gasUsed, fee_tGLM: ext.fee_tGLM }, "MEASURED")
      const after = await rawQuery(`$key = key(${perm.entityKey})`, { select: { key: true, owner: true, expiresAt: true } })
      r.add("extend", "flagged_entity_after_sponsor_extend", after.result ?? after.error, "MEASURED")
    })

  if (want("update"))
    await step("update", async () => {
      const u = await create("create_update_target", 2)
      const read = async () => {
        const t = await timed(() =>
          c.select({ $key: true, $createdAt: true, $updatedAt: true, $expiresAt: true, seq: true, payload: true }).from(Spike).where(eq("$key", keyVal(u.entityKey))).fetch(),
        )
        log(`read in ${t.ms} ms ${t.ok ? "" : t.error}`)
        return t.ok ? t.value.entities[0] : null
      }
      const before = await read()
      const newPayload = hexToBytes(randomPayload(3 * KIB))
      const patch = await write("patch_2KiB_to_3KiB", () => w.patchEntity(Spike, u.entityKey, { set: { seq: u64(2n), payload: newPayload } }))
      const post = await read()
      const shrink = await write("patch_3KiB_to_1KiB", () => w.patchEntity(Spike, u.entityKey, { set: { payload: hexToBytes(randomPayload(KIB)) } }))
      const q2 = await rawQuery(`seq = u64(2) AND tag = str('${run}')`, { select: { count: true } })
      r.add(
        "update",
        "patch_semantics",
        {
          sameKey: post?.$key === u.entityKey,
          expiryBefore: Number(before?.$expiresAt),
          expiryAfter: Number(post?.$expiresAt),
          expiryUnchanged: post?.$expiresAt === before?.$expiresAt,
          updatedAtBefore: Number(before?.$updatedAt),
          updatedAtAfter: Number(post?.$updatedAt),
          payloadReplaced: post?.payload?.value === bytesToHex(newPayload),
          seqNow: String(post?.seq?.value),
          indexFollowsPatch_count_seq2: q2.result ? Number(BigInt(q2.result.count)) : q2.error,
          patch_2to3KiB: { gasUsed: patch.gasUsed, fee_tGLM: patch.fee_tGLM },
          patch_shrink_3to1KiB: { gasUsed: shrink.gasUsed, fee_tGLM: shrink.fee_tGLM },
        },
        "MEASURED",
      )
    })

  if (want("expiry"))
    await step(
      "expiry",
      async () => {
        const short = await create("create_short_lived", 1, { expiration: ExpirationTime.fromBlocks(20) })
        const expiresAt = Number(short.expiresAt)
        log(`short entity expires at block ${expiresAt}, created in block ${short.block}`)
        const polls = []
        let gone
        const t0 = Date.now()
        while (Date.now() - t0 < 120_000) {
          const [head, q] = await Promise.all([c.getBlockNumber(), rawQuery(`$key = key(${short.entityKey})`, { select: { count: true } })])
          const count = q.result ? Number(BigInt(q.result.count)) : null
          polls.push({ head: Number(head), count, status: q.status })
          if (count === 0 && Number(head) >= expiresAt - 1) {
            gone = { goneObservedAtBlock: Number(head), blocksAfterExpiresAt: Number(head) - expiresAt }
            break
          }
          await sleep(3000)
        }
        const firstZero = polls.find((p) => p.count === 0)
        const lastOne = [...polls].reverse().find((p) => p.count === 1)
        const getAfter = await timed(() => c.getEntity(short.entityKey, Spike))
        const extAfter = await timed(() => w.extendEntity({ entityKey: short.entityKey, expiration: ExpirationTime.fromDays(1) }))
        const txStill = await timed(() => c.getTransaction({ hash: short.txHash }))
        r.add(
          "expiry",
          "short_lived",
          {
            lifetimeBlocks: 20,
            createdBlock: short.block,
            expiresAt,
            ...(gone ?? { goneObservedAtBlock: "timeout" }),
            polls: polls.length,
            lastSeenAliveAtHead: lastOne?.head,
            firstSeenGoneAtHead: firstZero?.head,
            getEntityAfter: getAfter.ok ? "found (unexpected)" : getAfter.error,
            extendAfter: extAfter.ok ? "accepted (unexpected)" : extAfter.error,
            createTxCalldataStillOnChain: txStill.ok ? `${(txStill.value.input.length - 2) / 2} bytes` : txStill.error,
          },
          "MEASURED",
        )
      },
      240_000,
    )

  if (want("queries"))
    await step("queries", async () => {
      const probe = await waitForReads()
      r.add("query", "budget_state_before_probes", { status: probe.status, cost: probe.cost, quotaUsedPct: probe.quotaUsedPct, ratelimit: probe.ratelimit }, "MEASURED")
      await queryProbes(ctx, { owner: account.address, tag: run })
    })

  if (want("poolLimit"))
    await step(
      "poolLimit",
      () => poolLimitProbe({ ...ctx, report: r, n: Number(process.env.POOL_N ?? 200), spent: () => spent, addSpend: (f) => (spent += f), budget: MAX_SPEND_WEI }),
      300_000,
    )

  if (want("burst"))
    await step(
      "burst",
      () =>
        throughputProbe({ ...ctx, run, spent: () => spent, addSpend: (f) => (spent += f), budget: MAX_SPEND_WEI, n: Number(process.env.BURST_N ?? 48), kib: Number(process.env.BURST_KIB ?? 1) }),
      240_000,
    )

  r.add("fee", "pass2_spent_tGLM", tglm(spent), "MEASURED")
}

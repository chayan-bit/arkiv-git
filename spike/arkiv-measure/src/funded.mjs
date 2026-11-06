// Funded probes: real writes on the Rabdi testnet with the throwaway key. Spend is tracked from receipts and
// capped; entities are deleted at the end. Uses the official TypeScript SDK; the raw path (lib.mjs) is used
// only where the SDK refuses before the node does (max-size create) and for the pre-signed throughput burst.
import { ExpirationTime } from "@arkiv-network/sdk"
import { desc, eq, gte, startsWith } from "@arkiv-network/sdk/query"
import { bytes, defineSchema, key as keyVal, str, u64 } from "@arkiv-network/sdk/schema"
import { generatePrivateKey } from "viem/accounts"
import { bytesToHex, hexToBytes } from "viem"
import { BLOCKS_PER_DAY, KIB, attr, createCalldata, errText, makeClients, randomPayload, sleep, timed, tglm, to } from "./lib.mjs"
import { queryProbes } from "./readonly.mjs"
import { throughputProbe } from "./throughput.mjs"

export const MIN_BALANCE_WEI = 40_000_000_000_000_000n // 0.04 tGLM: ladder + extend + patch + expiry + a small burst
const MAX_SPEND_WEI = 60_000_000_000_000_000n // hard stop for this run: 0.0163 tGLM went on an aborted first run, coordinator total budget is 0.08
const MAX_RAW_PAYLOAD = 130_336 // largest single bytes attr that passed the RPC edge (see readonly.mjs edgeSizeProbe)

export async function runFunded(ctx) {
  const { publicClient: c, walletClient: w, account, report: r } = ctx
  const run = `p0-${Date.now().toString(36)}`
  const Spike = defineSchema("arkiv-git.spike.p0", { tag: str().indexed(), seq: u64().indexed(), payload: bytes() })
  const created = []
  let spent = 0n
  const track = async (txHash, label) => {
    const rc = await c.waitForTransactionReceipt({ hash: txHash, timeout: 60_000 })
    const fee = rc.gasUsed * rc.effectiveGasPrice
    spent += fee
    if (spent > MAX_SPEND_WEI) throw new Error(`spend cap hit: ${tglm(spent)} tGLM`)
    return {
      label,
      block: Number(rc.blockNumber),
      gasUsed: Number(rc.gasUsed),
      effectiveGasPrice: Number(rc.effectiveGasPrice),
      fee_tGLM: tglm(fee),
      status: rc.status,
    }
  }
  const write = async (label, fn) => {
    const x = await fn()
    return { ...x, ...(await track(x.txHash, label)) }
  }
  const mkVals = (kib, seq = 1n) => ({ tag: run, seq: u64(seq), payload: hexToBytes(randomPayload(kib * KIB)) })
  const create = (label, kib, opts = { expiration: ExpirationTime.fromDays(1) }) =>
    write(label, async () => {
      const x = await w.createEntity(Spike, mkVals(kib), opts)
      created.push(x.entityKey)
      return x
    })

  // Each section is isolated: one failure is reported and the rest still run.
  const st = {}
  const step = async (name, fn, ms = 240_000) => {
    console.log(`-- step ${name}`)
    let timer
    try {
      await Promise.race([fn(), new Promise((_, rej) => { timer = setTimeout(() => rej(new Error(`step timed out after ${ms} ms`)), ms) })])
    } catch (e) {
      r.add("error", name, errText(e), "MEASURED")
    } finally {
      clearTimeout(timer)
    }
  }

  try {
    await step("ladder", async () => {
      // 1. Size ladder: fee per size at a 1-day lifetime.
      const ladder = []
      for (const kib of [1, 4, 16, 64]) {
        ladder.push({ kib, ...(await create(`create_${kib}KiB`, kib)) })
        st.ladder = ladder
      }
      r.add(
        "fee",
        "create_1day_by_size",
        ladder.map(({ kib, gasUsed, fee_tGLM, block, expiresAt }) => ({
          kib,
          gasUsed,
          fee_tGLM,
          lifetime_blocks: Number(expiresAt) - block,
        })),
        "MEASURED",
      )
    })
    await step("estimateVsReceipt", async () => {
      // Same raw create estimated, then mined: compares eth_estimateGas with the gas the receipt shows.
      const rows = []
      for (const kib of [4, 16, 64]) {
        const data = createCalldata({ attributes: [attr.str("tag", run, true), attr.bytes("payload", randomPayload(kib * KIB))], lifetime: BLOCKS_PER_DAY })
        const est = await c.estimateGas({ account: account.address, to, data })
        const rc = await track(await w.sendTransaction({ to, data, gas: (est * 12n) / 10n }), `raw_create_${kib}KiB`)
        rows.push({ kib, estimate: Number(est), gasUsed: rc.gasUsed, diff: rc.gasUsed - Number(est), fee_tGLM: rc.fee_tGLM, effectiveGasPrice: rc.effectiveGasPrice })
      }
      r.add("fee", "estimate_vs_receipt", rows, "MEASURED")
    })
    await step("maxCreate", async () => {
      // 2. Largest create the RPC edge lets through (single bytes attribute, raw path).
      // One bytes attribute only, 64 B under the edge boundary found by edgeSizeProbe (tx fee/nonce fields vary a little).
      // No estimateGas here: its request body would hit the same 256 KiB edge limit. 6M gas is above the 5.4M a 127 KiB create needs.
      const size = MAX_RAW_PAYLOAD - 64
      const data = createCalldata({ attributes: [attr.bytes("payload", randomPayload(size))], lifetime: BLOCKS_PER_DAY })
      const big = await timed(async () => track(await w.sendTransaction({ to, data, gas: 6_000_000n }), "create_max_raw"))
      r.add("limits", "max_create_on_chain", big.ok ? { payload_bytes: size, ...big.value } : big.error, "MEASURED")
    })
    await step("extend", async () => {
      // 3. Extension: owner, then a non-owner on a permissionlessExtension entity, then a non-owner on a plain one.
      const perm = await create("create_4KiB_permissionless", 4, {
        expiration: ExpirationTime.fromDays(1),
        flags: { permissionlessExtension: true },
      })
      const plain = await create("create_4KiB_plain", 4)
      const ownerExt = await write("extend_owner_to_2d", () =>
        w.extendEntity({ entityKey: perm.entityKey, expiration: ExpirationTime.fromDays(2) }),
      )
      r.add(
        "extend",
        "owner_extend_1d_to_2d",
        {
          gasUsed: ownerExt.gasUsed,
          fee_tGLM: ownerExt.fee_tGLM,
          newExpiresAt: Number(ownerExt.expiresAt),
          landed_block: ownerExt.block,
          blocks_from_landing: Number(ownerExt.expiresAt) - ownerExt.block,
        },
        "MEASURED",
      )
      const sponsor = makeClients(generatePrivateKey()) // ephemeral second account, never persisted
      const fund = await track(await w.sendTransaction({ to: sponsor.account.address, value: 3_000_000_000_000_000n }), "fund_sponsor")
      r.add("extend", "plain_transfer_gas", fund.gasUsed, "MEASURED")
      const sp = await timed(async () => {
        const x = await sponsor.walletClient.extendEntity({ entityKey: perm.entityKey, expiration: ExpirationTime.fromDays(3) })
        return { ...x, ...(await track(x.txHash, "extend_sponsor")) }
      })
      const after = await c
        .select({ $key: true, $owner: true })
        .from(Spike)
        .where(eq("$key", keyVal(perm.entityKey)))
        .fetch()
      const owner = after.entities[0]?.$owner
      r.add(
        "extend",
        "non_owner_extend_permissionless",
        sp.ok
          ? {
              ok: true,
              gasUsed: sp.value.gasUsed,
              paid_by_sponsor_tGLM: sp.value.fee_tGLM,
              ownerStillCreator: owner?.toLowerCase() === account.address.toLowerCase(),
            }
          : sp.error,
        "MEASURED",
      )
      const sp2 = await timed(() =>
        sponsor.walletClient.extendEntity({ entityKey: plain.entityKey, expiration: ExpirationTime.fromDays(3) }),
      )
      r.add("extend", "non_owner_extend_plain", sp2.ok ? "accepted (unexpected)" : sp2.error, "MEASURED")
      const shorter = await timed(() => w.extendEntity({ entityKey: perm.entityKey, expiration: ExpirationTime.fromDays(1) }))
      r.add("extend", "owner_shorten", shorter.ok ? "accepted (unexpected)" : shorter.error, "MEASURED")

      // 3b. Extension cost vs the formula: 16 KiB entity, +1 day.
      const e16 = st.ladder?.find((l) => l.kib === 16)
      if (e16) {
        const ext16 = await write("extend_16KiB_to_2d", () =>
          w.extendEntity({ entityKey: e16.entityKey, expiration: ExpirationTime.fromDays(2) }),
        )
        r.add("extend", "16KiB_1d_to_2d", { gasUsed: ext16.gasUsed, fee_tGLM: ext16.fee_tGLM }, "MEASURED")
      }
    })
    await step("update", async () => {
      // 4. Update semantics (patch replaces attributes whole; the key and expiry stay).
      const u = await create("create_update_target", 2)
      const read = async () =>
        (
          await c
            .select({ $key: true, $createdAt: true, $updatedAt: true, $expiresAt: true, seq: true, payload: true })
            .from(Spike)
            .where(eq("$key", keyVal(u.entityKey)))
            .fetch()
        ).entities[0]
      const before = await read()
      const newPayload = hexToBytes(randomPayload(3 * KIB))
      const patch = await write("patch_2KiB_to_3KiB", () =>
        w.patchEntity(Spike, u.entityKey, { set: { seq: u64(2n), payload: newPayload } }),
      )
      const post = await read()
      const shrink = await write("patch_3KiB_to_1KiB", () =>
        w.patchEntity(Spike, u.entityKey, { set: { payload: hexToBytes(randomPayload(KIB)) } }),
      )
      const q2 = await c
        .count()
        .from(Spike)
        .where(eq("seq", u64(2n)))
        .where(eq("tag", run))
        .fetch()
      r.add(
        "update",
        "patch_semantics",
        {
          sameKey: post?.$key === u.entityKey,
          expiryUnchanged: post?.$expiresAt === before?.$expiresAt,
          updatedAtMovedPastCreatedAt: post && before ? post.$updatedAt > before.$updatedAt : null,
          payloadReplaced: post?.payload?.value === bytesToHex(newPayload),
          seqNow: String(post?.seq?.value),
          indexFollowsPatch_count_seq2: Number(q2),
          patch_2to3KiB: { gasUsed: patch.gasUsed, fee_tGLM: patch.fee_tGLM },
          patch_shrink_3to1KiB: { gasUsed: shrink.gasUsed, fee_tGLM: shrink.fee_tGLM },
        },
        "MEASURED",
      )
    })
    await step("expiry", async () => {
      // 5. Expiry: a 20-block entity, polled until the query surface drops it.
      const short = await create("create_short_lived", 1, { expiration: ExpirationTime.fromBlocks(20) })
      const gone = await waitGone(c, short.entityKey, Number(short.expiresAt))
      created.splice(created.indexOf(short.entityKey), 1) // already purged: a delete of it would revert the whole cleanup batch
      const getAfter = await timed(() => c.getEntity(short.entityKey, Spike))
      const extAfter = await timed(() => w.extendEntity({ entityKey: short.entityKey, expiration: ExpirationTime.fromDays(1) }))
      const txStill = await timed(() => c.getTransaction({ hash: short.txHash }))
      r.add(
        "expiry",
        "short_lived",
        {
          expiresAt: Number(short.expiresAt),
          ...gone,
          getEntityAfter: getAfter.ok ? "found (unexpected)" : getAfter.error,
          extendAfter: extAfter.ok ? "accepted (unexpected)" : extAfter.error,
          createTxCalldataStillOnChain: txStill.ok ? `${(txStill.value.input.length - 2) / 2} bytes` : txStill.error,
        },
        "MEASURED",
      )
    })
    // 6. Throughput: pre-signed parallel burst from one account.
    await step("throughput", () =>
      throughputProbe({
        ...ctx,
        run,
        spent: () => spent,
        addSpend: (fee) => {
          spent += fee
        },
        budget: MAX_SPEND_WEI,
      }),
    )
    await step("queries", async () => {
      // 7. Annotation queries on our own data (SDK builder + raw latency).
      await queryProbes(ctx, { owner: account.address, tag: run })
      const sdkQ = await timed(() =>
        c
          .select({ $key: true, seq: true })
          .from(Spike)
          .ownedBy(account.address)
          .where(eq("tag", run))
          .where(gte("seq", u64(1n)))
          .orderBy(desc("seq"))
          .limit(5)
          .fetch(),
      )
      r.add(
        "query",
        "sdk_builder_owner_tag_range_order_ms",
        sdkQ.ok ? { ms: sdkQ.ms, rows: sdkQ.value.entities.length } : sdkQ.error,
        "MEASURED",
      )
      const pre = await timed(() =>
        c
          .count()
          .from(Spike)
          .where(startsWith("tag", run.slice(0, 4)))
          .fetch(),
      )
      r.add("query", "sdk_startsWith_count", pre.ok ? { count: Number(pre.value), ms: pre.ms } : pre.error, "MEASURED")
    })
  } finally {
    await cleanup({ w, created, track, r })
    r.add("fee", "total_spent_tGLM", tglm(spent), "MEASURED")
  }
}

const liveCount = (c, key) =>
  c
    .request({ method: "arkiv_query", params: [`$key = key(${key})`, { select: { count: true } }] })
    .then((x) => Number(BigInt(x.count)))
    .catch(() => null)

async function waitGone(c, key, expiresAt, timeoutMs = 150_000) {
  const t0 = Date.now()
  const probes = []
  while (Date.now() - t0 < timeoutMs) {
    const [head, found] = await Promise.all([c.getBlockNumber(), liveCount(c, key)])
    probes.push(Number(head))
    if (found === 0 && Number(head) >= expiresAt - 1)
      return { goneObservedAtBlock: Number(head), blocksAfterExpiresAt: Number(head) - expiresAt, polls: probes.length }
    await sleep(2000)
  }
  return { goneObservedAtBlock: "timeout", polls: probes.length }
}

async function cleanup({ w, created, track, r }) {
  const keys = [...new Set(created)]
  let gas = 0
  for (let i = 0; i < keys.length; i += 40) {
    const t = await timed(async () => {
      const x = await w.executeBatch({ deletes: keys.slice(i, i + 40).map((entityKey) => ({ entityKey })) })
      return track(x.txHash, "delete_batch")
    })
    if (t.ok) gas += t.value.gasUsed
    else r.add("cleanup", "delete_batch_error", t.error, "MEASURED") // entities still expire on their own
  }
  r.add("cleanup", `delete_${keys.length}_entities_gas_total`, gas, "MEASURED")
}

// Entry point: `node src/measure.mjs readonly|funded|all`. Read-only probes always run; funded probes run only
// when the throwaway address holds enough tGLM, otherwise the public address is printed for funding.
import { mkdirSync, writeFileSync } from "node:fs"
import { assertTestnet, jsonSafe, loadEnv, makeClients, makeReport, tglm } from "./lib.mjs"
import { annotationLimitProbes, chainProbes, contractAndHistoryProbes, edgeSizeProbe, extendProbes, gasModelProbes, queryProbes } from "./readonly.mjs"
import { MIN_BALANCE_WEI, runFunded } from "./funded.mjs"
import { runFunded2 } from "./funded2.mjs"

const mode = process.argv[2] ?? "all"
loadEnv()
const clients = makeClients(process.env.PRIVATE_KEY)
const report = makeReport()
const ctx = { ...clients, report }

await assertTestnet(clients.publicClient)
console.log(`address (public, testnet only): ${clients.account.address}`)
const balance = await clients.publicClient.getBalance({ address: clients.account.address })
report.add("account", "balance_tGLM", tglm(balance), "MEASURED")

if (mode === "funded2") await runFunded2(ctx)
if (mode !== "funded" && mode !== "funded2") {
  const { head } = await chainProbes(ctx)
  await edgeSizeProbe(ctx)
  await gasModelProbes({ ...ctx, head })
  await annotationLimitProbes(ctx)
  await extendProbes(ctx)
  await contractAndHistoryProbes(ctx)
  await queryProbes(ctx)
}
if (mode !== "readonly" && mode !== "funded2") {
  if (balance >= MIN_BALANCE_WEI || process.env.FORCE_FUNDED === "1")
    await runFunded(ctx) // FORCE_FUNDED=1 only for smoke-testing the code path with no funds
  else
    console.log(
      `SKIPPED funded probes: need >= ${tglm(MIN_BALANCE_WEI)} tGLM at ${clients.account.address} (faucet: https://hub.arkiv.network/faucet, wallet sign-in + captcha, do it by hand)`,
    )
}

mkdirSync(new URL("../results", import.meta.url).pathname, { recursive: true })
const out = new URL(`../results/${new Date().toISOString().replace(/[:.]/g, "-")}-${mode}.json`, import.meta.url).pathname
writeFileSync(out, jsonSafe({ address: clients.account.address, mode, rows: report.rows }))
console.log(`wrote ${out}`)

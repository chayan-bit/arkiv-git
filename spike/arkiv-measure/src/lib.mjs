// Shared plumbing for the Arkiv measurement probes: env/key handling, clients, raw ABI encoders, helpers.
// TESTNET ONLY: the chain id is asserted against Rabdi before anything is sent.
import { appendFileSync, existsSync, writeFileSync } from "node:fs"
import { randomBytes } from "node:crypto"
import { createPublicClient, createWalletClient, randomSalt } from "@arkiv-network/sdk"
import { rabdi } from "@arkiv-network/sdk/chains"
import { encodeAbiParameters, encodeFunctionData, http, parseAbi, parseAbiParameters, stringToBytes, toHex } from "viem"
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts"

export const RABDI_CHAIN_ID = 7794095
export const BLOCK_SECONDS = 2
export const BLOCKS_PER_DAY = 43_200n
export const BLOCKS_PER_MONTH = 30n * BLOCKS_PER_DAY // 1,296,000 (30 days)
export const KIB = 1024
export const TX_GAS_CAP = 1 << 24 // EIP-7825, enforced by the node

const ENV_PATH = new URL("../.env", import.meta.url).pathname

/** Creates .env with a fresh throwaway key on first run. The key is never printed. */
export function loadEnv() {
  if (!existsSync(ENV_PATH)) {
    writeFileSync(ENV_PATH, `PRIVATE_KEY=${generatePrivateKey()}\nARKIV_ACCESS_KEY=\n`, { mode: 0o600 })
  }
  process.loadEnvFile(ENV_PATH)
}

export function makeClients(privateKey) {
  const chain = rabdi({ accessKey: process.env.ARKIV_ACCESS_KEY ?? "" })
  const account = privateKeyToAccount(privateKey)
  const publicClient = createPublicClient({ chain, transport: http() })
  const walletClient = createWalletClient({ chain, transport: http(), account })
  return { chain, account, publicClient, walletClient }
}

export async function assertTestnet(publicClient) {
  const id = await publicClient.getChainId()
  if (id !== RABDI_CHAIN_ID) throw new Error(`refusing to run: chain id ${id} is not Rabdi (${RABDI_CHAIN_ID})`)
}

// ---- raw `execute` ABI encoders (mirror @arkiv-network/sdk entity/operations.ts) ----
// Used where the SDK would refuse before the node sees the request (limit probing) and for estimate-only probes.
const ATTR = "(bytes32 name, uint8 typeId, bytes value)[]"
const PAYLOAD = {
  1: parseAbiParameters(`(uint128 salt, uint64 expiresAt, uint64 minLifetime, uint8 creationFlags, ${ATTR} attributes)`),
  2: parseAbiParameters(`(bytes32 entityKey, ${ATTR} mutations)`),
  3: parseAbiParameters("(bytes32 entityKey, uint64 expiresAt, uint64 minLifetime)"),
}
const EXECUTE_ABI = parseAbi(["function execute((uint8 operation, bytes operationData)[] ops) returns (bytes32[] keys)"])
const INDEXED = 0x80
const name32 = (n) => toHex(n, { size: 32 })

export const attr = {
  bytes: (n, value) => ({ name: name32(n), typeId: 7, value }),
  str: (n, s, indexed = false) => ({ name: name32(n), typeId: 8 | (indexed ? INDEXED : 0), value: toHex(stringToBytes(s)) }),
  u64: (n, v, indexed = false) => ({ name: name32(n), typeId: 3 | (indexed ? INDEXED : 0), value: toHex(BigInt(v), { size: 32 }) }),
}
const byName = (a, b) => (a.name < b.name ? -1 : 1) // lowercase hex of equal width sorts like bytes

/** Calldata for execute([create]) with `lifetime` blocks (relative) and the given attributes. */
export function createCalldata({ attributes, lifetime, flags = 0 }) {
  const operationData = encodeAbiParameters(PAYLOAD[1], [
    { salt: randomSalt(), expiresAt: 0n, minLifetime: BigInt(lifetime), creationFlags: flags, attributes: [...attributes].sort(byName) },
  ])
  return encodeFunctionData({ abi: EXECUTE_ABI, functionName: "execute", args: [[{ operation: 1, operationData }]] })
}

/** Extend to an absolute block (`absolute`) or to `lifetime` blocks from the landing block. */
export function extendCalldata({ entityKey, lifetime = 0, absolute = 0n }) {
  return extendBatchCalldata([{ entityKey, lifetime, absolute }])
}

/** One tx, many extend ops: [{ entityKey, lifetime?, absolute? }]. */
export function extendBatchCalldata(list) {
  const ops = list.map(({ entityKey, lifetime = 0, absolute = 0n }) => ({
    operation: 3,
    operationData: encodeAbiParameters(PAYLOAD[3], [{ entityKey, expiresAt: BigInt(absolute), minLifetime: BigInt(lifetime) }]),
  }))
  return encodeFunctionData({ abi: EXECUTE_ABI, functionName: "execute", args: [ops] })
}

export const randomPayload = (n) => toHex(randomBytes(n)) // random bytes: ~all non-zero, like compressed git packs

// ---- helpers ----
// ARKIV_ADDRESS: sdk src/consts.ts and tokenomics_v1.md section 1
export const to = "0x4400000000000000000000000000000000000044"
export const tglm = (wei) => Number(wei) / 1e18
export const errText = (e) => (e?.details ?? e?.shortMessage ?? e?.message ?? String(e)).split("\n")[0].slice(0, 300)
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
export const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)]

export async function timed(fn) {
  const t0 = performance.now()
  try {
    return { ok: true, value: await fn(), ms: Math.round(performance.now() - t0) }
  } catch (e) {
    return { ok: false, error: errText(e), ms: Math.round(performance.now() - t0) }
  }
}

/** Collects labelled results, prints them as they arrive, serialises BigInt as strings. */
export function makeReport() {
  const rows = []
  const add = (probe, key, value, label) => {
    rows.push({ probe, key, value, label })
    if (process.env.ROWS_FILE) appendFileSync(process.env.ROWS_FILE, JSON.stringify({ probe, key, value, label }, (_, x) => (typeof x === "bigint" ? x.toString() : x)) + "\n")
    const v = typeof value === "object" ? JSON.stringify(value, (_, x) => (typeof x === "bigint" ? x.toString() : x)) : value
    console.log(`[${label}] ${probe}.${key} = ${v}`)
  }
  return { rows, add }
}

export function jsonSafe(x) {
  return JSON.stringify(x, (_, v) => (typeof v === "bigint" ? v.toString() : v), 2)
}

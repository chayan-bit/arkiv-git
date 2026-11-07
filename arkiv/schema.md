# Charter entity schema

This is the full list of entity kinds Charter writes to Arkiv, what each one is indexed by, who creates it, how long it lives, and which queries read it.
It is a reading guide for the Arkiv judges; the normative spec is `docs/p1/design.md` section 2 and section 3.
Every number here is taken from that design, from the P0 measurements in `docs/p0/arkiv-limits.md`, or from the code constants on `main`.

## Conventions shared by every kind

- Namespace: every attribute name starts with `ag.` so Charter entities never collide with other apps on the same chain.
- Common attributes: `ag.kind` (`str`, indexed), `ag.v` (`u64`, entity layout version, currently 1, not indexed), `ag.body` (`bytes`, never indexable).
- Every identifier attribute (`ag.rid`, `ag.nid`, `ag.h`, `ag.pack`, `ag.parent`, `ag.rev`, `ag.cob`) is `bytes32`, so private repos can later store a keyed hash of the value without changing the type or the index.
- Attributes are query hints only.
  After decoding the signed body, the client recomputes every attribute from it and ignores the entity if any value differs.
- Creation flags are `3` on every entity: bit 0 `readonly`, bit 1 `permissionlessExtension`.
  `readonly` means nobody, including the creator, can patch the entity's content.
  The owner key can still delete the entity or transfer its ownership (the node model in our test suite behaves this way; `$creator` stays the original address), which is why deletion is treated as a detectable denial of service by the completeness check below.
  `permissionlessExtension` means anyone can extend its lifetime but never change it, and the flag cannot be added after creation, so it is set on everything from the first write.
- Other create parameters: `expiresAt = 0`, `minLifetime` from the lifetime column below, `salt` of 16 random bytes.
  Charter never predicts entity keys and never patches, transfers or deletes; every cross-reference is a content hash found by query.
- Encoding of signed bodies: canonical CBOR (RFC 8949 deterministic), SHA-256 hashes, composite ML-DSA-65 + Ed25519 signatures inside a versioned envelope.
- Block time is 2 s, so 30 days is 1,296,000 blocks and 182.5 days is 7,884,000 blocks, the maximum lifetime the node accepts.
  Keepalive (`chart keepalive`) extends an entity to a fresh horizon counted from the landing block (default horizon 90 days, renewed when under 30 days remain).

## Who owns what: three addresses per repo

The owner of an entity is the EVM address that created it.
The user has one 32-byte seed on their machine; HKDF-SHA256 derives these secp256k1 keys from it:

| Role | Scope | Creates | Why separate |
| --- | --- | --- | --- |
| `wallet` | one per user | nothing; it holds the funds and tops up the other two | keeps value away from signing keys |
| `control(rid, i)` | one per repo and rotation index | control plane: `peer`, `bind`, `id`, `idacc`, `anchor`, `cob` | everything it creates is kept alive forever, so `entityNonce` equals the live count |
| `bulk(rid, i)` | one per repo and rotation index | `refs`, `manifest`, `chunk` | short-lived data cannot disturb the completeness check |

- The creator rule: an entity counts only if its `$creator` is an address named by a verified `bind` of the signer.
  Another account re-posting the exact signed bytes creates an entity that clients ignore.
- ML-DSA signing authority and EVM ownership are different things.
  The signature proves who wrote the content; the creator address decides whether clients look at it.
- Per-repo addresses make the completeness check exact and keep one user's repos unlinkable by address.

## Kinds

Status column: "main" means merged to `main` and covered by tests; "branch" means it exists only on a feature branch and is not merged.

| Kind | Status | Signed by | Created by | Lifetime | Indexed attributes (type) |
| --- | --- | --- | --- | --- | --- |
| `peer` | main | the peer (proof of possession) | `control` | 182.5 days, kept alive | `ag.nid` (bytes32), `ag.h` (bytes32) |
| `bind` | main | the peer | `control` | 182.5 days, kept alive | `ag.rid`, `ag.nid`, `ag.h` (bytes32), `ag.seq` (u64) |
| `id` | main | the proposer | proposer's `control` | 182.5 days, kept alive | `ag.rid`, `ag.h`, `ag.parent`, `ag.nid` (bytes32), `ag.seq` (u64), public repos also `ag.name`, `ag.lang`, `ag.lic` (str) |
| `idacc` | main | the accepting delegate | delegate's `control` | 182.5 days, kept alive | `ag.rid`, `ag.nid`, `ag.rev` (bytes32) |
| `anchor` | main | the peer | `control` | 182.5 days, kept alive | `ag.rid`, `ag.nid`, `ag.h` (bytes32), `ag.seq`, `ag.at` (u64) |
| `refs` | main | the peer | `bulk` | 30 days; only each peer's current head kept alive | `ag.rid`, `ag.nid`, `ag.h` (bytes32) |
| `manifest` | main | the pusher | `bulk` | 30 days, kept alive while an anchor names it | `ag.rid`, `ag.nid`, `ag.h`, `ag.pack` (bytes32) |
| `chunk` | main | nobody; bound by the manifest's chunk hash | `bulk` | 30 days, kept alive while its manifest is | `ag.rid`, `ag.pack` (bytes32), `ag.i` (u64) |
| `cob` | branch `feat/cob-ops` | the author | author's `control` | control lifetime, 182.5 days by default; keepalive does not extend it yet | `ag.rid`, `ag.nid`, `ag.h`, `ag.cob` (bytes32), `ag.type`, `ag.action` (str) |

The 30-day data lifetime is a deliberate choice: storage gas is linear in lifetime, so a long first lifetime buys nothing and a 64 KiB chunk costs 3.92M gas at 30 days against 10.1M at the maximum (`docs/p0/arkiv-limits.md`).
The control plane is small (about 0.9 to 1.1M gas per entity at the maximum lifetime) and must never lapse.

### `peer`

- Body: algorithm id, composite public key (1,984 B: ML-DSA-65 1,952 B plus Ed25519 32 B; the signature is 3,373 B), display alias that is never trusted.
- The envelope signer must equal `nid = SHA-256(tag || alg || pk)`, and the signature must verify under the key in the body.
  This is what lets anyone find a peer's public key by `nid` and nothing else.
- Each repo's control address writes its own byte-identical copy on the peer's first write to that repo, so the completeness check covers it.
- Queries:
  - `ag.kind = str('peer') AND ag.nid = bytes32(0x...)`

### `bind`

- Body: `rid`, `seq`, `prev` hash, the `control` address and the `bulk` address of the peer for this repo.
- Creator rule: bind seq 0 must be created by the control address it names; bind seq n+1 must be created by the control address of seq n.
  This chain is also how EVM keys rotate without touching the signing identity.
- Queries:
  - Bind chain: `ag.kind = str('bind') AND ag.rid = bytes32(0x...) AND ag.nid = bytes32(0x...) ORDER BY ag.seq:u64 ASC`
  - Search, repos a peer writes to: `ag.kind = str('bind') AND ag.nid = bytes32(0x...)`

### `id` (identity revision)

- Body: name, description, default branch, delegates (up to 16 nids), threshold, visibility, optional language and licence, parent hash.
- The repo id is `rid = SHA-256(tag || root envelope bytes)`, so the repo's name is the hash of its first identity revision.
- A revision takes effect only when a majority of the parent revision's delegates have signed or accepted it.
- Queries:
  - Root: `ag.kind = str('id') AND ag.h = bytes32(0x<rid>)`
  - Identity chain: `ag.rid = bytes32(0x<rid>) AND (ag.kind = str('id') OR ag.kind = str('idacc'))`, in creation order with offset paging and a count check
  - Search by name: `ag.kind = str('id') AND ag.name = str('charter')`, or `ag.name STARTSWITH str('arkiv')`
  - Search by language and licence: `ag.kind = str('id') AND ag.lang = str('rust') AND (ag.lic = str('MIT') OR ag.lic = str('Apache-2.0'))`

### `idacc` (identity acceptance)

- Body: `rid` and the hash of the revision being accepted.
- Only acceptances by the parent revision's delegates count, so anyone can propose and only delegates can ratify.
- Queries: read together with `id` in the identity-chain query above.

### `refs`

- Body: `rid`, object hash kind (SHA-1 or SHA-256), and the sorted list of (ref name, object id).
  Names must start with `refs/heads/`, `refs/tags/` or `refs/notes/`.
- One refs entity per push, signed by the pushing peer, up to about 62,100 B of body (65,536 B minus the 3,373 B composite signature and envelope framing, about 1,100 refs); larger sets fail the push with `E_REFS_TOO_LARGE`.
- Queries:
  - By content hash, to resume an interrupted push or to fetch the head named by an anchor: `ag.kind = str('refs') AND ag.h = bytes32(0x...) AND $creator = addr(0x<bulk>)`

### `anchor`

- Body: `rid`, `seq`, `prev` hash, the `refs` hash of this push, the manifests this push added, the identity revision the writer verified, and `at`, the head block number the writer saw.
- Anchors form a per-peer signed hash chain.
  The first valid anchor created for a given `seq` wins, and creating `seq + 1` needs the peer's ML-DSA key.
- `ag.at` exists because `$createdAt` is returned but cannot be queried or sorted on; `ag.at <= $createdAt` is enforced, so a writer can only understate recency.
- Queries:
  - Chain: `ag.kind = str('anchor') AND ag.rid = bytes32(0x...) AND ag.nid = bytes32(0x...) AND ($creator = addr(0x<c0>) OR ...) ORDER BY ag.seq:u64 ASC`
  - Incremental fetch from a high-water mark at seq `w`: the same query with `AND ag.seq >= u64(w)`
  - Search, pushed recently: `ag.kind = str('anchor') AND ag.at >= u64(297600) ORDER BY ag.at:u64 DESC`

### `manifest`

- Body: `rid`, `pack_id = SHA-256(pack bytes)`, git pack trailer, size (at most 100 MiB), chunk size (65,536 B), the chunk hashes as one byte string of 32 bytes each, and `requires`, the other peers' anchors whose packs must exist first.
- A full 100 MiB pack has 1,600 chunks, so the hash list is 51,200 B and the entity about 54,700 B, under the 64 KiB body cap.
- Queries:
  - By content hash: `ag.kind = str('manifest') AND ag.h = bytes32(0x...) AND $creator = addr(0x<bulk>)`
  - By pack: `ag.pack = bytes32(0x...)`

### `chunk`

- No envelope: `ag.body` is the raw pack bytes, at most 65,536 B, the last chunk of a pack shorter.
- Integrity comes from the manifest: `SHA-256(ag.body)` must equal entry `ag.i` of the manifest's hash list, so a chunk needs no signature and a seed or sponsor can verify availability without any key.
- One chunk per transaction, because the public RPC accepts a raw transaction of at most 131,030 B and two 64 KiB payloads are already 131,072 B.
- Queries (pages of 16 chunks, up to 8 pages in flight, page size halved on HTTP 413, 5xx or a timeout):
  - `ag.kind = str('chunk') AND ag.pack = bytes32(0x...) AND ($creator = addr(0x<bulk0>) OR ...) AND ag.i >= u64(0) AND ag.i <= u64(15) ORDER BY ag.i:u64`
  - Presence check after upload, metadata only: select `key` and `ag.i`, same predicate without the body.

### `cob` (collaborative object op, on branch `feat/cob-ops`)

- Not merged to `main`.
  The kind code and attribute names were reserved in P1; the op format and the verification rules are in `docs/p3/cob-ops.md` on that branch.
- Issues, patches, reviews, attestations and votes are all sequences of signed ops; one op is one entity.
- Body: `rid`, type name, object id, parent op ids, action, payload (canonical CBOR, at most 65,536 B, and about 62,100 B once the signed envelope has to fit the same 65,536 B attribute cap), the author's clock in milliseconds (display only).
- `ag.cob` of a create op is its own id, so one query returns the create op and every follow-up.
- Ordering is Arkiv creation order, never the author's clock.
- Verification drops (never errors on) ops that fail decoding, belong to another repo, come from an unknown signer, repeat an op id, reference an unknown object or parent, or change type.
- No creator rule on this branch: an op is valid whichever address created the entity, because the signature carries authorship.
- Queries:
  - `ag.kind = str('cob') AND ag.rid = bytes32(0x...)`, optionally `AND ag.cob = bytes32(0x...)` and `AND ag.type = str('xyz.arkiv.patch')`, offset-paged with a count check.

## Queries every client runs regardless of kind

- Completeness check, per control address `C` of a peer: read `entityNonce(C)` by `eth_call` to the Arkiv precompile `0x4400000000000000000000000000000000000044` (selector `0x36917bfd`), then count `$creator = addr(C)` with `select: {count: true}`, then read `entityNonce(C)` again.
  If the nonce moved, retry up to 3 times.
  If the live count is below the nonce, a control-plane entity has expired or been deleted and the fetch fails with `E_CONTROL_NOT_LIVE`; if it is above, `E_INCONSISTENT_VIEW`.
- Paging: keyset paging on `ag.seq` or `ag.i` wherever an ordering attribute exists, offset paging with a count check otherwise, never more than 100 rows per page.
- Query limits respected: at most 4 levels of `AND`/`OR`, 64 predicates and 8 KiB of query text; address and hash lists above 48 terms are split across queries.
- Not used because Arkiv does not offer them: `NOT`, `!=`, historical reads (`atBlock`), `eth_getLogs` ranges and `$createdAt` filters.
  See `friction.md`.

## Size and cost per transaction (measured inputs, derived outputs)

| Transaction | Sender | Calldata | Gas |
| --- | --- | --- | --- |
| first write to a repo: `peer` + `bind` (+ root `id` for the founder) | `control` | about 13 KB | about 3.2M at maximum lifetime |
| chunk | `bulk` | about 66.9 KB | 3.92M at 30 days, 2.72M at 1 day |
| manifest (full 100 MiB pack) | `bulk` | about 55.6 KB | about 3.3M at 30 days |
| refs (typical, under 4 KB) | `bulk` | under 5 KB | about 0.25M at 30 days |
| anchor | `control` | about 4.4 KB | about 0.9M at maximum lifetime |
| keepalive batch | any funded key | under 15 KB | at most 15M, up to 100 extensions |

Every transaction stays under the measured 131,030 B raw-transaction cap and the 16,777,216 gas cap.
Gas figures are derived from the Rabdi testnet measurements; mainnet pricing is unknown.

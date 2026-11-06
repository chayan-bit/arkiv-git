# P0: How arkiv-git reuses Radicle heartwood

Status: P0 decision record, reading task only (no builds).
Date: 2026-11-06.

## Sources and pinned revisions

- heartwood: cloned from the canonical Radicle seed `https://seed.radicle.xyz/z3gqcJUoA1n9HaHKufZs5FCSGazv5.git` at commit `e6fe5207` (2026-10-30, "just/check-scripts: Limit to tracked files").
- The workspace manifest names the canonical home as `https://radicle.network/seeds/seed.radicle.dev/rad:z3gqcJUoA1n9HaHKufZs5FCSGazv5` (`Cargo.toml:8`).
- All heartwood paths below are relative to `crates/` at `e6fe5207` unless stated.
- crates.io data: `https://crates.io/api/v1/crates/<name>`, queried 2026-11-06.
- git-remote-gcrypt: `https://github.com/spwhitton/git-remote-gcrypt` `master` (last commit `a5ff704d07`, 2026-08-15).
- Arkiv node (read-only reference): `~/Code/projects/arkiv-contrib/arkiv` at `2a3526d` (2026-11-03), mainly `docs/api-spec.md`.

## TLDR

- Pick: depend on a pinned set of published heartwood crates (`radicle`, `radicle-cob`, `radicle-crypto` and their leaf crates) as the format, signing and verification library, and write our own Arkiv sync layer and `git-remote-arkiv`.
- Do not fork, and do not depend on `radicle-node`, `radicle-protocol`, `radicle-fetch`, `radicle-cli` or `radicle-remote-helper`.
- The storage seam is not an abstract repository trait: heartwood storage is a local bare git repo on disk, so Arkiv becomes a sync layer under a local heartwood-layout cache.
- The sigrefs format and the COB engine are generic over small object/ref traits, so they can be read and verified straight from bytes fetched off Arkiv.
- heartwood already blocks replay-by-regrafting (signed `refs/rad/sigrefs-parent`) and local rollback (non-fast-forward abort), but has no protection for a fresh clone against a stale or forked sigrefs head; on-chain anchors fill exactly that gap.

## 1. Crate map

All heartwood crates are `MIT OR Apache-2.0` (`Cargo.toml:7`), edition 2024, MSRV 1.88.
"Node layer" means it depends on `radicle-node`, `radicle-protocol`, `radicle-fetch`, `mio` or the Noise transport.

| Crate | Purpose | crates.io (version, date) | Node layer? |
|---|---|---|---|
| `radicle` | Standard library: storage, identity doc, sigrefs, COB types (issue/patch/identity), profile, node control-socket client | 0.25.1, 2026-07-24 | No, but pulls `cyphernet` (address types), bundled `sqlite`, vendored `libgit2` (`radicle/Cargo.toml:31-58`) |
| `radicle-core` | Core types (`RepoId`, `NodeId`) | 0.4.1, 2026-07-24 | No |
| `radicle-crypto` | Ed25519 keys, signer traits, optional ssh-agent | 0.19.0, 2026-07-24 | No (`cyphernet` optional) |
| `radicle-oid` | Git object id type, SHA-1 only | 0.2.2, 2026-07-24 | No |
| `radicle-cob` | Collaborative-object engine (change DAG, signed changes) | 0.21.1, 2026-07-24 | No |
| `radicle-dag` | DAG data structure for COBs | 0.10.0, 2024-11-29 | No |
| `radicle-git-metadata` | Commit/author/trailer structs | 0.2.1, 2026-07-24 | No |
| `radicle-git-ref-format` | Ref-name validation (replaces external `git-ref-format`) | 0.2.0, 2026-07-24 | No |
| `radicle-localtime`, `radicle-log`, `radicle-signals`, `radicle-term`, `radicle-windows`, `radicle-systemd` | Utilities | various, 2026-07-24 | No |
| `radicle-surf` | Code browsing over git2 | 0.29.0, 2026-07-24 | No |
| `radicle-fetch` | Radicle fetch protocol over gix (`gix-protocol`, `gix-pack`, `gix-odb`) | 0.21.1, 2026-07-24 | Yes (transport side of replication) |
| `radicle-protocol` | Gossip/wire protocol | 0.9.1, 2026-07-24 | Yes |
| `radicle-node` | The P2P node (`mio`, Noise via `cyphernet`) | 0.21.1, 2026-07-24 | Yes |
| `radicle-cli` | `rad` CLI | 0.23.0, 2026-07-24 | Indirect (talks to node socket; dev-dep on node) |
| `radicle-remote-helper` | `git-remote-rad` | 0.18.1, 2026-07-24 | Indirect (depends on `radicle-cli`, calls node sync) |

Crates the brief named that are no longer in heartwood:

- `radicle-ssh` is gone from the workspace and from `Cargo.lock`; last release 0.10.0, 2025-08-12, Apache-2.0 only.
  Its role is now `ssh-key` plus `ssh-agent-lib` behind `radicle-crypto`'s `ssh` feature (`radicle-crypto/Cargo.toml:21,34-35`).
- `radicle-git-ext` (0.13.0) and `git-ref-format` (0.7.0), both 2026-06-22, live in the separate `radicle-git` repo; heartwood no longer uses them (0 hits in `Cargo.lock`).

## 2. Can we use `radicle` and `radicle-cob` without the node?

Yes for the dependency graph: `radicle` does not depend on `radicle-node`, `radicle-protocol` or `radicle-fetch` (`radicle/Cargo.toml:31-58`).
Its `node` module is only the Unix control-socket client and config/policy types (`radicle/src/node.rs:1166-1184`), which we simply never call.

The storage seam is concrete, not abstract:

- `ReadStorage`, `WriteStorage`, `ReadRepository`, `WriteRepository` and `SignRepository` exist (`radicle/src/storage.rs:438,464,495,668,727`).
- But they are not backend-neutral: `ReadRepository::path()` returns a filesystem path (`storage.rs:503`), and `blob`, `commit`, `revwalk` return `git2` types (`storage.rs:513,582,585`); `WriteRepository::raw()` returns `&git2::Repository` (`storage.rs:723`).
- `Storage` is a `PathBuf` plus user info (`radicle/src/storage/git.rs:80-83`), and `Repository` wraps a `git2::Repository` opened from disk (`storage/git.rs:297-302,416`).
- Conclusion: implementing these traits over Arkiv would mean emulating libgit2; Arkiv must be a sync layer underneath a local bare repo in heartwood's layout (`~/.arkiv-git/storage/<rid>`, namespaces `refs/namespaces/<nid>/...`).

The useful seams are lower down and genuinely generic:

- Sigrefs read/write is generic over four tiny public traits: `object::Reader` (`read_commit`, `read_blob`), `object::Writer` (`write_tree`, `write_commit`), `reference::Reader` (`find_reference`), `reference::Writer` (`write_reference`) (`radicle/src/storage/refs/sigrefs/git/object.rs:23,77`, `.../reference.rs:15,30`), with `git2::Repository` as just one impl (`sigrefs/git.rs:125-266`).
- `SignedRefsReader` is public (`sigrefs/read.rs:75-168`) and `Refs::save` is public (`storage/refs.rs:73`), so a client can verify a peer's sigrefs from four raw objects downloaded from Arkiv, before anything touches local storage.
- `radicle-cob` operations take a `Store` that is `object::Storage + change::Storage` (`radicle-cob/src/lib.rs:115-121`); the crate documents git2 as a convenience impl only (`radicle-cob/src/lib.rs` header, "Storage" section).
- Caveat: `Store` fixes `ObjectId = radicle_oid::Oid`, which is SHA-1 only (`radicle-oid/src/lib.rs:72-81`, `compile_error!` without `sha1`).

## 3. Formats we would reuse

### Identity document (`rad/id`)

- JSON `Doc { version, payload, delegates, threshold, visibility }` (`radicle/src/identity/doc.rs:693-701`; raw form `RawDoc` at `doc.rs:417-430`), stored as `embeds/radicle.json` (`doc.rs:38,1005`).
- Encoded as canonical JSON; its git blob SHA-1 is the document id (`Doc::encode`, `doc.rs:1011-1020`).
- Signing: a delegate signs the blob OID bytes, not the JSON (`Doc::sign`, `doc.rs:1024-1031`; `verify_signature` checks delegate membership then Ed25519 over the OID, `doc.rs:972-989`).
- `RepoId` is the SHA-1 blob OID of the root document (`radicle-core/src/repo.rs:45,163`; root lookup `storage/git.rs:896-903`).
- Updates are an identity COB of type `xyz.radicle.id` (`radicle/src/cob/identity.rs:33`): a revision carries a new blob plus the proposer's signature; delegates accept with a signature over the revision blob, verified against the parent document's delegates (`cob/identity.rs:557-561`).
- Acceptance needs a majority `n/2 + 1` of the parent's delegates (`doc.rs:996-998`); `threshold` is a different knob, the quorum for canonical refs such as the default branch (`doc.rs:809,942`).
- 1.10.0 (2026-08-05) rewrote identity evaluation (parent/child states, one accept per sibling set) and admits a remaining order-dependence between concurrent "redact" and "accept" (`../CHANGELOG.md:96-160`); Arkiv's total order removes that ambiguity if we evaluate in Arkiv order.
- `Visibility::Private { allow }` exists (`doc.rs:360,966`) but is replication access control only, not encryption; P2 cannot lean on it.

### Signed refs (`rad/sigrefs`)

- Per peer namespace, `refs/namespaces/<nid>/refs/rad/sigrefs` is a linear commit chain (`sigrefs/write.rs:103-119`).
- Each commit's tree has two blobs: `refs` and `signature` (`storage/refs.rs:28-31`).
- `refs` is the canonical payload, one `<sha1-hex> <refname>\n` line per ref, sorted (`Refs::canonical`, `refs.rs:170-184`).
- The signature is the peer's Ed25519 signature over exactly those bytes (`TreeWriter::write`, `sigrefs/write.rs:326-345`), using the node key (`NodeId`), not a delegate-only key.
- The payload must contain `refs/rad/root` (the identity root, anti-graft, "Root" level) and, since 1.7.0, `refs/rad/sigrefs-parent` set to the previous sigrefs commit (anti-replay, "Parent" level) (`radicle/src/git.rs:156,164,172`; `FeatureLevel` at `refs.rs:278-307`; `add_parent` at `refs.rs:226`).
- Verification checks the signature, that `refs/rad/root` matches the expected `RepoId`, and that `sigrefs-parent` equals the commit's real parent (`sigrefs/read.rs:26-32`, rid check around `read.rs:377-385`).

How a stale sigrefs is detected today:

- "Replay" in heartwood means re-wrapping an old signed payload in a new commit; the signed parent pointer makes that impossible at level Parent, and older levels fall back to rejecting duplicate signatures in history (`sigrefs/read.rs:156-192`).
- Rollback to an older but genuine head is caught only relative to local state: during fetch, special refs (`rad/id`, `rad/sigrefs`) of a delegate that are not a fast-forward abort the whole fetch, and for non-delegates the update is rejected (`radicle-fetch/src/refs.rs:163-183`; ancestry logic `radicle-fetch/src/git/repository.rs:226-270`).
- Ordinary refs listed in sigrefs are applied with `Policy::Allow`, i.e. force-updated (`radicle-fetch/src/stage.rs:527-532`), so freshness rests entirely on the sigrefs chain.

### COB change format

- A change is a git commit whose tree holds a `manifest` blob (`{type_name, version}`) plus one blob per op named `0`, `1`, ... and an optional `embeds/` tree (`radicle-cob/src/backend/git/change.rs:27,349-380`; `Manifest` at `radicle-cob/src/change/store.rs:158-165`).
- The author signs the tree OID with an `ExtendedSignature` (public key plus signature) stored PEM-encoded in the commit's `gpgsig` header (`change.rs:118,287-296`).
- Parents are the current tips plus related objects, and `Rad-Resource` / `Rad-Related` trailers name the resource and related commits (`change.rs:100-150,278-284`).
- The object id of a COB is the OID of its root change commit.

### Existing anti-rollback / anti-equivocation, and what anchoring adds

| Threat | heartwood today | With Arkiv anchors |
|---|---|---|
| Re-wrap old signed payload as a new commit | Blocked at level Parent (signed parent pointer) | Unchanged, reused |
| Seed serves an older genuine head to a client that has seen newer | Blocked by non-FF abort against local state | Unchanged, plus anchor high-water mark |
| Seed serves an older genuine head to a fresh clone (freeze attack) | Not detected | Blocked: client must use the latest anchored sequence number |
| Peer signs two divergent sigrefs chains and shows each to different peers (equivocation) | Not detected unless one node sees both and hits non-FF | Detected deterministically: both anchors are on-chain, first in Arkiv order wins, the other is published evidence |
| Proving when a ref existed | Commit timestamps only (self-asserted) | Arkiv block height of the anchor (trusted timestamp) |

No `equivocation`, `freeze` or `rollback attack` handling exists in the tree (grep at `e6fe5207`).

## 4. How `git-remote-rad` works, and the pattern for Arkiv

- It advertises `option`, `push` and `fetch`, not `connect` or `stateless-connect` (`radicle-remote-helper/src/main.rs:295-299`; parser `protocol.rs:37-64`).
- It never talks to the network itself: fetch shells out to `git fetch-pack` against the local storage repo (`fetch.rs:71-80`), push uses `git send-pack` into local storage, then calls `sign_refs` and asks the running node to sync, skipping sync if the node is down (`push.rs:434-486`).
- It depends on `radicle-cli` for patch handling (`refs/heads/patches/*`, `push.rs:255,344-610`), which is why we should not depend on this crate.
- `connect` / `stateless-connect` need a live git server speaking the pack protocol on the other end; Arkiv is an entity store, so they do not fit.
- git-remote-gcrypt is the matching precedent: `fetch` + `push` only (`git-remote-gcrypt:683-688`), push runs `git pack-objects`, uploads the pack under its hash and rewrites a signed, encrypted manifest of refs and packs (`git-remote-gcrypt:770-833`).
- gcrypt overwrites the manifest in place and has no freshness guarantee, which is exactly the hole on-chain anchors close.

Recommended shape for `git-remote-arkiv`:

1. Capabilities `option`, `fetch`, `push` (with `list` / `list for-push`).
2. Two stages like `git-remote-rad`: the helper moves objects between the working repo and the local heartwood-layout cache with `git fetch-pack` / `send-pack`.
3. The sync step replaces `node.sync`: on fetch it pulls anchors, verifies sigrefs, downloads pack chunks, runs `git index-pack`, and only then updates cache refs; on push it builds a thin pack with `git pack-objects --revs` against the last anchored refs, uploads chunks plus manifest, writes sigrefs, then writes the anchor last.
4. `list` returns refs computed from verified, anchored sigrefs (canonical refs via the identity `threshold`), never from unanchored data.

## 5. git library

- heartwood today is git2-first: `radicle` re-exports `git2` as `radicle::git::raw` (`radicle/src/git/raw.rs:1-20`, about 280 `raw::` uses in `radicle/src`), and `radicle-cob`, `radicle-surf` use git2 directly; libgit2 is vendored (`libgit2-sys 0.18.4+1.9.3` in `Cargo.lock`).
- gix is used only in `radicle-fetch` (protocol, pack, odb) and for packetline in `radicle-node`, pinned to older gix (`gix-hash 0.25`, `gix-pack 0.70`) than current (`gix 0.90`, `gix-pack 0.77`, 2026-10-22).
- Direction: a slow move off git2 in the type layer (`ecca50a5` "Avoid git2::Oid::zero", `8dba3ca9` "Decode radicle_oid::Oid without git2", `c7339615` cob Oid replacement), but storage, sigrefs impls and COBs remain git2 and nothing announces a full migration.
- heartwood shells out to `git` for pack transfer because libgit2 was much slower for file-protocol push/fetch (`../CHANGELOG.md:838`, `radicle-remote-helper/src/fetch.rs:71-80`).

Recommendation for arkiv-git P1: git2, reached through `radicle::git::raw` rather than a separate direct pin, plus the `git` CLI for `pack-objects` / `index-pack`, exactly as heartwood does.
Reason: the reused `radicle` APIs hand us git2 types, so adding gix would mean a second git stack and a third with duplicate `gix-hash` versions.
What flips it to gix: going net-new on storage (fork or vendor), or needing the SHA-256 object format inside git (gix has a `sha256` feature, git2 only `unstable-sha256`).

## 6. Maintenance

- Activity: 723 commits from 2025-11-01 to 2026-10-30; monthly counts in 2026 range from 19 (Sep) to 86 (Jul).
- Bus factor about 3: Lorenz Leutgeb 303, Fintan Halpenny 228, Adrian Duke 100, together 87%; the next contributor has 18.
- Releases (tags): 1.6.0 2026-01-07, 1.7.0 2026-03-18, 1.8.0 2026-03-26, 1.9.0 2026-05-18, 1.10.0 2026-08-05, 1.10.3 2026-09-08, so a minor roughly every 1-2 months.
- crates.io lags the binaries: the whole crate set was last published together on 2026-07-24.
- API stability is low: everything is pre-1.0, `radicle` has 22 published versions, and in 2026 alone sigrefs types were merged and migrated (`ec36d1c3`, `47063057`, Mar 2026), `WriteRepository::set_head` was split (`bfb28580`, Apr 2026) and identity evaluation semantics changed (1.10.0).
- The wire formats (sigrefs layout, identity doc, COB layout) have been stable and only extended with backward-compatible feature levels.

## 7. Recommendation: depend, do not fork

Pick: depend on exact-pinned published crates (`radicle = "=0.25.1"`, `radicle-cob = "=0.21.1"`, `radicle-crypto = "=0.19.0"`, plus their leaf crates), used for formats, signing, verification and the local cache; write the Arkiv sync layer and `git-remote-arkiv` ourselves.

Reasoning:

- We replace networking entirely, and everything we would delete in a fork (`radicle-node`, `radicle-protocol`, `radicle-fetch`, `radicle-cli`, `radicle-remote-helper`) is already a separate crate we can just not depend on.
- What we keep (identity doc, identity COB evaluation, sigrefs, COB engine, canonical refs) is the security-critical, subtle part, and it is the part heartwood keeps fixing (1.7.0 replay fix, 1.10.0 identity evaluation); depending gets those fixes for a version bump.
- Staying byte-compatible means a repo's RID, DIDs and sigrefs are identical on Radicle and on Arkiv, which keeps a bridge to the Radicle network possible.
- License is permissive (`MIT OR Apache-2.0`), so vendoring later is always allowed.
- Cost we accept: libgit2 and bundled sqlite in the build, and about one breaking upgrade per heartwood minor; exact pins and a thin adapter module contain this.

P2 keyed-hash object IDs do not have to fight heartwood:

- heartwood hard-codes SHA-1 everywhere it names objects: `radicle-oid` (SHA-1 only), the sigrefs payload format, `RepoId`, and COB ids.
- Members of a private repo hold the plaintext, so their local cache can stay ordinary SHA-1 git; only what is published to Arkiv needs hiding.
- So apply keyed hashing and encryption at the Arkiv boundary, as gcrypt does: encrypted packs, chunk ids as HMAC(epoch key, chunk), blinded annotation values, encrypted sigrefs and anchors, and a blinded RID.

What would flip the pick:

- To vendor a subset (sigrefs read/write, identity `Doc`, identity COB): if heartwood breaks those APIs more than once per quarter, or if we need to drop libgit2/sqlite from the client.
- To fork: if P2 requires keyed-hash IDs inside git itself (so local object graphs never use public SHA-1), or if we must change sigrefs semantics (for example putting the anchor sequence number inside the signed `refs` payload) in a way upstream will not take.

## 8. Mapping heartwood onto Arkiv

Arkiv facts this relies on (from `arkiv-contrib/arkiv@2a3526d`, to confirm against the target network in P0):

- Typed attributes `str` (128 B max, indexed), `u64`, `bytes32`, `bytes` (128 KiB max, never indexed), and others (`docs/api-spec.md:28-60`); at most 32 attributes per entity (`crates/arkiv-interfaces/src/constants.rs:71,77`).
- `creationFlags` bit0 `readonly` blocks PATCH but not lifecycle ops: the owner can still extend or delete (`crates/arkiv-reth-executor/src/arkiv.rs:308-312`); bit1 `permissionless_extension` lets anyone extend (`docs/api-spec.md:130`).
- `expiresAt = MAX_EXPIRES_AT` means permanent; otherwise lifetime is capped by `max_lifetime` (`docs/api-spec.md:115-127`; `docs/genesis-spec.md:49`).
- Owner is an EVM address, so Arkiv authorship is not Radicle authorship; every entity below is self-authenticating by an Ed25519 signature inside it, and `$owner` is only a spam filter.

All entities carry `ag.kind` (str, indexed), `ag.v` (u64, format version) and `ag.rid` (str, indexed) unless noted.

| Kind (`ag.kind`) | Other attributes (name: type) | Payload (`bytes`) | Writer | Client verification |
|---|---|---|---|---|
| `repo` | none | Canonical root identity JSON | Founder | git blob SHA-1 of payload equals `ag.rid`; doc parses; founder is the sole delegate (`storage/git.rs:896-910`) |
| `binding` | `ag.nid`: str (`did:key` / `z6Mk...`), `ag.addr`: addr, `ag.seq`: u64 | Statement `{rid, nid, evm_addr, seq}` plus Ed25519 sig | Each peer | Sig by `nid`; highest `seq` wins; lets queries filter by `$owner` |
| `cob` (identity revisions and other COB changes) | `ag.type`: str (e.g. `xyz.radicle.id`), `ag.cob`: str (object id hex), `ag.oid`: str (change commit hex) | Raw change commit, tree, manifest and op blobs | Any author; identity: delegates | Recompute object SHA-1s; `ExtendedSignature` over tree OID; identity: run heartwood identity evaluation in Arkiv order |
| `sigrefs` | `ag.nid`: str, `ag.oid`: str (sigrefs commit hex) | Raw sigrefs commit, tree, `refs` blob, `signature` blob | Each peer | Recompute SHA-1s; `SignedRefsReader` at `FeatureLevel::Parent`: sig over `refs`, `refs/rad/root == rid`, `sigrefs-parent == commit parent` |
| `pack-chunk` | `ag.h`: bytes32 (SHA-256 of chunk) | Up to 128 KiB of pack bytes; several chunk attributes per entity if tx limits allow | Pusher | SHA-256 matches the hash listed in a verified manifest; created with `permissionless_extension` so anyone can keep it alive (content-addressed, harmless) |
| `pack-manifest` | `ag.nid`: str, `ag.pack`: str (pack trailer SHA-1 hex) | `{pack_sha1, size, chunks: [(entity_key, sha256)], base_anchor_seq}` plus Ed25519 sig | Pusher | Sig by `nid`; reassembled pack's SHA-1 trailer and `git index-pack` succeed |
| `anchor` | `ag.nid`: str, `ag.seq`: u64 (indexed), `ag.oid`: str (sigrefs commit hex), `ag.prev`: bytes32 | Anchor statement (below) plus Ed25519 sig | Each peer, written last in a push | Rules below |

Anchor statement (canonical encoding, signed by the peer's node key with domain tag `arkiv-git/anchor/v1`):
`{rid, nid, seq, sigrefs_oid, sigrefs_parent_oid, prev = SHA-256(previous anchor statement) or zero, id_head, packs: [manifest digests added since prev], expires_at}`.
`id_head` pins the identity revision the writer saw, so a delegate removed by a later identity revision cannot keep counting toward canonical-ref quorum.

### Ref-anchoring algorithm

Ordering: an anchor's position is (block number, tx index, log index) of its `EntityCreated` event, which Arkiv is trusted to order; `$createdAt` alone is not enough because two anchors can share a block.

For each `(rid, nid)` the client builds the canonical chain:

1. Collect every `anchor` entity with that `rid` and `nid` whose signature verifies under `nid` and whose `ag.*` attributes match the signed statement; ignore the rest.
2. `A_0` is the first anchor in Arkiv order with `seq = 0` and `prev = 0`.
3. `A_{k+1}` is the first anchor in Arkiv order that comes after `A_k` and has `seq = k + 1` and `prev = SHA-256(A_k)`.
4. The head is the anchor with the highest `seq` reached this way.
5. Any other valid anchor with a `seq` already taken in the chain but different content is equivocation evidence: keep the first-in-order one, publish both signed statements as proof, and flag `nid` (a delegate so flagged should be removed by an identity revision).

Accept a sigrefs commit `S` for `nid` if and only if all of these hold:

- `S.oid == head.sigrefs_oid`, and `S` verifies with `SignedRefsReader` at `FeatureLevel::Parent`.
- `S.parent == head.sigrefs_parent_oid`, and that equals `A_{head.seq-1}.sigrefs_oid` (or both are absent at `seq = 0`), so the git chain and the anchor chain agree.
- `head.seq >= hwm[nid].seq`, where `hwm` is the client's persisted high-water mark; if equal, `SHA-256(head) == hwm[nid].hash`.
- Every pack named in anchors `A_0..head` (or in a later repack manifest that lists what it supersedes) is available and indexes, and every OID in `S.refs` resolves.

Why a replayed old sigrefs fails:

- If an attacker serves or re-posts the bytes of an old anchor `A_j` (`j < head.seq`), it is either the same `A_j` (a duplicate that changes nothing) or a later copy with a taken `seq` and identical content (ignored, since the first-in-order one is already in the chain), and in both cases its `sigrefs_oid` is not `head.sigrefs_oid`, so the old sigrefs is rejected.
- Creating `seq = head.seq + 1` needs the peer's Ed25519 key.
- A client that has seen a newer head also refuses anything below its `hwm`, even if Arkiv withholds data.

Push-side rule: the writer reads its current head, writes chunks, manifest and sigrefs, then the anchor with `seq = head.seq + 1` and `prev = SHA-256(head)`.
Two devices of the same peer racing produce self-equivocation; tooling must serialize pushes per `nid` (re-read the head and abort if it moved).

### BTL / expiry must not enable rollback

Rollback by lifecycle means the head vanishes (expiry or owner delete) while an older anchor is still visible, so the client would compute an older head.
Rules:

1. Anchors are created `readonly` and without `permissionless_extension`, so contents cannot be patched and only the owner can extend them (third parties could otherwise keep an old anchor alive past the head).
2. Monotone expiry: every new anchor gets `expiresAt >= expiresAt(previous anchor)`, and keep-alive extends the head (extending older anchors is pointless and forbidden by tooling); where the network allows it, anchors are permanent (`MAX_EXPIRES_AT`) and this rule becomes trivial.
3. Client check of the invariant: if any visible older anchor of the chain has `$expiresAt` later than the visible head's, the chain was truncated or the owner misbehaved; fail closed for that `nid` (accept nothing beyond `hwm`) and report it.
4. Under rules 1-2, when the head expires every older anchor has already expired, so the client sees no anchor at all; it then treats the peer as unavailable and never falls back to unanchored sigrefs.
5. Owner deletion of the head stays possible (`readonly` does not block delete, `arkiv-reth-executor/src/arkiv.rs:308`); mitigations are the local `hwm`, an optional audit of `EntityDeleted` events for known anchor keys inside the node's retained range, and the canonical-ref quorum (`threshold`), which limits one delegate's rollback to its own vote.
6. The owner EVM key can therefore roll back its own peer's anchors; P1 should derive it from, or keep it as safe as, the Radicle node key and record the binding entity.

## Contradictions with the plan's assumptions

- The brief describes Arkiv annotations as string/numeric with a BTL that must be extended; `arkiv-contrib/arkiv@2a3526d` instead has ten typed attributes, `bytes` payloads up to 128 KiB, a 32-attribute cap, PATCH, owner-transferable entities, and permanent entities (`MAX_EXPIRES_AT`); P0 must confirm which API the target testnet actually runs, since the mapping above assumes the typed one.
- "Ref heads are anchored so a replayed old ref is rejected" is partly already in heartwood: at `FeatureLevel::Parent` re-wrapped replays are already rejected; the new property anchoring adds is freshness for fresh clones and global equivocation detection.
- Storage is not pluggable behind a trait: the plan's "Arkiv replaces replication" holds, but there must be a local heartwood-layout bare repo; Arkiv cannot be "the storage".
- `readonly` does not make an anchor undeletable; rollback via owner delete is a residual risk tied to the EVM owner key, not the Radicle key.
- `radicle-ssh`, `radicle-git-ext` and `git-ref-format` are not heartwood crates any more.
- heartwood's "private" repos are access-controlled, not encrypted, so P2 encryption is new work regardless of the reuse choice.
- 100 MB clone gate: at 128 KiB per `bytes` attribute that is at least 800 chunk attributes; per-tx size and gas limits for multi-attribute entities are not specified in the spec and must be measured in P0.

# Why Arkiv, and what users lose if it were Postgres

Imagine the same git hosting product with a Postgres table of repos, signed refs and pack chunks behind an API.
Everything would work on a good day.
These are the things users would lose, and what Charter does about each one.

## 1. Rollback protection

The attack: a host (or whoever controls it) serves yesterday's branch head to a client that wants today's, hiding a security fix or reviving a vulnerable version.
With Postgres the operator owns the row order and the clock, so a replay of an old signed ref is indistinguishable from the truth.

What Arkiv adds:

- Arkiv gives one global creation order that every client reads the same way: block, transaction, entity key.
  A peer's pushes form a signed chain (each anchor commits to the hash of the previous one and to the refs of that push), and the first anchor created for a sequence number wins.
- Re-posting old signed bytes cannot displace the real head: another account's copy is ignored by the creator rule, and the peer's own account re-posting the old anchor has a taken sequence number and identical bytes, so it changes nothing.
  Creating the next anchor needs the peer's ML-DSA key, which the EVM keys do not carry.
- `entityNonce`, the count of entities an address has ever created, lets a client compare it with the number still alive.
  If the newest anchor expired or was hidden, the live count is below the nonce and the fetch fails closed.
- A local high-water mark on the user's machine remembers the newest anchor each client has seen, so a client that already saw the head refuses an older one.

Measured: on the live Rabdi run (2026-11-07), a client holding the newest mark was served a view that hid the newest anchor with `entityNonce` lowered to match, and got `E_STALE_HEAD`.
Replays from a foreign address and from the peer's own control address left a fresh clone at the newest head.

Where this stops: a fresh client with no mark, served by an RPC that lies consistently about both the entity list and `entityNonce`, would be fooled.
That is the stated trust model: Arkiv is trusted for ordering and availability.
Postgres would make that the whole story, with no signed chain, no nonce and no mark to catch an inconsistency.

## 2. Anyone can verify, and anyone can keep a repo alive

- Everything a client needs is public data on Arkiv plus signatures it checks itself: a stranger can audit a repo without our API, our permission or our uptime.
- Every entity carries `permissionlessExtension`, so lifetime extension is open to anyone with a funded key and no relationship to the owner.
  Measured on Rabdi: a second, unrelated key extended a flagged entity for 46,469 gas paid by itself, and the owner stayed the creator; the same key was refused on an unflagged entity.
- With Postgres the operator decides who can pay to keep data; when the operator stops paying or closes the account, the repo is gone for everybody.
- The flag lets anyone extend and never edit, because entities are also `readonly` (nobody can patch the content).

## 3. No host can rewrite or withhold history undetectably

- Rewriting: every ref set, identity revision and anchor is signed by a composite ML-DSA-65 + Ed25519 key, and a signature counts only if both halves verify.
  A host cannot forge a commit, a ref or an identity change.
- Withholding: dropping the newest anchor, or letting a control entity expire, fails closed (`E_CONTROL_NOT_LIVE`, `E_STALE_HEAD`, `E_NOT_LIVE`) instead of silently showing an older state.
- Content: chunks are checked against hashes in a signed manifest, packs are checked against their hash and by `git index-pack --strict`, and every object id in the head refs must be present and connected.
- The remaining power of the operator is refusing to serve or to include writes, which is visible as a failure (see CROPS in the README), not a quiet change of content.

## 4. Ownership per wallet, not per account row

- The user holds one 32-byte seed on their machine.
  From it come an ML-DSA + Ed25519 signing identity and secp256k1 wallets: one that funds, and per repo a control address and a bulk address.
- Ownership on Arkiv is the creating address, which the node records and nobody can change as the creator, and a signed bind entity links each peer to its per-repo addresses.
  No operator account exists that can be suspended, transferred or sold.
- Per-repo addresses make the completeness check exact and make one user's repos unlinkable by address.
- Honest limit: the owner key of an entity can still delete it or transfer ownership; `readonly` blocks content edits only.
  Deletion by a compromised EVM key is a denial of service that the completeness check turns into a loud failure, not a rewrite.
  secp256k1 is not post-quantum, which is why the signature that matters (ML-DSA) is separate from the key that owns the entity.

## What we keep off Arkiv, and why

| Stays on the user's machine | Why it is not on Arkiv |
| --- | --- |
| The local git cache (a bare repo with verified refs per peer) | It is derived data: it can always be rebuilt by a fresh fetch, and putting it on-chain would only cost gas |
| The seed and keys (`keys/seed`, file mode 600) | Anything on a public chain is permanent and public; a key there is a leaked key |
| The high-water marks and applied-pack list (`state.json`) | They are the client's own memory of what it has seen, which is exactly what must not be controllable by the operator |
| Pending uploads of an unfinished push | They are local intent; the chain is the source of truth, so a retry re-reads it |

The high-water mark is the important one.
If it lived on Arkiv, the party we guard against would hold the data that guards against it.
It only ever increases: the only reset is deleting the repo's local directory, which deliberately turns the client back into a fresh clone.

# How you use Arkiv

Four Arkiv features each change something a user of arkiv-git can see or do.
This file says which feature, and what shows up on the terminal.

## 1. Filters and indexed attributes: finding repos and fetching quickly

Arkiv indexes attributes, so arkiv-git can ask for exactly the entities it needs instead of scanning.

- `arkiv-git search` is a set of annotation queries on public repos: exact name, name prefix, language, licence (several allowed), a peer's repos, and "pushed in the last N days" newest first.
  Example shapes are in `schema.md`.
- Every hit is a hint.
  The client reduces it to a repo id, fetches the identity chain, verifies it, and drops hits whose current head no longer matches (a renamed repo, a changed licence).
  What you see are verified results with name, description, delegates, last anchor block and minimum expiry, so a dead or spam repo is visibly dead.
- Cloning uses the same indexes: a few control-plane queries by repo id and peer id, then chunk pages of 16 selected by pack hash and chunk index (`ag.pack`, `ag.i`), up to 8 pages in flight.
  A 100 MiB clone is about 110 queries.
- Because Arkiv has no `NOT`, no `!=` and no `$createdAt` filter, recency comes from a signed `ag.at` block number on every anchor, and exclusions run client side (see `friction.md`).

## 2. Ownership: only the peer's own control address counts

Arkiv records which address created each entity and that never changes.
arkiv-git uses it as the creator rule.

- Each peer signs a `bind` entity naming two addresses for a repo: a control address for identity, binds, and anchors, and a bulk address for refs, manifests and chunks.
- An entity is evidence only if its creator is one of the addresses named by that peer's valid bind chain.
  Any other account can copy the signed bytes of a real anchor, and clients still ignore the copy.
- What the user sees: a replay does nothing.
  On the live 2026-11-07 run, the old anchor and refs re-posted from a foreign address, and the old anchor re-posted from the peer's own control address, left a fresh clone at the newest head.
- Two valid anchors with the same sequence number and different bytes show up as `warning: E_EQUIVOCATION peer <nid> seq <k>` on every fetch and in `arkiv-git status`.
  The first one created wins, which every client computes the same way.

## 3. Expiry: keepalive, and a check that fails closed

Every Arkiv entity expires.
arkiv-git picks lifetimes by what the entity is for, and shows the user the cost of staying alive.

- Data (`refs`, `manifest`, `chunk`) is created for 30 days, because storage gas is linear in lifetime and a long first lifetime buys nothing.
- The control plane (`peer`, `bind`, `id`, `idacc`, `anchor`) is created for the maximum, 182.5 days, because it is small and the completeness check needs every one of them alive.
- `arkiv-git keepalive <repo>` extends everything the verified repo needs to a fresh horizon (default 90 days, for entities with under 30 days left), control plane first, in batches of up to 100 extensions.
  It reads only verified state, so it never pays to extend spam carrying the repo id.
  `--max-spend` caps the bill, `--dry-run` shows what it would do, and `--evm-key` lets a sponsor pay without the owner's keys.
- `arkiv-git status <repo>` prints each peer's head, the minimum expiry in days, and entities expiring within 14 days; a plain `git fetch` warns when the minimum is under 7 days.
- The completeness check compares each control address's `entityNonce` (entities ever created) with its live entity count.
  If any control entity has expired or been deleted, the fetch fails closed with `E_CONTROL_NOT_LIVE` and never falls back to an older head.
  If a data entity has expired, the clone fails with a message that names it:

```text
fatal: arkiv-git E_NOT_LIVE: pack 3f1c...a9 chunk 17 of 1600 is not live on Arkiv (expired or deleted).
This repo's keepalive lapsed; the data cannot be recovered through queries.
Peers that cloned earlier still hold it locally and can re-push it.
```

That sample comes from the design (`docs/p1/design.md` section 5), not from a captured run.

- Cost shape, derived from testnet measurements and not a mainnet price: about 28 GLM for a 100 MB repo for a year on Rabdi's placeholder pricing, mostly storage.
  The live 10 MiB gate spent 0.4352 tGLM including the push.

## 4. Permissionless extension: anyone can pay to keep it alive

- Every entity is created with the `permissionlessExtension` flag, set at creation and impossible to add later.
- What the user sees: a repo's keepalive is not tied to its owner.
  Co-maintainers, a sponsor or a CI job run `arkiv-git keepalive` with any funded key and no signing identity.
  On Rabdi we measured a second, unrelated key extending a flagged entity for 46,469 gas while the owner stayed unchanged.
- The flag lets anyone extend and never edit, because the entities are also `readonly`.
- The trade-off: anyone can also keep an old anchor alive after the head expired, which would defeat a "newer entity expires first" defence.
  arkiv-git replaces that defence with the creator rule plus the `entityNonce` completeness check, so a lapsed head fails closed instead of exposing an older one.

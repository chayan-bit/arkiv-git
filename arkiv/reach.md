# Who Charter is for

Charter is git hosting where the host is a public data layer instead of a company.
You push with plain `git push`, you clone with plain `git clone`, and no server of ours sits in between.
There are no user numbers in this file because there are no users yet; it is a hackathon project, and what follows is who it is built for.

## Open-source maintainers who want hosting that survives any single company

- A repo's name is the hash of its first signed identity document, not a URL on someone's domain.
  If a forge changes its terms, bans a project, is acquired or shuts down, the repo id and every signature in it are unchanged.
- The data lives on Arkiv and is verified by the client: a fetch checks every signature, every hash and the chain of signed ref updates before git ever sees a commit.
- Anyone can keep a repo alive, not only the owner, because every entity is created with `permissionlessExtension`.
  A maintainer, a sponsor or a CI job with a funded key can extend lifetimes without holding the maintainer's signing identity.
- Search works from public annotations (name, language, licence, recent pushes) and every hit is checked against its identity chain before it is shown, so spam is visible as spam.

What to know before relying on it: data on Arkiv expires unless someone pays to extend it.
That is a cost you can see (`chart status`, `chart keepalive`), not a surprise, but it is a real operating task.
Arkiv runs a single sequencer today, so it can refuse writes; see the CROPS section of the README.

## Teams who need private repos without trusting a host

- Private repos are in progress on a feature branch and are not merged.
  The design stores only ciphertext on Arkiv and uses keyed-hash identifiers so the chain does not reveal repo names or file hashes.
- The goal is that the host (Arkiv, an RPC provider, a mirror) never holds keys and cannot read content, and that members can be added and removed with key rotation.
- Today only public repos work end to end.
  Private repos are described honestly as work in progress, not a feature.
- One caveat holds even then: the RPC provider you query still sees which entities you ask for.
  Teams should use their own Arkiv node or a privacy-preserving RPC.

## AI agents and contributors who get paid by verifiable rules

- Issues, patches, reviews and attestations are signed operations in a shared, ordered log (collaborative objects).
  An agent's contribution is a signed op under a key anyone can check, in an order Arkiv fixes, not a row in someone's database.
- On feature branches (not merged to `main`), a repo can pin a small deterministic program by hash in its identity document.
  Every client runs the same program over the same ordered operations and ref updates and reaches the same state: which pushes a branch accepts, what is owed.
  A bounty template turns "this patch was merged" into a pay obligation, and a CI-attested template lets `main` move only to a commit that enough named attesters reported as passing.
- The program computes the obligation; checking that a payment really happened happens outside the program.
  We have not shipped escrow or settlement on `main`, and Arkiv cannot run contracts, so settlement would live on a separate chain.

## Try it today

- It runs today on Arkiv's Rabdi testnet, with test funds only.
  Never use real funds or mainnet keys with it.
- A local in-memory backend lets you try the whole flow without a chain.
  Both paths are in the README under "How to try it".
- The author intends to keep building after ETHGlobal Mumbai, starting with the items the README lists as in progress.

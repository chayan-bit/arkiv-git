# arkiv-git

Decentralized git hosting on Arkiv: a redesign of the Radicle (heartwood) protocol where Arkiv replaces gossip and seed nodes.
Arkiv is trusted for ordering, availability and timestamps only; clients verify every signature.

Plan of record: the "Arkiv Git: decentralized code hosting plan" doc (private Claude Doc).

## Phases in this repo

- P0 spike: measure Arkiv limits, confirm OpenMLS X-Wing support, decide fork vs depend on heartwood. Findings in `docs/p0/`.
- P1 core: `git-remote-arkiv`, identity documents, signed refs, ref anchoring, search.
- P2 private: encrypted repos with MLS groups (X-Wing), roles, rotation, blinded annotations.

Testnet only. Never use real funds or mainnet keys.

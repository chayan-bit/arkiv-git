# P3: accountable escrow

Status: implemented (store, escrow crate, CLI), hackathon scope.
Date: 2026-11-07.
Builds on `docs/p3/cob-ops.md`; code in `crates/charter-escrow`.

## Why

Repo programs want to move money: patch bounties, sealed-patch purchases, review staking, stake-to-submit and paid access.
Arkiv has native tGLM transfers but no contracts, so something has to hold the funds.
This version uses one escrow agent key now and moves to k-of-n threshold ECDSA later.

## Trust model

The agent is trusted to act, not to be honest.
It can steal (send funds anywhere) or stall (never pay), because it holds the only key.
It cannot do either undetectably: every deposit, payout and refund is a signed op tied to a chain tx, and anyone can recompute the books.
`chart escrow audit` needs no keys, reads only public ops, the program's obligations and chain txs, and exits 1 on any violation.
A misbehaving agent is therefore caught with public proof, and the delegates replace it with a new `appoint` op.
Arkiv is trusted for ordering and availability only, as for every other op.

What the audit checks:

1. Missing payout: a pay obligation whose purpose is funded, still unsettled `--grace` blocks (default 300) after it became due.
2. Unauthorized payout: a `paid` op with no matching pay obligation (key, recipient and amount), a `refunded` op for a deposit that is not refundable, or a mined tx of the escrow key with no settlement op (the mined nonce exceeds the number of verified settlements).
3. Double pay: two settlement ops with the same key, or two claiming the same tx.
4. Bad receipt: a settlement op whose tx is missing, reverted, or not a transfer from the escrow address to the named recipient of exactly the named amount.
5. Balance shortfall: the escrow balance is below credited deposits minus verified settlements.

The nonce check depends on one rule: the payout key sends payouts and refunds only.
The agent's ops are signed by its separate Charter identity and paid for by that identity's own wallet, never by the payout key.
A gas float (tGLM sent to the escrow address and not claimed as a deposit) pays payout gas; if gas eats into deposits, the audit reports a shortfall.

Known limits of this version:

- A deposit claim is first come, first served: whoever posts the first `deposit` op for a tx picks its purpose; refunds still go to the tx sender.
- Refunds return a whole deposit, and only when no pay obligation uses its purpose.
- A journaled transfer stuck at a low fee is rebroadcast, never replaced with a higher fee.
- An in-flight payout whose `paid` op is not posted yet reads as an unaccounted tx until the agent's next round.

## Keys

- `chart escrow init` creates `$CHARTER_HOME/escrow/keys/seed` (the escrow identity) and `$CHARTER_HOME/escrow/keys/payout` (the payout EVM key), both mode 0600.
- It prints the identity id, the escrow address and the identity's wallet address, never key material, and refuses to overwrite.
- `chart escrow show` prints the same three lines.

## Op formats

All escrow ops are `cob` ops (docs/p3/cob-ops.md) of type `xyz.arkiv.escrow`.
A delegate creates one escrow object per repo with action `create` and payload `{}`, then posts `appoint` in it; all later escrow ops are follow-ups of that object.
Amounts are decimal wei strings and tx hashes are `0x` hex strings, so payloads stay canonical CBOR without big integers.

| Action | Author | Payload |
| --- | --- | --- |
| `appoint` | a current delegate | `{escrow: <identity id>, address: <0x escrow address>}` |
| `deposit` | anyone (the payer) | `{tx, amount, purpose, expiry?}` |
| `paid` | the appointed escrow | `{key, tx, to, amount}` |
| `refunded` | the appointed escrow | `{key: "refund:<deposit tx>", deposit: <deposit tx>, tx, to, amount}` |

- The appointment in force is the latest `appoint` op authored by a delegate of the current identity head.
- A deposit is credited only if its tx is mined, succeeded, went to the escrow address, was not sent by the escrow itself, moved at least `amount`, `amount` is not zero, and no earlier deposit op claimed the same tx.
- `purpose` is a free-form key such as `bounty:<issue id>`; `expiry` is a block number after which an unconsumed deposit is refunded to the tx sender.
- `paid` and `refunded` ops by anyone other than the appointed escrow are ignored, so a forged `paid` op cannot block a payout.

## Obligations

The escrow pays only what the repo program says is owed.
Programs return `obligations`, each a map `#{ kind: "pay", to, amount, reason, key }`, plus two extension fields this crate reads:

- `purpose`: the deposit purpose that funds the obligation (defaults to `""`).
- `since_block`: the block at which it became due, for the audit's grace period (defaults to 0).

`to` is a `0x` address or an identity id; an identity id is paid at its latest control address for the repo.
`key` is unique per program, and the escrow settles each key at most once.
The crate reads obligations through `trait ObligationSource { fn obligations(&self, rid: &Rid) -> Result<Vec<Obligation>> }`.
`FixedObligations` is the fixed-list implementation used by the tests and by the CLI's `--obligations <file.json>` override.
By default the CLI reads the repo's pinned program through `ProgramObligations` (`crates/charter-cli/src/escrow.rs`), which fetches the repo into the local cache, folds the program with `charter_sync::program::obligations_in`, keeps the `pay` obligations and turns JSON integer amounts into decimal strings.
`since_block` comes from the program: an obligation that carries `due_order` (the `order` of the event that made it due, for example the accepted push in `bounty.rhai`) gets the block of that event.
Funds are allocated to unsettled pay obligations in program order, per purpose.

### Private repos

A private repo's program folds sealed ops, so only members can compute its obligations.
Every `chart escrow` command that names a repo (`appoint`, `deposit`, `run`, `audit`) detects a private repo the way the remote helper does and then runs over the repo's `Sealed` view.
The escrow's own ops are then sealed too, and the ledger and the obligations are read through the same view.
An escrow agent for a private repo must therefore run on a machine whose identity (`chart keys show`) is a member with at least the read role; a non-member gets `E_NOT_MEMBER`.
An auditor of a private repo's escrow must be a member for the same reason, so the audit is public only among members.
Chain transfers (deposits, payouts, refunds) are plain transactions and stay visible to everyone.
The agent reads its keyring once at start, so after a member removal rotates the key a long-running `escrow run` must be restarted to see newer ops.

## How programs use it

- Patch bounties: a maintainer deposits with purpose `bounty:<issue id>`, and `bounty.rhai` emits a pay obligation keyed by the issue to the merged patch's author once the patch is accepted.
- Sealed-patch purchases: the buyer deposits with purpose `sealed:<patch id>`, and the program emits the pay obligation to the seller only after it has verified the reveal op against the sealed commitment; an expiry refunds the buyer if no valid reveal arrives.
- Review staking: a reviewer deposits a stake with purpose `stake:<review id>` and an expiry; an honest review lets the expiry refund it, and a slashed review makes the program emit a pay obligation of the stake to the repo treasury address.
- Stake-to-submit and paid access follow the same two shapes: a deposit that is refunded on good behaviour, or a deposit the program pays to the maintainers.

## Agent

`chart escrow run --repo <rid> [--obligations <file>] [--once] [--interval <s>]` runs rounds; one round:

1. refuses to act unless the repo's appointment names this escrow's identity and address;
2. finishes any journaled transfer (below);
3. folds the ledger and the obligations, and pays each funded, unsettled pay obligation;
4. refunds each expired deposit whose purpose no pay obligation uses.

Crash safety: before a transfer is sent, `$CHARTER_HOME/escrow/journal/<rid>.json` records the settlement it is for and the exact tx parameters, nonce included.
After a crash, the next round checks for a settlement op with that key first, then for the journaled tx's receipt, posts the op if the tx landed, and otherwise re-signs and rebroadcasts the identical tx.
Because the retry reuses the journaled nonce, it can never become a second transfer, and keys that already have a settlement op are never planned again.
A lock file keeps one agent per repo.
Payout transfers are signed directly rather than through `TxPipeline::transfer`, because the pipeline returns the tx hash only after the tx lands, which leaves nothing to journal before a crash.

## Other commands

- `chart escrow appoint --repo <rid> --escrow <id> --address <0x>`: run by a delegate; prints the escrow object id.
- `chart escrow deposit --repo <rid> --amount <wei> --purpose <p> [--expiry <block>] [--tx <0x>]`: sends the amount from the wallet through `TxPipeline::transfer` (or claims an already sent `--tx`) and posts the deposit op.
- `chart escrow audit --repo <rid> [--obligations <file>] [--grace <blocks>]`: prints the ledger summary and one `FLAG` line per violation; exits 1 if there is any.

## Store support

The audit needs to know what a tx moved, so the `Arkiv` trait gained two read-only methods:

- `transaction(h) -> Option<TxInfo { hash, from, to, value, block_number }>`, from `eth_getTransactionByHash`;
- `tx_count_mined(a)`, from `eth_getTransactionCount` at `latest`; `tx_count` keeps its pending semantics for the pipeline.

MemArkiv records a `TxInfo` for every mined tx, and FileMemArkiv rebuilds them when it replays its log.

## Upgrade path

- Threshold ECDSA: replace the single payout key with a k-of-n key shared by independent operators using CGGMP21 (for example the `cggmp21` crate).
  The escrow address stays an ordinary EVM address, so ops, ledger and audit do not change; only signing a transfer becomes an interactive protocol among the operators.
  Stealing then needs k colluding operators, and stalling needs n - k + 1 of them refusing; the audit still proves either.
- Contract chain: on a chain with contracts, the same ops can drive an escrow contract that releases funds only against a program-verified obligation, removing the operator's ability to steal; the audit then serves as a cross-check.
- Claim binding: bind a deposit op to its tx sender (for example a signature by the sending key over the deposit op) to close the first-claim race.

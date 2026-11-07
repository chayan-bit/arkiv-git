# Scoped AI agents

Status: implemented as a repo program and tests, hackathon scope.
Date: 2026-11-07.
Builds on `docs/p3/programs.md`, `docs/p3/cob-ops.md`, `docs/p3/escrow.md` and `docs/p4/zk-sealed-patch.md`; code in `programs/agent-bounty.rhai`.

## Why

An AI agent working on a repo today has two bad options.
It can hold a human's full key, which lets it do anything that human can, forever.
Or it can use its own key, which has no standing: nothing says a human vouches for it or bounds what it may do.
A grant fixes this: a human delegate signs an op that gives the agent key a few scopes, a payout cap and an expiry, the pinned program enforces it on every client, and revocation is just another op.

## Why no protocol change

Ops are already signed by their author and folded in Arkiv order by the pinned program on every client.
So "which agent may do what until when" is program state, like a bounty or an approval.
The grant and revoke ops are ordinary `cob` ops of type `xyz.arkiv.agent`, so the wire format, the identity document and the remote helper are unchanged.
A repo that does not pin an agent-aware program simply ignores these ops.

## Op shapes

All payload ids are 64-char lowercase hex, as programs see them.
A delegate first creates one grants object (`op post --type xyz.arkiv.agent --action create --payload-json '{}'`), then posts follow-up ops in it.

| Action | Author | Payload |
| --- | --- | --- |
| `grant` | a human delegate | `{agent: <nid hex>, scopes: ["patch", ...], max_bounty: <int>, until: <arkiv order>}` |
| `revoke` | the granter or any human delegate | `{agent: <nid hex>}` |

- `scopes` names op types by the part after `xyz.arkiv.`: `patch`, `comment`, `review`, `attest`, `issue`, and so on.
- `max_bounty` is in the same smallest units as the bounty (wei for tGLM).
- `until` is an Arkiv order, the only clock a program has (`op.order`); an agent op with a larger order is rejected.
  `op.ts` is not used because the agent writes it.
- A later grant for the same agent replaces the earlier one, including a revoked one.
- Only a human delegate may grant: a non-delegate's grant would confer nothing a fresh key does not already have, so sub-agent grants are left out.
- `chart keys show` prints the nid in `z` base58 form; its hex form is what goes in `agent`.

## Rules the program enforces

`programs/agent-bounty.rhai` is `bounty.rhai` plus an "Agent delegation" section.

1. An op whose author holds a grant is checked before any other rule: it is ignored if the grant was revoked, if its order is past `until`, or if its scope is not granted.
2. Every ignored op is recorded in `state.rejected` under its op id with the reason, so `chart program status` shows it.
3. A revoke takes effect after its own order; agent ops folded before it keep their effect.
4. An agent is never a delegate: `is_human_delegate` excludes any granted key, even one listed as a delegate, so an agent can neither grant, nor approve, nor record a merge.
5. Approvals (`xyz.arkiv.review` `approve` with `{patch}`) count only from a human delegate who is not the patch author, so an agent's self-approval is ignored and recorded.
6. A merge (`xyz.arkiv.patch` `merged`) counts only from a human delegate.
7. `main` may move to an agent patch head only after one human approval, when `agent_needs_approval()` at the top of the script returns true (the default); the remote helper refuses the push locally with `E_PROGRAM_REJECTED` otherwise.
8. `agent_proof()` sets the proof policy for agent patches: `"required"` drops an agent patch without a matching proof, `"optional"` (the default) records `proven: true | false`, `"none"` ignores proofs.
   A matching proof is a verified `risc0` attachment for the pinned sealed-patch guest whose `spec_hash` equals the issue's `spec_hash` when the issue sets one.
9. When the merged agent patch head is accepted on `main`, the bounty becomes a `pay` obligation to the agent's nid if it is at most the grant's `max_bounty`, snapshotted when the patch was created; a larger bounty is not paid, stays open, and is recorded under `bounty:<issue id>` in `state.rejected`.
10. Patches by keys without a grant follow plain `bounty.rhai` rules.

The payout rule is one rule: the agent's nid.
The escrow pays an identity id at its latest control address for the repo, which the agent's own op posts and push create.

## Reusing the delegation section

Rhai programs here have no modules, so reuse is by copying.
The "Agent delegation" section of `agent-bounty.rhai` is self-contained: `agent_init(repo)` builds the state it needs, `agent_denial(state, op)` returns the reason an agent op is ignored (empty when it may proceed), `agent_admin(state, op)` handles grant and revoke, and `is_human_delegate(state, who)` replaces a plain delegate check.
A program such as `ci-attested.rhai` or `sealed-patch.rhai` gets scoped agents by calling these at the top of `on_op`.

## The agent loop

`crates/charter-cli/tests/agent_bounty.rs` runs the whole loop on the in-memory fake through the real binaries.

1. Alice (delegate) appoints an escrow, pins `agent-bounty.rhai`, grants the agent `patch` with a cap, opens an issue with a bounty and deposits it.
2. The agent, in its own `CHARTER_HOME`, polls `chart op list --repo <url> --json`, picks the first issue with a bounty that no patch names, commits and pushes the commit to its own namespace.
3. It runs `chart op attach-proof --type xyz.arkiv.patch --action create --payload-json '{head, issue}' --receipt <file> --public <file>`, which posts the committed real receipt as blob ops (skipping chunks already on the repo) and then the patch with `proofs: [...]`; a second poll finds nothing open.
4. Alice fetches the agent's branch from `arkiv::<rid>/<agent nid>`; her push of `main` is refused until she approves, then she approves, records the merge and pushes.
5. `program status` shows the `pay` obligation to the agent, `proven: true`, and the agent's out-of-scope comment as rejected; `chart escrow run --once` pays it and a keyless `escrow audit` is clean.

With the real CLI the loop is the same commands in a shell loop: `op list --json`, `git push`, `op attach-proof`; `docs/p3/mcp.md` exposes them to an AI agent as MCP tools.

## Threat model

What a grant authorizes: ops of the granted scopes, until `until` or a revoke, and bounty payouts up to `max_bounty` per patch, only after a human approved the patch and pushed it to `main`.
It never authorizes ref updates: only delegates' pushes are fold events, and the agent is not a delegate.

What a stolen agent key can do: post in-scope ops (spam patches, comments) until revoked, and claim the payout of a patch that a human then approved and merged.
It cannot move `main`, approve, merge, grant, or exceed the cap, so the loss is bounded by the cap times what humans approve.
It cannot touch the human's own key or funds.

How revoke works: any human delegate (or the granter) posts `revoke`; every client folds it in Arkiv order, so every agent op ordered after it is ignored everywhere.
Ops ordered before it keep their effect, which is what "effective from that op's order" means.
A thief racing the revoke can still land ops ordered before it.

## Limits

- `until` is an Arkiv order, and orders are ranks in the client's creation-ordered query; if entities ordered before an agent op expire, later ranks shift down and an op near the bound can flip from expired to valid.
  Use revoke for a hard stop and keep op lifetimes longer than the grant.
- An agent key used before its first grant is an ordinary contributor; its earlier patches follow plain bounty rules with no cap.
- The sealed-patch proof is about the issue's spec and a sealed key, not the pushed git head; binding the proof to the commit needs a guest that hashes the tree.
- The approval is per patch, not per head: there is no `revise` in this program, so a patch's head is fixed at creation.
- `state.rejected` grows with every ignored op and is bounded only by the engine's 10,000-entry map limit; a spammer with a granted key can fill it, after which further rejections are skipped with a log line instead of recorded; they are still ignored.
- One escrow and one payout rule; paying the granter instead of the agent is a one-line change in `on_push`, not a grant option.

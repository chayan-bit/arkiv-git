# Live Rabdi: an AI agent claims a bounty through arkiv-git-mcp

Commands below were recorded before the rename to Charter; `arkiv-git` is now `chart`.

Date: 2026-11-07 (UTC clock 2026-11-06 23:xx).
Network: Rabdi testnet, chain id 7794095, RPC https://rpc.rabdi.db-chain.testnet.arkiv.network, explorer https://rabdi.explorer.arkiv.network.
Code: `demo/agent-bounty.sh`, `crates/arkiv-git-mcp`, `crates/arkiv-git-cli/tests/rabdi_agent.rs` on `feat/mcp`.
Every live entity was written with a 1 day lifetime (43,200 blocks), so the repo expires around block 354,300.

Video (live recording, 39 s at 10x): https://raw.githubusercontent.com/chayan-bit/arkiv-git/demo-assets/agent-bounty-codex-live.mp4 (GIF: https://raw.githubusercontent.com/chayan-bit/arkiv-git/demo-assets/agent-bounty-codex-live.gif).

## Who did what

| Actor | Identity | How it acted |
| --- | --- | --- |
| alice, the human delegate | nid `z6YXr8uhgLeZjs3G5j9DiDhBDPxzqe7wcMi6hVLwsmrEz`, wallet [0x0a25...9E2b](https://rabdi.explorer.arkiv.network/address/0x0a253FFA8c79c723F7Ca0BeD7c372ae083589E2b) | `demo/agent-bounty.sh alice` and `merge` (the real `arkiv-git` and `git` binaries) |
| the agent | nid `zDZCJpyYKvKQrwSeiK5JKrDrMGTRnTU94Ut9qyhZ552fP` (hex `ba8b9d41...9a80b6`), wallet [0xF5dB...bDc5](https://rabdi.explorer.arkiv.network/address/0xF5dBf96186436fbA05BE0C9b853d3D43D4e5bDc5), control address for the repo [0xecfb...7933](https://rabdi.explorer.arkiv.network/address/0xecfb355e539dbf8f7680c4bfb1e93abb34697933) | OpenAI Codex CLI 0.164.0, `codex exec --json`, model `gpt-6.1-sol` (ChatGPT login), with `arkiv-git-mcp` as its only arkiv-git interface |
| the escrow | id `zCcDfUNZAJPtdKukdxohT6AFehVNQ8B7P8sx8CfrmX1aK`, payout address [0x0e22...0a36](https://rabdi.explorer.arkiv.network/address/0x0e228198c9C8C44716b74fB9Ab9Ce109f60f0a36), op wallet 0x43F2...5341 | `arkiv-git escrow run --once` |
| bank | wallet [0xd259...24aA](https://rabdi.explorer.arkiv.network/address/0xd259b853416bE9CFCD31c35Df10f00D90ea024aA) | funded the others from the throwaway key and collected the leftovers |

Repo: `arkiv::zEFXyo2GHn5eJ3KWESaoeSFZZNUWUBb91C7jrusARmNeW`, program `rhai/1 9dc4dc9d...c5c8de` (`programs/agent-bounty.rhai` with `agent_proof()` set to `"required"`).

## Results

| Step | Result | Evidence |
| --- | --- | --- |
| alice creates the repo with `add()` returning `a - b`, appoints the escrow, pins the program | PASS | escrow `create` and `appoint` ops at blocks 311,116 and 311,120; program `publish` at 311,130 |
| alice grants the agent scope `patch`, cap 0.002 tGLM, and opens an issue with a 0.001 tGLM bounty and the receipt's `spec_hash` | PASS | agent `grant` at 311,146; issue `76f13028...0361ce` at 311,153 |
| alice deposits the bounty with the escrow | PASS | escrow `deposit` op at 311,162 |
| the Codex agent finds the open bounty | PASS | its first call is `op_list` on the repo; it picks issue `76f13028...` because no patch names it |
| the agent clones, fixes and pushes | PASS | `repo_clone` into `work`, one sandboxed `apply_patch` edit (`a - b` to `a + b`), `git_push` to `agent/76f13028` returned head `2bfd609bba3d771cc30a182431e0249fc7bfabce` |
| the agent attaches the real RISC Zero receipt and posts the patch | PASS | `attach_proof` posted 8 `xyz.arkiv.blob` ops (blocks 311,275 to 311,308) and patch `14ecb288f9162b8c3d97492dd72d5d53395286b094afe387833fc4ecef54f2bf` at 311,312; the fold shows `proven: true` |
| `main` refuses the agent head before a human approval | PASS | alice's push failed with `E_PROGRAM_REJECTED: agent patch 14ecb288... needs a human delegate approval` |
| alice approves, records the merge and pushes `main` | PASS | review `create` and `approve` at 311,332 and 311,337, patch `merged` at 311,343; `program status` shows `head: refs/heads/main 2bfd609...` and a `pay` obligation of 1,000,000,000,000,000 wei to the agent's nid |
| the escrow pays the agent | PASS | tx [0x1d4696d6...daef351](https://rabdi.explorer.arkiv.network/tx/0x1d4696d6dcda488615bfcfb50e88205c29a7598fef2da9ee397ae74d1daef351), block 311,384, status 1, from the escrow address to the agent's control address, value 1,000,000,000,000,000 wei; `paid` op at 311,389 |
| keyless audit | PASS, no flags | `escrow audit` by the bank identity: credited 1 deposit, settled 1 op, balance 979,000,000,000,000 wei, owes 0 |

The agent's control address held 2,590,016,000,000,000 wei right after the payout, so it held 1,590,016,000,000,000 wei just before it: the payout is the only transfer to that address after the agent's last op at block 311,312.
The agent spent 589,015 input and 1,045 output tokens.

## How it was run

```sh
MODE=live ROOT=/tmp/agent-demo demo/agent-bounty.sh setup
lockf <lock> env ARKIV_GIT_RABDI=1 ARKIV_GIT_AGENT_ROOT=/tmp/agent-demo ARKIV_GIT_AGENT_FUND_WEI=60000000000000000 \
  cargo test -p arkiv-git-cli --test rabdi_agent bank_fund -- --ignored
ARKIV_GIT_RABDI=1 ARKIV_GIT_AGENT_ROOT=/tmp/agent-demo ARKIV_GIT_AGENT_PAY=<alice>:18000000000000000,<agent>:32000000000000000,<escrow wallet>:4000000000000000,<escrow address>:1000000000000000 \
  cargo test -p arkiv-git-cli --test rabdi_agent bank_pay -- --ignored
MODE=live ROOT=/tmp/agent-demo demo/agent-bounty.sh alice
cd /tmp/arkiv-git && nix-shell -p vhs --run 'vhs demo/agent-bounty.tape'   # recap, agent, merge, verify
cargo test -p arkiv-git-cli --test rabdi_agent bank_collect -- --ignored   # then bank_return under the lock
```

The agent step is one `codex exec --json --ephemeral --ignore-user-config` run with `-s workspace-write` (no network for the model's shell), `-C` the resolved session directory, and the MCP server passed with `-c mcp_servers.arkiv-git.*` (command, cwd, the agent's `ARKIV_GIT_HOME`, `env_vars = ["ARKIV_ACCESS_KEY"]`, `tool_timeout_sec = 900`, `default_tools_approval_mode = "approve"`).
The access key reached only the MCP server's environment by name; it is not in the prompt, the Codex config or the recording, and no key or seed was printed.
`~/.codex/config.toml` was not touched.

## Spend

- Left the throwaway key: 0.06 tGLM in one transfer to the bank, under the shared lock (budget 0.08).
- Returned to the throwaway key: 0.029675571 tGLM.
- Left on Rabdi: 0.000979 tGLM gas float on the escrow payout address (kept so a later audit stays clean, since a sweep from the payout key would be an unsettled escrow tx) and 0.000039 tGLM on the bank as the sweep's gas reserve; each peer address keeps at most its own sweep reserve.
- Net cost of the whole run: about 0.0293 tGLM, most of it the agent's 8 receipt blob ops (about 0.0027 tGLM each per `docs/live/private-proofs.md`).

## What went wrong on the way

- Headless Claude Code could not authenticate here (`claude auth status`: `loggedIn: false`; `claude -p` in the desktop app's session: "OAuth session expired and could not be refreshed"), so the agent is Codex, as the user asked.
- The first Codex attempt on this repo stopped after `repo_clone`: its sandbox refuses a writable root reached through a symlink (`/tmp` on macOS), so the edit failed and nothing was pushed or posted; `agent` now resolves the session directory with `pwd -P`.
  alice's ops from that attempt are the ones reused here, so the recording starts with a `recap` that reads them back live instead of re-running `alice`.
- Rabdi serves state at the chain head only ("GolemDB cannot read at a past commit"), so the recorded `verify` step's before and after balance reads at the payout block both read the head and printed `delta 0`.
  That one line is masked in the published video from 35.8 s; the payout tx line above it and the explorer link below it are untouched, and `verify` now prints the receipt's block, sender, recipient and value plus the head balance instead.

## Honest gaps

- The agent's blob ops are recorded in `state.rejected` as "scope blob not granted", because the grant names only `patch`; proofs still verify (blob bytes are read from all ops) and the patch is `proven: true`, but a grant for proof-carrying agents should include `blob`, or the program should exempt blob ops.
- `program status` still lists the paid bounty under `owed`, because `agent-bounty.rhai` settles on `xyz.arkiv.payment` ops while the escrow writes `xyz.arkiv.escrow` `paid`; the escrow itself does not pay twice (its ledger settles by key) and the audit shows owes 0.
- The 1 day lifetime means the repo and its ops expire around block 354,300; no keepalive reserve was left, so the live evidence beyond that point is this report, the video and the explorer's tx history.
- The model reading the issue had no tests to run; the fix is a one-line change that the proof does not bind to the commit (see `docs/p3/agents.md`, Limits).

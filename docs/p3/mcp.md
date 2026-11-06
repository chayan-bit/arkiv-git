# MCP server for AI agents

Status: implemented and tested on the in-memory fake; the live Rabdi agent run is blocked, see "Live demo".
Date: 2026-11-07.
Builds on `docs/p3/agents.md` (scoped grants), `docs/p3/cob-ops.md`, `docs/p3/escrow.md`; code in `crates/arkiv-git-mcp`.

## Why

`docs/p3/agents.md` gives an agent key bounded authority, but the agent still has to drive a CLI by parsing text.
`arkiv-git-mcp` exposes the same commands as Model Context Protocol tools with JSON schemas, so any MCP client (Claude Code, Claude Desktop, other agent harnesses) can find a bounty, push a fix, attach a proof and post the patch.

## SDK survey

| Candidate | Version, licence | Maintenance | Fit |
| --- | --- | --- | --- |
| `rmcp` (modelcontextprotocol/rust-sdk, the official SDK) | 3.9.0, Apache-2.0 | pushed 2026-11-05, about 4,100 stars, 45.8 M downloads, 67 open issues, many maintainers | `#[tool]` macros generate the JSON schema from `schemars` structs; stdio transport behind `transport-io` |
| `rust-mcp-sdk` (rust-mcp-stack) | 2.0.0, MIT | updated 2026-08-27, about 200 stars, 0.3 M downloads | full-featured, but a much smaller user base and more single-maintainer risk |
| `mcp-protocol-sdk` (mcp-rust) | 0.5.1 | last release 2025-08-03 | stale, behind the current protocol |

We picked `rmcp`: it is the reference implementation, it is actively maintained, its licence is compatible with this workspace's `MIT OR Apache-2.0`, and with `default-features = false` plus `server`, `macros`, `transport-io` it adds only `rmcp`, `rmcp-macros`, `schemars_derive`, `serde_derive_internals` and `signal-hook-registry` to the lockfile (`schemars`, `tokio` and `serde` were already there).

## Design

The server is a thin wrapper: every tool runs one `arkiv-git` (or `git`) child process, so signing, fetching, sealed views of private repos, program folds and escrow rules stay in the CLI with no duplication.
It finds the CLI through `ARKIV_GIT_BIN`, else `arkiv-git` next to its own executable, else `arkiv-git` on `PATH`, and it puts that directory first on the children's `PATH` so `git` finds `git-remote-arkiv`.
Children inherit the server's environment, so the agent's identity is whatever `ARKIV_GIT_HOME` points at and the backend is `ARKIV_GIT_BACKEND` (Rabdi by default, with the optional `ARKIV_ACCESS_KEY`).
No tool takes or returns key material.

A failed child is a tool error (`isError: true`) whose text is `{"code", "message", "exit", "stdout"}`.
`code` is the first stable `E_*` code in the child's stderr (for example `E_PROGRAM_REJECTED`, `E_INSUFFICIENT_FUNDS`, `E_BAD_FORMAT`), else `usage` or `flagged` from the CLI's own line, else `null` (a plain git failure); `exit` is the CLI exit status (1 verification, 2 usage, 3 transport, 4 funds).

## Tools

| Tool | Arguments | Runs | Returns |
| --- | --- | --- | --- |
| `repo_clone` | `repo`, `dir` | `git clone arkiv::<rid> <dir>` | `{dir}` |
| `op_list` | `repo`, optional `type`, `action`, `object` | `arkiv-git op list --json` | JSON array of verified ops `{id, object, author, type, action, parents, ts_ms, block, payload}` |
| `op_post` | `repo`, `type`, `action`, optional `object`, `parents`, `payload` | `arkiv-git op post` | `{id}` |
| `attach_proof` | `repo`, `payload`, `receipt`, `public`, optional `type` (default `xyz.arkiv.patch`), `action` (default `create`), `object`, `parents` | `arkiv-git op attach-proof` | `{id}` of the op that carries `proofs` |
| `program_status` | `repo` | `arkiv-git program status` | the fold: state, heads, rejected pushes, obligations |
| `escrow_status` | `repo` | `arkiv-git escrow audit` (keyless) | the ledger; a flagged ledger is an error with code `flagged` |

`repo` is `arkiv::<rid>` or the bare rid.
`action` filtering in `op_list` happens in the server, because the CLI filters only by type and object.
There is no push tool: the agent pushes with `git push <url> HEAD:refs/heads/<branch>` from its clone, which the remote helper already handles, including program refusals.

`arkiv-git op attach-proof` is the CLI half of `attach_proof`: it chunks the receipt into `xyz.arkiv.blob` ops of at most 30,000 bytes, skips chunks whose `sha256` is already posted on the repo (so a retry after `E_INSUFFICIENT_FUNDS` only posts what is missing), then posts the given op with `{kind: "risc0", receipt, chunks, public}` appended to its payload's `proofs`.

## Setup with Claude Code

Build or install both binaries so they sit in one directory:

```sh
cargo install --path crates/arkiv-git-cli
cargo install --path crates/arkiv-git-mcp
arkiv-git keys init   # the agent's own identity, never a human's key
```

Register the server for the current project:

```sh
claude mcp add arkiv-git -e ARKIV_GIT_HOME="$HOME/.arkiv-git-agent" -- arkiv-git-mcp
```

Or commit an `.mcp.json`:

```json
{
  "mcpServers": {
    "arkiv-git": {
      "command": "arkiv-git-mcp",
      "env": { "ARKIV_GIT_HOME": "/path/to/agent/.arkiv-git" }
    }
  }
}
```

Put `ARKIV_ACCESS_KEY` in the environment Claude Code starts in rather than in the JSON, so it never lands in a committed file.
A headless agent run allows only the MCP tools, git and file edits:

```sh
claude -p "<task>" --mcp-config .mcp.json --strict-mcp-config \
  --allowedTools "mcp__arkiv-git__*" "Bash(git:*)" Read Edit Write
```

## Tests

`crates/arkiv-git-mcp/src/tests.rs` drives every tool through the real `arkiv-git` binary on the file-persisted fake chain: post and list with type and action filters, clone, `attach_proof` twice to check blob reuse and the `proofs` entry, `program_status`, and coded errors from the CLI (`E_*` with exit 1), from git (exit 128) and from the escrow audit.
`crates/arkiv-git-cli/tests/agent_bounty.rs` now posts its proof with `op attach-proof` and still checks `proven: true`, the approval gate and the escrow payout end to end.
A stdio handshake (`initialize`, `tools/list`) against the built binary lists the six tools with their schemas.

## Live demo

`demo/agent-bounty.sh` runs the whole bounty with headless Claude Code as the agent: `setup` creates the alice, escrow, agent and bank identities, `alice` creates the repo, appoints the escrow, pins `agent-bounty.rhai` with `agent_proof()` set to `"required"`, grants the agent `patch` with a cap, opens an issue whose `spec_hash` matches the committed receipt and deposits the bounty, `agent` runs `claude -p` with only this MCP server, git and file edits allowed, `merge` shows `main` refusing the agent head until alice approves, records the merge and runs `escrow run --once`, and `verify` runs the keyless audit and reads the payout tx and the agent's balance delta from the chain.
`crates/arkiv-git-cli/tests/rabdi_agent.rs` funds the demo from the throwaway key (one transfer, at most 0.08 tGLM) and sweeps it back; `demo/agent-bounty.tape` records the live phases.

The live run has not happened.
In this environment the standalone `claude` CLI is not logged in (`claude auth status` reports `loggedIn: false`), and a `claude -p` started from inside the Claude desktop app's session fails with "OAuth session expired and could not be refreshed".
No testnet funds were spent.
To run it, log the CLI in once (`claude auth login`), then follow the steps in the header of `demo/agent-bounty.sh` with `MODE=live`.
Video: not recorded yet.

## Limits

- One child process per call: `op_list` and `program_status` refetch from Arkiv each time, which is fine for an agent loop but not for a hot path.
- `program_status` returns the CLI's text lines (the state line is JSON) rather than a parsed object.
- The server trusts its own environment: whoever starts it chooses the identity it signs with, so give an agent its own `ARKIV_GIT_HOME` and a grant, never a delegate's home.

#!/usr/bin/env bash
# An AI agent works an arkiv-git bounty through the MCP server (docs/live/agent-bounty.md).
#
#   demo/agent-bounty.sh setup|alice|recap|agent|merge|verify|all
#
# ROOT holds one home per actor (alice, escrow, agent, bank) and state.env.
# MODE=mem runs on the file-persisted fake chain; MODE=live runs on Rabdi, with the
# wallets funded between `setup` and `alice` (crates/arkiv-git-cli/tests/rabdi_agent.rs).
# Keys stay in each actor's $ARKIV_GIT_HOME; nothing here prints key material.
#
# Live run (needs a logged-in `codex` CLI and ARKIV_ACCESS_KEY in the environment):
#   MODE=live ROOT=<dir> demo/agent-bounty.sh setup
#   lockf <lock> env ARKIV_GIT_RABDI=1 ARKIV_GIT_AGENT_ROOT=<dir> ARKIV_GIT_AGENT_FUND_WEI=<wei> \
#     cargo test -p arkiv-git-cli --test rabdi_agent bank_fund -- --ignored
#   ARKIV_GIT_AGENT_PAY=<alice wallet>:<wei>,<agent wallet>:<wei>,<escrow wallet>:<wei>,<escrow address>:<gas float> \
#     ... --test rabdi_agent bank_pay -- --ignored
#   MODE=live ROOT=<dir> demo/agent-bounty.sh alice   # then agent, merge, verify (demo/agent-bounty.tape)
#   ... --test rabdi_agent bank_collect, then bank_return (keep a keepalive reserve if the repo stays live)
# MODE=mem cannot fund the escrow's gas float, so there `merge` stops at `escrow run`.
set -euo pipefail

REPO_DIR=$(cd "$(dirname "$0")/.." && pwd)
ROOT=${ROOT:-$PWD/agent-demo}
MODE=${MODE:-mem}
BIN=${BIN:-$REPO_DIR/target/debug}
FIX=$REPO_DIR/crates/arkiv-git-zk/tests/fixtures
BOUNTY=${BOUNTY:-1000000000000000}  # 0.001 tGLM
CAP=${CAP:-2000000000000000}        # the agent's per-patch payout cap
RPC=https://rpc.rabdi.db-chain.testnet.arkiv.network
EXPLORER=https://rabdi.explorer.arkiv.network
CODEX_MODEL=${CODEX_MODEL:-gpt-6.1-sol}

export PATH="$BIN:$PATH" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=demo GIT_AUTHOR_EMAIL=demo@example.com
export GIT_COMMITTER_NAME=demo GIT_COMMITTER_EMAIL=demo@example.com
if [ "$MODE" = mem ]; then
  export ARKIV_GIT_BACKEND="mem:$ROOT/chain"
else
  unset ARKIV_GIT_BACKEND
  export ARKIV_GIT_DATA_LIFETIME=${LIFETIME:-43200} ARKIV_GIT_CONTROL_LIFETIME=${LIFETIME:-43200}
fi
mkdir -p "$ROOT"
STATE=$ROOT/state.env
touch "$STATE"
# shellcheck disable=SC1090
. "$STATE"

say() { printf '\n\033[1;36m== %s\033[0m\n' "$*"; }
keep() { echo "$1=$2" >>"$STATE"; eval "$1=\$2"; }
as() { local who=$1; shift; HOME=$ROOT/$who ARKIV_GIT_HOME=$ROOT/$who/.arkiv-git "$@"; }
field() { sed -n "s/^$1 *//p" | awk '{print $1}'; }
hex_of() { python3 -c 'import sys
a="123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
n=0
for c in sys.argv[1][1:]: n=n*58+a.index(c)
print(n.to_bytes(32,"big").hex())' "$1"; }
post() { # who type action object payload -> op id
  local obj=() ; [ -n "$4" ] && obj=(--object "$4" --parent "$4")
  as "$1" arkiv-git op post --repo "$URL" --type "$2" --action "$3" ${obj[@]+"${obj[@]}"} --payload-json "$5"
}
balance() { curl -s -X POST -H 'content-type: application/json' "$RPC" \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getBalance\",\"params\":[\"$1\",\"$2\"]}" |
  python3 -c 'import json,sys; print(int(json.load(sys.stdin)["result"],16))'; }

setup() {
  say "four fresh identities: alice (delegate), escrow, agent, bank"
  for who in alice agent bank; do mkdir -p "$ROOT/$who"; as $who arkiv-git keys init; done
  mkdir -p "$ROOT/escrow"
  local out; out=$(as escrow arkiv-git escrow init); echo "$out"
  keep ESC_ID "$(echo "$out" | field escrow:)"
  keep ESC_ADDR "$(echo "$out" | field address:)"
  keep ESC_WALLET "$(echo "$out" | field wallet:)"
  keep AGENT_NID "$(as agent arkiv-git keys show | field nid:)"
  if [ "$MODE" = mem ]; then
    for who in alice agent; do as $who arkiv-git fund --dev 1 >/dev/null; done
    HOME=$ROOT/escrow ARKIV_GIT_HOME=$ROOT/escrow/.arkiv-git/escrow arkiv-git fund --dev 1 >/dev/null
  fi
}

alice() {
  say "alice creates a public repo with a bug in add()"
  keep URL "$(as alice arkiv-git init --name "agent-bounty-$(date +%s)" | field url:)"
  local work=$ROOT/alice/work
  mkdir -p "$work/src" && cd "$work" && git init -q -b main
  printf 'pub fn add(a: i32, b: i32) -> i32 {\n    a - b\n}\n' >src/lib.rs
  git add -A && git commit -qm "feat: add" && as alice git push -q "$URL" main
  echo "url: $URL"
  say "alice appoints the escrow and pins agent-bounty.rhai with proofs required"
  as alice arkiv-git escrow appoint --repo "$URL" --escrow "$ESC_ID" --address "$ESC_ADDR"
  sed 's/^    "optional"$/    "required"/' "$REPO_DIR/programs/agent-bounty.rhai" >"$ROOT/agent-bounty.rhai"
  as alice arkiv-git program pin --repo "$URL" --file "$ROOT/agent-bounty.rhai" | grep program:
  say "alice grants the agent scope patch with a cap of $CAP wei"
  local grants; grants=$(post alice xyz.arkiv.agent create "" '{}')
  post alice xyz.arkiv.agent grant "$grants" \
    "{\"agent\":\"$(hex_of "$AGENT_NID")\",\"scopes\":[\"patch\"],\"max_bounty\":$CAP,\"until\":1000000}" >/dev/null
  say "alice opens an issue with a $BOUNTY wei bounty and deposits it"
  local spec; spec=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["spec_hash"])' "$FIX/demo.public.json")
  keep ISSUE "$(post alice xyz.arkiv.issue create "" \
    "{\"title\":\"add() subtracts; fix src/lib.rs\",\"bounty\":$BOUNTY,\"spec_hash\":\"$spec\"}")"
  echo "issue: $ISSUE"
  as alice arkiv-git escrow deposit --repo "$URL" --amount "$BOUNTY" --purpose "bounty:$ISSUE"
}

agent() {
  say "the agent (headless Codex CLI) works the bounty through arkiv-git-mcp"
  local dir=$ROOT/agent/session
  mkdir -p "$dir"
  # Codex's sandbox refuses a writable root reached through a symlink (macOS /tmp).
  dir=$(cd "$dir" && pwd -P)
  cp "$FIX/demo.receipt" "$FIX/demo.public.json" "$dir/"
  local env="{ARKIV_GIT_HOME=\"$ROOT/agent/.arkiv-git\", GIT_CONFIG_GLOBAL=\"/dev/null\""
  env+=", GIT_AUTHOR_NAME=\"agent\", GIT_AUTHOR_EMAIL=\"agent@example.com\""
  env+=", GIT_COMMITTER_NAME=\"agent\", GIT_COMMITTER_EMAIL=\"agent@example.com\"}"
  local prompt="You are an AI agent with a scoped grant on the arkiv-git repo $URL.
Work only through the arkiv-git MCP tools and file edits in ./work.
1) op_list the repo and pick the xyz.arkiv.issue create op with a bounty that no
xyz.arkiv.patch names yet. 2) repo_clone it into work. 3) Fix what the issue asks by editing
the file in ./work. 4) git_push dir work, branch agent/<first 8 chars of the issue id>, with a
commit message. 5) attach_proof with payload {\"title\": <short>, \"head\": <the pushed head>,
\"issue\": <issue id>}, receipt demo.receipt, public demo.public.json. Reply with the patch op id."
  # The MCP server runs outside Codex's sandbox and gets the access key from this
  # environment by name; the model's own shell never sees it and has no network.
  echo "codex model: $CODEX_MODEL"
  codex exec --json --ephemeral -m "$CODEX_MODEL" --ignore-user-config --ignore-rules --skip-git-repo-check \
    -s workspace-write -C "$dir" \
    -c "mcp_servers.arkiv-git.command=\"$BIN/arkiv-git-mcp\"" \
    -c "mcp_servers.arkiv-git.cwd=\"$dir\"" \
    -c "mcp_servers.arkiv-git.env=$env" \
    -c 'mcp_servers.arkiv-git.env_vars=["ARKIV_ACCESS_KEY","ARKIV_GIT_BACKEND","ARKIV_GIT_DATA_LIFETIME","ARKIV_GIT_CONTROL_LIFETIME"]' \
    -c 'mcp_servers.arkiv-git.tool_timeout_sec=900' \
    -c 'mcp_servers.arkiv-git.default_tools_approval_mode="approve"' \
    "$prompt" </dev/null 2>"$ROOT/agent/codex.stderr" | tee "$ROOT/agent/session.jsonl" | python3 "$REPO_DIR/demo/show-agent.py"
}

recap() {
  say "the live repo so far: alice's ops, read back from Rabdi"
  echo "url: $URL"
  as bank arkiv-git op list --repo "$URL" --json | python3 -c 'import json,sys
for o in json.load(sys.stdin):
    p = json.dumps(o["payload"], separators=(",", ":"))
    print(o["block"], o["type"], o["action"], o["author"][:9], p[:90])'
  as bank arkiv-git program status --repo "$URL" | grep -E "^program:" || true
}

merge() {
  local patch; patch=$(as alice arkiv-git op list --repo "$URL" --type xyz.arkiv.patch --json |
    python3 -c 'import json,sys; print(json.load(sys.stdin)[-1]["id"])')
  local work=$ROOT/alice/work branch="agent/${ISSUE:0:8}"
  say "alice fetches the agent's branch; main refuses it before a human approval"
  cd "$work" && as alice git fetch -q "$URL/$AGENT_NID" "refs/heads/$branch"
  git merge -q --ff-only FETCH_HEAD && git log --oneline -1
  as alice git push -q "$URL" main 2>&1 | grep -o 'E_PROGRAM_REJECTED.*' || true
  say "alice approves, records the merge and pushes main"
  local review; review=$(post alice xyz.arkiv.review create "" "{\"patch\":\"$patch\"}")
  post alice xyz.arkiv.review approve "$review" "{\"patch\":\"$patch\"}" >/dev/null
  post alice xyz.arkiv.patch merged "$patch" "{\"patch\":\"$patch\",\"closes\":\"$ISSUE\"}" >/dev/null
  as alice git push -q "$URL" main
  as alice arkiv-git program status --repo "$URL" | grep -E '^obligation' || true
  say "the escrow pays the agent"
  as escrow arkiv-git escrow run --repo "$URL" --once
}

verify() {
  say "keyless audit and the payout on chain"
  as bank arkiv-git escrow audit --repo "$URL"
  local paid; paid=$(as bank arkiv-git op list --repo "$URL" --type xyz.arkiv.escrow --json |
    python3 -c 'import json,sys; p=[o["payload"] for o in json.load(sys.stdin) if o["action"]=="paid"][-1]; print(p["tx"], p["to"], p["amount"])')
  read -r tx to amount <<<"$paid"
  echo "payout tx: $tx to $to amount $amount wei"
  [ "$MODE" = mem ] && return
  # Rabdi serves state at the chain head only, so the delta comes from the mined receipt.
  curl -s -X POST -H 'content-type: application/json' "$RPC" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getTransactionByHash\",\"params\":[\"$tx\"]}" |
    python3 -c 'import json,sys; t=json.load(sys.stdin)["result"]
print("mined in block %d: %s -> %s, value %d wei" % (int(t["blockNumber"],16), t["from"], t["to"], int(t["value"],16)))'
  echo "agent $to balance now: $(balance "$to" latest) wei"
  echo "$EXPLORER/tx/$tx"
}

case "${1:-all}" in
  setup | alice | recap | agent | merge | verify) "$1" ;;
  all) setup; alice; agent; merge; verify ;;
  *) echo "usage: $0 setup|alice|recap|agent|merge|verify|all" >&2; exit 2 ;;
esac

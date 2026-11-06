#!/usr/bin/env bash
# Branch protection with a pinned repo program, end to end on a local MemArkiv chain.
# Build first: `nix develop -c cargo build -p arkiv-git-cli`.
# Usage: demo/programs.sh [workdir]   (default /tmp/arkiv-programs-demo, wiped first)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEMO="${1:-/tmp/arkiv-programs-demo}"
BIN="${BIN:-$ROOT/target/debug}"
rm -rf "$DEMO" && mkdir -p "$DEMO"
export PATH="$BIN:$PATH"
export ARKIV_GIT_BACKEND="mem:$DEMO/chain.log"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1

say() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }

# Runs a command as user $1 (own HOME and arkiv-git home).
as() {
  local who="$1"; shift
  mkdir -p "$DEMO/$who"
  HOME="$DEMO/$who" ARKIV_GIT_HOME="$DEMO/$who/.arkiv-git" "$@"
}
g() { local who="$1"; shift; as "$who" git -c user.name="$who" -c user.email="$who@example.com" \
  -c init.defaultBranch=main -c commit.gpgsign=false "$@"; }

# MemArkiv faucet: append a funding record to the fake chain log (fake chain only).
faucet() {
  python3 - "$DEMO/chain.log" "$1" <<'PY'
import os, struct, sys
log, addr = sys.argv[1], bytes.fromhex(sys.argv[2][2:])
rec = lambda tag, p: tag + struct.pack('<I', len(p)) + p
with open(log, 'ab') as f:
    if os.path.getsize(log) == 0:
        f.write(rec(b'C', struct.pack('>Q', 7)))
    f.write(rec(b'F', addr + (10**24).to_bytes(32, 'big')))
PY
}

new_user() {
  local wallet
  wallet="$(as "$1" arkiv-git keys init | awk '/wallet:/ {print $2}')"
  faucet "$wallet"
  echo "$1: funded wallet $wallet"
}

say "Alice (delegate) and Bob (contributor) get keys and test funds"
new_user alice
new_user bob

say "Alice creates a repo and pushes main before any program is pinned"
URL="$(as alice arkiv-git init --name guarded | awk '/url:/ {print $2}')"
echo "repo: $URL"
WORK="$DEMO/work"
g alice init -q "$WORK"
echo "v0" > "$WORK/README"
g alice -C "$WORK" add -A
g alice -C "$WORK" commit -q -m "feat: v0"
g alice -C "$WORK" push -q "$URL" main
echo "main is $(g alice -C "$WORK" rev-parse --short HEAD)"

say "Alice pins programs/branch-protection.rhai (source published as an op, hash pinned in the identity)"
as alice arkiv-git program pin --repo "$URL" --file "$ROOT/programs/branch-protection.rhai" | grep -E 'published|program:'

say "Alice commits v1 and pushes straight to main: the helper refuses it locally, no gas spent"
echo "v1" > "$WORK/README"
g alice -C "$WORK" commit -q -am "feat: v1"
HEAD1="$(g alice -C "$WORK" rev-parse HEAD)"
g alice -C "$WORK" push "$URL" main 2>&1 | grep -E 'E_PROGRAM_REJECTED' || true

say "Bob opens a patch at v1; Alice reviews and approves that exact head"
PATCH="$(as bob arkiv-git op post --repo "$URL" --type xyz.arkiv.patch --action create \
  --payload-json "{\"title\":\"v1\",\"head\":\"$HEAD1\"}")"
echo "patch: $PATCH"
REVIEW="$(as alice arkiv-git op post --repo "$URL" --type xyz.arkiv.review --action create \
  --payload-json "{\"patch\":\"$PATCH\"}")"
as alice arkiv-git op post --repo "$URL" --type xyz.arkiv.review --action approve \
  --object "$REVIEW" --parent "$REVIEW" --payload-json "{\"patch\":\"$PATCH\",\"head\":\"$HEAD1\"}" >/dev/null
echo "approved by alice at ${HEAD1:0:7}"

say "The same push now passes the program"
g alice -C "$WORK" push -q "$URL" main
echo "pushed main ${HEAD1:0:7}"

say "Carol clones with no keys: her client folds the program and gets the accepted head"
g carol clone -q "$URL" "$DEMO/carol-clone"
g carol -C "$DEMO/carol-clone" log --oneline -1

say "Program status: state, canonical heads, rejected pushes, obligations"
as carol arkiv-git program status --repo "$URL"

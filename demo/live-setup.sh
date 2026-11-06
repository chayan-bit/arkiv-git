#!/usr/bin/env bash
# Off-camera setup and teardown for demo/arkiv-git.tape, which runs every scene on the live Rabdi testnet.
#
#   ARKIV_GIT_LIVE_VIDEO=<scratch dir> ARKIV_GIT_FUND_LOCK=<lock file> demo/live-setup.sh fund
#   nix-shell -p vhs --run 'vhs demo/arkiv-git.tape'
#   ARKIV_GIT_LIVE_VIDEO=<scratch dir> demo/live-setup.sh sweep
#
# `fund` creates the peer homes (alice, bob, dave with keys; carol keyless), links the stage to /tmp/agl,
# writes the access key to <dir>/access.sh (mode 600) for the tape's hidden step, moves one bank transfer
# from the throwaway key in spike/arkiv-measure/.env under the shared lock, and pays alice and bob from the bank.
# `sweep` returns every peer's wallet, control and bulk balance to the bank and the bank's rest to the throwaway.
# Nothing here prints a key: `keys init` prints only the nid and wallet, and funding goes through the live test helpers.
set -euo pipefail

cmd=${1:?usage: live-setup.sh fund|sweep}
REPO=$(cd "$(dirname "$0")/.." && pwd)
ROOT=${ARKIV_GIT_LIVE_VIDEO:?set ARKIV_GIT_LIVE_VIDEO to a scratch dir}
STAGE=$ROOT/stage
HOMES=$STAGE/.h
MILLI=000000000000000
BIN=${CARGO_TARGET_DIR:-$REPO/target}/release
export ARKIV_GIT_RABDI=1 ARKIV_GIT_LIVE_BANK=$ROOT/bank

live_test() { # <test file> <test name> [env...]
  local file=$1 name=$2
  shift 2
  (cd "$REPO" && env "$@" cargo test -q -p arkiv-git-cli --test "$file" "$name" -- --ignored --exact --nocapture)
}

keys() { # <home>
  local h=$1/.arkiv-git
  ARKIV_GIT_HOME=$h "$BIN/arkiv-git" keys show 2>/dev/null ||
    ARKIV_GIT_HOME=$h "$BIN/arkiv-git" keys init
}

case $cmd in
fund)
  LOCK=${ARKIV_GIT_FUND_LOCK:?set ARKIV_GIT_FUND_LOCK}
  mkdir -p "$ROOT/bank" "$HOMES"/{alice,bob,carol,dave}
  ln -sfn "$STAGE" /tmp/agl
  ln -sfn "$BIN" "$STAGE/.bin"
  cp "$REPO/programs/branch-protection.rhai" "$STAGE/"
  (umask 077 && grep '^ARKIV_ACCESS_KEY=' "$REPO/spike/arkiv-measure/.env" | sed 's/^/export /' >"$ROOT/access.sh")
  for u in bank .h/alice .h/bob .h/dave; do
    echo "$u:"
    keys "$( [ "$u" = bank ] && echo "$ROOT/bank" || echo "$STAGE/$u")"
  done
  lockf "$LOCK" bash -c "cd '$REPO' && ARKIV_GIT_RABDI=1 ARKIV_GIT_LIVE_BANK='$ROOT/bank' cargo test -q -p arkiv-git-cli --test rabdi_private bank_fund -- --ignored --exact --nocapture"
  live_test rabdi_private bank_pay ARKIV_GIT_LIVE_PAY_HOME="$HOMES/alice" ARKIV_GIT_LIVE_PAY_WEI="25$MILLI"
  live_test rabdi_private bank_pay ARKIV_GIT_LIVE_PAY_HOME="$HOMES/bob" ARKIV_GIT_LIVE_PAY_WEI="15$MILLI"
  ;;
sweep)
  live_test rabdi_private bank_collect ARKIV_GIT_LIVE_COLLECT="$HOMES"
  live_test rabdi_private bank_sweep
  ;;
*)
  echo "usage: live-setup.sh fund|sweep" >&2
  exit 2
  ;;
esac

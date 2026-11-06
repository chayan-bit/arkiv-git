#!/usr/bin/env bash
# Delegate merge on Arkiv (docs/p3/arkiv-ci.md): checks in `program status` that CI passed
# at the patch head, approves the patch at that head (unless quorum is already met), then
# pushes main to the head. The remote helper folds the pinned program again and refuses
# with E_PROGRAM_REJECTED before any transaction if main may not move there.
# Usage: scripts/arkiv-merge.sh <arkiv-url> <patch-id>   (run inside a clone)
set -euo pipefail
# shellcheck source=scripts/arkiv-lib.sh
. "$(dirname "$0")/arkiv-lib.sh"

url=${1:?usage: arkiv-merge.sh <arkiv-url> <patch-id>}
patch=${2:?usage: arkiv-merge.sh <arkiv-url> <patch-id>}
[[ $patch =~ ^[0-9a-f]{64}$ ]] || die "patch id must be 64 hex digits"

line=$(ops_json "$url" | patch_heads | grep "^$patch " || true)
[ -n "$line" ] || die "no patch $patch"
read -r _ author head <<<"$line"

state=$(arkiv-git program status --repo "$url" | sed -n 's/^state: //p')
[ -n "$state" ] || die "no program state; is a program pinned?"
passes=$(jq -r --arg h "$head" '[(.ci[$h] // {})[] | select(. == "pass")] | length' <<<"$state")
need=$(jq -r '.need // 0' <<<"$state")
[ "$passes" -ge "$need" ] || die "CI has $passes of $need passes at $head; not approving"

approvals=$(jq -r --arg p "$patch" '.patches[$p].approvals // {} | length' <<<"$state")
quorum=$(jq -r '.quorum // 1' <<<"$state")
if [ "$approvals" -lt "$quorum" ]; then
  review=$(arkiv-git op post --repo "$url" --type xyz.arkiv.review --action create \
    --payload-json "$(jq -cn --arg p "$patch" '{patch: $p}')")
  arkiv-git op post --repo "$url" --type xyz.arkiv.review --action approve --object "$review" \
    --parent "$review" --payload-json "$(jq -cn --arg p "$patch" --arg h "$head" \
    '{patch: $p, head: $h}')" >/dev/null
fi

fetch_head "$url" "$author" "$head" || die "cannot fetch $head from $author"
git update-ref "refs/arkiv/merge" "$head"
git push -q "$url" "refs/arkiv/merge:refs/heads/main"
echo "merged $patch: main -> $head"

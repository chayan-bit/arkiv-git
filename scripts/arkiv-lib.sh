# shellcheck shell=bash
# Shared helpers for the arkiv-*.sh scripts (docs/p3/arkiv-ci.md). Needs arkiv-git, git, jq.

die() {
  echo "$(basename "$0"): $*" >&2
  exit 1
}

for tool in arkiv-git git jq; do
  command -v "$tool" >/dev/null || die "$tool not found on PATH"
done

ops_json() { arkiv-git op list --repo "$1" --json; }

my_nid() { arkiv-git keys show | sed -n 's/^nid:[[:space:]]*//p'; }

# Reads `op list --json`; prints "patch author head" per patch with the author's latest
# create/revise head. Ids and oids are validated, since they reach git command lines.
patch_heads() {
  jq -r '
    reduce (.[] | select(.type == "xyz.arkiv.patch")) as $o ({};
      if $o.action == "create" then .[$o.object] = {author: $o.author, head: $o.payload.head}
      elif $o.action == "revise" and .[$o.object].author == $o.author
      then .[$o.object].head = $o.payload.head
      else . end)
    | to_entries[]
    | select((.value.head | type) == "string"
        and (.value.head | test("^([0-9a-f]{40}|[0-9a-f]{64})$"))
        and (.value.author | test("^z[1-9A-HJ-NP-Za-km-z]+$")))
    | "\(.key) \(.value.author) \(.value.head)"'
}

# fetch_head <url> <author nid> <head>: fetches the author's branches into the current
# repo and checks that the head commit arrived.
fetch_head() {
  git fetch -q "$1/$2" "+refs/heads/*:refs/arkiv/$2/*" && git cat-file -e "$3^{commit}"
}

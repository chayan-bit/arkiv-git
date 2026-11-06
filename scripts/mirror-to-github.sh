#!/usr/bin/env bash
# One-way mirror of the canonical main of the Arkiv repo to GitHub (docs/p3/arkiv-ci.md).
# Refuses unless the GitHub main is an ancestor of the Arkiv main; never force-pushes.
# Usage: scripts/mirror-to-github.sh [arkiv-url] [git-remote]   (run inside a clone)
set -euo pipefail

url=${1:-arkiv::zBZ3MRTENmgPBebpq97ZPTbxeFZNaS8VX6Es25hpdJsSJ}
remote=${2:-origin}

git fetch -q "$url" main
arkiv_main=$(git rev-parse FETCH_HEAD)
git fetch -q "$remote" main
github_main=$(git rev-parse FETCH_HEAD)
if ! git merge-base --is-ancestor "$github_main" "$arkiv_main"; then
  echo "mirror-to-github: $remote main $github_main is not an ancestor of Arkiv main" \
    "$arkiv_main; refusing" >&2
  exit 1
fi
git push -q "$remote" "$arkiv_main:refs/heads/main"
echo "mirrored $arkiv_main to $remote main"

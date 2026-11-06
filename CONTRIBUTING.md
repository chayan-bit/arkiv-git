# Contributing

## Develop arkiv-git on Arkiv

The canonical source of arkiv-git is the Arkiv repo `arkiv::zBZ3MRTENmgPBebpq97ZPTbxeFZNaS8VX6Es25hpdJsSJ`.
GitHub is a read-only mirror plus the host of release binaries, so changes land through Arkiv, not through GitHub pull requests.
The full workflow and trust model are in `docs/p3/arkiv-ci.md`.

1. Install arkiv-git (`docs/install.md`), create your key with `arkiv-git keys init`, send testnet funds to the wallet it prints (`arkiv-git fund` estimates how much), and clone with `git clone arkiv::zBZ3MRTENmgPBebpq97ZPTbxeFZNaS8VX6Es25hpdJsSJ arkiv-git`.
2. Commit on a branch based on the current `main`, then open or update a patch with `scripts/arkiv-patch.sh <url> <branch>`, which prints the patch id.
3. The CI runner (`scripts/arkiv-ci.sh`) tests the patch head and posts a signed attestation; check it with `arkiv-git op list --repo <url> --type xyz.arkiv.attest`.
4. A delegate merges with `scripts/arkiv-merge.sh <url> <patch-id>`, which works only once CI passed at the patch head.
5. A maintainer mirrors `main` to GitHub with `scripts/mirror-to-github.sh`.

Before posting a patch, run the same checks locally: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo test --workspace --all-features`.
Use conventional commits (`feat:`, `fix:`, `docs:` and so on), and keep Markdown at one sentence per line without em dashes.

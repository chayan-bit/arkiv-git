# Live demo video: every scene on Rabdi

Commands below were recorded before the rename to Charter; `arkiv-git` is now `chart`.

Date: 2026-11-07, recorded 04:53 to 05:03 IST.
Network: Rabdi testnet, chain id 7794095, RPC https://rpc.rabdi.db-chain.testnet.arkiv.network, explorer https://rabdi.explorer.arkiv.network.
Code: `main` at accd6ab plus `demo/arkiv-git.tape` and `demo/live-setup.sh` on `demo/live-video`.

## Video

- GIF: https://github.com/chayan-bit/arkiv-git/raw/demo-assets/arkiv-git-demo-live.gif (1.2 MB).
- MP4: https://github.com/chayan-bit/arkiv-git/raw/demo-assets/arkiv-git-demo-live.mp4 (1.8 MB, 2:30).
- Source: `demo/arkiv-git.tape`, rendered once with `nix-shell -p vhs --run 'vhs demo/arkiv-git.tape'`.
- The real session took about 7.5 minutes and plays at 3x; the third line on screen says so.
- No scene uses the in-memory `mem:` backend or `fund --dev`, and nothing was edited or cut after rendering.

## What runs on camera

| # | Scene | Live result |
|---|---|---|
| 1 | alice runs `init` and `git push`, bob does a fresh `git clone` | bob's clone shows `46b7aaf feat: v0`; a raw `arkiv_query` curl returns the identity entity and its creator, printed as an explorer link |
| 2 | alice pins `programs/branch-protection.rhai`, then pushes directly | `E_PROGRAM_REJECTED: main needs a patch at 339836d... with 1 delegate approvals`, refused before any tx |
| 3 | bob (not a delegate) pushes v1 to his namespace and posts a patch op, alice posts a review and an approve, fetches `arkiv::<rid>/<bob nid>` and pushes | push accepted `46b7aaf..7dab526`; carol (no keys) clones `7dab526 feat: v1 by bob`, and `program status` prints `head: refs/heads/main 7dab526...` |
| 4 | alice creates a private repo and pushes, bob publishes a key package, alice adds him as Read | bob's clone decrypts to `7dab526`; dave (keys, not a member) gets `E_NOT_MEMBER` |
| 5 | keyless clone of the long-lived README repo | `c9cf57a feat: v1`, and `status` shows its delegate with 13 days of minimum expiry |

## Ids and addresses

- Public repo (scenes 1 to 3): rid `zHjgiuZ6CyjYe7kc1zwumXkxiFV7QQGmcpQxtcMZNKLTJ`, name `arkiv-git-live-video`.
- Its identity entities: `0x29d2977dafe1c1bf37ae2abd6a8bf160b881236c2c3f9670a99526b6eb973928` (created at block 310,825) and `0x5cc432404b40868915f42227a3f6074a4232109273cfd88ebc5da586c1e8d810` (the revision that pins the program).
- Both were created by alice's control address [0xef209765297c9c5cf286f24c6c4c504ae3219c32](https://rabdi.explorer.arkiv.network/address/0xef209765297c9c5cf286f24c6c4c504ae3219c32).
- Private repo (scene 4): rid `z45jKSpZY9ootV3BRipRN5WJLrGJK3iU553bVyVGiRuev`, name `vault` (sealed, so the name does not appear on chain).
- Long-lived repo (scene 5): rid `z5b15ZhFLNackCqiyAL5y4dMj4MFGfVrtgLmZ9PPDU1AH`, created earlier and kept alive with `arkiv-git keepalive`.
- alice wallet: [0x7ee3dE4e69b7BF4C96A134BCd049024877Fb8c33](https://rabdi.explorer.arkiv.network/address/0x7ee3dE4e69b7BF4C96A134BCd049024877Fb8c33).
- bob wallet: [0xEb71B5542f606e44156e4aAf7529B09D127a2E94](https://rabdi.explorer.arkiv.network/address/0xEb71B5542f606e44156e4aAf7529B09D127a2E94), nid `zAvJiNA9DDLUn45BHHMmJMsFR4Yg82RMkPxPDgDNj8Sgo`.
- dave wallet (never funded): 0x2588E24654bAf250C7C4b0A8Fd492D398031Ed58; carol has no keys at all.
- Bank wallet used to fund the peers: [0x35C4A3B5611F8aF4DEDE0a9b1fC1Fc8EC49e0Fa4](https://rabdi.explorer.arkiv.network/address/0x35C4A3B5611F8aF4DEDE0a9b1fC1Fc8EC49e0Fa4).
- Every entity of the two new repos was written with a 129,600 block lifetime (3 days), so they expire around block 440,400, which is why the clients print "expires in 2 days".

## How it was run

`demo/live-setup.sh fund` created the peer homes, moved 0.05 tGLM from the throwaway key to the bank in one transfer under the shared lock, and paid alice 0.025 and bob 0.015 from the bank.
The tape's hidden first step sources the access key from a mode 600 file, so the key is never typed or shown.
`demo/live-setup.sh sweep` then returned every peer's wallet, control and bulk balance to the bank, and the bank's balance to the throwaway key.

## Spend

- Left the throwaway key: 0.05 tGLM, the only send from it.
- Returned to the throwaway key: 0.03949 tGLM; 0.000039 tGLM stays on the bank as the sweep's own gas reserve.
- Net cost of the whole recording, including the funding and sweep transfers: about 0.0105 tGLM.

## Checks

- One frame per second of the MP4 (150 frames) was OCR'd with tesseract, and no frame shows a private key, seed, the access key, a `/Users` path or the scratch directory.
- The first frame starts at the title comment, so the hidden setup never reaches the video.
- The tape contains no key or `/Users` path; it reads the scratch location from `$ARKIV_GIT_LIVE_VIDEO` and works under the `/tmp/agl` symlink.
- Before spending, the same commands passed on the `mem:` backend in an unrecorded rehearsal.

## Not in this video

- The zero-knowledge receipt and the timelock are left out to keep it short and cheap; their live results are in `docs/live/private-proofs.md`.
- The proof-gated AI agent bounty has its own recording.

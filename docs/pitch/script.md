# Charter pitch video script

ETHGlobal Mumbai 2026.
Total narration: 414 words, about 166 s at 150 words per minute, in a 3:00 cut (the rest is pauses for terminal output).
Voice: first person, direct, technical, no filler.
Sentences are capitalised because this is read aloud; the raw lowercase register is used only for the tweet.
Every claim carries a status: LIVE (ran on the Rabdi testnet), FAKE (ran only on the in-memory Arkiv, the `mem:` backend) or NOT YET.
Scenes 2 to 5 use only the all-live clip; FAKE now applies only to parts of scene 6, and any clip of a FAKE run carries an "in-memory chain" caption.

## Clip to use

`https://github.com/chayan-bit/charter/raw/demo-assets/arkiv-git-demo-live.gif` (MP4 at the same path, `.mp4`, 2:30, played at 3x as its caption says).
Source tape: `demo/charter.tape`; ids, addresses and checks in `docs/live/demo-video.md`.
Every scene runs live on the Rabdi testnet: 1 init, push and a fresh clone by another user, 2 pinned program refuses a direct push, 3 bob's patch, alice's approval, accepted push and a keyless clone of the head, 4 private repo where the member clones and an outsider gets `E_NOT_MEMBER`, 5 keyless clone of the long-lived README repo.
The older clip `arkiv-git-demo.gif` (mostly in-memory) stays on the branch but is superseded; only its offline `zk verify` scene is still referenced below.

## Scene table

| # | Time | Scene | Source status | Words |
|---|---|---|---|---|
| 1 | 0:00-0:16 | problem | card | 38 |
| 2 | 0:16-0:33 | the answer | LIVE | 42 |
| 3 | 0:33-1:03 | live clone | LIVE | 67 |
| 4 | 1:03-1:30 | program rejects a push | LIVE | 61 |
| 5 | 1:30-1:55 | private repo | LIVE | 56 |
| 6 | 1:55-2:38 | proof-gated bounty to an AI agent | FAKE / NOT YET live | 101 |
| 7 | 2:38-2:54 | why Arkiv | card | 37 |
| 8 | 2:54-3:00 | close | card | 12 |
| | | Total | | 414 |

## Narration and on-screen detail

Each scene lists what is on screen first, then the narration, one sentence per bullet.

### Scene 1 - problem (0:00-0:16, 38 words)

On screen: black card, large text, then the Charter name.

- GitHub is a single point of censorship and trust, because one company decides who hosts, who reads and what stays up.
- Radicle fixes the trust part with signed refs, but it needs always-on peers to carry your code.

### Scene 2 - the answer (0:16-0:33, 42 words)

On screen: terminal, live clip scene 1.
Commands shown: `as alice chart init --name charter-live-video`, `as alice git -C work push $URL main`, `as bob git clone -q $URL bob-clone && git -C bob-clone log --oneline`, then the raw `arkiv_query` curl and the explorer link for the creator address.
Caption: "Rabdi testnet, 3x".

- Charter ports Radicle to Arkiv, a data chain with no smart contracts, so the host is a public data layer.
- It is stock git, `git push arkiv::` and `git clone arkiv::`, signed with post-quantum ML-DSA-65 plus Ed25519, and both halves must verify.

### Scene 3 - live clone (0:33-1:03, 67 words)

On screen: live clip scene 5, then a browser at `https://rabdi.explorer.arkiv.network/address/0xc051EAe280800fdF4bAd0A2665aCdB1B4963688F` (the public control address, safe to show).
Command shown: `git clone arkiv::z5b15ZhFLNackCqiyAL5y4dMj4MFGfVrtgLmZ9PPDU1AH live && git -C live log --oneline`.
Caption: "Rabdi testnet, no keys".
Expected output: `c9cf57a feat: v1` at the top.
The live clone needs an access key in the environment; export it off screen before recording and never show it.

- This is live on the Rabdi testnet, and I am cloning with no keys.
- The client checks every signature and hash and follows the signed anchor chain before git sees a commit.
- A replayed old head is ignored and a hidden newest head fails closed with `E_STALE_HEAD`, both measured on Rabdi.
- A 10 MiB push took about 520 seconds, the fresh clone 13, and fsck was clean.

### Scene 4 - program rejects a push (1:03-1:30, 61 words)

On screen: live clip scenes 2 and 3.
Commands shown: `chart program pin --repo $URL --file branch-protection.rhai`, a direct `git push` that prints `E_PROGRAM_REJECTED`, bob's push to his namespace and his patch op, alice's review and approve, the same v1 pushed and accepted, carol's keyless clone and `chart program status`.
Caption: "Rabdi testnet, 3x".

- A repo can pin a Rhai program by hash, so every client runs the same rules.
- Here main only moves to a patch a delegate approved, so my direct push is refused before it spends any gas.
- After review and approval the same push goes through, and a stranger clones the result.
- Programs can also emit obligations, like pay this key.

### Scene 5 - private repo (1:30-1:55, 56 words)

On screen: live clip scene 4 (alice adds bob, bob's clone decrypts, `as dave git clone $PRIV dave-vault` fails with `E_NOT_MEMBER`), then a cut to `docs/live/private-proofs.md` scrolled to the rows "carol's push refused" and "bob cannot read content pushed after removal".
Captions: clip "Rabdi testnet, 3x", table "Rabdi testnet, 2026-11-07".

- Private repos are MLS groups with post-quantum X-Wing key exchange, and Arkiv only stores ciphertext.
- An outsider gets `E_NOT_MEMBER`, a read-only member cannot push, and the raw chain view does not show the repo name.
- On Rabdi, after I removed a member, he could not read anything pushed afterwards.
- Roles are enforced by the client today.

### Scene 6 - proof-gated bounty to an AI agent [if the agent demo lands] (1:55-2:38, 101 words)

On screen, if the demo lands:

1. A delegate grants the agent key a scope, a bounty cap and an expiry: `chart op post --repo $URL --type xyz.arkiv.agent --action grant --payload-json '{"agent":"<nid hex>","scopes":["patch"],"max_bounty":<wei>,"until":<order>}'`.
2. The AI agent drives the MCP server: lists issues, claims the bounty issue, posts a patch with a RISC Zero receipt attached. Fill in the exact MCP tool names from the `feat/mcp` branch when it merges; none exist on `main` yet.
3. `chart program status` shows the patch with `proven: true`, a human delegate approves, main moves.
4. `chart escrow run` pays the bounty, then `chart escrow audit --repo $URL` exits 0 with no FLAG lines.

Captions: "in-memory chain" unless the agent demo was recorded on Rabdi, and say "live" only if it was.
Status: the agent grant, proof policy and bounty obligation pass end to end on MemArkiv (FAKE); a real proof was verified on a live Rabdi public repo; the escrow payout and the MCP agent have no live Rabdi run recorded.

- Now the part I care about: an AI agent that earns money under rules anyone can check.
- A delegate signs a grant with scopes, a bounty cap and an expiry, and revoking it is another op.
- The agent claims a bounty through the MCP server and attaches a RISC Zero proof that its sealed patch meets the spec, and the repo program decides whether a proof is required.
- The program computes what is owed, and an escrow agent pays it on-chain.
- That agent can steal or stall, but not undetectably, because the audit needs no keys and fails on any violation.

Fallback if the agent demo does not land (same slot, 72 words, replaces the narration above; on screen the older clip's scene 3 where `chart zk verify` passes offline (no chain involved) and then fails after one public input is changed, with the op table from `docs/p3/agents.md` and the audit command from `docs/p3/escrow.md` shown over it):

- Now the part I care about: AI agents that earn money under rules anyone can check.
- A delegate signs a scoped, capped, revocable grant for the agent key, and the pinned program enforces it.
- Here a RISC Zero receipt shows a sealed patch meets its spec, verifying offline and failing when one input changes.
- This runs on the in-memory chain today, and the live agent claim is the next thing I record.

### Scene 7 - why Arkiv (2:38-2:54, 37 words)

On screen: three-line card: "Global order = rollback protection", "Anyone can extend a repo's lifetime", "Wallet-owned, no account to ban".

- Why Arkiv and not Postgres: one global order that every client reads the same way gives rollback protection.
- Anyone with a funded key can extend a repo's lifetime, and ownership is a wallet, not an account row.

### Scene 8 - close (2:54-3:00, 12 words)

On screen: repo URL large, the rid below it in small text.

- MIT or Apache, testnet only, builds with `nix develop`.
- Charter, at github.com/chayan-bit/charter.

## Recording checklist

- Resolution 1920x1080, terminal font size 22 or larger (the VHS tape renders at its own size; for a fresh screen capture use a 100x28 terminal), dark theme, no notifications, clock hidden.
- The live clip is already rendered; to re-record it, build with `cargo build --release`, run `demo/live-setup.sh fund`, render with `nix-shell -p vhs --run 'vhs demo/charter.tape'`, then run `demo/live-setup.sh sweep` (about 0.01 tGLM per take, see `docs/live/demo-video.md`).
- Scene 3 needs the live repo to still be alive: it was created 2026-11-07 with 1 day lifetimes and then extended by `chart keepalive` to at least 14 days, so it should be alive, but run `chart status arkiv::z5b15ZhFLNackCqiyAL5y4dMj4MFGfVrtgLmZ9PPDU1AH` first to confirm.
- Do one dry clone of the live rid in a scratch `$CHARTER_HOME` right before recording to confirm it still resolves and prints `c9cf57a feat: v1`.
- Scene 6: pre-post the grant, the issue and the receipt blobs (each 30,000-byte blob op takes about 4 to 6 s and costs about 0.0027 tGLM), and pre-fund the agent, escrow and bank wallets, so the take only shows the claim, the fold and the payout.
- Scene 6 on Rabdi: only claim it is live if that run completed uninterrupted; otherwise keep the "in-memory chain" caption.
- Never type or echo `ARKIV_ACCESS_KEY`; the tape sources it from a mode 600 file in a hidden step.
- Never show: seed files, `$CHARTER_HOME/keys`, `$CHARTER_HOME/escrow/keys`, the access key, any `/Users/...` path.
  Use `export PS1='$ '` and run from `/tmp/agl` as the tape does; check the shell history, window title and tab bar for the home directory.
- Wallet addresses and the rid are public and fine to show; testnet only, say so in the close.
- Keep the final cut at or under 3:00 and under 50 MB for the submission form; leave 1 s of silence at the end.

## Tweet (280 characters or fewer)

Charter: git hosting with no host. Stock `git push arkiv::` and `git clone arkiv::` on Arkiv's data chain, post-quantum signed (ML-DSA-65 + Ed25519), rules in pinned Rhai programs, MLS private repos. Keyless clone live on Rabdi: github.com/chayan-bit/charter

## Tagline for the ETHGlobal form

Charter is decentralized git hosting on Arkiv: stock `git push` and `git clone` over `arkiv::` URLs, post-quantum signed refs, pinned Rhai repo programs and MLS private repos, with push, clone, branch protection and private repos shown live on the Rabdi testnet. It also lets scoped AI agents claim bounties paid by an auditable escrow and gated by RISC Zero proofs, implemented and tested on an in-memory Arkiv.

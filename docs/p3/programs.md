# Repo programs

A repo program is a small Rhai script that decides which ref updates a repo accepts and what the repo owes.
The identity doc pins the script by hash, so every honest client runs the same program.
Each client folds the program over the same Arkiv-ordered ops and ref updates, so all of them compute the same state and accept the same branch heads.
The engine lives in `crates/arkiv-git-program`; it is pure and does no I/O.

## Pinning

The identity body has an optional field `13: program`, a map `{ 1: engine, 2: hash }`.
`engine` is the engine tag; the only tag is `rhai/1`, and any other tag fails to load.
`hash` is `H("program", source)`, the tagged SHA-256 of the exact UTF-8 source bytes (design section 2.1).
A body without a pin omits key 13, so its canonical CBOR, its `h` and its signatures are unchanged.
Changing the pin is an ordinary identity revision and needs the usual delegate quorum.
`arkiv-git program pin --repo <rid> --file <path.rhai>` compiles the script, posts its source as an op, then proposes a revision with the new pin.

## Source distribution

The source travels as a collaborative-object op (docs/p3/cob-ops.md) of type `xyz.arkiv.program` and action `publish`, with payload `{ engine, source }`.
Publish ops hang off one program object per repo; `program pin` creates that object with a `create` op the first time.
A client uses the source of the first verified publish op whose `engine` equals the pinned engine and whose `Program::hash(source)` equals the pinned hash.
Anyone may post a publish op, because the hash, not the author, decides which source counts.
A pin with no matching source, a source that does not compile, or a fold that fails (for example in `init`) is `E_PROGRAM_MISSING`.
The client then refuses to compute heads at all, so fetch and clone fail instead of falling back to the plain delegate rule.

## Determinism

The engine is built without floats (`no_float`, `only_i64`), without time (`no_time`), without custom syntax and without modules (`no_module`), so there is no `import`, no module resolver and no file access.
`eval` is a disabled symbol, so a script that uses it fails to compile.
`print` and `debug` write to the fold's log, never to stdout.
Rhai maps are `BTreeMap`s, so map iteration and `keys()` are in sorted key order, and the JSON state has sorted keys.
The only host function is `sha256_hex(string or blob)`, which returns lowercase hex and is pure.
Each fold uses a fresh engine.

## Limits

Each ABI call may run 200,000 operations, and a whole fold may run 5,000,000.
Strings are capped at 64 KiB, arrays and maps at 10,000 entries, call depth at 32 and expression depth at 64 (32 inside functions).
A call that hits a limit fails like any other runtime error.

## ABI

All ids and oids reach the script as lowercase hex strings.
Integers are `i64`; a value that does not fit makes the event fail.

### `fn init(repo)` (required)

`repo` is `#{ rid, delegates: [hex ids], quorum, name }`.
It returns the initial state, any value, usually a map.
A program without `init` fails to load, and an error in `init` fails the fold.

### `fn on_op(state, op)` (optional)

`op` is `#{ id, author, type, action, object, parents: [..], payload, order, ts }`.
`payload` is the op's canonical CBOR converted to Rhai values: maps with string keys, integers, strings, booleans, arrays, byte strings as blobs, and null as `()`.
It returns the new state.
An error, including running out of operations or a payload that does not convert, skips the op, logs it and leaves the state unchanged.

### `fn check_push(state, push)` (optional)

`push` is `#{ author, ref, old, head, order, ff }`.
`old` is the hex oid of the last accepted head of that ref, or `""` if the program has accepted none.
`head` is the new oid; Rhai reserves the word `new`, so the key is `head` even though the Rust and JSON field is `new`.
`ff` is true when the ref has no accepted head, when `head` is empty (a delete) or equals the accepted head, or when the accepted head is a git ancestor of `head`.
Ancestry comes from a host callback (`Program::fold_with`); when the host cannot tell (objects missing, or `fold` and `fold_from`, which pass no callback), `ff` falls back to whether the pusher's claimed `old` equals the accepted head.
It returns `()` to allow a fast-forward, `true` to allow the push even if it is not a fast-forward, `false` or a string to reject, where the string is the reason.
An error, including running out of operations, rejects the push (fail closed).
Without `check_push` every fast-forward is allowed.

### `fn on_push(state, push)` (optional)

It runs after a push is accepted and returns the new state.
An error logs and leaves the state unchanged.

### `fn obligations(state)` (optional)

It runs once after the last event and returns an array of maps, for example `#{ kind: "pay", to, amount, reason }` or `#{ kind: "release_key", object, by }`.
An error logs and yields no obligations.
Obligations are claims the program computes; whether they were met on chain is checked outside the program.

## Fold semantics

Events are ops and ref updates interleaved in Arkiv order; an event with a smaller order than the one before it fails the fold.
The engine keeps the last accepted head per ref.
A rejected push does not move that head, so later pushes on the ref are checked against the last accepted one.
A push that is not a fast-forward (see `ff`) is rejected with `not a fast-forward of the accepted head`, unless `check_push` returns `true`.
Accepted and rejected pushes in the outcome carry the accepted head as `old`, not the pusher's claim.
A push with an empty `new` deletes the accepted head of that ref.
The outcome holds `state_json`, the accepted and rejected pushes (with reasons), the obligations and the log lines.

## Enforcement

Every client enforces the pinned program; there is no server to trust.
The sync crate (`crates/arkiv-git-sync/src/program.rs`) builds the fold input from what P1 already verifies.

### Events

Ops are the verified COB ops of the repo, converted to `ProgramOp`.
Ref updates come from the verified anchor chains of the delegates of the current identity head, the same peers whose refs vote under P1.
Each anchor after the first contributes one `RefUpdate` per ref whose oid differs from that delegate's previous signed refs: `old` is the previous oid (empty for a new ref) and `new` the signed one (empty for a deleted ref); the fold then replaces `old` with its accepted head.
Pushes by non-delegates are not events, as under P1 they never become canonical.
Ops, anchors and identity entities are read in one creation-ordered query, so their ranks in that result give one Arkiv order for all events.
The ranks differ between clients only by entities that expired or were created later, never in relative order, so every client folds the same sequence.

### Seeding

The program applies from the identity revision that set the current pin, at the position where that revision took effect.
Pushes before that position are accepted as they were: the fold starts with the canonical heads (design 3.8, under the pinning revision's delegates and threshold) computed from each delegate's last signed refs before the pin.
Ops before the pin are folded normally, so a patch or a bounty opened before the pin still counts.
Only the refs entities of the last pre-pin push and later ones are read, so older refs entities may expire without breaking the fold.

### Fetch and clone

`fetch::sync` builds the verified view, then calls `program::enforce`, which folds the program and stores the result in `View::program`.
`fetch::listing` then lists the program's last accepted head of every ref instead of the delegate quorum result.
A rejected update never moves a canonical head, so every client ignores it, including pushes forced through a modified client.
The peer namespaces `refs/namespaces/<nid>/` still mirror each delegate's signed refs, so the rejected commits stay inspectable.
`arkiv-git program status --repo <rid>` prints the state JSON, the canonical heads, every rejected push with its reason, and the obligations.

### Push

`git-remote-arkiv` prepares the push locally, then calls `program::check_push`, which folds again with the would-be ref updates appended after everything on chain.
If the program rejects one of them, the helper discards the prepared push and fails with `E_PROGRAM_REJECTED: <reason>` before any transaction is sent, so no gas is spent on a push the program would ignore.
The check applies to delegates only, because only their pushes are events.
The check is advisory for the pusher; the binding rule is the fetch-side fold that every other client runs.

### Escrow hook

`arkiv_git_sync::program::obligations(arkiv, rid)` returns the obligations of the pinned program (empty without a pin), for the escrow crate to act on.

### Replay protection

The program changes only which head is canonical; it does not touch the P1 checks.
Anchor chains, high-water marks and `E_STALE_HEAD` still apply per peer before the fold sees any update, so a withheld or replayed anchor fails the fetch exactly as without a program.
A rejected push still advances its author's anchor chain and mark; the next push of that delegate builds on it as usual.

### Known limits

`ff` uses git ancestry in the local cache: fetch decides it after applying every delegate's packs, `program status` and `obligations` fetch first, and the helper checks after staging the push, so all of them have the objects.
A delegate whose push was rejected is therefore not stuck: its next push of a descendant of the accepted head is a fast-forward, whatever its own refs held.
A client that lacks an object falls back to comparing the pusher's previous oid with the accepted head, which can disagree with clients that have it; P1's connectivity check makes this a corner case.
Every client re-folds the whole history on each fetch; there is no cached state.

## Rust API

`Program::load(engine_tag, source)` compiles a program and checks for `init`.
`Program::hash(source)` gives the pin hash.
`Program::fold(repo: RepoInfo, events)` takes `Event::Op(ProgramOp)` and `Event::Push(RefUpdate)` and returns an `Outcome`.
`Program::fold_from(repo, seed, events)` starts the accepted heads at `seed` instead of empty; `Outcome::heads` holds the last accepted head per ref.
`ProgramOp` mirrors the verified COB op: `id`, `author`, `rid`, `object`, `type_name`, `action`, `parents`, `payload` (canonical CBOR), `ts_ms`, `arkiv_order`.

## Dry run

`arkiv-git program check --file <path.rhai> --events <events.json>` folds a local file and prints the outcome as JSON.
The events file is `{ "repo": { rid, delegates, quorum, name }, "events": [...] }`.
Each event has `"event": "op"` or `"event": "push"`.
An op has `id`, `author`, `object`, `type`, `action`, `payload` and `arkiv_order`, plus optional `rid`, `parents` and `ts_ms`.
Its `payload` is plain JSON, converted to canonical CBOR (floats are rejected).
A push has `author`, `ref`, `old` (optional, empty for a new ref), `new` and `arkiv_order`.
All ids and oids are hex.

## Templates

The `programs/` directory holds four demo programs, each tested from disk in the program crate.

- `branch-protection.rhai`: `refs/heads/main` moves only to a patch head that at least `repo.quorum` delegates approved at that exact head; a revise clears the approvals.
- `ci-attested.rhai`: `main` moves only to a commit that k of the n attesters listed at the top of the script reported as passing CI.
- `bounty.rhai`: an issue bounty plus pledges becomes a `pay` obligation to the patch author once the merged patch head is accepted on `main`, and a payment receipt op clears it; the receipt's chain check happens outside the program.
- `sealed-patch.rhai`: a commit op pins `sha256(key)`, a later reveal counts only if the key hashes to it, and unrevealed commitments are `release_key` obligations.

Rhai functions cannot read top-level constants without modules, so configuration such as the attester list is written as small functions at the top of the script.

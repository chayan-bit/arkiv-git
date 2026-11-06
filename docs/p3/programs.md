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
`arkiv-git program pin --repo <rid> --file <path.rhai>` compiles the script, then proposes a revision with the new pin.

TODO (after the COB merge): distribute the program source as a COB op of type `xyz.arkiv.program`, and have clients fetch the source whose hash matches the pin.

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
`old` is the hex oid the pusher says it replaces, or `""` for a new ref.
`head` is the new oid; Rhai reserves the word `new`, so the key is `head` even though the Rust and JSON field is `new`.
`ff` is true when `old` equals the last accepted head of that ref, or when both are absent.
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
A push whose `old` is not the last accepted head is rejected with `not a fast-forward of the accepted head`, unless `check_push` returns `true`.
"Fast-forward" here means `old` matches the accepted head; the engine does not walk git ancestry.
A push with an empty `new` deletes the accepted head of that ref.
The outcome holds `state_json`, the accepted and rejected pushes (with reasons), the obligations and the log lines.

## Rust API

`Program::load(engine_tag, source)` compiles a program and checks for `init`.
`Program::hash(source)` gives the pin hash.
`Program::fold(repo: RepoInfo, events)` takes `Event::Op(ProgramOp)` and `Event::Push(RefUpdate)` and returns an `Outcome`.
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

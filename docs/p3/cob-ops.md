# P3: collaborative-object ops

Status: implemented (core, sync, CLI), hackathon scope.
Date: 2026-11-07.
Builds on `docs/p1/design.md` sections 2.1 (envelope, attributes), 2.2 (the reserved `cob` kind) and 3.2 (ordering).

## Model

Issues, patches, reviews, attestations, payments and votes are all collaborative objects (COBs).
Each object is a sequence of signed ops; programs fold the verified ops of an object into its state later.
The design mirrors Radicle COBs, but Arkiv creation order replaces the git change DAG as the ordering source.
This layer checks authorship and causality only; who may do what is the folding program's decision.

## Op format

An op is a `cob` entity (kind code 9) whose envelope body is canonical CBOR:

```cddl
CobOpBody = {
  1: bstr .size 32,      ; rid
  2: tstr,               ; type_name, 1..64 B of [a-z0-9._-], e.g. "xyz.arkiv.patch"
  3: bstr .size 32,      ; object id, 32 zero bytes iff action = "create"
  4: [* bstr .size 32],  ; parents: ids of ops of the same object this op saw, may be empty
  5: tstr,               ; action, 1..32 B of [a-z0-9._-]
  6: bstr,               ; payload: one canonical CBOR value, at most 65,536 B
  7: uint,               ; ts_ms, the author's clock, display only
}
```

- Keys 1 to 4 follow the P1 reservation (rid, type, object, parents); the reserved `[op bstr...]` list became one op per entity with an `action` and a `payload`.
- The op is signed in the standard envelope with the author's composite identity key, signature context `arkiv-git/cob/v1`.
- The op id is the standard content hash `H("cob", envelope bytes)`.
- A create op has object = zero, action `create` and no parents; the object's id is the create op's id.
- The payload may hold integers, byte and text strings, arrays, maps, booleans and null, nested at most 32 deep.
- Floats, tags, undefined, simple values and indefinite lengths are rejected, so every payload has one byte form.
- The payload cap is 65,536 B, but the whole envelope must also fit the 65,536 B `ag.body` cap, so with ML-DSA-65 the usable body is 62,183 B.

## Attributes

| Name | Type | Value |
| --- | --- | --- |
| `ag.kind` | `str` | `cob` |
| `ag.v`, `ag.body` | as in P1 | layout version, envelope bytes |
| `ag.rid` | `bytes32` | field 1 |
| `ag.nid` | `bytes32` | author (envelope signer) |
| `ag.h` | `bytes32` | op id |
| `ag.cob` | `bytes32` | object id: field 3, or the op id for a create op |
| `ag.type` | `str` | field 2 |
| `ag.action` | `str` | field 5 |

- Nine attributes, well under the 32-attribute limit; the longest string is 64 B, under the 128 B limit.
- Setting `ag.cob` of a create op to its own id lets one `ag.cob = X` query return the create op and every follow-up.
- As in P1, attributes are query hints only: a decoded op whose attributes differ from the ones derived from its body is ignored.

## Writing

- `charter-sync::cob::post` validates the body, signs it and sends one create op from the author's control address, since `cob` is a control-plane kind.
- A writer with no bind for the repo gets its `peer` and `bind_0` entities in the same tx, which also makes it a known identity.
- Funding goes through the existing rolling top-up from the wallet.
- The entity lifetime is the control lifetime (`CHARTER_CONTROL_LIFETIME`, default 182.5 days).
- An invalid or oversize op fails with `E_BAD_FORMAT` before anything is sent.

## Verification

`charter-core::verify_ops` takes the observed entities and the known peer keys and returns `Vec<VerifiedOp { id, author, body, arkiv_order }>`.
It walks the entities in Arkiv creation order and drops, never errors on, any op that:

1. fails decoding, the canonical re-encode check, the field rules or the attribute check;
2. carries another repo's rid;
3. is signed by a nid with no valid self-signed `peer` document (any peer counts, delegates or not);
4. has a signature that does not verify under that key;
5. repeats the id of an op already accepted (a third party re-posting signed bytes adds nothing);
6. is not a create op and its object has no earlier accepted create op;
7. has a type_name different from its object's create op;
8. names a parent that is not an earlier accepted op of the same object.

- Rule 8 drops parents from other objects, parents from later ops, and parents that were themselves dropped.
- Ordering is Arkiv creation order (block, then rank in the creation-ordered query result, then entity key), the same source anchors use; `ts_ms` never orders anything.
- `arkiv_order` compares ops within one `list` result; ranks are not comparable across separate queries.
- Arkiv is trusted for ordering and availability only; content, authorship and causality come from the signed bytes.

## Reading

- `charter-sync::cob::list(rid, object, type_name)` queries `kind = cob AND rid [AND ag.cob] [AND ag.type]` with the standard offset paging (100 rows per page, count check), resolves each signer's `peer` document, and returns `verify_ops` of the result.

## CLI

- `chart op post --repo <rid> --type <t> --action <a> [--object <id>] [--parent <id>...] --payload-json '<json>'` converts the JSON to canonical CBOR and prints the op id in hex.
- JSON floats (including `1.0`) are rejected with `E_BAD_FORMAT`; map keys are sorted by their encoded bytes.
- `chart op list --repo <rid> [--object <id>] [--type <t>] [--json]` prints the verified ops in order, one line each, or one JSON array.
- In JSON output, byte strings show as `{"$bytes": "<hex>"}`, non-text map keys as their JSON text, and integers outside the i64 and u64 ranges as decimal strings.

## Not in this version

- Keepalive does not extend `cob` entities yet, so ops expire after the control lifetime unless someone extends them.
- No completeness check: a withheld or expired op is simply absent, and ops that cite it as a parent are dropped.
- No creator rule: an op is valid whichever EVM address created the entity, because the signature carries authorship.
- No P2 encryption of payloads.

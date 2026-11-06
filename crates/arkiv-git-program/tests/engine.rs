//! Engine hardening, ABI defaults and fold semantics.

mod common;

use arkiv_git_program::*;
use common::*;
use serde_json::json;

const COUNTER: &str = r#"
fn init(repo) { #{ n: 0, name: repo.name, quorum: repo.quorum } }
fn on_op(state, op) {
    if op.payload.bad == true { throw "bad op"; }
    state.n += op.payload.inc;
    state.last = op.author;
    state
}
"#;

#[test]
fn rejects_unknown_engine_and_missing_init() {
    assert!(matches!(
        Program::load("lua/1", COUNTER),
        Err(ProgramError::UnknownEngine(_))
    ));
    assert_eq!(
        Program::load(ENGINE_RHAI_V1, "fn x() { 1 }").err(),
        Some(ProgramError::NoInit)
    );
}

#[test]
fn hash_is_tagged_sha256_of_source() {
    assert_eq!(Program::hash("x"), arkiv_git_crypto::h("program", b"x"));
    assert_ne!(Program::hash("x"), arkiv_git_crypto::sha256(b"x"));
}

#[test]
fn float_literal_fails_to_load() {
    let r = Program::load(ENGINE_RHAI_V1, "fn init(repo) { 1.5 }");
    assert!(matches!(r, Err(ProgramError::Compile(_))), "{r:?}");
}

#[test]
fn eval_is_disabled() {
    let r = Program::load(ENGINE_RHAI_V1, r#"fn init(repo) { eval("40 + 2") }"#);
    assert!(matches!(r, Err(ProgramError::Compile(_))), "{r:?}");
}

#[test]
fn import_and_timestamp_are_unavailable() {
    assert!(Program::load(ENGINE_RHAI_V1, r#"import "x" as y; fn init(r) { 0 }"#).is_err());
    let p = load("fn init(r) { timestamp() }");
    assert!(matches!(p.fold(repo(), []), Err(ProgramError::Init(_))));
}

#[test]
fn same_events_give_byte_identical_state_across_engines() {
    let events = || {
        vec![
            op(1, 0xd1, 9, "t", "a", json!({"inc": 2})),
            op(
                2,
                0xd2,
                9,
                "t",
                "a",
                json!({"inc": 5, "z": [1, "x"], "m": {"b": 1, "a": 2}}),
            ),
        ]
    };
    let a = load(COUNTER).fold(repo(), events()).expect("fold a");
    let b = load(COUNTER).fold(repo(), events()).expect("fold b");
    assert_eq!(a.state_json, b.state_json);
    assert_eq!(state(&a)["n"], 7);
    assert_eq!(state(&a)["last"], hx(0xd2));
}

#[test]
fn maps_iterate_in_sorted_key_order() {
    let p = load(r#"fn init(r) { let m = #{ zeta: 1, alpha: 2, mid: 3 }; m.keys() }"#);
    let o = p.fold(repo(), []).expect("fold");
    assert_eq!(o.state_json, r#"["alpha","mid","zeta"]"#);
}

#[test]
fn failing_op_is_skipped_and_logged() {
    let o = load(COUNTER)
        .fold(
            repo(),
            [
                op(1, 0xd1, 9, "t", "a", json!({"inc": 2})),
                op(2, 0xd1, 9, "t", "a", json!({"inc": 3, "bad": true})),
            ],
        )
        .expect("fold");
    assert_eq!(state(&o)["n"], 2);
    assert_eq!(o.logs.len(), 1);
    assert!(o.logs[0].contains("skipped"), "{:?}", o.logs);
}

#[test]
fn print_and_debug_are_captured_not_emitted() {
    let p = load(r#"fn init(r) { print("hi"); debug("dbg"); 0 }"#);
    let o = p.fold(repo(), []).expect("fold");
    assert_eq!(o.logs[0], "hi");
    assert!(o.logs[1].contains("dbg"));
}

#[test]
fn init_error_fails_the_fold() {
    let p = load(r#"fn init(r) { throw "no" }"#);
    assert!(matches!(p.fold(repo(), []), Err(ProgramError::Init(_))));
}

#[test]
fn fuel_exhaustion_in_check_push_rejects() {
    let p = load("fn init(r) { 0 }\nfn check_push(s, p) { loop { } }");
    let o = p.fold(repo(), [main(1, None, 1)]).expect("fold");
    assert!(o.accepted.is_empty());
    assert!(
        o.rejected[0].1.starts_with("program error"),
        "{:?}",
        o.rejected
    );
}

#[test]
fn fold_budget_caps_total_work() {
    // Each op burns most of a call budget; the fold budget runs out before the last push.
    let src = "fn init(r) { 0 }\nfn on_op(s, o) { let i = 0; while i < 40000 { i += 1; } s }";
    let mut events: Vec<Event> = (1..=60).map(|n| op(n, 1, 1, "t", "a", empty())).collect();
    events.push(main(61, None, 1));
    let o = load(&format!("{src}\nfn check_push(s, p) {{ true }}"))
        .fold(repo(), events)
        .expect("fold");
    assert!(
        o.logs.iter().any(|l| l.contains("budget")),
        "{:?}",
        o.logs.last()
    );
    assert_eq!(o.rejected.len(), 1);
}

#[test]
fn default_rule_is_fast_forward_of_accepted_head() {
    let o = load("fn init(r) { 0 }")
        .fold(
            repo(),
            [
                main(1, None, 1),
                main(2, Some(9), 2), // stale old: rejected
                main(3, Some(1), 3), // checked against the last accepted head
            ],
        )
        .expect("fold");
    assert_eq!(o.accepted.len(), 2);
    assert_eq!(o.accepted[1].new, oid(3));
    assert_eq!(o.rejected[0].1, "not a fast-forward of the accepted head");
}

#[test]
fn check_push_true_overrides_fast_forward_and_string_rejects() {
    let src = r#"fn init(r) { 0 }
fn check_push(s, p) { if p.ref == "refs/heads/force" { true } else if !p.ff { () } else { "nope" } }"#;
    let o = load(src)
        .fold(
            repo(),
            [
                push(1, 1, "refs/heads/force", Some(7), 1),
                push(2, 1, "refs/heads/x", None, 1),
                push(3, 1, "refs/heads/y", Some(7), 1),
            ],
        )
        .expect("fold");
    assert_eq!(o.accepted.len(), 1);
    assert_eq!(o.rejected[0].1, "nope");
    assert_eq!(o.rejected[1].1, "not a fast-forward of the accepted head");
}

#[test]
fn out_of_order_events_fail() {
    let r = load("fn init(r) { 0 }").fold(repo(), [main(5, None, 1), main(4, Some(1), 2)]);
    assert_eq!(r.err(), Some(ProgramError::OutOfOrder(4)));
}

#[test]
fn payload_cbor_converts_to_rhai_values() {
    let src = r#"fn init(r) { [] }
fn on_op(s, o) { s.push(o.payload); s.push(type_of(o.payload.b)); s }"#;
    let o = load(src)
        .fold(
            repo(),
            [op(
                1,
                1,
                1,
                "t",
                "a",
                json!({"i": -3, "s": "x", "b": true, "n": null, "a": [1]}),
            )],
        )
        .expect("fold");
    assert_eq!(
        state(&o),
        json!([{"a": [1], "b": true, "i": -3, "n": null, "s": "x"}, "bool"])
    );
}

#[test]
fn events_parse_from_json() {
    let raw = json!([
        {"event": "op", "id": hx(1), "author": hx(2), "object": hx(3), "type": "t",
         "action": "a", "payload": {"inc": 4}, "arkiv_order": 1},
        {"event": "push", "author": hx(2), "ref": "refs/heads/main", "old": "",
         "new": oid_hex(1), "arkiv_order": 2}
    ]);
    let events: Vec<Event> = serde_json::from_value(raw).expect("events");
    let o = load(COUNTER).fold(repo(), events).expect("fold");
    assert_eq!(state(&o)["n"], 4);
    assert_eq!(o.accepted.len(), 1);
}

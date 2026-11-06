//! `programs/ci-gated.rhai`: main needs quorum delegate approvals AND passing CI
//! attestations from the configured runner, both at the exact patch head.

mod common;

use arkiv_git_program::{Event, Outcome};
use common::*;
use serde_json::json;

const PATCH: u8 = 0x50;
/// The arkiv-git CI runner nid z76sCb7au55NQrrbUC8YdoGoLSnwqw5vBvFzumNtjCBoi in hex,
/// the default attester of the template.
const RUNNER: &str = "5aa6c964c24dda968807ce8334b6183bfe7e074848ede2f26f71c024f0c2b075";

fn runner() -> [u8; 32] {
    hex::decode(RUNNER)
        .expect("hex")
        .try_into()
        .expect("32 bytes")
}

fn create_patch(order: u64, head: u8) -> Event {
    let payload = json!({"title": "t", "head": oid_hex(head)});
    op(order, 5, PATCH, "xyz.arkiv.patch", "create", payload)
}

fn approve(order: u64, author: u8, head: u8) -> Event {
    let payload = json!({"patch": hx(PATCH), "head": oid_hex(head)});
    op(order, author, 0x51, "xyz.arkiv.review", "approve", payload)
}

fn attest_by(order: u64, author: [u8; 32], head: u8, status: &str) -> Event {
    let payload = json!({
        "head": oid_hex(head), "status": status, "suite": "cargo", "log_sha256": "00",
    });
    let mut e = op(order, 0, 0x40, "xyz.arkiv.attest", "ci", payload);
    if let Event::Op(o) = &mut e {
        o.author = author;
    }
    e
}

fn attest(order: u64, head: u8, status: &str) -> Event {
    attest_by(order, runner(), head, status)
}

fn fold(events: Vec<Event>) -> Outcome {
    template("ci-gated").fold(repo(), events).expect("fold")
}

fn reason(o: &Outcome) -> &str {
    &o.rejected.first().expect("a rejected push").1
}

#[test]
fn approval_without_ci_is_rejected_with_a_ci_reason() {
    let o = fold(vec![
        create_patch(1, 1),
        approve(2, 0xd1, 1),
        approve(3, 0xd2, 1),
        main(4, None, 1),
    ]);
    assert!(o.accepted.is_empty());
    assert!(reason(&o).contains("CI"), "{}", reason(&o));
    assert!(!reason(&o).contains("approval"), "{}", reason(&o));
}

#[test]
fn ci_without_approval_is_rejected_with_an_approval_reason() {
    let o = fold(vec![
        create_patch(1, 1),
        attest(2, 1, "pass"),
        main(3, None, 1),
    ]);
    assert!(o.accepted.is_empty());
    assert!(reason(&o).contains("approval"), "{}", reason(&o));
    assert!(!reason(&o).contains("CI"), "{}", reason(&o));
}

#[test]
fn approvals_and_ci_pass_at_the_head_are_accepted() {
    let o = fold(vec![
        create_patch(1, 1),
        attest(2, 1, "pass"),
        approve(3, 0xd1, 1),
        approve(4, 0xd2, 1),
        main(5, None, 1),
    ]);
    assert_eq!(o.accepted.len(), 1, "{:?}", o.rejected);
}

#[test]
fn attestation_from_a_non_attester_is_ignored() {
    let o = fold(vec![
        create_patch(1, 1),
        attest_by(2, id(0xd1), 1, "pass"), // a delegate is not an attester
        approve(3, 0xd1, 1),
        approve(4, 0xd2, 1),
        main(5, None, 1),
    ]);
    assert!(o.accepted.is_empty());
    assert!(reason(&o).contains("has 0"), "{}", reason(&o));
}

#[test]
fn attestation_for_a_different_head_is_ignored() {
    let o = fold(vec![
        create_patch(1, 1),
        attest(2, 9, "pass"),
        approve(3, 0xd1, 1),
        approve(4, 0xd2, 1),
        main(5, None, 1),
    ]);
    assert!(o.accepted.is_empty());
    assert!(reason(&o).contains("CI"), "{}", reason(&o));
}

#[test]
fn a_revised_head_needs_fresh_ci_and_approvals() {
    let revise = op(
        5,
        5,
        PATCH,
        "xyz.arkiv.patch",
        "revise",
        json!({"head": oid_hex(2)}),
    );
    let o = fold(vec![
        create_patch(1, 1),
        attest(2, 1, "pass"),
        approve(3, 0xd1, 1),
        approve(4, 0xd2, 1),
        revise,
        main(6, None, 2),
    ]);
    assert!(o.accepted.is_empty());
    let why = reason(&o);
    assert!(why.contains("approval") && why.contains("CI"), "{why}");
}

#[test]
fn a_later_fail_overrides_an_earlier_pass() {
    let o = fold(vec![
        create_patch(1, 1),
        attest(2, 1, "pass"),
        attest(3, 1, "fail"),
        approve(4, 0xd1, 1),
        approve(5, 0xd2, 1),
        main(6, None, 1),
    ]);
    assert!(o.accepted.is_empty());
    assert!(reason(&o).contains("CI"), "{}", reason(&o));
    assert_eq!(state(&o)["ci"][oid_hex(1)][RUNNER], "fail");
}

#[test]
fn other_refs_are_not_gated() {
    let o = fold(vec![push(1, 0xd1, "refs/heads/feature", None, 3)]);
    assert_eq!(o.accepted.len(), 1);
}

#[test]
fn the_fold_is_deterministic() {
    let events = || {
        vec![
            create_patch(1, 1),
            attest(2, 1, "pass"),
            approve(3, 0xd2, 1),
            approve(4, 0xd1, 1),
            main(5, None, 1),
            main(6, Some(1), 2),
        ]
    };
    let (a, b) = (fold(events()), fold(events()));
    assert_eq!(a.state_json, b.state_json);
    assert_eq!(a.accepted, b.accepted);
    assert_eq!(a.rejected, b.rejected);
    assert_eq!(a.accepted.len(), 1);
}

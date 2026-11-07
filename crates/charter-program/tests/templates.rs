//! The demo programs in `programs/`, loaded from disk: a happy path and a rejection each.

mod common;

use common::*;
use serde_json::json;

const PATCH: u8 = 0x50;
const ISSUE: u8 = 0x60;

fn create_patch(order: u64, author: u8, head: u8) -> charter_program::Event {
    op(
        order,
        author,
        PATCH,
        "xyz.arkiv.patch",
        "create",
        json!({"title": "t", "head": oid_hex(head)}),
    )
}

fn approve(order: u64, author: u8, head: u8) -> charter_program::Event {
    let payload = json!({"patch": hx(PATCH), "head": oid_hex(head)});
    op(order, author, 0x51, "xyz.arkiv.review", "approve", payload)
}

#[test]
fn branch_protection_accepts_quorum_approved_head() {
    let o = template("branch-protection")
        .fold(
            repo(),
            [
                create_patch(1, 5, 1),
                approve(2, 0xd1, 1),
                approve(3, 0xd2, 1),
                main(4, None, 1),
            ],
        )
        .expect("fold");
    assert_eq!(o.accepted.len(), 1);
    assert!(o.rejected.is_empty());
}

#[test]
fn branch_protection_rejects_short_quorum_and_stale_approvals() {
    let events = [
        create_patch(1, 5, 1),
        approve(2, 0xd1, 1),
        approve(3, 0x99, 1), // not a delegate
        main(4, None, 1),
        op(
            5,
            5,
            PATCH,
            "xyz.arkiv.patch",
            "revise",
            json!({"head": oid_hex(2)}),
        ),
        approve(6, 0xd2, 1), // approves the old head
        main(7, None, 2),
        push(8, 5, "refs/heads/feature", None, 3), // other refs are unprotected
    ];
    let o = template("branch-protection")
        .fold(repo(), events)
        .expect("fold");
    assert_eq!(o.rejected.len(), 2);
    assert!(
        o.rejected[0].1.contains("2 delegate approvals"),
        "{:?}",
        o.rejected
    );
    assert_eq!(o.accepted.len(), 1);
    assert_eq!(o.accepted[0].name, "refs/heads/feature");
}

fn attest(order: u64, author: u8, commit: u8, result: &str) -> charter_program::Event {
    let payload = json!({"commit": oid_hex(commit), "result": result});
    op(order, author, 0x40, "xyz.arkiv.attest", "ci", payload)
}

#[test]
fn ci_attested_accepts_k_of_n_passes() {
    let o = template("ci-attested")
        .fold(
            repo(),
            [
                attest(1, 0xa1, 1, "pass"),
                attest(2, 0xa3, 1, "pass"),
                main(3, None, 1),
            ],
        )
        .expect("fold");
    assert_eq!(o.accepted.len(), 1);
}

#[test]
fn ci_attested_rejects_failures_and_strangers() {
    let events = [
        attest(1, 0xa1, 1, "pass"),
        attest(2, 0xa2, 1, "fail"),
        attest(3, 0xb1, 1, "pass"), // not an attester
        attest(4, 0xa1, 1, "pass"), // duplicate counts once
        main(5, None, 1),
    ];
    let o = template("ci-attested").fold(repo(), events).expect("fold");
    assert!(o.accepted.is_empty());
    assert!(o.rejected[0].1.ends_with("has 1"), "{:?}", o.rejected);
}

fn bounty_events(merged_head: u8) -> Vec<charter_program::Event> {
    vec![
        op(
            1,
            1,
            ISSUE,
            "xyz.arkiv.issue",
            "create",
            json!({"title": "bug", "bounty": 100}),
        ),
        op(
            2,
            2,
            0x61,
            "xyz.arkiv.bounty",
            "pledge",
            json!({"issue": hx(ISSUE), "amount": 50}),
        ),
        create_patch(3, 5, 1),
        op(
            4,
            0xd1,
            PATCH,
            "xyz.arkiv.patch",
            "merged",
            json!({"patch": hx(PATCH), "closes": hx(ISSUE)}),
        ),
        main(5, None, merged_head),
    ]
}

fn receipt(order: u64, amount: u64) -> charter_program::Event {
    let payload = json!({"tx": "0xabc", "to": hx(5), "amount": amount});
    op(order, 0xd1, 0x62, "xyz.arkiv.payment", "receipt", payload)
}

#[test]
fn bounty_owes_patch_author_until_receipt() {
    let p = template("bounty");
    let o = p.fold(repo(), bounty_events(1)).expect("fold");
    let reason = format!("bounty for issue {}", hx(ISSUE));
    let key = format!("bounty:{}", hx(ISSUE));
    assert_eq!(
        o.obligations,
        vec![
            json!({"kind": "pay", "to": hx(5), "amount": 150, "reason": reason,
            "key": key, "purpose": key, "due_order": 5})
        ]
    );
    let mut paid = bounty_events(1);
    paid.push(receipt(6, 150));
    assert!(p.fold(repo(), paid).expect("fold").obligations.is_empty());
}

#[test]
fn bounty_needs_the_merged_head_on_main_and_an_exact_receipt() {
    let p = template("bounty");
    assert!(
        p.fold(repo(), bounty_events(9))
            .expect("fold")
            .obligations
            .is_empty()
    );
    let mut short = bounty_events(1);
    short.push(receipt(6, 149));
    assert_eq!(p.fold(repo(), short).expect("fold").obligations.len(), 1);
}

fn sealed(order: u64, action: &str, payload: serde_json::Value) -> charter_program::Event {
    op(order, 5, 0x70, "xyz.arkiv.sealed", action, payload)
}

fn commitment() -> serde_json::Value {
    let c = hex::encode(charter_crypto::sha256(b"secret"));
    json!({"object": hx(0x71), "commitment": c})
}

#[test]
fn sealed_patch_accepts_matching_reveal() {
    let o = template("sealed-patch")
        .fold(
            repo(),
            [
                sealed(1, "commit", commitment()),
                sealed(2, "reveal", json!({"key": "secret"})),
            ],
        )
        .expect("fold");
    assert_eq!(state(&o)["revealed"][hx(0x70)], "secret");
    assert!(o.obligations.is_empty());
}

#[test]
fn sealed_patch_rejects_wrong_key_and_keeps_obligation() {
    let o = template("sealed-patch")
        .fold(
            repo(),
            [
                sealed(1, "commit", commitment()),
                sealed(2, "reveal", json!({"key": "guess"})),
            ],
        )
        .expect("fold");
    assert_eq!(state(&o)["revealed"], empty());
    assert_eq!(o.obligations[0]["kind"], "release_key");
    assert_eq!(o.obligations[0]["target"], hx(0x71));
    assert!(o.logs[0].contains("does not match"), "{:?}", o.logs);
}

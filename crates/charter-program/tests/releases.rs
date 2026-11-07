//! `programs/ci-gated.rhai` release records (docs/p3/releases.md): a delegate's
//! `xyz.arkiv.release` op is valid only for a CI-passed main commit whose every artifact
//! has `need()` matching `xyz.arkiv.attest` `artifact` attestations.

mod common;

use charter_program::{Event, Outcome};
use common::*;
use serde_json::{Value, json};

const PATCH: u8 = 0x50;
const RELEASE: u8 = 0x60;
/// The template's default attester (the Charter CI runner).
const RUNNER: &str = "5aa6c964c24dda968807ce8334b6183bfe7e074848ede2f26f71c024f0c2b075";
const SHA_A: &str = "aa00000000000000000000000000000000000000000000000000000000000000";
const SHA_B: &str = "bb00000000000000000000000000000000000000000000000000000000000000";

fn by(mut e: Event, author: [u8; 32]) -> Event {
    if let Event::Op(o) = &mut e {
        o.author = author;
    }
    e
}

fn runner() -> [u8; 32] {
    hex::decode(RUNNER).expect("hex").try_into().expect("32")
}

/// Patch at head 1, both CI and quorum approvals at that head.
fn landed(ci: bool) -> Vec<Event> {
    let mut v = vec![
        op(
            1,
            5,
            PATCH,
            "xyz.arkiv.patch",
            "create",
            json!({"head": oid_hex(1)}),
        ),
        approve(2, 0xd1),
        approve(3, 0xd2),
    ];
    if ci {
        let p = json!({"head": oid_hex(1), "status": "pass", "suite": "s", "log_sha256": "00"});
        v.push(by(op(4, 0, 0x40, "xyz.arkiv.attest", "ci", p), runner()));
    }
    v
}

fn approve(order: u64, d: u8) -> Event {
    let p = json!({"patch": hx(PATCH), "head": oid_hex(1)});
    op(order, d, 0x51, "xyz.arkiv.review", "approve", p)
}

fn artifact(name: &str, sha: &str, locations: &[&str]) -> Value {
    json!({"name": name, "target": "t", "sha256": sha, "size": 3, "locations": locations})
}

fn release(order: u64, author: u8, artifacts: Value) -> Event {
    let p = json!({"tag": "v1", "commit": oid_hex(1), "artifacts": artifacts});
    op(order, author, RELEASE, "xyz.arkiv.release", "create", p)
}

fn attest(order: u64, name: &str, sha: &str) -> Event {
    let p = json!({"tag": "v1", "commit": oid_hex(1), "name": name, "sha256": sha});
    by(
        op(order, 0, 0x40, "xyz.arkiv.attest", "artifact", p),
        runner(),
    )
}

fn fold(events: Vec<Event>) -> Outcome {
    template("ci-gated").fold(repo(), events).expect("fold")
}

fn rel(o: &Outcome) -> Value {
    state(o)["releases"]["v1"].clone()
}

fn missing(o: &Outcome) -> String {
    rel(o)["missing"].to_string()
}

/// Landed through the gate (main push accepted), released, attested.
fn valid_events() -> Vec<Event> {
    let mut v = landed(true);
    v.push(main(5, None, 1));
    v.push(release(
        6,
        0xd1,
        json!([artifact("a", SHA_A, &["file:///a"])]),
    ));
    v.push(attest(7, "a", SHA_A));
    v
}

#[test]
fn a_released_main_commit_with_ci_and_attested_artifacts_is_valid() {
    let o = fold(valid_events());
    assert_eq!(o.accepted.len(), 1, "{:?}", o.rejected);
    let r = rel(&o);
    assert_eq!(r["valid"], true, "{r}");
    assert_eq!(r["missing"], json!([]));
    assert_eq!(r["commit"], oid_hex(1));
    assert_eq!(r["artifacts"][0]["sha256"], SHA_A);
    assert_eq!(state(&o)["main"][oid_hex(1)], true);
}

#[test]
fn an_approved_patch_head_counts_when_the_push_predates_the_pin() {
    let mut v = landed(true); // no push event: main was seeded at head 1
    v.push(release(
        6,
        0xd1,
        json!([artifact("a", SHA_A, &["file:///a"])]),
    ));
    v.push(attest(7, "a", SHA_A));
    assert_eq!(rel(&fold(v))["valid"], true);
}

#[test]
fn a_release_by_a_non_delegate_is_ignored() {
    let mut v = landed(true);
    v.push(release(6, 5, json!([artifact("a", SHA_A, &["file:///a"])])));
    assert_eq!(state(&fold(v))["releases"], json!({}));
}

#[test]
fn a_commit_without_a_ci_pass_is_invalid_with_a_ci_reason() {
    let mut v = landed(false);
    v.push(release(
        6,
        0xd1,
        json!([artifact("a", SHA_A, &["file:///a"])]),
    ));
    v.push(attest(7, "a", SHA_A));
    let o = fold(v);
    assert_eq!(rel(&o)["valid"], false);
    assert!(missing(&o).contains("CI"), "{}", missing(&o));
}

#[test]
fn a_commit_that_never_reached_main_is_invalid() {
    let p = json!({"tag": "v1", "commit": oid_hex(9), "artifacts": [artifact("a", SHA_A, &["x"])]});
    let o = fold(vec![op(1, 0xd1, RELEASE, "xyz.arkiv.release", "create", p)]);
    assert_eq!(rel(&o)["valid"], false);
    assert!(missing(&o).contains("main"), "{}", missing(&o));
}

#[test]
fn an_artifact_without_an_attestation_is_invalid() {
    let mut v = landed(true);
    let arts = json!([artifact("a", SHA_A, &["x"]), artifact("b", SHA_B, &["y"])]);
    v.push(release(6, 0xd1, arts));
    v.push(attest(7, "a", SHA_A));
    let o = fold(v);
    assert_eq!(rel(&o)["valid"], false);
    let m = missing(&o);
    assert!(m.contains("artifact b") && !m.contains("artifact a"), "{m}");
}

#[test]
fn an_attestation_of_a_different_sha256_is_invalid() {
    let mut v = landed(true);
    v.push(release(6, 0xd1, json!([artifact("a", SHA_A, &["x"])])));
    v.push(attest(7, "a", SHA_B));
    let o = fold(v);
    assert_eq!(rel(&o)["valid"], false);
    assert!(missing(&o).contains("artifact a"), "{}", missing(&o));
}

#[test]
fn an_attestation_from_a_non_attester_is_ignored() {
    let mut v = landed(true);
    v.push(release(6, 0xd1, json!([artifact("a", SHA_A, &["x"])])));
    v.push(by(attest(7, "a", SHA_A), id(0xd1)));
    assert_eq!(rel(&fold(v))["valid"], false);
}

#[test]
fn a_revise_adding_a_location_keeps_validity_and_cannot_change_a_hash() {
    let mut v = valid_events();
    let p = json!({"artifacts": [artifact("a", SHA_A, &["https://m/a"]), artifact("a", SHA_B, &["evil"])]});
    v.push(op(8, 0xd2, RELEASE, "xyz.arkiv.release", "revise", p));
    let late = json!({"tag": "v1", "commit": oid_hex(2), "artifacts": []});
    v.push(op(9, 0xd1, 0x61, "xyz.arkiv.release", "create", late)); // a tag is created once
    let r = rel(&fold(v));
    assert_eq!(r["valid"], true, "{r}");
    assert_eq!(r["commit"], oid_hex(1));
    assert_eq!(r["artifacts"][0]["sha256"], SHA_A);
    assert_eq!(
        r["artifacts"][0]["locations"],
        json!(["file:///a", "https://m/a"])
    );
    assert_eq!(r["artifacts"].as_array().map(Vec::len), Some(1));
}

#[test]
fn the_release_fold_is_deterministic() {
    let (a, b) = (fold(valid_events()), fold(valid_events()));
    assert_eq!(a.state_json, b.state_json);
    assert_eq!(a.accepted, b.accepted);
}

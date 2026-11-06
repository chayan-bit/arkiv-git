//! Proofs as program policy (docs/p3/programs.md, "Proofs"): the committed real receipt is
//! attached through blob ops, verified by `proofs::attach`, and `sealed-patch.rhai` decides.

use arkiv_git_program::{ENGINE_RHAI_V1, Event, Outcome, Program, ProgramOp, RepoInfo};
use arkiv_git_sync::cob::json::to_cbor;
use arkiv_git_sync::program::proofs::{BLOB_CHUNK, BLOB_TYPE, ProofCache, attach};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const RECEIPT: &[u8] = include_bytes!("../../arkiv-git-zk/tests/fixtures/demo.receipt");
const PUBLIC: &str = include_str!("../../arkiv-git-zk/tests/fixtures/demo.public.json");
const SCRIPT: &str = include_str!("../../../programs/sealed-patch.rhai");
const ISSUE: u8 = 0x60;
const SEALED: u8 = 0x70;

fn sha(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

fn public() -> Value {
    serde_json::from_str(PUBLIC).expect("fixture")
}

fn op(order: u64, object: u8, ty: &str, action: &str, payload: Value) -> ProgramOp {
    ProgramOp {
        id: [order as u8; 32],
        author: [5; 32],
        rid: [0xee; 32],
        object: [object; 32],
        type_name: ty.into(),
        action: action.into(),
        parents: vec![],
        payload: to_cbor(&payload).expect("payload"),
        ts_ms: 0,
        arkiv_order: order,
        proofs: vec![],
    }
}

/// Blob ops (orders 1..) carrying `receipt`, and the proof entry that references them.
fn attachment(receipt: &[u8], public: Value) -> (Vec<ProgramOp>, Value) {
    let chunks: Vec<&[u8]> = receipt.chunks(BLOB_CHUNK).collect();
    let blobs = (1u64..)
        .zip(&chunks)
        .map(|(n, c)| {
            op(
                n,
                0x40 + n as u8,
                BLOB_TYPE,
                "create",
                json!({"data": hex::encode(c)}),
            )
        })
        .collect();
    let hashes: Vec<String> = chunks.iter().map(|c| sha(c)).collect();
    let entry =
        json!({"kind": "risc0", "receipt": sha(receipt), "chunks": hashes, "public": public});
    (blobs, entry)
}

fn issue(proof: &str, spec_hash: &str) -> ProgramOp {
    let payload = json!({"title": "fix add", "proof": proof, "spec_hash": spec_hash});
    op(100, ISSUE, "xyz.arkiv.issue", "create", payload)
}

fn commit(proofs: Vec<Value>) -> ProgramOp {
    let payload = json!({
        "object": hex::encode([0x50; 32]),
        "commitment": public()["key_hash"],
        "issue": hex::encode([ISSUE; 32]),
        "proofs": proofs,
    });
    op(101, SEALED, "xyz.arkiv.sealed", "commit", payload)
}

fn fold(ops: Vec<ProgramOp>, cache: &mut ProofCache) -> Outcome {
    let repo = RepoInfo {
        rid: [0xee; 32],
        delegates: vec![[0xd1; 32]],
        quorum: 1,
        name: "demo".into(),
    };
    let mut ops = attach(ops, cache);
    ops.sort_by_key(|o| o.arkiv_order);
    Program::load(ENGINE_RHAI_V1, SCRIPT)
        .expect("load")
        .fold(repo, ops.into_iter().map(Event::Op))
        .expect("fold")
}

fn sealed(o: &Outcome) -> Value {
    let state: Value = serde_json::from_str(&o.state_json).expect("state");
    state["sealed"][hex::encode([SEALED; 32])].clone()
}

fn spec_hash() -> String {
    public()["spec_hash"]
        .as_str()
        .expect("spec_hash")
        .to_owned()
}

fn run(issue_op: ProgramOp, receipt: Option<(&[u8], Value)>) -> Outcome {
    let mut ops = vec![issue_op];
    let proofs = match receipt {
        Some((bytes, public)) => {
            let (blobs, entry) = attachment(bytes, public);
            ops.extend(blobs);
            vec![entry]
        }
        None => vec![],
    };
    ops.push(commit(proofs));
    fold(ops, &mut ProofCache::default())
}

#[test]
fn required_proof_accepts_the_matching_sealed_commit() {
    let o = run(issue("required", &spec_hash()), Some((RECEIPT, public())));
    assert_eq!(sealed(&o)["proven"], true, "{}", o.state_json);
    assert_eq!(o.obligations.len(), 1, "release_key is owed");
    assert!(
        SCRIPT.contains(&arkiv_git_zk::image_id_hex()),
        "script pins the guest"
    );
}

#[test]
fn required_proof_rejects_a_missing_proof() {
    let o = run(issue("required", &spec_hash()), None);
    assert!(sealed(&o).is_null(), "{}", o.state_json);
    assert!(
        o.logs
            .iter()
            .any(|l| l.contains("requires a matching proof"))
    );
}

#[test]
fn required_proof_rejects_a_tampered_receipt() {
    let mut bad = RECEIPT.to_vec();
    let mid = bad.len() / 2;
    bad[mid] ^= 1;
    let (blobs, entry) = attachment(&bad, public());
    let mut ops = blobs;
    ops.push(commit(vec![entry]));
    let checked = attach(ops.clone(), &mut ProofCache::default());
    let proofs = &checked.last().expect("commit").proofs;
    assert_eq!((proofs.len(), proofs[0].ok), (1, false));
    ops.push(issue("required", &spec_hash()));
    assert!(sealed(&fold(ops, &mut ProofCache::default())).is_null());
}

#[test]
fn valid_proof_of_a_different_spec_is_rejected() {
    // The receipt verifies (ok == true), but for a spec the issue did not name.
    let other = sha(b"+ something else\n");
    let o = run(issue("required", &other), Some((RECEIPT, public())));
    assert!(sealed(&o).is_null(), "{}", o.state_json);

    // Claiming the issue's spec on that receipt fails the exact journal match instead.
    let mut forged = public();
    forged["spec_hash"] = other.clone().into();
    let (blobs, entry) = attachment(RECEIPT, forged);
    let mut ops = blobs;
    ops.push(commit(vec![entry]));
    let checked = attach(ops, &mut ProofCache::default());
    assert!(!checked.last().expect("commit").proofs[0].ok);
}

#[test]
fn optional_accepts_without_a_proof_and_none_ignores_proofs() {
    let o = run(issue("optional", &spec_hash()), None);
    assert_eq!(sealed(&o)["proven"], false, "{}", o.state_json);
    let none = run(issue("none", ""), None);
    assert!(sealed(&none).is_object() && sealed(&none).get("proven").is_none());
    // An issue without `proof` takes the template default (`default_proof()`: optional).
    let default = op(
        100,
        ISSUE,
        "xyz.arkiv.issue",
        "create",
        json!({"title": "t"}),
    );
    assert_eq!(sealed(&run(default, None))["proven"], false);
}

#[test]
fn folds_are_byte_identical_and_reuse_the_cache() {
    let (blobs, entry) = attachment(RECEIPT, public());
    let mut ops = blobs;
    ops.extend([issue("required", &spec_hash()), commit(vec![entry])]);
    let mut cache = ProofCache::default();
    let first = fold(ops.clone(), &mut cache);
    assert_eq!(
        first.state_json,
        fold(ops, &mut ProofCache::default()).state_json
    );

    // A re-fold from the saved cache needs no receipt bytes: the result is reused.
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("proofs.json");
    cache.save(&path).expect("save");
    let (_, entry) = attachment(RECEIPT, public());
    let without_blobs = vec![issue("required", &spec_hash()), commit(vec![entry])];
    let again = fold(without_blobs, &mut ProofCache::load(&path));
    assert_eq!(sealed(&again)["proven"], true);
}

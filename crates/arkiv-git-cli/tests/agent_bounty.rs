//! A scoped AI agent works a bounty end to end on MemArkiv (docs/p3/agents.md): alice
//! grants an agent key `patch` scope with a cap under `agent-bounty.rhai`; the agent polls
//! `op list --json`, claims the open bounty, pushes its commit and posts a patch carrying
//! the committed real proof with `op attach-proof`; alice approves and merges; the escrow
//! pays the agent.

mod common;

use std::path::Path;

use arkiv_git_cli::backend::MEM_CHAIN_ID;
use arkiv_git_store::fake::FileMemArkiv;
use arkiv_git_store::{Address, U256};
use arkiv_git_sync::ids::parse_id;
use common::e2e::*;
use common::programs::*;
use serde_json::{Value, json};

const TGLM: u128 = 1_000_000_000_000_000_000;
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../arkiv-git-zk/tests/fixtures");

fn first_word<'a>(out: &'a str, key: &str) -> &'a str {
    field(out, key).split_whitespace().next().unwrap_or("")
}

/// An appointed, funded escrow for `url`.
fn escrow(root: &Path, log: &Path, alice: &User, url: &str) -> User {
    let esc = User::new(root, "escrow", log);
    let out = esc.arkiv(&["escrow", "init"]);
    let (id, address) = (first_word(&out, "escrow:"), first_word(&out, "address:"));
    fund(log, first_word(&out, "wallet:"));
    let mem = FileMemArkiv::open(log, MEM_CHAIN_ID).expect("open");
    let a: Address = address.parse().expect("address");
    mem.fund(a, U256::from(TGLM / 10)).expect("gas float");
    let appoint = [
        "escrow",
        "appoint",
        "--repo",
        url,
        "--escrow",
        id,
        "--address",
        address,
    ];
    alice.arkiv(&appoint);
    esc
}

fn ops(u: &User, url: &str) -> Vec<Value> {
    let out = u.arkiv(&["op", "list", "--repo", url, "--json"]);
    serde_json::from_str::<Value>(&out)
        .expect("json")
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// One round of the agent loop: claim the first issue with a bounty that no patch names
/// yet. Returns `(issue, patch, branch)`, or `None` when nothing is open.
fn agent_round(ai: &User, url: &str, work: &Path) -> Option<(String, String, String)> {
    let all = ops(ai, url);
    let is = |o: &Value, ty: &str| o["type"] == ty && o["action"] == "create";
    let claimed: Vec<&Value> = all
        .iter()
        .filter(|o| is(o, "xyz.arkiv.patch"))
        .map(|o| &o["payload"]["issue"])
        .collect();
    let issue = all
        .iter()
        .filter(|o| is(o, "xyz.arkiv.issue") && o["payload"]["bounty"].as_u64() > Some(0))
        .map(|o| o["id"].clone())
        .find(|id| !claimed.contains(&id))?;
    let issue = issue.as_str().expect("id").to_owned();

    let branch = format!("agent/{}", &issue[..8]);
    ai.git(work, &["checkout", "-q", "-b", &branch]);
    let head = commit(ai, work, "fixed by the agent\n");
    ai.git(
        work,
        &["push", "-q", url, &format!("HEAD:refs/heads/{branch}")],
    );
    let payload = json!({"title": "agent fix", "head": head, "issue": issue}).to_string();
    let (receipt, public) = (
        format!("{FIXTURES}/demo.receipt"),
        format!("{FIXTURES}/demo.public.json"),
    );
    let attach = [
        "op",
        "attach-proof",
        "--repo",
        url,
        "--type",
        "xyz.arkiv.patch",
        "--action",
        "create",
        "--payload-json",
        &payload,
        "--receipt",
        &receipt,
        "--public",
        &public,
    ];
    let patch = ai.arkiv(&attach).trim().to_owned();
    Some((issue, patch, branch))
}

fn status(u: &User, url: &str) -> String {
    u.arkiv(&["program", "status", "--repo", url])
}

#[test]
fn granted_agent_claims_a_bounty_and_the_escrow_pays_it() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (root, log) = (tmp.path(), tmp.path().join("chain.log"));
    let alice = user(root, "alice", &log);
    let (url, work, _) = repo_with_main(&alice, root, "agent-bounty");
    let esc = escrow(root, &log, &alice, &url);
    alice.arkiv(&[
        "program",
        "pin",
        "--repo",
        &url,
        "--file",
        &program("agent-bounty"),
    ]);

    let ai = user(root, "ai", &log);
    let ai_nid = field(&ai.arkiv(&["keys", "show"]), "nid:").to_owned();
    let ai_hex = hex::encode(parse_id(&ai_nid).expect("nid"));
    let grants = post(&alice, &url, "xyz.arkiv.agent", "create", None, "{}");
    let cap = TGLM;
    let grant =
        json!({"agent": ai_hex, "scopes": ["patch"], "max_bounty": cap, "until": 1_000_000});
    post(
        &alice,
        &url,
        "xyz.arkiv.agent",
        "grant",
        Some(&grants),
        &grant.to_string(),
    );

    let bounty = TGLM / 2;
    let issue_body = json!({"title": "crash", "bounty": bounty}).to_string();
    let issue = post(&alice, &url, "xyz.arkiv.issue", "create", None, &issue_body);
    let (amount, purpose) = (TGLM.to_string(), format!("bounty:{issue}"));
    let deposit = [
        "escrow",
        "deposit",
        "--repo",
        &url,
        "--amount",
        &amount,
        "--purpose",
        &purpose,
    ];
    alice.arkiv(&deposit);

    let ai_work = root.join("ai-work");
    ai.git(root, &["clone", "-q", &url, &ai_work.to_string_lossy()]);
    let (claimed, patch, branch) = agent_round(&ai, &url, &ai_work).expect("an open bounty");
    assert_eq!(claimed, issue);
    assert!(
        agent_round(&ai, &url, &ai_work).is_none(),
        "claimed bounties stay claimed"
    );
    post(
        &ai,
        &url,
        "xyz.arkiv.comment",
        "create",
        None,
        r#"{"body":"lgtm"}"#,
    );

    let peer = format!("{url}/{ai_nid}");
    alice.git(
        &work,
        &["fetch", "-q", &peer, &format!("refs/heads/{branch}")],
    );
    alice.git(&work, &["merge", "-q", "--ff-only", "FETCH_HEAD"]);
    let refused = alice.run("git", &work, &["push", "-q", &url, "main"]);
    let err = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success() && err.contains("needs a human delegate approval"),
        "{err}"
    );

    let review_body = json!({"patch": patch}).to_string();
    let review = post(
        &alice,
        &url,
        "xyz.arkiv.review",
        "create",
        None,
        &review_body,
    );
    post(
        &alice,
        &url,
        "xyz.arkiv.review",
        "approve",
        Some(&review),
        &review_body,
    );
    let merged = json!({"patch": patch, "closes": issue}).to_string();
    post(
        &alice,
        &url,
        "xyz.arkiv.patch",
        "merged",
        Some(&patch),
        &merged,
    );
    alice.git(&work, &["push", "-q", &url, "main"]);

    let st = status(&alice, &url);
    assert!(
        st.contains(&format!(r#""to":"{ai_hex}""#)),
        "pay goes to the agent: {st}"
    );
    assert!(
        st.contains(r#""proven":true"#),
        "the attached receipt verified: {st}"
    );
    assert!(st.contains("scope comment not granted"), "{st}");

    let ran = esc.arkiv(&["escrow", "run", "--repo", &url, "--once"]);
    assert!(ran.contains(&format!("paid {purpose}")), "{ran}");
    let report = User::new(root, "auditor", &log).arkiv(&["escrow", "audit", "--repo", &url]);
    assert!(!report.contains("FLAG"), "{report}");
}

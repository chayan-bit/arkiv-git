//! Push funding (design 4.2 step 6): one up-front wallet check for the predicted spend plus
//! one window's reserve, then rolling top-ups; a top-up the wallet cannot cover stops the
//! push with `E_INSUFFICIENT_FUNDS` and the next push resumes with only what is missing.

mod common;

use arkiv_git_core::{CHUNK_SIZE, Kind};
use arkiv_git_store::{Arkiv, U256};
use arkiv_git_sync::write::chunk_op;
use common::*;

const PACK: usize = 2_000_000;

/// `alice` with a repo and one 2 MB noise commit not yet pushed.
async fn setup(
    net: &Net,
) -> (
    Peer,
    arkiv_git_core::Rid,
    tempfile::TempDir,
    std::path::PathBuf,
) {
    let alice = Peer::new(net, 1);
    let rid = init(net, &alice, "funding").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let work = work_repo(tmp.path(), "w");
    commit(&work, &[("big", &noise(3, PACK))], "feat: big");
    (alice, rid, tmp, work)
}

/// Wallet + control + bulk balance, the funds the up-front check may count.
async fn pooled(net: &Net, p: &Peer, rid: &arkiv_git_core::Rid) -> U256 {
    let mut sum = U256::ZERO;
    for a in [
        p.keys.wallet().address(),
        p.keys.control_address(rid),
        p.keys.bulk_address(rid),
    ] {
        sum += net.balance(a).await.expect("balance");
    }
    sum
}

/// Sets alice's wallet so the pooled balance is exactly `total`.
async fn pool_to(net: &Net, p: &Peer, rid: &arkiv_git_core::Rid, total: U256) {
    let wallet = p.keys.wallet().address();
    let others = pooled(net, p, rid).await - net.balance(wallet).await.expect("b");
    net.mem.set_balance(wallet, total - others);
}

async fn required(net: &Net, p: &Peer, rid: &arkiv_git_core::Rid, work: &std::path::Path) -> U256 {
    let repo = p.repo(rid);
    let push = prepare(net, p, &repo, work, &[upd("refs/heads/main")])
        .await
        .expect("prepare");
    push.cost().await.expect("cost").required()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_leaves_at_most_one_window_on_the_bulk_address() {
    let net = Net::new();
    let (alice, rid, _tmp, work) = setup(&net).await;
    push(&net, &alice, &rid, &work, &["refs/heads/main"])
        .await
        .expect("push");
    let bulk = alice.keys.bulk_address(&rid);
    let probe = chunk_op(
        &rid,
        &[0; 32],
        0,
        &vec![0xa5; CHUNK_SIZE],
        alice.cfg.data_lifetime,
    )
    .expect("op");
    let gas = net.estimate(bulk, &[probe]).await.expect("estimate");
    let base = net.base_fee().await.expect("fee");
    let window = alice.cfg.pipeline.window_reserve(gas, base);
    let left = net.balance(bulk).await.expect("balance");
    assert!(left <= window, "bulk keeps {left} > one window {window}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_fails_up_front_only_when_the_wallet_cannot_cover_spend_plus_a_window() {
    let net = Net::new();
    let (alice, rid, _tmp, work) = setup(&net).await;
    let need = required(&net, &alice, &rid, &work).await;
    // Estimates move a little with random salts, so test either side with a margin.
    pool_to(&net, &alice, &rid, need - need / U256::from(10)).await;
    let r = push(&net, &alice, &rid, &work, &["refs/heads/main"]).await;
    assert_eq!(code(r), "E_INSUFFICIENT_FUNDS");
    let bulk = alice.keys.bulk_address(&rid);
    assert!(created_by(&net, bulk).await.is_empty(), "nothing sent");

    pool_to(&net, &alice, &rid, need + need / U256::from(20)).await;
    push(&net, &alice, &rid, &work, &["refs/heads/main"])
        .await
        .expect("spend plus one window is enough");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_top_up_the_wallet_cannot_cover_stops_mid_push_and_the_retry_resumes() {
    let net = Net::new();
    let (alice, rid, tmp, work) = setup(&net).await;
    let need = required(&net, &alice, &rid, &work).await;
    pool_to(&net, &alice, &rid, need + need / U256::from(20)).await;
    let base = net.base_fee().await.expect("fee");
    let head = net.head().await.expect("head");
    net.hold_base_fee(head + 2, 2 * base);
    let r = push(&net, &alice, &rid, &work, &["refs/heads/main"]).await;
    assert_eq!(code(r), "E_INSUFFICIENT_FUNDS");
    let total = arkiv_git_core::chunk_count(PACK as u64) as usize;
    let landed = entities(&net, &rid, Kind::Chunk).await.len();
    assert!(
        landed > 0 && landed < total,
        "stopped mid-push after {landed} chunks"
    );
    assert!(alice.repo(&rid).pending_path().exists());

    // Time passes before the user retries: txs in flight at the failure land first.
    net.heal();
    net.mine(3);
    net.mem.set_base_fee(base);
    net.fund(alice.keys.wallet().address());
    push(&net, &alice, &rid, &work, &["refs/heads/main"])
        .await
        .expect("retry");
    let chunks = entities(&net, &rid, Kind::Chunk).await.len();
    // The pack is the 2 MB blob plus a few hundred bytes, still 31 chunks.
    assert_eq!(chunks, total, "every chunk landed once, none sent twice");
    let bulk = alice.keys.bulk_address(&rid);
    assert_eq!(
        created_by(&net, bulk).await.len(),
        total + 2,
        "plus one manifest, one refs"
    );
    assert_eq!(entities(&net, &rid, Kind::Anchor).await.len(), 1);
    let (_, l) = clone_into(&net, &Peer::new(&net, 5), &rid, &tmp.path().join("d"))
        .await
        .expect("clone");
    assert_eq!(l.refs[0].1, rev(&work, "main"));
}

/// A pack smaller than one chunk is priced at its real size (found live on Rabdi, where a
/// 300 B push asked for 0.027 tGLM and spent about 0.001). Its reserve is still four txs
/// (chunk, manifest, refs, anchor) at the size of the largest signed envelope, which is
/// under two full 64 KiB chunks' max fee; probing a full chunk made it over four.
#[tokio::test(flavor = "multi_thread")]
async fn a_tiny_push_is_priced_at_its_real_chunk_size() {
    let net = Net::new();
    let alice = Peer::new(&net, 1);
    let rid = init(&net, &alice, "tiny").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let work = work_repo(tmp.path(), "w");
    commit(&work, &[("README", b"tiny\n".as_slice())], "feat: tiny");
    let need = required(&net, &alice, &rid, &work).await;
    let probe = chunk_op(
        &rid,
        &[0; 32],
        0,
        &vec![0xa5; CHUNK_SIZE],
        alice.cfg.data_lifetime,
    )
    .expect("op");
    let full = net
        .estimate(alice.keys.bulk_address(&rid), &[probe])
        .await
        .expect("estimate");
    let base = net.base_fee().await.expect("fee");
    let two_full_chunks = alice.cfg.pipeline.reserve(2, full, base);
    assert!(
        need < two_full_chunks,
        "a tiny push needs {need}, more than two full chunks' max fee {two_full_chunks}"
    );
    pool_to(&net, &alice, &rid, need + need / U256::from(20)).await;
    push(&net, &alice, &rid, &work, &["refs/heads/main"])
        .await
        .expect("the real price is enough");
}

//! Paging rules (design 4.8) against `MemArkiv` (design 8.3 "Paging").
#![cfg(feature = "fake")]

mod common;

use arkiv_git_store::fake::MemArkiv;
use arkiv_git_store::paging::{keyset, offset_all};
use arkiv_git_store::{AttributeValue as V, Attrs, Pred, Query, RowExt, Select, attr, create_op};
use common::{CHAIN, DAY, exec, funded, small};

fn kind(k: &str) -> Pred {
    Pred::eq(attr("ag.kind"), k)
}

/// `n` entities of `kind`, the i-th with `ag.seq = seq(i)`, 100 per tx.
async fn populate(fake: &MemArkiv, kind: &str, n: u64, seq: impl Fn(u64) -> u64) {
    let key = funded(fake);
    let ops: Vec<_> = (0..n).map(|i| small(kind, seq(i), i.into())).collect();
    for chunk in ops.chunks(100) {
        exec(fake, &key, chunk.to_vec()).await;
    }
}

fn seqs(rows: &[arkiv_git_store::EntityData]) -> Vec<u64> {
    rows.iter()
        .map(|r| r.attr_u64("ag.seq").expect("seq"))
        .collect()
}

#[tokio::test]
async fn keyset_pages_past_100_anchors_in_seq_order() {
    let fake = MemArkiv::new(CHAIN);
    populate(&fake, "anchor", 250, |i| 249 - i).await;
    populate(&fake, "other", 5, |i| i).await;
    let rows = keyset(&fake, &kind("anchor"), "ag.seq", 0, Select::default())
        .await
        .expect("paged");
    assert_eq!(seqs(&rows), (0..250).collect::<Vec<_>>());
    let tail = keyset(
        &fake,
        &kind("anchor"),
        "ag.seq",
        240,
        Select {
            attrs: Attrs::All,
            ..Select::default()
        },
    )
    .await
    .expect("tail");
    assert_eq!(
        seqs(&tail),
        (240..250).collect::<Vec<_>>(),
        "incremental fetch from a high-water mark"
    );
}

#[tokio::test]
async fn keyset_dedups_values_that_straddle_pages() {
    let fake = MemArkiv::new(CHAIN);
    populate(&fake, "bind", 199, |i| i / 2).await;
    let rows = keyset(&fake, &kind("bind"), "ag.seq", 0, Select::default())
        .await
        .expect("paged");
    assert_eq!(rows.len(), 199);
    let mut keys: Vec<_> = rows.iter().map(|r| r.key.expect("key")).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), 199);
}

#[tokio::test]
async fn more_than_a_page_sharing_one_seq_is_e_paging() {
    let fake = MemArkiv::new(CHAIN);
    populate(&fake, "anchor", 101, |_| 7).await;
    let err = keyset(&fake, &kind("anchor"), "ag.seq", 0, Select::default())
        .await
        .expect_err("stuck");
    assert_eq!(err.code(), "E_PAGING");
}

#[tokio::test]
async fn keyset_halves_the_page_on_413() {
    let fake = MemArkiv::new(CHAIN);
    let key = funded(&fake);
    let ops: Vec<_> = (0..20u64)
        .map(|i| {
            let attrs = [
                ("ag.kind", V::Str("chunk".into()), true),
                ("ag.seq", V::U64(i), true),
                ("ag.body", V::Bytes(vec![1; 4096]), false),
            ];
            create_op(&attrs, DAY, i.into()).expect("op")
        })
        .collect();
    exec(&fake, &key, ops).await;
    fake.fail_pages_above(30_000);
    let body = Select {
        attrs: Attrs::Named(vec!["ag.body".into()]),
        ..Select::default()
    };
    let rows = keyset(&fake, &kind("chunk"), "ag.seq", 0, body)
        .await
        .expect("shrunk pages");
    assert_eq!(seqs(&rows), (0..20).collect::<Vec<_>>());
    assert!(
        rows.iter()
            .all(|r| r.attr_bytes("ag.body").map(|b| b.len()) == Some(4096))
    );
    fake.fail_pages_above(100);
    let err = keyset(&fake, &kind("chunk"), "ag.seq", 0, Select::default())
        .await
        .expect_err("even 1 row is too big");
    assert_eq!(err.code(), "E_RPC");
}

#[tokio::test]
async fn offset_paging_restarts_on_count_change_then_gives_up() {
    let fake = MemArkiv::new(CHAIN);
    populate(&fake, "id", 230, |i| i).await;
    let q = Query::from(kind("id"));
    let rows = offset_all(&fake, &q, Select::default())
        .await
        .expect("paged");
    assert_eq!(rows.len(), 230);
    let with_seq = Select {
        attrs: Attrs::Named(vec!["ag.seq".into()]),
        ..Select::default()
    };
    let rows = offset_all(&fake, &q, with_seq.clone())
        .await
        .expect("paged");
    // Creation order: by tx (100 creates each), then by entity key within a tx.
    let got = seqs(&rows);
    for (i, tx) in got.chunks(100).enumerate() {
        let mut tx = tx.to_vec();
        tx.sort();
        let lo = 100 * i as u64;
        assert_eq!(tx, (lo..lo + tx.len() as u64).collect::<Vec<_>>());
    }
    fake.lie_counts(2);
    assert_eq!(
        offset_all(&fake, &q, with_seq.clone())
            .await
            .expect("after a restart")
            .len(),
        230
    );
    fake.lie_counts(1_000);
    let err = offset_all(&fake, &q, with_seq)
        .await
        .expect_err("never settles");
    assert_eq!(err.code(), "E_INCONSISTENT_VIEW");
}

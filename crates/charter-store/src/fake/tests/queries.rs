//! Query language, paging and ordering, GolemDB quirks.

use super::*;
use crate::OrderBy;

#[tokio::test]
async fn query_operators_paging_and_order() {
    let (fake, key) = setup();
    let seqs = [2u64, 1, 2, 1, 3];
    let ops: Vec<Operation> = seqs
        .iter()
        .enumerate()
        .map(|(i, s)| blob(&format!("k{i}"), *s, i as u128))
        .collect();
    for op in ops {
        exec(&fake, &key, vec![op]).await;
    }
    let k: Vec<B256> = (0..5)
        .map(|n| derive_key(CHAIN, key.address(), n, u128::from(n as u8)))
        .collect();
    let q = |p: Pred| keys(&fake, p, QueryOpts::default());
    assert_eq!(q(Pred::eq(attr("ag.i"), 1u64)).await, vec![k[1], k[3]]);
    assert_eq!(
        q(Pred::And(vec![
            Pred::ge(attr("ag.i"), 2u64),
            Pred::le(attr("ag.i"), 2u64)
        ]))
        .await,
        vec![k[0], k[2]]
    );
    assert_eq!(
        q(Pred::StartsWith(attr("ag.kind"), "k4".into())).await,
        vec![k[4]]
    );
    assert_eq!(
        q(Pred::Or(vec![
            Pred::eq(attr("ag.kind"), "k0"),
            Pred::eq(Field::Key, Value::Key(k[3]))
        ]))
        .await,
        vec![k[0], k[3]]
    );
    assert_eq!(q(Pred::eq(Field::Owner, key.address())).await.len(), 5);
    assert_eq!(q(Pred::ge(Field::ExpiresAt, 1u64)).await.len(), 5);
    let paged = QueryOpts {
        offset: 3,
        limit: 1,
        ..QueryOpts::default()
    };
    assert_eq!(keys(&fake, Pred::All, paged).await, vec![k[3]]);
    let past = QueryOpts {
        offset: 9,
        ..QueryOpts::default()
    };
    assert!(keys(&fake, Pred::All, past).await.is_empty());
    let asc = Query::from(Pred::All).order_by_u64("ag.i");
    assert_eq!(
        keys(&fake, asc.clone(), QueryOpts::default()).await,
        vec![k[1], k[3], k[0], k[2], k[4]],
        "ties in creation order"
    );
    let desc = Query {
        order: asc.order.map(|o| OrderBy { desc: true, ..o }),
        ..asc
    };
    assert_eq!(
        keys(&fake, desc, QueryOpts::default()).await,
        vec![k[4], k[0], k[2], k[1], k[3]]
    );
    let bad = QueryOpts {
        limit: 101,
        ..QueryOpts::default()
    };
    assert!(fake.query(&Pred::All.into(), &bad).await.is_err());
}

#[test]
fn not_and_bang_eq_and_at_block_are_rejected_as_on_golemdb() {
    let opts = json!({ "limit": "0x64" });
    for q in [
        "NOT (a = u64(1))",
        "a != u64(1)",
        "a = u64(1) AND NOT (b = u64(2))",
    ] {
        assert!(eval::run(&[], q, &opts, 1).is_err(), "{q}");
    }
    assert!(eval::run(&[], "*", &json!({ "limit": "0x65" }), 1).is_err());
    assert!(eval::run(&[], "*", &json!({ "atBlock": "0x1" }), 1).is_err());
    assert!(
        eval::run(&[], "*", &json!({ "bogus": 1 }), 1).is_err(),
        "unknown options are refused"
    );
    assert!(eval::run(&[], "*", &json!({}), 1).is_ok());
}

#[tokio::test]
async fn key_and_bytes32_are_one_type_as_on_golemdb() {
    let (fake, key) = setup();
    let h = B256::repeat_byte(0x5a);
    exec(
        &fake,
        &key,
        vec![create_op(&[("ag.h", V::Bytes32(h.0), true)], LIFE, 1).expect("op")],
    )
    .await;
    let k = derive_key(CHAIN, key.address(), 0, 1);
    assert_eq!(
        keys(
            &fake,
            Pred::eq(attr("ag.h"), Value::Key(h)),
            QueryOpts::default()
        )
        .await,
        vec![k]
    );
    let typed = Query::from(Pred::eq(Field::Key, Value::Bytes32(k)));
    assert!(
        typed.render().is_err(),
        "the parser types system attributes, so the builder refuses this"
    );
}

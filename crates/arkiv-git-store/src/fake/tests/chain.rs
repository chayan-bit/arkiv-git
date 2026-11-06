//! Clock, nonces, fees, base fee and fault hooks.

use super::*;

#[tokio::test]
async fn the_clock_moves_only_when_mined() {
    let (fake, key) = setup();
    let h0 = fake.head().await.expect("head");
    let tx = sign(
        &fake,
        &key,
        None,
        ARKIV_ADDRESS,
        execute_calldata(&[blob("c", 0, 1)]),
    )
    .await;
    let hash = fake.send(tx).await.expect("send");
    assert_eq!(fake.head().await.expect("head"), h0);
    assert_eq!(fake.receipt(hash).await.expect("r"), None);
    assert_eq!(
        fake.tx_count(key.address()).await.expect("n"),
        1,
        "pending counts toward the next nonce"
    );
    assert_eq!(fake.mine(), h0 + 1);
    assert_eq!(
        fake.receipt(hash).await.expect("r").map(|r| r.block_number),
        Some(h0 + 1)
    );
}

#[tokio::test]
async fn nonces_balances_fees_and_transfers() {
    let (fake, key) = setup();
    let tx = sign(
        &fake,
        &key,
        None,
        ARKIV_ADDRESS,
        execute_calldata(&[blob("n", 0, 1)]),
    )
    .await;
    assert_eq!(
        fake.send(tx.clone()).await.expect("send"),
        fake.send(tx.clone()).await.expect("already known")
    );
    let rival = sign(
        &fake,
        &key,
        Some(0),
        ARKIV_ADDRESS,
        execute_calldata(&[blob("n", 1, 2)]),
    )
    .await;
    assert!(
        matches!(fake.send(rival.clone()).await, Err(StoreError::Rpc(m)) if m.contains("underpriced"))
    );
    let before = fake.balance(key.address()).await.expect("bal");
    fake.mine();
    let r = fake.receipt(tx.hash()).await.expect("r").expect("mined");
    let after = fake.balance(key.address()).await.expect("bal");
    assert_eq!(
        before - after,
        U256::from(r.gas_used) * U256::from(r.effective_gas_price)
    );
    assert_eq!(
        fake.send(rival).await.map_err(|e| e.code()),
        Err("E_RPC"),
        "nonce too low"
    );
    let poor = PrivateKeySigner::random();
    let t = sign(&fake, &poor, None, ARKIV_ADDRESS, Bytes::new()).await;
    assert_eq!(
        fake.send(t).await.map_err(|e| e.code()),
        Err("E_INSUFFICIENT_FUNDS")
    );
    let pay = TxParams {
        nonce: 1,
        to: poor.address(),
        value: U256::from(5),
        input: Bytes::new(),
        gas_limit: 21_000,
        max_fee_per_gas: 2 * MIN_BASE_FEE,
        max_priority_fee_per_gas: 0,
    };
    fake.send(SignedTx::sign(&key, CHAIN, pay).expect("sign"))
        .await
        .expect("send");
    fake.mine();
    assert_eq!(
        fake.balance(poor.address()).await.expect("bal"),
        U256::from(5)
    );
    let wrong = SignedTx::sign(
        &key,
        CHAIN + 1,
        TxParams {
            nonce: 2,
            to: poor.address(),
            value: U256::ZERO,
            input: Bytes::new(),
            gas_limit: 21_000,
            max_fee_per_gas: 2 * MIN_BASE_FEE,
            max_priority_fee_per_gas: 0,
        },
    )
    .expect("sign");
    assert!(
        fake.send(wrong).await.is_err(),
        "other chain ids are refused"
    );
}

#[tokio::test]
async fn base_fee_follows_eip1559_with_a_floor() {
    let (fake, key) = setup();
    for i in 0..4u64 {
        let op = create_op(
            &[("b", V::Bytes(vec![9; 120_000]), false)],
            43_200,
            u128::from(i),
        )
        .expect("op");
        fake.send(tx(
            &key,
            i,
            ARKIV_ADDRESS,
            execute_calldata(&[op]),
            7_000_000,
        ))
        .await
        .expect("send");
    }
    let n = fake.mine();
    assert!(
        fake.block_gas_of(n, key.address()) > 15_000_000,
        "over half full"
    );
    let risen = fake.base_fee().await.expect("fee");
    assert!(risen > MIN_BASE_FEE);
    fake.mine();
    assert!(
        fake.base_fee().await.expect("fee") < risen,
        "falls when blocks are light"
    );
    for _ in 0..40 {
        fake.mine();
    }
    assert_eq!(
        fake.base_fee().await.expect("fee"),
        MIN_BASE_FEE,
        "never below the floor"
    );
    fake.set_base_fee(5 * MIN_BASE_FEE);
    assert_eq!(fake.base_fee().await.expect("fee"), 5 * MIN_BASE_FEE);
}

#[tokio::test]
async fn fault_drop_sends() {
    let (fake, key) = setup();
    fake.drop_sends(1, 1);
    let t = |n: u64| {
        tx(
            &key,
            n,
            ARKIV_ADDRESS,
            execute_calldata(&[blob("d", n, n.into())]),
            200_000,
        )
    };
    let (t0, t1, t2) = (t(0), t(1), t(2));
    for t in [&t0, &t1, &t2] {
        fake.send(t.clone()).await.expect("accepted");
    }
    fake.mine();
    assert!(fake.receipt(t0.hash()).await.expect("r").is_some());
    assert_eq!(fake.receipt(t1.hash()).await.expect("r"), None, "dropped");
    assert_eq!(
        fake.receipt(t2.hash()).await.expect("r"),
        None,
        "stuck behind the gap"
    );
    fake.send(t1.clone()).await.expect("resend");
    fake.mine();
    assert!(fake.receipt(t2.hash()).await.expect("r").is_some());
    fake.drop_next(1);
    let t3 = sign(
        &fake,
        &key,
        None,
        ARKIV_ADDRESS,
        execute_calldata(&[blob("d", 3, 4)]),
    )
    .await;
    fake.send(t3).await.expect("accepted");
    assert_eq!(fake.pending(), 0);
}

#[tokio::test]
async fn fault_revert_at_op() {
    let (fake, key) = setup();
    fake.revert_next_at(1);
    let r = exec(&fake, &key, vec![blob("v", 0, 1), blob("v", 1, 2)]).await;
    assert!(!r.success && r.gas_used > 0);
    assert_eq!(
        fake.entity_nonce(key.address()).await.expect("n"),
        0,
        "rolled back whole"
    );
    assert_eq!(
        fake.tx_count(key.address()).await.expect("n"),
        1,
        "nonce still consumed"
    );
    assert!(
        exec(&fake, &key, vec![blob("v", 0, 1)]).await.success,
        "one-shot"
    );
}

#[tokio::test]
async fn faults_withhold_lie_and_page_caps() {
    let (fake, key) = setup();
    let body = create_op(
        &[
            ("ag.kind", V::Str("w".into()), true),
            ("ag.body", V::Bytes(vec![1; 4096]), false),
        ],
        LIFE,
        1,
    )
    .expect("op");
    exec(&fake, &key, vec![body, blob("w2", 0, 2)]).await;
    let a = key.address();
    let k0 = derive_key(CHAIN, a, 0, 1);
    fake.withhold(k0);
    assert!(
        !keys(&fake, Pred::All, QueryOpts::default())
            .await
            .contains(&k0)
    );
    assert_eq!(
        fake.query(&Pred::All.into(), &QueryOpts::count_only())
            .await
            .expect("q")
            .count,
        Some(1)
    );
    assert_eq!(
        fake.entity_nonce(a).await.expect("n"),
        2,
        "the truth still counts it"
    );
    fake.lie_entity_nonce(a, 9);
    assert_eq!(fake.entity_nonce(a).await.expect("n"), 9);
    fake.fail_pages_above(2_000);
    let body_opts = QueryOpts {
        select: Select {
            attrs: Attrs::All,
            ..Select::default()
        },
        ..QueryOpts::default()
    };
    let k1 = derive_key(CHAIN, a, 1, 2);
    assert_eq!(
        keys(&fake, Pred::All, QueryOpts::default()).await,
        vec![k1],
        "small pages pass"
    );
    let fake2 = MemArkiv::new(CHAIN);
    fake2.fund(a, U256::from(10u128.pow(20)));
    exec(
        &fake2,
        &key,
        vec![create_op(&[("ag.body", V::Bytes(vec![1; 4096]), false)], LIFE, 1).expect("op")],
    )
    .await;
    fake2.fail_pages_above(2_000);
    assert!(matches!(
        fake2.query(&Pred::All.into(), &body_opts).await,
        Err(StoreError::Http { status: 413, .. })
    ));
    fake.lie_counts(2);
    let mut seen = Vec::new();
    for _ in 0..3 {
        seen.push(
            fake.query(&Pred::All.into(), &QueryOpts::count_only())
                .await
                .expect("q")
                .count,
        );
    }
    assert_eq!(seen, vec![Some(3), Some(2), Some(1)]);
    let row = fake
        .query(&Pred::All.into(), &QueryOpts::with_attrs(&["ag.i"]))
        .await
        .expect("q")
        .rows
        .remove(0);
    assert_eq!(row.attr_u64("ag.i"), Some(0));
}

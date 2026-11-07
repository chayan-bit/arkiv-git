//! Entity rules: creation order, attributes, caps, expiry, extension, ownership.

use super::*;

#[tokio::test]
async fn creation_order_keys_and_entity_nonce() {
    let (fake, key) = setup();
    let a = key.address();
    exec(&fake, &key, vec![blob("x", 1, 10), blob("x", 2, 11)]).await;
    exec(&fake, &key, vec![blob("x", 3, 12)]).await;
    assert_eq!(fake.entity_nonce(a).await.expect("nonce"), 3);
    let expected: Vec<B256> = (0..3)
        .map(|n| derive_key(CHAIN, a, n, 10 + u128::from(n)))
        .collect();
    let got = keys(&fake, Pred::All, QueryOpts::default()).await;
    let mut first_tx = expected[..2].to_vec();
    first_tx.sort();
    assert_eq!(
        got,
        [first_tx, vec![expected[2]]].concat(),
        "tx order, then key order within a tx"
    );
}

#[tokio::test]
async fn attribute_rules_match_the_node() {
    let (fake, key) = setup();
    let a = key.address();
    let named = |n: usize| -> Vec<(String, V, bool)> {
        (0..n)
            .map(|i| (format!("a{i:02}"), V::U64(1), true))
            .collect()
    };
    let op = |v: Vec<(String, V, bool)>| {
        let refs: Vec<(&str, V, bool)> = v
            .iter()
            .map(|(n, x, i)| (n.as_str(), x.clone(), *i))
            .collect();
        create_op(&refs, LIFE, 1).expect("op")
    };
    assert!(fake.estimate(a, &[op(named(32))]).await.is_ok());
    assert_eq!(
        revert_reason(&fake, a, &[op(named(33))]).await,
        "TooManyAttributes"
    );
    let cases = [
        (vec![wire("1bad", 0x83, vec![0; 32])], "Ident32"),
        (vec![wire("s", 0x88, vec![b'x'; 129])], "InvalidValueType"),
        (vec![wire("b", 0x87, vec![1])], "InvalidValueType"),
        (vec![wire("b", 0x07, vec![1; 131_073])], "InvalidValueType"),
        (
            vec![wire("b", 0x83, vec![0; 32]), wire("a", 0x83, vec![0; 32])],
            "AttributesNotSorted",
        ),
        (
            vec![wire("a", 0x83, vec![0; 32]), wire("a", 0x83, vec![0; 32])],
            "AttributesNotSorted",
        ),
        (vec![wire("t", 0x00, vec![])], "TombstoneInCreate"),
    ];
    for (attrs, want) in cases {
        let got = revert_reason(&fake, a, &[raw_create(attrs, 3, LIFE, 0)]).await;
        assert!(got.contains(want), "{got} vs {want}");
    }
    let ok = [
        wire("n.a-m_e", 0x88, vec![b'x'; 128]),
        wire("z", 0x07, vec![1; 1024]),
    ];
    assert!(
        fake.estimate(a, &[raw_create(ok.to_vec(), 3, LIFE, 0)])
            .await
            .is_ok(),
        "128 B str, 32 B name grammar"
    );
    assert_eq!(
        revert_reason(&fake, a, &[raw_create(vec![], 4, LIFE, 0)]).await,
        "ReservedCreationFlags"
    );
}

#[tokio::test]
async fn tx_caps_and_the_per_sender_window() {
    let (fake, key) = setup();
    let big = create_op(&[("b", V::Bytes(vec![7; 131_000]), false)], 10, 1).expect("op");
    let t = sign(&fake, &key, None, ARKIV_ADDRESS, execute_calldata(&[big])).await;
    assert_eq!(
        fake.send(t).await.map_err(|e| e.code()),
        Err("E_TX_TOO_LARGE")
    );
    let p = TxParams {
        nonce: 0,
        to: ARKIV_ADDRESS,
        value: U256::ZERO,
        input: Bytes::new(),
        gas_limit: TX_GAS_CAP + 1,
        max_fee_per_gas: 2 * MIN_BASE_FEE,
        max_priority_fee_per_gas: 0,
    };
    let over = SignedTx::sign(&key, CHAIN, p).expect("sign");
    assert_eq!(
        fake.send(over).await.map_err(|e| e.code()),
        Err("E_GAS_CAP")
    );
    let huge = create_op(
        &[("b", V::Bytes(vec![7; 126 * 1024]), false)],
        MAX_LIFETIME,
        1,
    )
    .expect("op");
    assert_eq!(
        fake.estimate(key.address(), &[huge])
            .await
            .map_err(|e| e.code()),
        Err("E_GAS_CAP"),
        "measured 19.7M"
    );

    for i in 0..16 {
        let t = tx(
            &key,
            i,
            ARKIV_ADDRESS,
            execute_calldata(&[blob("w", i, 1)]),
            200_000,
        );
        fake.send(t).await.expect("within window");
    }
    let t = tx(
        &key,
        16,
        ARKIV_ADDRESS,
        execute_calldata(&[blob("w", 16, 1)]),
        200_000,
    );
    assert!(matches!(fake.send(t).await, Err(StoreError::Rpc(m)) if m.contains("txpool is full")));
    let n = fake.mine();
    assert_eq!(fake.pending(), 0, "all 16 fit in one block");
    assert!(fake.block_gas_of(n, key.address()) > 0);
}

#[tokio::test]
async fn a_block_holds_at_most_30m_gas() {
    let fake = MemArkiv::new(CHAIN);
    let senders: Vec<PrivateKeySigner> = (0..3).map(|_| PrivateKeySigner::random()).collect();
    for k in &senders {
        fake.fund(k.address(), U256::from(10u128.pow(20)));
        for i in 0..5 {
            let tx = sign(
                &fake,
                k,
                Some(i),
                ARKIV_ADDRESS,
                execute_calldata(&[blob("g", i, 1)]),
            )
            .await;
            fake.send(tx).await.expect("send");
        }
    }
    fake.mine();
    assert_eq!(fake.pending(), 15 - 1, "16M gas limits: one tx per block");
}

#[tokio::test]
async fn expiry_by_block() {
    let (fake, key) = setup();
    let op = create_op(&[("ag.kind", V::Str("e".into()), true)], 3, 1).expect("op");
    let r = exec(&fake, &key, vec![op]).await;
    let count = || async {
        fake.query(&Pred::All.into(), &QueryOpts::count_only())
            .await
            .expect("q")
            .count
    };
    for _ in 0..2 {
        assert_eq!(count().await, Some(1));
        fake.mine();
    }
    assert_eq!(fake.head().await.expect("head"), r.block_number + 2);
    assert_eq!(count().await, Some(1), "live while head < expiresAt");
    fake.mine();
    assert_eq!(count().await, Some(0), "gone at head = expiresAt");
    let k = derive_key(CHAIN, key.address(), 0, 1);
    assert!(
        revert_reason(&fake, key.address(), &[extend_op(k, 10)])
            .await
            .contains("EntityNotFound")
    );
}

#[tokio::test]
async fn extension_rules() {
    let (fake, owner) = setup();
    let other = PrivateKeySigner::random();
    fake.fund(other.address(), U256::from(10u128.pow(20)));
    let flagged = blob("f", 0, 1);
    let plain = raw_create(vec![wire("p", 0x83, vec![0; 32])], 0, LIFE, 0);
    exec(&fake, &owner, vec![flagged, plain]).await;
    let (kf, kp) = (
        derive_key(CHAIN, owner.address(), 0, 1),
        derive_key(CHAIN, owner.address(), 1, 1),
    );
    let o = other.address();
    assert!(
        exec(&fake, &other, vec![extend_op(kf, LIFE * 2)])
            .await
            .success,
        "anyone may extend a flagged entity"
    );
    assert!(
        revert_reason(&fake, o, &[extend_op(kp, LIFE * 2)])
            .await
            .contains("NotOwner")
    );
    assert!(
        exec(&fake, &owner, vec![extend_op(kp, LIFE * 2)])
            .await
            .success,
        "owner may extend"
    );
    assert!(
        revert_reason(&fake, o, &[extend_op(kf, 5)])
            .await
            .contains("ExpiryNotExtended"),
        "never shorter"
    );
    let exp = fake
        .query(
            &Pred::eq(Field::Key, Value::Key(kf)).into(),
            &QueryOpts {
                select: Select {
                    expires_at: true,
                    ..Select::default()
                },
                ..QueryOpts::default()
            },
        )
        .await
        .expect("q")
        .rows[0]
        .expires_at
        .expect("exp");
    assert!(
        fake.estimate(o, &[Operation::extend_expiry(kf, exp, 0)])
            .await
            .is_ok(),
        "equal is a no-op"
    );
    assert!(
        revert_reason(&fake, o, &[extend_op(kf, MAX_LIFETIME + 1)])
            .await
            .contains("ExpiryBeyondMaxLifetime")
    );
    let a = owner.address();
    assert!(
        revert_reason(&fake, a, &[raw_create(vec![], 3, MAX_LIFETIME + 1, 0)])
            .await
            .contains("ExpiryBeyondMaxLifetime")
    );
    assert!(
        revert_reason(&fake, a, &[raw_create(vec![], 3, 0, 1)])
            .await
            .contains("ExpiryDeadOnArrival")
    );
}

#[tokio::test]
async fn readonly_owner_delete_and_transfer() {
    let (fake, owner) = setup();
    let other = PrivateKeySigner::random();
    exec(
        &fake,
        &owner,
        vec![
            blob("r", 0, 1),
            raw_create(vec![wire("p", 0x83, vec![0; 32])], 0, LIFE, 0),
        ],
    )
    .await;
    let a = owner.address();
    let (kr, kp) = (derive_key(CHAIN, a, 0, 1), derive_key(CHAIN, a, 1, 1));
    let set = vec![wire("q", 0x83, vec![0; 32])];
    let patch = |k| Operation::patch(k, set.clone());
    assert!(
        revert_reason(&fake, a, &[patch(kr)])
            .await
            .contains("ReadOnlyEntity")
    );
    assert!(
        exec(&fake, &owner, vec![patch(kp)]).await.success,
        "a plain entity can be patched"
    );
    assert!(
        revert_reason(&fake, other.address(), &[Operation::delete(kr)])
            .await
            .contains("NotOwner")
    );
    assert!(
        exec(
            &fake,
            &owner,
            vec![Operation::transfer_ownership(kp, other.address())]
        )
        .await
        .success
    );
    let row = fake
        .query(
            &Pred::eq(Field::Key, Value::Key(kp)).into(),
            &QueryOpts {
                select: Select {
                    owner: true,
                    creator: true,
                    ..Select::default()
                },
                ..QueryOpts::default()
            },
        )
        .await
        .expect("q")
        .rows
        .remove(0);
    assert_eq!(
        (row.owner, row.creator),
        (Some(other.address()), Some(a)),
        "creator is immutable"
    );
    assert!(
        exec(&fake, &owner, vec![Operation::delete(kr)])
            .await
            .success
    );
    assert_eq!(
        keys(&fake, Pred::eq(Field::Creator, a), QueryOpts::default()).await,
        vec![kp]
    );
    assert_eq!(
        fake.entity_nonce(a).await.expect("n"),
        2,
        "entityNonce counts every entity ever created"
    );
}

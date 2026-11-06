//! The audit flags each violation type, from public data only.

mod common;

use arkiv_git_escrow::ops::{self, PAID, SettlePayload};
use arkiv_git_escrow::{FixedObligations, Flag};
use arkiv_git_store::{Address, TxHash, U256};
use common::*;

#[test]
fn a_funded_obligation_left_unpaid_is_a_missing_payout() {
    big(async {
        let w = World::new().await;
        w.deposit(TGLM, "bounty:1", None).await;
        let src = FixedObligations(vec![pay(
            "b1",
            Address::repeat_byte(0xb0),
            TGLM,
            "bounty:1",
        )]);
        let flags = w.audit(&src, 0).await.flags;
        assert!(
            has(
                &flags,
                |f| matches!(f, Flag::MissingPayout { key, .. } if key == "b1")
            ),
            "{flags:?}"
        );
        assert!(
            w.audit(&src, 1_000).await.flags.is_empty(),
            "within the grace period"
        );
        let unfunded = FixedObligations(vec![pay(
            "b2",
            Address::repeat_byte(0xb0),
            2 * TGLM,
            "bounty:1",
        )]);
        assert!(
            w.audit(&unfunded, 0).await.flags.is_empty(),
            "an unfunded obligation is not due"
        );
    });
}

#[test]
fn a_silent_transfer_is_unauthorized_and_a_shortfall() {
    big(async {
        let w = World::new().await;
        w.deposit(TGLM, "bounty:1", None).await;
        let thief = Address::repeat_byte(0x66);
        w.transfer(&w.payout, thief, TGLM).await;
        let flags = w.audit(&FixedObligations::default(), 0).await.flags;
        assert!(
            has(
                &flags,
                |f| matches!(f, Flag::UnauthorizedPayout { detail } if detail.contains("no settlement op"))
            ),
            "{flags:?}"
        );
        assert!(
            has(&flags, |f| matches!(f, Flag::Shortfall { .. })),
            "{flags:?}"
        );
    });
}

#[test]
fn a_payout_with_no_obligation_is_unauthorized() {
    big(async {
        let w = World::new().await;
        let bob = Address::repeat_byte(0xb0);
        w.deposit(TGLM, "bounty:1", None).await;
        let src = FixedObligations(vec![pay("b1", bob, TGLM / 2, "bounty:1")]);
        w.agent(&src).run_once().await.expect("paid");
        let flags = w.audit(&FixedObligations::default(), 0).await.flags;
        assert!(
            has(
                &flags,
                |f| matches!(f, Flag::UnauthorizedPayout { detail } if detail.contains("no pay obligation"))
            ),
            "{flags:?}"
        );
        let other = FixedObligations(vec![pay(
            "b1",
            Address::repeat_byte(1),
            TGLM / 2,
            "bounty:1",
        )]);
        let flags = w.audit(&other, 0).await.flags;
        assert!(
            has(
                &flags,
                |f| matches!(f, Flag::UnauthorizedPayout { detail } if detail.contains("owed to"))
            ),
            "{flags:?}"
        );
    });
}

#[test]
fn paying_a_key_twice_is_a_double_pay_and_a_fake_tx_a_bad_receipt() {
    big(async {
        let w = World::new().await;
        let bob = Address::repeat_byte(0xb0);
        w.deposit(TGLM, "bounty:1", None).await;
        let src = FixedObligations(vec![pay("b1", bob, TGLM / 4, "bounty:1")]);
        w.agent(&src).run_once().await.expect("paid");
        let tx = w.transfer(&w.payout, bob, TGLM / 4).await;
        let settle = |tx| SettlePayload {
            key: "b1".into(),
            tx,
            to: bob,
            amount: (TGLM / 4).to_string(),
            deposit: None,
        };
        let (cfg, id) = (&w.agent.cfg, &w.escrow.id);
        ops::post(&w.net, cfg, id, &w.rid, w.object, PAID, &settle(tx))
            .await
            .expect("op");
        let flags = w.audit(&src, 0).await.flags;
        assert!(
            has(
                &flags,
                |f| matches!(f, Flag::DoublePay { key, ops } if key == "b1" && ops.len() == 2)
            ),
            "{flags:?}"
        );
        assert_eq!(w.balance(bob).await, U256::from(TGLM / 2));

        let fake = settle(TxHash::repeat_byte(0x42));
        ops::post(&w.net, cfg, id, &w.rid, w.object, PAID, &fake)
            .await
            .expect("op");
        let flags = w.audit(&src, 0).await.flags;
        assert!(
            has(
                &flags,
                |f| matches!(f, Flag::BadReceipt { reason, .. } if reason.contains("not mined"))
            ),
            "{flags:?}"
        );
    });
}

#[test]
fn settlement_ops_by_anyone_but_the_escrow_are_ignored() {
    big(async {
        let w = World::new().await;
        let bob = Address::repeat_byte(0xb0);
        w.deposit(TGLM, "bounty:1", None).await;
        let src = FixedObligations(vec![pay("b1", bob, TGLM / 4, "bounty:1")]);
        let forged = SettlePayload {
            key: "b1".into(),
            tx: TxHash::repeat_byte(1),
            to: bob,
            amount: (TGLM / 4).to_string(),
            deposit: None,
        };
        let (cfg, keys) = (&w.payer.cfg, &w.payer.keys);
        ops::post(&w.net, cfg, keys, &w.rid, w.object, PAID, &forged)
            .await
            .expect("op");
        let r = w.agent(&src).run_once().await.expect("round");
        assert_eq!(
            r.paid,
            vec!["b1".to_string()],
            "a forged paid op cannot block a payout"
        );
        assert!(w.audit(&src, 0).await.flags.is_empty());
    });
}

//! Deposits, payouts, crash recovery and refunds against MemArkiv.

mod common;

use arkiv_git_escrow::{FixedObligations, ledger};
use arkiv_git_store::{Address, Arkiv, U256};
use common::*;

#[test]
fn deposit_then_obligation_then_paid_once_and_audit_passes() {
    big(async {
        let w = World::new().await;
        let bob = Address::repeat_byte(0xb0);
        w.deposit(TGLM, "bounty:1", None).await;
        let none = FixedObligations::default();
        let r = w.agent(&none).run_once().await.expect("round");
        assert!(r.paid.is_empty() && r.refunded.is_empty());

        let src = FixedObligations(vec![pay("b1", bob, TGLM / 2, "bounty:1")]);
        let r = w.agent(&src).run_once().await.expect("round");
        assert_eq!(r.paid, vec!["b1".to_string()]);
        assert_eq!(w.balance(bob).await, U256::from(TGLM / 2));
        let again = w.agent(&src).run_once().await.expect("idempotent");
        assert!(again.paid.is_empty());
        assert_eq!(
            w.balance(bob).await,
            U256::from(TGLM / 2),
            "never paid twice"
        );

        let l = ledger::load(&w.net, &w.rid).await.expect("ledger");
        assert_eq!(l.settlements.len(), 1);
        assert!(l.settlements[0].chain.is_ok());
        let report = w.audit(&src, 0).await;
        assert!(report.flags.is_empty(), "{:?}", report.flags);
        assert_eq!(report.owed, U256::from(TGLM / 2));
    });
}

#[test]
fn an_underfunded_obligation_waits_for_deposits() {
    big(async {
        let w = World::new().await;
        let bob = Address::repeat_byte(0xb0);
        w.deposit(TGLM / 4, "bounty:2", None).await;
        let src = FixedObligations(vec![pay("b2", bob, TGLM / 2, "bounty:2")]);
        let r = w.agent(&src).run_once().await.expect("round");
        assert!(r.paid.is_empty() && r.skipped.len() == 1, "{r:?}");
        w.deposit(TGLM / 4, "bounty:2", None).await;
        let r = w.agent(&src).run_once().await.expect("round");
        assert_eq!(r.paid, vec!["b2".to_string()]);
    });
}

#[test]
fn a_crash_after_the_transfer_before_the_paid_op_pays_once() {
    big(async {
        let w = World::new().await;
        let bob = Address::repeat_byte(0xb0);
        w.deposit(TGLM, "bounty:1", None).await;
        let src = FixedObligations(vec![pay("b1", bob, TGLM / 2, "bounty:1")]);
        let intent = {
            let agent = w.agent(&src);
            let l = agent.ledger().await.expect("l");
            let (plan, _) = agent.plan(&l).await.expect("plan");
            agent.start(&plan[0]).await.expect("sent")
        }; // crash: the agent is gone and no paid op was posted
        w.net.mine(1);
        assert!(
            w.net.receipt(intent.tx).await.expect("r").is_some(),
            "transfer landed"
        );

        let r = w.agent(&src).run_once().await.expect("recovered");
        assert_eq!(r.paid, vec!["b1".to_string()]);
        assert_eq!(w.balance(bob).await, U256::from(TGLM / 2));
        assert_eq!(
            w.net.tx_count_mined(w.escrow.address()).await.expect("n"),
            1
        );
        assert_eq!(w.agent(&src).journal().expect("journal"), None);
        let l = ledger::load(&w.net, &w.rid).await.expect("ledger");
        assert_eq!(l.settlements[0].tx, intent.tx);
        assert!(w.audit(&src, 0).await.flags.is_empty());
    });
}

#[test]
fn a_dropped_transfer_is_rebroadcast_not_repaid() {
    big(async {
        let w = World::new().await;
        let bob = Address::repeat_byte(0xb0);
        w.deposit(TGLM, "bounty:1", None).await;
        let src = FixedObligations(vec![pay("b1", bob, TGLM / 2, "bounty:1")]);
        w.net.mem.drop_next(1);
        assert!(
            w.agent(&src).run_once().await.is_err(),
            "not mined, journal kept"
        );
        assert!(w.agent(&src).journal().expect("j").is_some());
        let r = w.agent(&src).run_once().await.expect("rebroadcast");
        assert_eq!(r.paid, vec!["b1".to_string()]);
        assert_eq!(w.balance(bob).await, U256::from(TGLM / 2));
        assert!(w.audit(&src, 0).await.flags.is_empty());
    });
}

#[test]
fn forged_and_reused_deposits_are_not_credited() {
    big(async {
        let w = World::new().await;
        let elsewhere = Address::repeat_byte(0xee);
        let stray = w.transfer(&w.payer.keys.wallet(), elsewhere, TGLM).await;
        w.claim(stray, TGLM, "bounty:1", None).await;
        let real = w.deposit(TGLM, "bounty:1", None).await;
        w.claim(real, TGLM, "bounty:other", None).await;
        let small = w
            .transfer(&w.payer.keys.wallet(), w.escrow.address(), 10)
            .await;
        w.claim(small, 11, "bounty:1", None).await;

        let l = ledger::load(&w.net, &w.rid).await.expect("ledger");
        assert_eq!(l.deposits.len(), 1);
        assert_eq!(
            (l.deposits[0].tx, l.deposits[0].purpose.as_str()),
            (real, "bounty:1")
        );
        let why: Vec<&str> = l.rejected.iter().map(|(_, r)| r.as_str()).collect();
        assert!(why[0].contains("not the escrow"), "{why:?}");
        assert!(why[1].contains("already claimed"), "{why:?}");
        assert!(why[2].contains("< 11"), "{why:?}");
    });
}

#[test]
fn an_expired_unused_deposit_is_refunded_to_its_sender() {
    big(async {
        let w = World::new().await;
        let payer = w.payer.keys.wallet().address();
        let expiry = w.head().await + 50;
        let tx = w.deposit(TGLM, "bounty:9", Some(expiry)).await;
        let none = FixedObligations::default();
        assert!(
            w.agent(&none)
                .run_once()
                .await
                .expect("round")
                .refunded
                .is_empty()
        );
        w.net.mine(60);
        let before = w.balance(payer).await;
        let r = w.agent(&none).run_once().await.expect("round");
        assert_eq!(r.refunded, vec![ledger::refund_key(&tx)]);
        assert_eq!(w.balance(payer).await - before, U256::from(TGLM));
        assert!(
            w.agent(&none)
                .run_once()
                .await
                .expect("again")
                .refunded
                .is_empty()
        );
        let report = w.audit(&none, 0).await;
        assert!(report.flags.is_empty(), "{:?}", report.flags);

        let used = w.deposit(TGLM, "bounty:10", Some(w.head().await)).await;
        w.net.mine(2);
        let src = FixedObligations(vec![pay(
            "b10",
            Address::repeat_byte(1),
            2 * TGLM,
            "bounty:10",
        )]);
        let r = w.agent(&src).run_once().await.expect("round");
        assert!(
            r.refunded.is_empty(),
            "a purpose with obligations is not refunded: {used}"
        );
    });
}

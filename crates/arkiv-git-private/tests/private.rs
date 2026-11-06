//! P2 private repos end to end on `MemArkiv`: membership, rotation, publication, leakage.

mod common;

use arkiv_git_core::{AttrValue, Kind, Observed, Rid};
use arkiv_git_private::seal::{MIN_BUCKET, sealed_len};
use arkiv_git_private::{Keyring, Role, Sealed};
use arkiv_git_store::Pred;
use arkiv_git_sync::observe;
use common::*;

async fn ring(net: &Net, p: &Peer, rid: &Rid) -> arkiv_git_sync::Result<Keyring> {
    arkiv_git_private::ring_for(net, &p.cfg, Some(&p.keys), rid)
        .await?
        .ok_or_else(|| {
            arkiv_git_sync::SyncError::coded(arkiv_git_core::ErrorCode::Internal, "public")
        })
}

async fn private_repo(net: &Net, alice: &Peer, name: &str) -> Rid {
    arkiv_git_private::init(net, &alice.cfg, &alice.keys, &meta(name))
        .await
        .expect("init --private")
}

/// `bob` publishes a key package and `alice` adds him with `role`.
async fn invite(net: &Net, alice: &Peer, bob: &Peer, rid: &Rid, role: Role) {
    let kp = arkiv_git_private::publish_key_package(net, &bob.cfg, &bob.keys)
        .await
        .expect("key package");
    let nid = arkiv_git_private::add_member(net, &alice.cfg, &alice.keys, rid, &kp, role)
        .await
        .expect("add");
    assert_eq!(nid, bob.keys.nid());
}

async fn private_push(
    net: &Net,
    p: &Peer,
    rid: &Rid,
    work: &std::path::Path,
) -> arkiv_git_sync::Result<()> {
    let ring = ring(net, p, rid).await?;
    ring.require(&p.keys.nid(), Role::Write)?;
    push(&Sealed::new(net, ring), p, rid, work, &["refs/heads/main"]).await
}

async fn raw_kind(net: &Net, k: Kind) -> Vec<Observed> {
    observe::all(net, observe::kind(k)).await.expect("query")
}

#[test]
fn member_push_then_clone_and_non_member_is_refused() {
    run(async {
        let net = Net::new();
        let (alice, bob, carol) = (Peer::new(&net, 1), Peer::new(&net, 2), Peer::new(&net, 3));
        let rid = private_repo(&net, &alice, "secret-demo").await;
        invite(&net, &alice, &bob, &rid, Role::Write).await;

        let tmp = tempfile::tempdir().expect("tmp");
        let work = work_repo(tmp.path(), "work");
        commit(
            &work,
            &[("a.txt", b"hello"), ("big", &noise(1, 150_000))],
            "feat: one",
        );
        private_push(&net, &alice, &rid, &work)
            .await
            .expect("alice push");

        let bob_ring = ring(&net, &bob, &rid)
            .await
            .expect("bob joins from the welcome");
        let dest = tmp.path().join("bob");
        let (view, listing) = clone_into(&Sealed::new(&net, bob_ring), &bob, &rid, &dest)
            .await
            .expect("bob clones");
        assert_eq!(listing.refs[0].1, rev(&work, "main"));
        assert_eq!(view.identity.head().body.name, "secret-demo");
        git(&dest, &["fsck", "--full"]);

        commit(&dest, &[("b.txt", b"from bob")], "feat: bob");
        private_push(&net, &bob, &rid, &dest)
            .await
            .expect("writer pushes");

        let r = ring(&net, &carol, &rid).await;
        assert_eq!(code(r), "E_NOT_MEMBER");
        let r = clone_into(&net, &carol, &rid, &tmp.path().join("carol")).await;
        assert_ne!(code(r), "OK", "the plain view finds nothing");
    });
}

#[test]
fn removal_rotates_the_key_and_locks_the_removed_member_out() {
    run(async {
        let net = Net::new();
        let (alice, bob, dan) = (Peer::new(&net, 1), Peer::new(&net, 2), Peer::new(&net, 4));
        let rid = private_repo(&net, &alice, "rotate").await;
        invite(&net, &alice, &bob, &rid, Role::Read).await;
        invite(&net, &alice, &dan, &rid, Role::Read).await;
        let tmp = tempfile::tempdir().expect("tmp");
        let work = work_repo(tmp.path(), "work");
        let old = commit(&work, &[("a", b"before")], "feat: before");
        private_push(&net, &alice, &rid, &work).await.expect("push");
        let before = ring(&net, &bob, &rid)
            .await
            .expect("bob")
            .current()
            .expect("cur")
            .0;
        assert_eq!(
            code(private_push(&net, &bob, &rid, &work).await),
            "E_ROLE_DENIED"
        );

        arkiv_git_private::remove_member(&net, &alice.cfg, &alice.keys, &rid, &bob.keys.nid())
            .await
            .expect("remove");
        let new = commit(&work, &[("a", b"after")], "feat: after");
        private_push(&net, &alice, &rid, &work)
            .await
            .expect("push after removal");

        let alice_ring = ring(&net, &alice, &rid).await.expect("alice");
        let after = alice_ring.current().expect("cur").0;
        assert_eq!(after, before + 1, "removal is one commit, one new epoch");
        let dan_ring = ring(&net, &dan, &rid).await.expect("dan");
        assert_eq!(
            dan_ring.current().expect("cur").0,
            after,
            "remaining member follows"
        );
        let bob_ring = ring(&net, &bob, &rid).await.expect("bob keeps old keys");
        assert_eq!(bob_ring.current().expect("cur").0, before);
        assert!(!bob_ring.epochs.contains_key(&after));

        // The new anchor stays sealed for bob, opens for dan; bob's clone stops at the old head.
        let anchors = |r: Keyring| async {
            let s = Sealed::new(&net, r);
            let f = Pred::And(vec![
                observe::kind(Kind::Anchor),
                observe::b32("ag.rid", &rid),
            ]);
            observe::all(&s, f).await.expect("anchors")
        };
        let opened = |v: &[Observed]| v.iter().filter(|o| o.attr("ag.ep").is_none()).count();
        assert_eq!(opened(&anchors(dan_ring).await), 2);
        assert_eq!(opened(&anchors(bob_ring.clone()).await), 1);
        let dest = tmp.path().join("bob");
        let got = clone_into(&Sealed::new(&net, bob_ring), &bob, &rid, &dest).await;
        if let Ok((_, listing)) = got {
            assert_eq!(listing.refs[0].1, old);
            assert_ne!(listing.refs[0].1, new);
        }
    });
}

#[test]
fn published_epoch_key_makes_that_epoch_public() {
    run(async {
        let net = Net::new();
        let (alice, eve) = (Peer::new(&net, 1), Peer::new(&net, 5));
        let rid = private_repo(&net, &alice, "sponsorware").await;
        let tmp = tempfile::tempdir().expect("tmp");
        let work = work_repo(tmp.path(), "work");
        commit(&work, &[("a", b"released")], "feat: release");
        private_push(&net, &alice, &rid, &work).await.expect("push");
        assert_eq!(code(ring(&net, &eve, &rid).await), "E_NOT_MEMBER");

        let epoch = ring(&net, &alice, &rid)
            .await
            .expect("alice")
            .current()
            .expect("cur")
            .0;
        let exported =
            arkiv_git_private::export_epoch_key(&alice.cfg.home, &rid, epoch).expect("export");
        let grant =
            arkiv_git_private::publish_epoch_key(&net, &alice.cfg, &alice.keys, &rid, epoch)
                .await
                .expect("publish");
        assert_eq!(grant, exported);
        assert_eq!(grant.len(), 64);
        let eve_ring = ring(&net, &eve, &rid).await.expect("readable now");
        let (_, listing) = clone_into(
            &Sealed::new(&net, eve_ring),
            &eve,
            &rid,
            &tmp.path().join("e"),
        )
        .await
        .expect("anyone clones");
        assert_eq!(listing.refs[0].1, rev(&work, "main"));
    });
}

#[test]
fn ciphertext_hides_rid_and_name_and_is_padded() {
    run(async {
        let net = Net::new();
        let alice = Peer::new(&net, 1);
        let rid = private_repo(&net, &alice, "hidden-name").await;
        let tmp = tempfile::tempdir().expect("tmp");
        let work = work_repo(tmp.path(), "work");
        commit(&work, &[("a", &noise(7, 100_000))], "feat: data");
        private_push(&net, &alice, &rid, &work).await.expect("push");

        let mut all = vec![];
        for k in [
            Kind::Id,
            Kind::Bind,
            Kind::Anchor,
            Kind::Refs,
            Kind::Manifest,
            Kind::Chunk,
            Kind::Mls,
        ] {
            all.extend(raw_kind(&net, k).await);
        }
        assert!(all.len() >= 8, "found {} entities", all.len());
        let needle = |hay: &[u8], n: &[u8]| hay.windows(n.len()).any(|w| w == n);
        for o in &all {
            for a in &o.attributes {
                let leak = match &a.value {
                    AttrValue::Bytes32(b) => *b == rid,
                    AttrValue::Str(s) => s.contains("hidden-name"),
                    AttrValue::Bytes(b) => needle(b, &rid) || needle(b, b"hidden-name"),
                    AttrValue::U64(_) => false,
                };
                assert!(!leak, "{} leaks in {:?}", a.name, o.attr("ag.kind"));
            }
        }
        let sealed: Vec<usize> = all
            .iter()
            .filter(|o| {
                o.attr("ag.ep").is_some()
                    && o.attr("ag.kind") != Some(&AttrValue::Str("mls".into()))
            })
            .filter_map(|o| match o.attr("ag.body") {
                Some(AttrValue::Bytes(b)) => Some(b.len()),
                _ => None,
            })
            .collect();
        assert!(
            sealed.len() >= 6,
            "id, bind, anchor, refs, manifest, chunks are sealed"
        );
        for len in sealed {
            let padded = len - sealed_len(0) + MIN_BUCKET;
            assert!(
                padded.is_power_of_two() || padded.is_multiple_of(4096),
                "{len} is not a bucket"
            );
        }
        assert!(
            raw_kind(&net, Kind::Id)
                .await
                .iter()
                .all(|o| o.attr("ag.lang").is_none())
        );
    });
}

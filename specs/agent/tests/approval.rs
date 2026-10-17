#[path = "../../policy/tests/support.rs"]
mod support;

use hoardini_core::proposals::{self, ProposalKind};
use hoardini_core::store::Db;
use support::*;

// Traces: AGENT-APPROVAL-KEYED
#[test]
fn approving_one_proposal_never_changes_another_and_both_survive_a_reopen() {
    let fx = Fx::new();
    let f1 = fx.file("one.bin", &data(10_000, 1));
    let f2 = fx.file("two.bin", &data(10_000, 2));
    let p1 = fx.propose(ProposalKind::Delete, &[&f1], None).unwrap();
    let p2 = fx.propose(ProposalKind::Delete, &[&f2], None).unwrap();
    assert_ne!(p1.id, p2.id);
    let p2_before = serde_json::to_value(fx.get(&p2.id)).unwrap();

    let ex = fx.executor();
    let r = ex.approve(&p1.id, &ids(&p1)).unwrap();
    assert!(r.approved);
    assert_eq!(r.items, ids(&p1));
    assert_eq!(wait(&ex, &r.job_id).status, "done");
    assert!(!f1.exists() && f2.exists());
    assert_eq!(serde_json::to_value(fx.get(&p2.id)).unwrap(), p2_before);
    drop(ex);

    let path = fx.db.path().to_path_buf();
    let db = Db::open(&path).unwrap();
    let a = proposals::get(&db, &p1.id).unwrap().unwrap();
    let b = proposals::get(&db, &p2.id).unwrap().unwrap();
    assert_eq!(a.status, "done");
    assert_eq!(a.job_id.as_deref(), Some(r.job_id.as_str()));
    assert_eq!(a.items[0].status, "done");
    assert_eq!(b.status, "pending");
    assert_eq!(serde_json::to_value(&b).unwrap(), p2_before);

    let declined = proposals::decline(&db, &p2.id).unwrap();
    assert_eq!(declined.status, "declined");
    assert_eq!(proposals::get(&db, &p1.id).unwrap().unwrap().status, "done");
}

// Traces: AGENT-NO-DIRECT
#[test]
fn approve_runs_only_pending_proposals_and_their_own_selectable_items() {
    let fx = Fx::new();
    let f1 = fx.file("a.bin", &data(1000, 1));
    let f2 = fx.file("b.bin", &data(1000, 2));
    let f3 = fx.file("c.bin", &data(1000, 3));
    let missing = fx.work.join("gone.bin");
    let pending = fx
        .propose(ProposalKind::Delete, &[&f1, &missing], None)
        .unwrap();
    let other = fx.propose(ProposalKind::Delete, &[&f2], None).unwrap();
    let declined = fx.propose(ProposalKind::Delete, &[&f3], None).unwrap();
    proposals::decline(&fx.db, &declined.id).unwrap();
    let ex = fx.executor();
    let blocked_id = item(&pending, &missing).id;
    assert_eq!(item(&pending, &missing).status, "blocked");

    let attempts: Vec<(&str, Vec<i64>)> = vec![
        ("no-such-proposal", vec![1]),
        (&declined.id, ids(&declined)),
        (&pending.id, ids(&other)),
        (&pending.id, vec![blocked_id]),
        (&pending.id, vec![]),
        (&pending.id, vec![i64::MAX]),
    ];
    for (pid, items) in &attempts {
        assert!(
            ex.approve(pid, items).is_err(),
            "{pid} {items:?} must be rejected"
        );
    }
    assert_eq!(count(&fx.db, "SELECT COUNT(*) FROM job"), 0);
    assert_eq!(count(&fx.db, "SELECT COUNT(*) FROM action"), 0);
    assert!(f1.exists() && f2.exists() && f3.exists());
    assert_eq!(fx.get(&pending.id).status, "pending");
    assert_eq!(fx.get(&declined.id).status, "declined");

    let ok_id = item(&pending, &f1).id;
    let r = ex.approve(&pending.id, &[ok_id]).unwrap();
    wait(&ex, &r.job_id);
    assert!(!f1.exists());
    let again = ex.approve(&pending.id, &[ok_id]).unwrap_err().to_string();
    assert!(again.contains("not pending"), "{again}");
    assert_eq!(count(&fx.db, "SELECT COUNT(*) FROM job"), 1);
    assert!(f2.exists() && f3.exists());
}

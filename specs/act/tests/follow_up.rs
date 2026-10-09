use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hoardini_core::actions::{Executor, Job};
use hoardini_core::memory::{Kind, NewMemory, Scope, remember_with};
use hoardini_core::policy::Verdict;
use hoardini_core::proposals::{self, NewItem, Proposal, ProposalKind};

use super::support::*;
use super::{compress_proposal, dedupe_proposal, dup_fixture, silently_alter};

fn terminal(rx: &mut tokio::sync::broadcast::Receiver<Job>, job_id: &str) -> Job {
    loop {
        let job = rx.blocking_recv().unwrap();
        if job.id == job_id && matches!(job.status.as_str(), "done" | "failed" | "cancelled") {
            return job;
        }
    }
}

fn follow_up_of(fx: &Fx, p: &Proposal) -> Proposal {
    let id = fx
        .get(&p.id)
        .follow_up_id
        .unwrap_or_else(|| panic!("no follow-up for {}", p.id));
    fx.get(&id)
}

fn run(fx: &Fx, ex: &Executor, p: &Proposal, chosen: &[i64]) -> (Job, Option<String>) {
    let mut rx = ex.subscribe();
    let r = ex.approve(&p.id, chosen).unwrap();
    let job = terminal(&mut rx, &r.job_id);
    // Read at the moment the terminal update arrives: the follow-up must already be committed.
    (job, fx.get(&p.id).follow_up_id)
}

fn children(fx: &Fx, p: &Proposal) -> i64 {
    fx.db
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM proposal WHERE parent_id = ?1",
            [&p.id],
            |r| r.get(0),
        )
        .unwrap()
}

// Traces: ACT-FOLLOW-UP
#[test]
fn the_follow_up_holds_every_item_not_done_with_its_approved_tick_re_examined() {
    let fx = Fx::new();
    let done = fx.file("done.bin", &data(8192, 1));
    let grown = fx.file("grown.bin", &data(8192, 2));
    let unticked = fx.file("unticked.bin", &data(8192, 3));
    let vanished = fx.file("vanished.bin", &data(8192, 4));
    let held_late = fx.file("held-late.bin", &data(8192, 5));
    let held_early = fx.file("held-early.bin", &data(8192, 6));
    let denied = fx.file(r"private\denied.bin", &data(8192, 7));
    let early = std::fs::File::open(&held_early).unwrap();

    let mut req = fx.request(ProposalKind::Delete, &[], None);
    req.conversation_id = Some("main".into());
    for (p, v) in [
        (&done, Verdict::Safe),
        (&grown, Verdict::Review),
        (&unticked, Verdict::Safe),
        (&vanished, Verdict::Safe),
        (&held_late, Verdict::Safe),
        (&held_early, Verdict::Safe),
        (&denied, Verdict::Safe),
    ] {
        req.items.push(NewItem {
            path: s(p),
            verdict: Some(v),
            reason: Some(format!(
                "reason for {}",
                p.file_name().unwrap().to_string_lossy()
            )),
        });
    }
    let p = proposals::create_with(&fx.db, &fx.env, req).unwrap();
    assert_eq!(item(&p, &held_early).status, "blocked");
    let parent_grown_alloc = item(&p, &grown).alloc;

    std::fs::write(&grown, data(256 * 1024, 8)).unwrap();
    std::fs::remove_file(&vanished).unwrap();
    let late = std::fs::File::open(&held_late).unwrap();
    drop(early);
    remember_with(
        &fx.db.conn().unwrap(),
        NewMemory::new(
            Scope::Path,
            &s(&fx.work.join("private")),
            Kind::Protect,
            "private stuff",
            "user",
        ),
        1,
        &s(&fx.profile),
    )
    .unwrap();

    let ex = fx.executor();
    let chosen: Vec<i64> = [&done, &grown, &vanished, &held_late, &denied]
        .iter()
        .map(|f| item(&p, f).id)
        .collect();
    let (job, seen) = run(&fx, &ex, &p, &chosen);
    assert_eq!(job.status, "done", "{job:?}");
    assert!(
        seen.is_some(),
        "follow-up committed before the terminal update"
    );
    drop(late);

    let parent = fx.get(&p.id);
    assert_eq!(parent.status, "partial");
    assert_eq!(item(&parent, &done).status, "done");
    let f = follow_up_of(&fx, &p);
    assert_eq!(seen.as_deref(), Some(f.id.as_str()));
    assert_eq!(f.parent_id.as_deref(), Some(p.id.as_str()));
    assert_eq!(f.status, "pending");
    assert_eq!(f.kind, ProposalKind::Delete);
    assert_eq!(f.conversation_id.as_deref(), Some("main"));
    assert_eq!(f.summary, p.summary);

    let paths: Vec<String> = f.items.iter().map(|i| i.path.to_lowercase()).collect();
    assert_eq!(f.items.len(), 4, "{paths:#?}");
    for gone in [&done, &vanished, &denied] {
        assert!(
            !paths.contains(&s(gone).to_lowercase()),
            "{}",
            gone.display()
        );
    }
    let refused = f
        .dropped
        .iter()
        .find(|d| d.path.eq_ignore_ascii_case(&s(&denied)))
        .expect("a path denied since is listed as dropped");
    assert!(
        refused.reason.contains("private stuff"),
        "{}",
        refused.reason
    );

    let g = item(&f, &grown);
    assert!(
        g.selected,
        "approved, skipped as changed: comes back ticked"
    );
    assert_eq!(g.status, "pending");
    assert!(g.alloc > parent_grown_alloc, "alloc is re-measured");
    assert_eq!(g.verdict, Some(Verdict::Review));
    assert_eq!(g.reason.as_deref(), Some("reason for grown.bin"));

    let u = item(&f, &unticked);
    assert!(!u.selected, "left unticked: comes back unticked");
    assert_eq!(u.status, "pending");
    assert_eq!(u.reason.as_deref(), Some("reason for unticked.bin"));

    let hl = item(&f, &held_late);
    assert_eq!(hl.status, "blocked", "still open at re-examination");
    assert!(!hl.selected, "a newly blocked item is unticked");
    assert!(hl.block_reason.as_deref().unwrap().starts_with("Open in "));

    let he = item(&f, &held_early);
    assert_eq!(he.status, "pending", "no longer open");
    assert!(!he.selected, "it was blocked, so never approved");
    assert_eq!(he.block_reason, None);
}

// Traces: ACT-FOLLOW-UP
#[test]
fn nothing_left_means_no_follow_up_and_dedupe_gets_none() {
    let fx = Fx::new();
    let ex = fx.executor();

    let a = fx.file("a.bin", &data(8192, 1));
    let b = fx.file("b.bin", &data(8192, 2));
    let all = fx.propose(ProposalKind::Delete, &[&a, &b], None).unwrap();
    let (job, seen) = run(&fx, &ex, &all, &ids(&all));
    assert_eq!(job.status, "done");
    assert_eq!(seen, None);
    assert_eq!(children(&fx, &all), 0);

    let c = fx.file("c.bin", &data(8192, 3));
    let d = fx.file("d.bin", &data(8192, 4));
    let gone = fx.propose(ProposalKind::Delete, &[&c, &d], None).unwrap();
    std::fs::remove_file(&d).unwrap();
    let (_, seen) = run(&fx, &ex, &gone, &ids(&gone));
    assert_eq!(seen, None, "the only item not done no longer exists");
    assert_eq!(children(&fx, &gone), 0);

    let dup = dup_fixture(&fx);
    silently_alter(&dup.lookalike);
    let dd = dedupe_proposal(&fx, &dup, "delete", &[&dup.twin, &dup.lookalike]);
    let (_, seen) = run(&fx, &ex, &dd, &ids(&dd));
    assert_ne!(item(&fx.get(&dd.id), &dup.lookalike).status, "done");
    assert_eq!(seen, None);
    assert_eq!(children(&fx, &dd), 0);
}

// Traces: ACT-FOLLOW-UP
#[test]
fn failed_cancelled_archive_and_compress_jobs_carry_kind_destination_and_format() {
    let fx = Fx::new();
    let (dest, _) = fx.local_destination("nas");
    let ex = fx.executor();
    ex.set_copy_hook(Some(Arc::new(|tmp: &Path| {
        std::fs::write(tmp, b"garbage").unwrap();
    })));
    let bad = fx.file(r"docs\bad.bin", &data(100_000, 1));
    let p = fx
        .propose(ProposalKind::Archive, &[&bad], Some(&dest.id))
        .unwrap();
    let (job, seen) = run(&fx, &ex, &p, &ids(&p));
    assert_eq!(job.status, "failed");
    assert!(seen.is_some());
    let f = follow_up_of(&fx, &p);
    assert_eq!(f.kind, ProposalKind::Archive);
    assert_eq!(
        f.destination.as_ref().map(|d| d.id.as_str()),
        Some(dest.id.as_str())
    );
    assert!(item(&f, &bad).selected, "a failed item comes back ticked");

    // Cancel while the first copy waits in the hook; the second never starts.
    let reached = Arc::new(AtomicBool::new(false));
    let go = Arc::new(AtomicBool::new(false));
    let (r2, g2) = (reached.clone(), go.clone());
    ex.set_copy_hook(Some(Arc::new(move |_: &Path| {
        r2.store(true, Ordering::SeqCst);
        while !g2.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
    })));
    let first = fx.file(r"docs\first.bin", &data(100_000, 2));
    let second = fx.file(r"docs\second.bin", &data(100_000, 3));
    let p = fx
        .propose(ProposalKind::Archive, &[&first, &second], Some(&dest.id))
        .unwrap();
    let mut rx = ex.subscribe();
    let r = ex.approve(&p.id, &ids(&p)).unwrap();
    let start = Instant::now();
    while !reached.load(Ordering::SeqCst) {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the copy never started"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ex.cancel(&r.job_id));
    go.store(true, Ordering::SeqCst);
    let job = terminal(&mut rx, &r.job_id);
    assert_eq!(job.status, "cancelled");
    assert!(fx.get(&p.id).follow_up_id.is_some());
    let f = follow_up_of(&fx, &p);
    let s2 = item(&f, &second);
    assert!(s2.selected && s2.status == "pending", "{s2:#?}");
    ex.set_copy_hook(None);

    let dir = fx.work.join("project");
    fx.file(r"project\a.txt", &data(8192, 4));
    hoardini_win::create_junction(&dir.join("jn"), &fx.work.join("docs")).unwrap();
    let c = compress_proposal(&fx, &[&dir], "lzx");
    let (job, seen) = run(&fx, &ex, &c, &ids(&c));
    assert_eq!(item(&fx.get(&c.id), &dir).status, "failed", "{job:?}");
    assert!(seen.is_some());
    let f = follow_up_of(&fx, &c);
    assert_eq!(
        (f.kind, f.format.as_deref()),
        (ProposalKind::Compress, Some("lzx"))
    );
    assert!(item(&f, &dir).selected);
}

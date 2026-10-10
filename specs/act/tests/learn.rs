use std::path::{Path, PathBuf};
use std::sync::Arc;

use hoardini_core::actions::{Executor, Job};
use hoardini_core::memory::knowledge::upsert_with;
use hoardini_core::memory::learn::{learned_for, undo};
use hoardini_core::memory::location::location_at;
use hoardini_core::memory::{Detail, Kind, Knowledge, Memory, Scope, get};
use hoardini_core::proposals::{self, Proposal, ProposalKind};
use hoardini_core::store::now_ms;
use serde_json::{Value, json};

use super::support::*;
use super::{dedupe_proposal, dup_fixture, silently_alter};

fn run(ex: &Executor, p: &Proposal, chosen: &[i64]) -> Job {
    wait(ex, &ex.approve(&p.id, chosen).unwrap().job_id)
}

fn row(fx: &Fx, path: &Path) -> Option<Memory> {
    location_at(&fx.db.conn().unwrap(), &s(&fx.profile), &s(path)).unwrap()
}

fn entries(m: &Memory, list: &str) -> Vec<Value> {
    let d = m.detail.as_ref().unwrap();
    match list {
        "cleaned" => d.cleaned.clone(),
        _ => d.kept.clone(),
    }
}

fn job_rows(fx: &Fx) -> i64 {
    count(
        &fx.db,
        "SELECT COUNT(*) FROM memory WHERE source LIKE 'job:%'",
    )
}

fn mmdd(ms: i64) -> String {
    hoardini_core::history::fmt_date(ms)[5..].to_string()
}

// Traces: MEM-LEARN-JOB
#[test]
fn done_items_go_to_cleaned_and_unticked_ones_to_kept_with_exact_entries() {
    let fx = Fx::new();
    let w = s(&fx.work);
    fx.file(r"code\proj\target\debug\app.bin", &data(40_000, 1));
    let target = fx.work.join(r"code\proj\target");
    let one = fx.file(r"notes\one.log", &data(9_000, 2));
    let two = fx.file(r"notes\two.log", &data(9_000, 3));
    let odd = fx.file(r"misc\odd.dat", &data(9_000, 4));
    let keep = fx.file(r"keep\big.bin", &data(20_000, 5));
    let gone = fx.file(r"misc\gone.bin", &data(9_000, 6));
    let conn = fx.db.conn().unwrap();
    let (code, _) = upsert_with(
        &conn,
        Knowledge {
            scope: Scope::Path,
            key: format!(r"{w}\code"),
            kind: Kind::Verdict,
            text: "Not identified.".into(),
            source: "index".into(),
            detail: Detail {
                category: "unknown".into(),
                ..Detail::default()
            },
        },
        1,
        &s(&fx.profile),
    )
    .unwrap();

    let p = fx
        .propose(
            ProposalKind::Delete,
            &[&target, &one, &two, &odd, &keep, &gone],
            None,
        )
        .unwrap();
    std::fs::remove_file(&gone).unwrap();
    let chosen: Vec<i64> = [&target, &one, &two, &odd, &gone]
        .iter()
        .map(|f| item(&p, f).id)
        .collect();
    let ex = fx.executor();
    let t0 = now_ms();
    let job = run(&ex, &p, &chosen);
    let t1 = now_ms();
    assert_eq!(job.status, "done", "{job:?}");
    let after = fx.get(&p.id);
    assert_eq!(item(&after, &gone).status, "skipped");

    let code = get(&conn, code.id).unwrap().unwrap();
    assert_eq!(code.source, "index", "an existing row keeps its source");
    let cleaned = entries(&code, "cleaned");
    assert_eq!(cleaned.len(), 1);
    let c = cleaned[0].as_object().unwrap();
    let mut keys: Vec<&str> = c.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["bytes", "job", "kind", "ms", "path", "rule"]);
    let ms = c["ms"].as_i64().unwrap();
    assert!((t0..=t1).contains(&ms), "{ms} not in {t0}..{t1}");
    assert_eq!(
        cleaned[0],
        json!({
            "ms": ms,
            "bytes": item(&p, &target).alloc,
            "path": s(&target),
            "rule": format!(r"{w}\code\**\target"),
            "job": p.id,
            "kind": "delete",
        })
    );

    let made = row(&fx, &fx.work).expect("a job row at the deepest common folder");
    assert_eq!(made.source, format!("job:{}", p.id));
    assert_eq!(made.kind, Kind::Verdict);
    assert_eq!(made.text, "You cleaned 3 items here.");
    let d = made.detail.as_ref().unwrap();
    assert_eq!(
        (d.category.as_str(), d.verdict, d.by.as_deref()),
        ("unknown", None, Some("job"))
    );
    let rules: Vec<(String, String)> = entries(&made, "cleaned")
        .iter()
        .map(|e| {
            (
                e["path"].as_str().unwrap().to_string(),
                e["rule"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        rules,
        [
            (s(&one), format!(r"{w}\notes\*.log")),
            (s(&two), format!(r"{w}\notes\*.log")),
            (s(&odd), s(&odd)),
        ]
    );
    assert_eq!(
        entries(&made, "kept"),
        [json!({"ms": ms, "path": s(&keep), "size": item(&p, &keep).alloc, "job": p.id})]
    );

    let learned = after.learned.clone();
    let bytes = |fs: &[&PathBuf]| -> u64 { fs.iter().map(|f| item(&p, f).alloc as u64).sum() };
    let human = hoardini_core::paths::humanize_bytes;
    assert_eq!(learned.len(), 2, "{learned:#?}");
    assert_eq!(learned[0].memory_id, code.id);
    assert_eq!(
        learned[0].text,
        format!(
            r"target folders in {w}\code — cleaned {} on {}",
            human(bytes(&[&target])),
            mmdd(ms)
        )
    );
    assert_eq!(learned[1].memory_id, made.id);
    assert_eq!(
        learned[1].text,
        format!(
            "3 items in {w} — cleaned {} on {}; 1 item you left unticked is kept",
            human(bytes(&[&one, &two, &odd])),
            mmdd(ms)
        )
    );
    let follow = fx.get(after.follow_up_id.as_deref().unwrap());
    assert!(
        follow.learned.is_empty(),
        "a pending card has learned nothing"
    );
}

// Traces: MEM-LEARN-JOB
#[test]
fn the_card_anchor_names_the_new_row_and_undo_takes_back_only_this_job() {
    let fx = Fx::new();
    let w = s(&fx.work);
    let deep = fx.file(r"cards\deep\x\file.bin", &data(9_000, 1));
    let p = fx.propose(ProposalKind::Delete, &[&deep], None).unwrap();
    let conn = fx.db.conn().unwrap();
    conn.execute(
        "UPDATE proposal SET anchor = ?1 WHERE id = ?2",
        [format!(r"{w}\cards"), p.id.clone()],
    )
    .unwrap();
    let ex = fx.executor();
    assert_eq!(run(&ex, &p, &ids(&p)).status, "done");
    assert!(row(&fx, &fx.work.join(r"cards\deep\x")).is_none());
    let made = row(&fx, &fx.work.join("cards")).expect("the row sits at the card's anchor");
    assert_eq!(made.text, "You cleaned file.bin here.");

    let other = fx.file(r"cards\other.bin", &data(9_000, 2));
    let q = fx.propose(ProposalKind::Delete, &[&other], None).unwrap();
    assert_eq!(run(&ex, &q, &ids(&q)).status, "done");
    let both = get(&conn, made.id).unwrap().unwrap();
    assert_eq!(
        entries(&both, "cleaned").len(),
        2,
        "the second job reuses the row"
    );
    assert_eq!(both.source, format!("job:{}", p.id));

    assert!(undo(&conn, &p.id, made.id).unwrap());
    let left = get(&conn, made.id)
        .unwrap()
        .expect("another job's entry keeps the row");
    let paths: Vec<Value> = entries(&left, "cleaned")
        .iter()
        .map(|e| e["path"].clone())
        .collect();
    assert_eq!(paths, [json!(s(&other))]);
    assert!(learned_for(&conn, &p.id).unwrap().is_empty());
    assert!(
        !undo(&conn, &p.id, made.id).unwrap(),
        "nothing left to undo"
    );

    let third = fx.file(r"solo\a.bin", &data(9_000, 3));
    let r = fx.propose(ProposalKind::Delete, &[&third], None).unwrap();
    assert_eq!(run(&ex, &r, &ids(&r)).status, "done");
    let solo = row(&fx, &fx.work.join("solo")).unwrap();
    assert!(undo(&conn, &r.id, solo.id).unwrap());
    assert!(
        get(&conn, solo.id).unwrap().is_none(),
        "the job made it and nothing else is in it"
    );
}

// Traces: MEM-LEARN-JOB
#[test]
fn failed_declined_and_dedupe_jobs_learn_nothing() {
    let fx = Fx::new();
    let (dest, _) = fx.local_destination("nas");
    let ex = fx.executor();
    ex.set_copy_hook(Some(Arc::new(|tmp: &Path| {
        std::fs::write(tmp, b"garbage").unwrap();
    })));
    let bad = fx.file(r"docs\bad.bin", &data(100_000, 1));
    let a = fx
        .propose(ProposalKind::Archive, &[&bad], Some(&dest.id))
        .unwrap();
    assert_eq!(run(&ex, &a, &ids(&a)).status, "failed");
    ex.set_copy_hook(None);

    let idle = fx.file(r"idle\x.bin", &data(9_000, 2));
    let d = fx.propose(ProposalKind::Delete, &[&idle], None).unwrap();
    proposals::decline(&fx.db, &d.id).unwrap();

    let dup = dup_fixture(&fx);
    silently_alter(&dup.lookalike);
    let dd = dedupe_proposal(&fx, &dup, "delete", &[&dup.twin, &dup.lookalike]);
    let job = run(&ex, &dd, &ids(&dd));
    assert_eq!(item(&fx.get(&dd.id), &dup.twin).status, "done", "{job:?}");

    for p in [&a, &d, &dd] {
        assert!(fx.get(&p.id).learned.is_empty(), "{}", p.id);
    }
    assert_eq!(job_rows(&fx), 0);
    assert_eq!(
        count(
            &fx.db,
            "SELECT COUNT(*) FROM memory WHERE detail_json LIKE '%\"job\":%'"
        ),
        0
    );
}

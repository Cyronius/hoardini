use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::*;
use crate::index::{RefreshOptions, WinLister, refresh};
use crate::store::Db;

fn noise(n: usize, seed: u32) -> Vec<u8> {
    let mut x = 0x9E37_79B9_u32 ^ seed;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 24) as u8
        })
        .collect()
}

fn setup() -> (tempfile::TempDir, Db, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let root = PathBuf::from(root.to_string_lossy().trim_start_matches(r"\\?\"));
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let db = Db::open(root.join("h.db")).unwrap();
    (tmp, db, work)
}

fn index(db: &Db, work: &Path) {
    let opts = RefreshOptions {
        threshold: Some(0),
        hotspots_first: false,
        ..RefreshOptions::default()
    };
    let out = refresh(
        &db.conn().unwrap(),
        &WinLister,
        &work.to_string_lossy(),
        &opts,
    )
    .unwrap();
    assert!(out.completed);
}

fn opts() -> DupOptions {
    DupOptions {
        min_size: 1000,
        ..DupOptions::default()
    }
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn groups_true_copies_only() {
    let (_t, db, work) = setup();
    let a = noise(300_000, 1);
    std::fs::write(work.join("a.bin"), &a).unwrap();
    std::fs::write(work.join("a-copy.bin"), &a).unwrap();
    // Same size, same head and tail, different middle: only the full hash tells them apart.
    let mut tweaked = a.clone();
    tweaked[150_000] ^= 0xFF;
    std::fs::write(work.join("a-lookalike.bin"), &tweaked).unwrap();
    std::fs::write(work.join("other.bin"), noise(300_000, 9)).unwrap();
    std::fs::hard_link(work.join("a.bin"), work.join("a-link.bin")).unwrap();
    index(&db, &work);

    let conn = db.conn().unwrap();
    let r = find(&conn, &opts(), &no_cancel()).unwrap();
    assert_eq!(r.groups.len(), 1, "{:?}", r.groups);
    let g = &r.groups[0];
    let names: Vec<_> = g
        .paths
        .iter()
        .map(|p| p.rsplit('\\').next().unwrap().to_string())
        .collect();
    assert_eq!(g.paths.len(), 2, "{names:?}");
    assert!(names.contains(&"a-copy.bin".to_string()));
    assert!(names.iter().any(|n| n == "a.bin" || n == "a-link.bin"));
    assert_eq!(g.wasted_bytes(), 300_000);
    assert_eq!(r.pending, 0);
}

#[test]
fn lone_hard_links_are_not_duplicates() {
    let (_t, db, work) = setup();
    std::fs::write(work.join("x.bin"), noise(50_000, 3)).unwrap();
    std::fs::hard_link(work.join("x.bin"), work.join("y.bin")).unwrap();
    index(&db, &work);
    let r = find(&db.conn().unwrap(), &opts(), &no_cancel()).unwrap();
    assert!(r.groups.is_empty());
}

#[test]
fn second_run_reuses_hashes_and_keeps_group_ids() {
    let (_t, db, work) = setup();
    let a = noise(200_000, 4);
    std::fs::write(work.join("p.bin"), &a).unwrap();
    std::fs::write(work.join("q.bin"), &a).unwrap();
    index(&db, &work);
    let conn = db.conn().unwrap();
    let first = find(&conn, &opts(), &no_cancel()).unwrap();
    assert!(first.hashed_bytes >= 400_000);
    let second = find(&conn, &opts(), &no_cancel()).unwrap();
    assert_eq!(second.hashed_bytes, 0);
    assert_eq!(second.groups[0].id, first.groups[0].id);
    assert_eq!(hint(&conn), Some((200_000, 1)));
}

#[test]
fn a_changed_copy_leaves_the_group() {
    let (_t, db, work) = setup();
    let a = noise(120_000, 5);
    std::fs::write(work.join("p.bin"), &a).unwrap();
    std::fs::write(work.join("q.bin"), &a).unwrap();
    index(&db, &work);
    let conn = db.conn().unwrap();
    assert_eq!(find(&conn, &opts(), &no_cancel()).unwrap().groups.len(), 1);
    std::fs::write(work.join("q.bin"), noise(120_000, 6)).unwrap();
    index(&db, &work);
    let r = find(&conn, &opts(), &no_cancel()).unwrap();
    assert!(r.groups.is_empty());
    assert_eq!(hint(&conn), None);
}

#[test]
fn zero_budget_still_reports_pending() {
    let (_t, db, work) = setup();
    let a = noise(100_000, 7);
    std::fs::write(work.join("p.bin"), &a).unwrap();
    std::fs::write(work.join("q.bin"), &a).unwrap();
    index(&db, &work);
    let o = DupOptions {
        budget: Duration::ZERO,
        ..opts()
    };
    let r = find(&db.conn().unwrap(), &o, &no_cancel()).unwrap();
    assert_eq!(r.pending, 2);
    assert!(r.groups.is_empty());
}

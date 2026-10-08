use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use hoardini_core::history::{
    self, ScanRecord, Since, SummaryDir, changes, hotspots, record_scan_rows, scans, summary_dirs,
    summary_files,
};
use hoardini_core::index::{FakeFs, RefreshOptions, RefreshOutcome, node_by_path, refresh};
use hoardini_core::store::{Db, now_ms};

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
const DAY_MS: i64 = 86_400_000;

struct Env {
    _dir: tempfile::TempDir,
    db: Db,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("h.db")).unwrap();
    Env { _dir: dir, db }
}

fn run(e: &Env, fs: &FakeFs) -> RefreshOutcome {
    let out = refresh(
        &e.db.conn().unwrap(),
        fs,
        fs.root(),
        &RefreshOptions {
            workers: 2,
            ..RefreshOptions::default()
        },
    )
    .unwrap();
    assert!(out.completed);
    out
}

// Traces: HIST-SUMMARY
#[test]
fn completed_refresh_stores_big_dirs_and_files() {
    let fs = FakeFs::new(r"C:\");
    fs.file_alloc(r"C:\big\movie.mkv", 300 * MIB, 300 * MIB);
    fs.file_alloc(r"C:\edge\a.bin", 200 * MIB, 200 * MIB);
    fs.file_alloc(r"C:\edge\b.bin", 56 * MIB, 56 * MIB);
    fs.file_alloc(r"C:\under\a.bin", 100 * MIB, 100 * MIB);
    fs.file_alloc(r"C:\under\c.bin", 100 * MIB - 4096, 100 * MIB - 4096);
    fs.hard_link(r"C:\big\movie.mkv", r"C:\links\x.bin");
    fs.set_volume(2048 * GIB, 100 * GIB);
    let e = env();
    let out = run(&e, &fs);
    let conn = e.db.conn().unwrap();
    let scan_id = out.scan_id.unwrap();

    let dirs: BTreeMap<String, (u64, u64)> = summary_dirs(&conn, scan_id)
        .unwrap()
        .into_iter()
        .map(|d| (d.path, (d.alloc, d.count)))
        .collect();
    let root_size = 300 * MIB + 256 * MIB + 200 * MIB - 4096;
    let want: BTreeMap<String, (u64, u64)> = [
        (r"C:\".to_string(), (root_size, 5)),
        (r"C:\big".to_string(), (300 * MIB, 1)),
        (r"C:\edge".to_string(), (256 * MIB, 2)),
    ]
    .into_iter()
    .collect();
    assert_eq!(dirs, want);

    let files: BTreeMap<String, u64> = summary_files(&conn, scan_id)
        .unwrap()
        .into_iter()
        .map(|f| {
            assert!(f.mtime_ms.is_some());
            (f.path, f.alloc)
        })
        .collect();
    let want: BTreeMap<String, u64> = [
        (r"C:\big\movie.mkv".to_string(), 300 * MIB),
        (r"C:\edge\a.bin".to_string(), 200 * MIB),
        (r"C:\under\a.bin".to_string(), 100 * MIB),
    ]
    .into_iter()
    .collect();
    assert_eq!(files, want);

    let scan = history::scan_by_id(&conn, scan_id).unwrap().unwrap();
    assert_eq!(scan.indexed_bytes, root_size);
    assert_eq!(scan.file_count, 5);
    assert_eq!(scan.dir_count, 5);
    assert_eq!(scan.free_bytes, Some(100 * GIB));
    assert_eq!(scan.used_bytes, Some(1948 * GIB));
    assert_eq!(
        scan.unaccounted_bytes,
        Some((1948 * GIB - root_size) as i64)
    );

    let cancel = Arc::new(AtomicBool::new(true));
    let interrupted = refresh(
        &conn,
        &fs,
        r"C:\",
        &RefreshOptions {
            cancel: Some(cancel),
            ..RefreshOptions::default()
        },
    )
    .unwrap();
    assert!(!interrupted.completed);
    assert_eq!(scans(&conn, scan.volume_id).unwrap().len(), 1);
}

fn has(list: &[history::Change], path: &str) -> Option<i64> {
    list.iter().find(|c| c.path == path).map(|c| c.delta)
}

// Traces: HIST-CHANGES
#[test]
fn changes_report_grown_shrunk_new_and_removed() {
    let fs = FakeFs::new(r"C:\");
    fs.file_alloc(r"C:\Users\josha\models\a.gguf", 3 * GIB, 3 * GIB);
    fs.file_alloc(r"C:\Users\josha\cache\blob1", 600 * MIB, 600 * MIB);
    fs.file_alloc(r"C:\Users\josha\cache\blob2", 424 * MIB, 424 * MIB);
    fs.file_alloc(r"C:\old\keep.bin", 400 * MIB, 400 * MIB);
    fs.file_alloc(r"C:\old\more.bin", 100 * MIB, 100 * MIB);
    fs.file_alloc(r"C:\logs\app.log", 300 * MIB, 300 * MIB);
    fs.set_volume(1024 * GIB, 100 * GIB);
    let e = env();
    let first = run(&e, &fs);

    fs.file_alloc(r"C:\Users\josha\models\b.gguf", 2 * GIB, 2 * GIB);
    fs.remove(r"C:\old");
    fs.remove(r"C:\Users\josha\cache\blob1");
    fs.resize(r"C:\Users\josha\cache\blob2", 300 * MIB, 300 * MIB);
    fs.file_alloc(r"C:\new\sub\n.bin", GIB, GIB);
    fs.resize(
        r"C:\logs\app.log",
        300 * MIB + 512 * 1024,
        300 * MIB + 512 * 1024,
    );
    fs.set_volume(1024 * GIB, 98 * GIB);
    let second = run(&e, &fs);

    let conn = e.db.conn().unwrap();
    let ch = changes(&conn, None, Since::Previous, 50).unwrap().unwrap();
    assert_eq!(
        (ch.from.id, ch.to.id),
        (first.scan_id.unwrap(), second.scan_id.unwrap())
    );
    assert_eq!(ch.free_delta, Some(-2 * GIB as i64));

    assert_eq!(
        has(&ch.new, r"C:\Users\josha\models\b.gguf"),
        Some(2 * GIB as i64)
    );
    assert_eq!(has(&ch.new, r"C:\new"), Some(GIB as i64));
    assert_eq!(
        has(&ch.new, r"C:\new\sub"),
        None,
        "covered by its new parent"
    );
    assert_eq!(
        has(&ch.grew, r"C:\Users\josha\models"),
        None,
        "explained by the new file"
    );
    assert_eq!(has(&ch.grew, r"C:\Users\josha"), None);

    assert_eq!(has(&ch.removed, r"C:\old"), Some(-(500 * MIB as i64)));
    assert_eq!(
        has(&ch.removed, r"C:\old\keep.bin"),
        Some(-(400 * MIB as i64))
    );
    assert_eq!(
        has(&ch.removed, r"C:\Users\josha\cache\blob1"),
        Some(-(600 * MIB as i64))
    );
    assert_eq!(
        has(&ch.shrank, r"C:\Users\josha\cache\blob2"),
        Some(-(124 * MIB as i64))
    );
    assert_eq!(
        has(&ch.shrank, r"C:\Users\josha\cache"),
        Some(-(724 * MIB as i64))
    );
    assert!(
        ch.grew
            .iter()
            .chain(&ch.shrank)
            .all(|c| c.path != r"C:\logs"),
        "sub-MiB noise is not a change"
    );

    let by_id = changes(&conn, None, Since::Scan(first.scan_id.unwrap()), 50)
        .unwrap()
        .unwrap();
    assert_eq!(by_id, ch);
    let by_days = changes(&conn, None, Since::Days(30), 50).unwrap().unwrap();
    assert_eq!(by_days.from.id, first.scan_id.unwrap());

    let capped = changes(&conn, None, Since::Previous, 1).unwrap().unwrap();
    assert_eq!(capped.removed.len(), 1);
    assert_eq!(capped.omitted.removed, ch.removed.len() - 1);
}

fn synthetic_volume(e: &Env) -> i64 {
    let fs = FakeFs::new(r"C:\");
    fs.file(r"C:\seed.bin", MIB);
    run(e, &fs).volume_id
}

fn add_scan(e: &Env, volume_id: i64, finished_ms: i64, dirs: &[(&str, u64)]) -> i64 {
    let rows: Vec<SummaryDir> = dirs
        .iter()
        .map(|(p, a)| SummaryDir {
            path: p.to_string(),
            alloc: *a,
            count: 1,
        })
        .collect();
    record_scan_rows(
        &e.db.conn().unwrap(),
        &ScanRecord {
            volume_id,
            kind: "full".into(),
            mode: "background".into(),
            started_ms: finished_ms - 1000,
            finished_ms,
            ..ScanRecord::default()
        },
        &rows,
        &[],
    )
    .unwrap()
}

// Traces: HIST-HOTSPOT
#[test]
fn hotspot_rank_combines_size_growth_and_recurrence() {
    let e = env();
    let vol = synthetic_volume(&e);
    let g = GIB;
    let series: Vec<(&str, [u64; 11])> = vec![
        (r"C:\static", [50 * g; 11]),
        (
            r"C:\steady",
            [
                g,
                2 * g,
                3 * g,
                4 * g,
                5 * g,
                6 * g,
                7 * g,
                8 * g,
                9 * g,
                10 * g,
                11 * g,
            ],
        ),
        (
            r"C:\spike",
            [
                2 * g,
                2 * g,
                2 * g,
                2 * g,
                2 * g,
                2 * g,
                2 * g,
                2 * g,
                2 * g,
                2 * g,
                11 * g,
            ],
        ),
        (
            r"C:\bigger-spike",
            [
                20 * g,
                20 * g,
                20 * g,
                20 * g,
                20 * g,
                20 * g,
                20 * g,
                20 * g,
                20 * g,
                20 * g,
                29 * g,
            ],
        ),
        (
            r"C:\refill",
            [
                3 * g,
                3 * g,
                0,
                3 * g,
                3 * g,
                3 * g,
                3 * g,
                3 * g,
                3 * g,
                3 * g,
                3 * g,
            ],
        ),
        (
            r"C:\no-refill",
            [
                0,
                0,
                0,
                3 * g,
                3 * g,
                3 * g,
                3 * g,
                3 * g,
                3 * g,
                3 * g,
                3 * g,
            ],
        ),
        (
            r"C:\old-growth",
            [
                g,
                10 * g,
                10 * g,
                10 * g,
                10 * g,
                10 * g,
                10 * g,
                10 * g,
                10 * g,
                10 * g,
                10 * g,
            ],
        ),
        (r"C:\flat-10", [10 * g; 11]),
    ];
    let base = now_ms() + 1000;
    for i in 0..11 {
        let mut dirs: Vec<(&str, u64)> = vec![(r"C:\", 200 * g)];
        for (p, s) in &series {
            if s[i] > 0 {
                dirs.push((p, s[i]));
            }
        }
        add_scan(&e, vol, base + i as i64 * 1000, &dirs);
    }
    let conn = e.db.conn().unwrap();
    let ranked = hotspots(&conn, vol, 20).unwrap();
    let score = |p: &str| {
        ranked
            .iter()
            .find(|h| h.path == p)
            .unwrap_or_else(|| panic!("{p} missing"))
            .score
    };
    assert!(
        ranked.iter().all(|h| h.path != r"C:\"),
        "the volume root is never a hotspot"
    );
    assert!(score(r"C:\steady") > score(r"C:\spike"), "recurrence");
    assert!(score(r"C:\spike") > score(r"C:\static"), "growth over size");
    assert!(score(r"C:\bigger-spike") > score(r"C:\spike"), "size");
    assert!(
        score(r"C:\refill") > score(r"C:\no-refill"),
        "cleaned and refilled"
    );
    assert_eq!(
        score(r"C:\old-growth"),
        score(r"C:\flat-10"),
        "growth before the last N refreshes does not count"
    );
    let steady = ranked.iter().find(|h| h.path == r"C:\steady").unwrap();
    assert_eq!(
        (steady.growth, steady.recurrence, steady.size),
        (9 * g, 9, 11 * g)
    );
    let order: Vec<&str> = ranked.iter().map(|h| h.path.as_str()).collect();
    assert_eq!(order[0], r"C:\steady");
}

// Traces: HIST-RETENTION
#[test]
fn retention_keeps_last_20_and_one_per_month() {
    let e = env();
    let fs = FakeFs::new(r"C:\");
    fs.file(r"C:\seed.bin", MIB);
    let vol = run(&e, &fs).volume_id;
    let months = [1_585_699_200_000i64, 1_588_291_200_000, 1_590_969_600_000];
    let mut april = Vec::new();
    for (m, start) in months.iter().enumerate() {
        for d in 0..10 {
            let id = add_scan(&e, vol, start + d * DAY_MS, &[(r"C:\x", GIB)]);
            if m == 0 {
                april.push(id);
            }
        }
    }
    run(&e, &fs);
    let conn = e.db.conn().unwrap();
    let kept = scans(&conn, vol).unwrap();
    assert_eq!(kept.len(), 21);
    let april_left: Vec<i64> = kept
        .iter()
        .filter(|s| s.finished_ms < months[1])
        .map(|s| s.id)
        .collect();
    assert_eq!(
        april_left,
        vec![*april.last().unwrap()],
        "newest of an old month stays"
    );
    assert_eq!(
        kept.iter()
            .filter(|s| s.finished_ms < months[2] && s.finished_ms >= months[1])
            .count(),
        8
    );
    assert!(summary_dirs(&conn, april[0]).unwrap().is_empty());
    assert_eq!(
        summary_dirs(&conn, *april.last().unwrap()).unwrap().len(),
        1
    );
    assert!(node_by_path(&conn, r"C:\seed.bin").unwrap().is_some());
}

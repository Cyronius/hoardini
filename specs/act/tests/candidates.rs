use std::path::Path;

use hoardini_core::compress::{self, CompressCandidate};
use hoardini_core::index::{FakeFs, RefreshOptions, refresh};
use hoardini_core::policy::PolicyEnv;
use hoardini_core::store::Db;

const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;
const DAY: i64 = 86_400_000;
const T0: i64 = 1_700_000_000_000;
const NOW: i64 = T0 + 365 * DAY;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x800;

struct Disk {
    _tmp: tempfile::TempDir,
    db: Db,
    env: PolicyEnv,
}

fn disk() -> Disk {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("h.db")).unwrap();
    let env = PolicyEnv::with_paths(Path::new(r"C:\Users\me"), Path::new(r"D:\hoardini-data"));
    let fs = FakeFs::new(r"C:\");
    fs.file(r"C:\data\corpus\a.txt", 2 * GIB);
    fs.file(r"C:\data\corpus\b.txt", 3 * GIB / 2);
    fs.file(r"C:\data\recent\x.txt", 3 * GIB);
    fs.touch(r"C:\data\recent\x.txt", NOW - 5 * DAY);
    fs.file_alloc(r"C:\data\packed\a.db", 5 * GIB, 8 * GIB);
    fs.file(r"C:\data\ntfs\x.txt", 3 * GIB);
    fs.set_attrs(
        r"C:\data\ntfs",
        FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_COMPRESSED,
        0,
    );
    fs.file(r"C:\data\linked\big.txt", 3 * GIB);
    fs.junction(r"C:\data\linked\j");
    fs.file(r"C:\media\v1.mp4", 3 * GIB / 2);
    fs.file(r"C:\media\v2.mp4", 3 * GIB / 2);
    fs.file(r"C:\small\x.txt", GIB);
    fs.file(r"C:\games\steamapps\common\GameA\data.pak", 3 * GIB);
    fs.file(r"C:\games\steamapps\common\GameB\data.pak", 4 * GIB);
    fs.file(
        r"C:\games\steamapps\common\GameB\movies\intro.bk2",
        100 * MIB,
    );
    fs.file(r"C:\vm\disk.vhdx", 10 * GIB);
    fs.file(r"C:\vm\notes.txt", 2 * MIB);
    fs.file(r"C:\Windows\big\x.bin", 3 * GIB);
    fs.file(r"C:\Users\me\AppData\Local\Temp\old\x.bin", 3 * GIB);
    fs.file(r"C:\Users\me\Datasets\set\x.csv", 3 * GIB);
    fs.file(r"C:\proj\old\x.log", 5 * GIB / 2);
    fs.file(r"C:\proj\active.txt", GIB);
    fs.touch(r"C:\proj\active.txt", NOW - DAY);
    let out = refresh(&db.conn().unwrap(), &fs, r"C:\", &RefreshOptions::default()).unwrap();
    assert!(out.completed);
    Disk { _tmp: tmp, db, env }
}

fn listed(d: &Disk) -> Vec<CompressCandidate> {
    compress::candidates(&d.db.conn().unwrap(), &d.env, r"C:\", NOW).unwrap()
}

// Traces: ACT-COMPRESS-CANDIDATES
#[test]
fn candidates_are_old_uncompressed_compressible_folders_of_2_gib_or_more() {
    let d = disk();
    let got: Vec<(String, u64)> = listed(&d).into_iter().map(|c| (c.path, c.size)).collect();
    assert_eq!(
        got,
        vec![
            (
                r"C:\games\steamapps\common\GameB".to_string(),
                4 * GIB + 100 * MIB
            ),
            (r"C:\data\corpus".to_string(), 7 * GIB / 2),
            (r"C:\Users\me\Datasets\set".to_string(), 3 * GIB),
            (r"C:\games\steamapps\common\GameA".to_string(), 3 * GIB),
            (r"C:\proj\old".to_string(), 5 * GIB / 2),
        ]
    );
}

// Traces: ACT-COMPRESS-CANDIDATES
#[test]
fn a_cached_estimate_rides_along_while_the_folder_is_unchanged() {
    let d = disk();
    let corpus = listed(&d)
        .into_iter()
        .find(|c| c.path == r"C:\data\corpus")
        .unwrap();
    assert!(corpus.estimate.is_none() && !corpus.proposable());
    let conn = d.db.conn().unwrap();
    let put = |size: u64, saved: u64| {
        conn.execute(
            "INSERT OR REPLACE INTO compress_estimate(path, size, newest_ms, logical, saved_bytes, ratio, estimated_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, 0.5, 1)",
            rusqlite::params![
                corpus.path,
                size as i64,
                corpus.newest_ms,
                (7 * GIB / 2) as i64,
                saved as i64
            ],
        )
        .unwrap();
    };
    put(corpus.size, 2 * GIB);
    let c = listed(&d)
        .into_iter()
        .find(|c| c.path == r"C:\data\corpus")
        .unwrap();
    assert_eq!(c.estimate.as_ref().unwrap().saved_bytes, 2 * GIB);
    assert!(c.proposable());
    put(corpus.size + 4096, 2 * GIB);
    let stale = listed(&d)
        .into_iter()
        .find(|c| c.path == r"C:\data\corpus")
        .unwrap();
    assert!(stale.estimate.is_none(), "a size change makes it stale");
}

// Traces: ACT-COMPRESS-CANDIDATES
#[test]
fn proposable_needs_1_gib_and_20_percent() {
    assert!(compress::proposable(GIB, 5 * GIB));
    assert!(!compress::proposable(GIB - 1, 4 * GIB));
    assert!(!compress::proposable(3 * GIB / 2, 8 * GIB));
    assert!(compress::proposable(2 * GIB, 10 * GIB));
    assert!(!compress::proposable(2 * GIB - 1, 10 * GIB));
}

fn text(n: usize) -> Vec<u8> {
    b"compressible text line 0123456789\n"
        .iter()
        .copied()
        .cycle()
        .take(n)
        .collect()
}

// Traces: ACT-COMPRESS-CANDIDATES
#[test]
fn the_top_candidates_are_estimated_once_and_again_after_a_change() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("h.db")).unwrap();
    let mut cands = Vec::new();
    for name in ["first", "second"] {
        let dir = tmp.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("log.txt"), text(4 << 20)).unwrap();
        cands.push(CompressCandidate {
            path: dir.to_string_lossy().into_owned(),
            size: 4 << 20,
            logical: 4 << 20,
            files: 1,
            newest_ms: Some(1),
            estimate: None,
        });
    }
    assert_eq!(compress::estimate_top(&db, &cands, 1).unwrap(), 1);
    let rows: Vec<(String, i64, f64)> = db
        .conn()
        .unwrap()
        .prepare("SELECT path, saved_bytes, ratio FROM compress_estimate")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, cands[0].path);
    assert!(rows[0].1 > 2 << 20 && rows[0].2 < 0.5, "{rows:?}");

    assert_eq!(compress::estimate_top(&db, &cands, 1).unwrap(), 0);
    assert_eq!(compress::estimate_top(&db, &cands, 2).unwrap(), 1);
    cands[0].newest_ms = Some(2);
    assert_eq!(compress::estimate_top(&db, &cands, 2).unwrap(), 1);
}

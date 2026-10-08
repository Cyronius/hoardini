use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use hoardini_core::evidence::{PathKind, inspect};
use hoardini_core::index::{self, RefreshOptions, WinLister, node_by_path};
use hoardini_core::store::Db;

use super::Gguf;

const MS_2020: i64 = 1_577_836_800_000;

struct Fx {
    dir: tempfile::TempDir,
    db: Db,
}

impl Fx {
    fn new() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("h.db")).unwrap();
        Fx { dir, db }
    }

    fn root(&self) -> PathBuf {
        let r = self.dir.path().join("data");
        std::fs::create_dir_all(&r).unwrap();
        r
    }

    fn index(&self, root: &Path) {
        index::set_threshold_setting(&self.db.conn().unwrap(), 1).unwrap();
        let out = index::refresh(
            &self.db.conn().unwrap(),
            &WinLister,
            &root.to_string_lossy(),
            &RefreshOptions {
                workers: 2,
                ..RefreshOptions::default()
            },
        )
        .unwrap();
        assert!(out.completed);
    }
}

fn write_gguf(path: &Path) {
    let mut g = Gguf::new(1, 2);
    g.kv_str("general.architecture", "llama");
    g.kv_str("general.name", "Fixture 7B");
    g.tensor("w", &[1000, 1000], 1);
    let mut b = g.bytes;
    b.resize(b.len() + (256 << 10), 0);
    std::fs::write(path, b).unwrap();
}

fn atime(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path).unwrap().accessed().unwrap()
}

// Traces: EVID-ATIME-PRESERVE
#[test]
fn inspect_leaves_atime_unchanged_cached_or_not() {
    let fx = Fx::new();
    let root = fx.root();
    let f = root.join("m.gguf");
    write_gguf(&f);
    hoardini_win::set_file_times(&f, Some(MS_2020), None).unwrap();
    let before = atime(&f);

    let ev = inspect(&fx.db, &f).unwrap();
    assert_eq!(ev.identity.as_ref().unwrap().kind, "gguf");
    assert_eq!(atime(&f), before, "unindexed inspect");
    inspect(&fx.db, &f).unwrap();
    assert_eq!(atime(&f), before, "second unindexed inspect");

    fx.index(&root);
    inspect(&fx.db, &f).unwrap();
    assert_eq!(atime(&f), before, "indexed, cache miss");
    inspect(&fx.db, &f).unwrap();
    assert_eq!(atime(&f), before, "indexed, cache hit");
}

// Traces: EVID-ATIME-PRESERVE
#[test]
fn unreadable_file_is_unknown_no_access_without_reading() {
    use std::os::windows::fs::OpenOptionsExt;
    let fx = Fx::new();
    let f = fx.root().join("locked.gguf");
    write_gguf(&f);
    // A no-share handle makes the preserving open fail.
    let _hold = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&f)
        .unwrap();
    let ev = inspect(&fx.db, &f).unwrap();
    let id = ev.identity.unwrap();
    assert_eq!(id.note.as_deref(), Some("unknown (no access)"));
    assert!(id.gguf.is_none());
}

// Traces: EVID-ATIME-PRESERVE
#[test]
fn cloud_placeholder_is_never_read() {
    let fx = Fx::new();
    let f = fx.root().join("cloud.gguf");
    write_gguf(&f);
    let st = std::process::Command::new("attrib")
        .arg("+O")
        .arg(&f)
        .status()
        .unwrap();
    assert!(st.success(), "attrib +O");
    let ev = inspect(&fx.db, &f).unwrap();
    let id = ev.identity.unwrap();
    assert_eq!(id.kind, "cloud placeholder");
    assert!(id.gguf.is_none());
}

// Traces: EVID-IN-USE
#[test]
fn file_holder_is_listed_and_released() {
    let fx = Fx::new();
    let f = fx.root().join("held.bin");
    std::fs::write(&f, vec![1u8; 4096]).unwrap();
    assert!(inspect(&fx.db, &f).unwrap().in_use.is_empty());

    let held = File::open(&f).unwrap();
    let ev = inspect(&fx.db, &f).unwrap();
    assert!(
        ev.in_use.iter().any(|p| p.pid == std::process::id()),
        "{:?}",
        ev.in_use
    );
    assert!(ev.summary.contains("IN USE by"), "{}", ev.summary);
    drop(held);
    assert!(inspect(&fx.db, &f).unwrap().in_use.is_empty());
}

// Traces: EVID-IN-USE
#[test]
fn directory_reports_holders_of_files_inside() {
    let fx = Fx::new();
    let root = fx.root();
    let sub = root.join("models").join("m1");
    std::fs::create_dir_all(&sub).unwrap();
    let mut w = File::create(sub.join("weights.bin")).unwrap();
    w.write_all(&vec![7u8; 300_000]).unwrap();
    w.sync_all().unwrap();
    std::fs::write(sub.join("small.txt"), b"x").unwrap();
    fx.index(&root);

    let ev = inspect(&fx.db, &root.join("models")).unwrap();
    assert_eq!(ev.kind, PathKind::Dir);
    assert!(
        ev.in_use.iter().any(|p| p.pid == std::process::id()),
        "{:?}",
        ev.in_use
    );
    drop(w);
    assert!(
        inspect(&fx.db, &root.join("models"))
            .unwrap()
            .in_use
            .is_empty()
    );
}

// Traces: EVID-IN-USE
#[test]
fn directory_sizes_come_from_index_totals() {
    let fx = Fx::new();
    let root = fx.root();
    let d = root.join("pkg");
    std::fs::create_dir_all(d.join("inner")).unwrap();
    std::fs::write(d.join("a.bin"), vec![1u8; 200_000]).unwrap();
    std::fs::write(d.join("inner").join("b.bin"), vec![2u8; 100_000]).unwrap();
    fx.index(&root);

    let node = node_by_path(&fx.db.conn().unwrap(), &d.to_string_lossy())
        .unwrap()
        .unwrap();
    let ev = inspect(&fx.db, &d).unwrap();
    assert_eq!(ev.alloc_bytes, Some(node.size));
    assert_eq!(ev.logical_bytes, Some(300_000));
    assert_eq!(ev.file_count, Some(2));
    assert!(ev.summary.starts_with("folder, "), "{}", ev.summary);
}

#[test]
fn model_folder_is_identified_by_its_largest_file() {
    let fx = Fx::new();
    let root = fx.root();
    let d = root.join("llama");
    std::fs::create_dir_all(&d).unwrap();
    write_gguf(&d.join("m.gguf"));
    std::fs::write(d.join("readme.md"), b"hi").unwrap();
    fx.index(&root);
    let ev = inspect(&fx.db, &d).unwrap();
    assert_eq!(ev.identity.as_ref().unwrap().kind, "gguf");
    assert!(
        ev.summary.starts_with("folder of GGUF model"),
        "{}",
        ev.summary
    );
}

#[test]
fn cache_is_reused_until_the_file_changes() {
    let fx = Fx::new();
    let root = fx.root();
    let f = root.join("c.gguf");
    write_gguf(&f);
    fx.index(&root);
    inspect(&fx.db, &f).unwrap();

    let conn = fx.db.conn().unwrap();
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM evidence_cache", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    conn.execute(
        "UPDATE evidence_cache SET json = json_set(json, '$.identity.kind', 'from-cache')",
        [],
    )
    .unwrap();
    let hit = inspect(&fx.db, &f).unwrap();
    assert_eq!(hit.identity.unwrap().kind, "from-cache");

    hoardini_win::set_file_times(&f, None, Some(MS_2020)).unwrap();
    let miss = inspect(&fx.db, &f).unwrap();
    assert_eq!(miss.identity.unwrap().kind, "gguf");
}

#[test]
fn missing_path_reports_missing() {
    let fx = Fx::new();
    let ev = inspect(&fx.db, &fx.root().join("nope.bin")).unwrap();
    assert_eq!(ev.kind, PathKind::Missing);
    assert_eq!(ev.summary, "missing");
}

#[test]
fn summary_is_one_line_with_the_decisive_facts() {
    let fx = Fx::new();
    let f = fx.root().join("s.gguf");
    write_gguf(&f);
    let ev = inspect(&fx.db, &f).unwrap();
    assert!(!ev.summary.contains('\n'));
    assert!(
        ev.summary.starts_with("GGUF model, Fixture 7B, llama"),
        "{}",
        ev.summary
    );
    assert!(ev.last_used.iter().any(|s| s.source == "mtime"));
}

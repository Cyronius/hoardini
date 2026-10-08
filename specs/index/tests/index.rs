use std::collections::BTreeSet;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use hoardini_core::index::{
    self, DirLister, FakeFs, NodeKind, RefreshOptions, RefreshOutcome, SnapRow, WinLister,
    node_by_path, patch_removed, patch_rescan, refresh, snapshot, verify_totals,
};
use hoardini_core::store::Db;

const KIB: u64 = 1 << 10;
const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

struct Env {
    _dir: tempfile::TempDir,
    db: Db,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("h.db")).unwrap();
    Env { _dir: dir, db }
}

fn opts(workers: usize) -> RefreshOptions<'static> {
    RefreshOptions {
        workers,
        ..RefreshOptions::default()
    }
}

fn run(e: &Env, lister: &dyn DirLister, root: &str, workers: usize) -> RefreshOutcome {
    let out = refresh(&e.db.conn().unwrap(), lister, root, &opts(workers)).unwrap();
    assert!(out.completed);
    out
}

fn fresh(fs: &FakeFs, threshold: Option<u64>) -> Vec<SnapRow> {
    let e = env();
    let conn = e.db.conn().unwrap();
    if let Some(t) = threshold {
        index::set_threshold_setting(&conn, t).unwrap();
    }
    run(&e, fs, fs.root(), 3);
    snapshot(&conn, fs.root()).unwrap()
}

fn assert_same(got: &[SnapRow], want: &[SnapRow]) {
    for (g, w) in got.iter().zip(want.iter()) {
        assert_eq!(g, w, "first differing row");
    }
    let gp: Vec<&str> = got.iter().map(|r| r.path.as_str()).collect();
    let wp: Vec<&str> = want.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(gp, wp, "row sets differ");
}

fn assert_consistent(e: &Env) {
    let problems = verify_totals(&e.db.conn().unwrap()).unwrap();
    assert!(problems.is_empty(), "{problems:#?}");
}

fn size(e: &Env, path: &str) -> u64 {
    node_by_path(&e.db.conn().unwrap(), path)
        .unwrap()
        .unwrap_or_else(|| panic!("{path} not indexed"))
        .size
}

fn sample_tree() -> FakeFs {
    let fs = FakeFs::new(r"C:\");
    for i in 0..10 {
        fs.file(&format!(r"C:\Users\josha\.cache\pip\f{i}.whl"), 50 * KIB);
    }
    fs.file(r"C:\Users\josha\.cache\pip\big0.whl", 3 * MIB);
    fs.file(r"C:\Users\josha\.cache\pip\big1.whl", 3 * MIB);
    fs.file(r"C:\Users\josha\models\a.gguf", 500 * MIB);
    fs.file(r"C:\Users\josha\models\b.gguf", 300 * MIB);
    fs.file(r"C:\Users\josha\Downloads\setup.exe", 80 * MIB);
    fs.file(r"C:\Users\josha\Downloads\notes.txt", 2 * KIB);
    fs.file(r"C:\Users\josha\Downloads\thing", 2 * MIB);
    for i in 0..20 {
        let sz = if i % 4 == 0 { 2 * MIB } else { 30 * KIB };
        fs.file(&format!(r"C:\code\proj\target\debug\o{i}.rlib"), sz);
    }
    fs.file(r"C:\code\proj\src\main.rs", 5 * KIB);
    fs.file(r"C:\code\old\x.bin", 40 * MIB);
    fs.file(r"C:\code\old\deep\y.bin", 4 * MIB);
    fs.file(r"C:\code\old\deep\z.txt", 4 * KIB);
    fs.junction(r"C:\link-me");
    fs.file(r"C:\shared\big.bin", 50 * MIB);
    fs.hard_link(r"C:\shared\big.bin", r"C:\Users\josha\models\big-link.bin");
    fs.file(r"C:\shared\tiny.txt", 10 * KIB);
    fs.hard_link(r"C:\shared\tiny.txt", r"C:\shared\tiny2.txt");
    fs.file(r"C:\locked\inside.bin", 9 * MIB);
    fs.deny(r"C:\locked");
    fs.wof_file(r"C:\wof\sys.dll", 4 * MIB, MIB);
    fs.placeholder(r"C:\Users\josha\OneDrive\video.mp4", 3 * GIB);
    fs
}

// Traces: INDEX-ALLOC-SIZE
#[test]
fn sizes_are_allocated_bytes_with_wof_sparse_and_placeholders() {
    let fs = FakeFs::new(r"C:\");
    fs.file_alloc(r"C:\data\a.bin", 8 * MIB, 5 * MIB);
    fs.sparse_file(r"C:\data\sparse.img", MIB, 10 * GIB);
    fs.sparse_file(r"C:\data\empty-sparse.img", 0, 2 * MIB);
    fs.wof_file(r"C:\data\wof.dll", 4 * MIB, 20 * KIB);
    fs.placeholder(r"C:\data\cloud.mp4", 5 * GIB);
    fs.file(r"C:\data\tiny.txt", 100);
    let e = env();
    run(&e, &fs, r"C:\", 2);
    let conn = e.db.conn().unwrap();
    let get = |p: &str| node_by_path(&conn, p).unwrap().unwrap();

    assert_eq!(get(r"C:\data\a.bin").size, 8 * MIB);
    assert_eq!(get(r"C:\data\a.bin").logical, 5 * MIB);
    assert_eq!(get(r"C:\data\wof.dll").size, 20 * KIB);
    assert_eq!(get(r"C:\data\wof.dll").logical, 4 * MIB);
    assert_eq!(get(r"C:\data\sparse.img").size, MIB);
    assert_eq!(get(r"C:\data\empty-sparse.img").size, 0);
    let cloud = get(r"C:\data\cloud.mp4");
    assert_eq!((cloud.size, cloud.logical), (0, 5 * GIB));
    assert!(cloud.placeholder);
    assert_eq!(get(r"C:\data").size, 8 * MIB + MIB + 20 * KIB + 4 * KIB);
    assert_eq!(fs.opened(), vec![r"C:\data\wof.dll".to_string()]);
    assert_consistent(&e);
}

// Traces: INDEX-ALLOC-SIZE
#[test]
fn real_directory_sizes_and_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("tree");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(
        root.join("sub").join("big.bin"),
        vec![7u8; 2 * MIB as usize + 1],
    )
    .unwrap();
    std::fs::write(
        root.join("sub").join("mid.bin"),
        vec![1u8; 70 * KIB as usize],
    )
    .unwrap();
    for i in 0..3 {
        std::fs::write(root.join("sub").join(format!("t{i}.txt")), b"0123456789").unwrap();
    }
    std::fs::write(root.join("top.txt"), b"hello").unwrap();
    let e = env();
    let root_s = root.to_string_lossy().into_owned();
    run(&e, &WinLister, &root_s, 2);
    let conn = e.db.conn().unwrap();
    let sub = node_by_path(&conn, &format!(r"{root_s}\sub"))
        .unwrap()
        .unwrap();
    let big = node_by_path(&conn, &format!(r"{root_s}\sub\big.bin"))
        .unwrap()
        .unwrap();
    assert_eq!(big.kind, NodeKind::File);
    assert_eq!(big.logical, 2 * MIB + 1);
    assert!(big.size >= big.logical && big.size < big.logical + MIB);
    assert!(
        node_by_path(&conn, &format!(r"{root_s}\sub\mid.bin"))
            .unwrap()
            .is_none()
    );
    assert_eq!(sub.small_count, 4);
    assert_eq!(sub.files, 5);
    assert_eq!(sub.logical, 2 * MIB + 1 + 70 * KIB + 30);
    let top = node_by_path(&conn, &root_s).unwrap().unwrap();
    assert_eq!(top.files, 6);
    assert_eq!(top.logical, sub.logical + 5);
    assert_consistent(&e);
}

// Traces: INDEX-HARDLINK-ONCE
#[test]
fn hard_links_count_once_with_a_deterministic_owner() {
    let fs = FakeFs::new(r"C:\");
    fs.file(r"C:\b\model.gguf", 200 * MIB);
    fs.hard_link(r"C:\b\model.gguf", r"C:\a\copy.gguf");
    fs.hard_link(r"C:\b\model.gguf", r"C:\c\third.gguf");
    fs.file(r"C:\c\small.txt", 100 * KIB);
    fs.hard_link(r"C:\c\small.txt", r"C:\d\small2.txt");
    let unknown = [0u8; 16];
    fs.file_with_id(r"C:\z\one.bin", 3 * MIB, 3 * MIB, unknown);
    fs.file_with_id(r"C:\z\two.bin", 3 * MIB, 3 * MIB, unknown);
    let model = 200 * MIB;
    let small = 100 * KIB;
    for fs_name in ["NTFS", "ReFS"] {
        fs.set_fs_name(fs_name);
        for _ in 0..4 {
            let e = env();
            run(&e, &fs, r"C:\", 4);
            let conn = e.db.conn().unwrap();
            let get = |p: &str| node_by_path(&conn, p).unwrap().unwrap();
            assert_eq!(get(r"C:\").size, model + small + 6 * MIB);
            assert!(
                !get(r"C:\a\copy.gguf").hardlink_dup,
                "smallest path owns the bytes"
            );
            assert!(get(r"C:\b\model.gguf").hardlink_dup);
            assert!(get(r"C:\c\third.gguf").hardlink_dup);
            assert_eq!(get(r"C:\a").size, model);
            assert_eq!(get(r"C:\b").size, 0);
            assert_eq!(
                get(r"C:\c").small_bytes + get(r"C:\d").small_bytes,
                small,
                "small links fold once"
            );
            assert_eq!(get(r"C:\z").size, 6 * MIB, "unknown ids are never merged");
            assert_consistent(&e);
        }
    }
}

// Traces: INDEX-HARDLINK-ONCE
#[test]
fn real_hard_link_counts_once_when_ids_are_known() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("tree");
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::create_dir_all(root.join("b")).unwrap();
    let f = root.join("a").join("f.bin");
    std::fs::write(&f, vec![3u8; 2 * MIB as usize]).unwrap();
    std::fs::hard_link(&f, root.join("b").join("g.bin")).unwrap();
    let ids_known = hoardini_win::list_dir(&root.join("a"))
        .unwrap()
        .iter()
        .any(|e| e.file_id != [0; 16]);
    let e = env();
    let root_s = root.to_string_lossy().into_owned();
    run(&e, &WinLister, &root_s, 2);
    let top = node_by_path(&e.db.conn().unwrap(), &root_s)
        .unwrap()
        .unwrap();
    let expected_links = if ids_known { 1 } else { 2 };
    assert_eq!(top.files, expected_links);
    assert_eq!(top.logical, expected_links * 2 * MIB);
    assert_consistent(&e);
}

// Traces: INDEX-NO-FOLLOW
#[test]
fn junctions_and_symlinks_are_recorded_not_followed() {
    let fs = FakeFs::new(r"C:\");
    fs.junction(r"C:\j");
    fs.file(r"C:\j\x.bin", 5 * GIB);
    fs.symlink_dir(r"C:\s");
    fs.file(r"C:\s\y.bin", 5 * GIB);
    fs.reparse_dir(r"C:\t", 0);
    fs.file(r"C:\t\z.bin", 5 * GIB);
    fs.symlink_file(r"C:\f.lnk");
    fs.reparse_dir(r"C:\cloud", 0x9000_601A);
    fs.file(r"C:\cloud\doc.bin", 2 * MIB);
    let e = env();
    run(&e, &fs, r"C:\", 2);
    let listed = fs.listed();
    for p in [r"C:\j", r"C:\s", r"C:\t"] {
        assert!(
            !listed.iter().any(|l| l.eq_ignore_ascii_case(p)),
            "{p} was listed"
        );
    }
    assert!(listed.iter().any(|l| l == r"C:\cloud"));
    let conn = e.db.conn().unwrap();
    for p in [r"C:\j", r"C:\s", r"C:\t", r"C:\f.lnk"] {
        let n = node_by_path(&conn, p).unwrap().unwrap();
        assert_eq!((n.kind, n.size), (NodeKind::Link, 0), "{p}");
    }
    assert!(node_by_path(&conn, r"C:\j\x.bin").unwrap().is_none());
    assert_eq!(size(&e, r"C:\"), 2 * MIB);
    assert_consistent(&e);
}

// Traces: INDEX-NO-FOLLOW
#[test]
fn real_junction_is_not_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("tree");
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(
        root.join("target").join("big.bin"),
        vec![1u8; 2 * MIB as usize],
    )
    .unwrap();
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(root.join("link"))
        .arg(root.join("target"))
        .output()
        .unwrap();
    assert!(status.status.success(), "mklink /J failed: {status:?}");
    let e = env();
    let root_s = root.to_string_lossy().into_owned();
    run(&e, &WinLister, &root_s, 2);
    let conn = e.db.conn().unwrap();
    let link = node_by_path(&conn, &format!(r"{root_s}\link"))
        .unwrap()
        .unwrap();
    assert_eq!(link.kind, NodeKind::Link);
    assert!(
        node_by_path(&conn, &format!(r"{root_s}\link\big.bin"))
            .unwrap()
            .is_none()
    );
    let top = node_by_path(&conn, &root_s).unwrap().unwrap();
    assert_eq!((top.files, top.logical), (1, 2 * MIB));
}

fn lcg(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *seed >> 33
}

// Traces: INDEX-SMALL-FOLD
#[test]
fn small_files_fold_into_parents_and_totals_add_up() {
    let fs = FakeFs::new(r"C:\");
    let mut seed = 42u64;
    let mut total_alloc = 0u64;
    let mut total_logical = 0u64;
    let mut files = 0u64;
    let mut expect_rows = Vec::new();
    let mut expect_small = Vec::new();
    for a in 0..4 {
        for b in 0..3 {
            let dir = format!(r"C:\d{a}\e{b}");
            let mut small = (0u64, 0u64);
            for f in 0..12 {
                let logical = match lcg(&mut seed) % 4 {
                    0 => MIB + lcg(&mut seed) % (4 * MIB),
                    1 => MIB - 1,
                    _ => lcg(&mut seed) % (300 * KIB),
                };
                let alloc = logical.div_ceil(4096) * 4096;
                let p = format!(r"{dir}\f{f}.dat");
                fs.file(&p, logical);
                total_alloc += alloc;
                total_logical += logical;
                files += 1;
                if alloc.max(logical) >= MIB {
                    expect_rows.push((p, alloc));
                } else {
                    small.0 += 1;
                    small.1 += alloc;
                }
            }
            expect_small.push((dir, small));
        }
    }
    let e = env();
    run(&e, &fs, r"C:\", 3);
    let conn = e.db.conn().unwrap();
    for (p, alloc) in &expect_rows {
        assert_eq!(node_by_path(&conn, p).unwrap().unwrap().size, *alloc, "{p}");
    }
    for (dir, (count, bytes)) in &expect_small {
        let n = node_by_path(&conn, dir).unwrap().unwrap();
        assert_eq!((n.small_count, n.small_bytes), (*count, *bytes), "{dir}");
        for f in 0..12 {
            let p = format!(r"{dir}\f{f}.dat");
            let has_row = node_by_path(&conn, &p).unwrap().is_some();
            assert_eq!(has_row, expect_rows.iter().any(|(r, _)| *r == p), "{p}");
        }
    }
    let root = node_by_path(&conn, r"C:\").unwrap().unwrap();
    assert_eq!(
        (root.size, root.logical, root.files),
        (total_alloc, total_logical, files)
    );
    assert_consistent(&e);
}

// Traces: INDEX-THRESHOLD
#[test]
fn threshold_defaults_to_1_mib_and_changes_apply_at_next_refresh() {
    let fs = sample_tree();
    fs.file(r"C:\edge\exact.bin", MIB);
    fs.file_alloc(r"C:\edge\below.bin", MIB - 4096, MIB - 1);
    let e = env();
    let conn = e.db.conn().unwrap();
    run(&e, &fs, r"C:\", 2);
    let st = index::index_status(&conn, Some(r"C:\")).unwrap().unwrap();
    assert_eq!(st.threshold_bytes, Some(MIB));
    assert!(node_by_path(&conn, r"C:\edge\exact.bin").unwrap().is_some());
    assert!(node_by_path(&conn, r"C:\edge\below.bin").unwrap().is_none());

    for t in [64 * KIB, 4 * MIB, MIB] {
        index::set_threshold_setting(&conn, t).unwrap();
        run(&e, &fs, r"C:\", 2);
        assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, Some(t)));
        assert_consistent(&e);
    }
}

fn mutate_round_one(fs: &FakeFs) {
    fs.file(r"C:\Users\josha\models\c.gguf", 700 * MIB);
    fs.file(r"C:\code\new\x\y.bin", 2 * MIB);
    fs.file(r"C:\code\new\x\y.txt", 3 * KIB);
    fs.remove(r"C:\code\old");
    fs.remove(r"C:\Users\josha\Downloads\notes.txt");
    fs.remove(r"C:\Users\josha\models\b.gguf");
    fs.resize(r"C:\Users\josha\models\a.gguf", 600 * MIB, 600 * MIB);
    fs.resize(r"C:\Users\josha\.cache\pip\f3.whl", 2 * MIB, 2 * MIB);
    fs.resize(r"C:\Users\josha\.cache\pip\big1.whl", 100 * KIB, 100 * KIB);
    fs.rename(r"C:\code\proj", r"C:\code\project");
    fs.rename(
        r"C:\Users\josha\Downloads\setup.exe",
        r"C:\Users\josha\Downloads\setup-old.exe",
    );
    fs.remove(r"C:\Users\josha\Downloads\thing");
    fs.file(r"C:\Users\josha\Downloads\thing\inner.bin", 2 * MIB);
    fs.allow(r"C:\locked");
    fs.file(r"C:\code\secret\s.bin", 7 * MIB);
    fs.deny(r"C:\code\secret");
    fs.remove(r"C:\shared\big.bin");
    fs.hard_link(r"C:\Users\josha\models\a.gguf", r"C:\code\new\link2.bin");
    fs.resize(r"C:\wof\sys.dll", 2 * MIB, 4 * MIB);
    fs.touch(r"C:\code\project\src\main.rs", 1_800_000_000_000);
}

fn mutate_round_two(fs: &FakeFs) {
    fs.remove(r"C:\Users\josha\.cache\pip");
    fs.junction(r"C:\Users\josha\.cache\pip");
    fs.rename(r"C:\Users\josha\models", r"C:\Users\josha\llm");
    fs.file(r"C:\Users\josha\llm\d.gguf", 900 * MIB);
    fs.remove(r"C:\code\new\link2.bin");
    fs.remove(r"C:\link-me");
    fs.dir(r"C:\link-me");
    fs.file(r"C:\link-me\now-real.bin", 3 * MIB);
    fs.placeholder(r"C:\Users\josha\OneDrive\more.mp4", GIB);
}

// Traces: INDEX-REFRESH-EQUIV
#[test]
fn refresh_after_changes_equals_a_fresh_build() {
    let fs = sample_tree();
    let e = env();
    let conn = e.db.conn().unwrap();
    run(&e, &fs, r"C:\", 3);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));

    mutate_round_one(&fs);
    run(&e, &fs, r"C:\", 3);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));
    assert_consistent(&e);

    mutate_round_two(&fs);
    run(&e, &fs, r"C:\", 3);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));
    assert_consistent(&e);

    run(&e, &fs, r"C:\", 1);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));
}

// Traces: INDEX-ACTION-PATCH
#[test]
fn executed_actions_patch_rows_and_every_ancestor() {
    let fs = sample_tree();
    fs.file(r"C:\code\dup\a-copy.gguf", 500 * MIB);
    let e = env();
    let conn = e.db.conn().unwrap();
    run(&e, &fs, r"C:\", 2);

    let b = r"C:\Users\josha\models\b.gguf";
    let b_size = size(&e, b);
    let before: Vec<u64> = [
        r"C:\Users\josha\models",
        r"C:\Users\josha",
        r"C:\Users",
        r"C:\",
    ]
    .iter()
    .map(|p| size(&e, p))
    .collect();
    fs.remove(b);
    assert!(patch_removed(&conn, b).unwrap());
    assert!(node_by_path(&conn, b).unwrap().is_none());
    for (p, was) in [
        r"C:\Users\josha\models",
        r"C:\Users\josha",
        r"C:\Users",
        r"C:\",
    ]
    .iter()
    .zip(before)
    {
        assert_eq!(size(&e, p), was - b_size, "{p}");
    }
    assert_consistent(&e);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));

    fs.remove(r"C:\code\old");
    assert!(patch_removed(&conn, r"C:\code\old").unwrap());
    assert!(
        node_by_path(&conn, r"C:\code\old\deep\y.bin")
            .unwrap()
            .is_none()
    );
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));

    let a = r"C:\Users\josha\models\a.gguf";
    fs.resize(a, 100 * MIB, 500 * MIB);
    let n = patch_rescan(&conn, &fs, a).unwrap().unwrap();
    assert_eq!((n.size, n.logical), (100 * MIB, 500 * MIB));
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));

    let pip = r"C:\Users\josha\.cache\pip";
    let small_before = node_by_path(&conn, pip).unwrap().unwrap().small_count;
    fs.remove(r"C:\Users\josha\.cache\pip\f3.whl");
    assert!(
        patch_rescan(&conn, &fs, r"C:\Users\josha\.cache\pip\f3.whl")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        node_by_path(&conn, pip).unwrap().unwrap().small_count,
        small_before - 1
    );
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));

    fs.remove(r"C:\code\dup\a-copy.gguf");
    fs.hard_link(a, r"C:\code\dup\a-copy.gguf");
    patch_rescan(&conn, &fs, r"C:\code\dup\a-copy.gguf").unwrap();
    assert!(node_by_path(&conn, a).unwrap().unwrap().hardlink_dup);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));

    fs.remove(r"C:\code\dup\a-copy.gguf");
    assert!(patch_removed(&conn, r"C:\code\dup\a-copy.gguf").unwrap());
    assert!(!node_by_path(&conn, a).unwrap().unwrap().hardlink_dup);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));

    fs.remove(r"C:\Users\josha\Downloads");
    assert!(
        patch_rescan(&conn, &fs, r"C:\Users\josha\Downloads")
            .unwrap()
            .is_none()
    );
    fs.file(r"C:\code\project-new\big.bin", 9 * MIB);
    patch_rescan(&conn, &fs, r"C:\code\project-new").unwrap();
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));
    assert_consistent(&e);
}

fn paths(rows: &[SnapRow]) -> BTreeSet<String> {
    rows.iter().map(|r| r.path.to_lowercase()).collect()
}

// Traces: INDEX-INTERRUPT-SAFE
#[test]
fn interrupted_refresh_deletes_nothing_and_stays_consistent() {
    let fs = sample_tree();
    let e = env();
    let conn = e.db.conn().unwrap();
    run(&e, &fs, r"C:\", 2);
    let before = paths(&snapshot(&conn, r"C:\").unwrap());
    let vol = index::index_status(&conn, None).unwrap().unwrap().volume_id;
    let scans_before = hoardini_core::history::scans(&conn, vol).unwrap().len();
    mutate_round_one(&fs);

    for stop_after in [1usize, 4, 9] {
        let cancel = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(AtomicUsize::new(0));
        let (c, s) = (cancel.clone(), seen.clone());
        fs.on_list(move |_| {
            if s.fetch_add(1, Ordering::SeqCst) + 1 >= stop_after {
                c.store(true, Ordering::SeqCst);
            }
        });
        let out = refresh(
            &conn,
            &fs,
            r"C:\",
            &RefreshOptions {
                workers: 2,
                cancel: Some(cancel),
                ..RefreshOptions::default()
            },
        )
        .unwrap();
        fs.clear_hook();
        assert!(!out.completed);
        assert_eq!(out.scan_id, None);
        assert_eq!(out.rows_deleted, 0);
        let after = paths(&snapshot(&conn, r"C:\").unwrap());
        let lost: Vec<&String> = before.difference(&after).collect();
        assert!(
            lost.is_empty(),
            "rows deleted by an interrupted refresh: {lost:?}"
        );
        assert_consistent(&e);
    }
    assert_eq!(
        hoardini_core::history::scans(&conn, vol).unwrap().len(),
        scans_before
    );
    run(&e, &fs, r"C:\", 2);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));
}

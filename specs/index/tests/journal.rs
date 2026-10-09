use std::io;
use std::path::Path;
use std::sync::Mutex;

use hoardini_core::index::{
    JournalOutcome, JournalRefresh, JournalSource, NeedsFull, RefreshKind, RefreshRun, WinJournal,
    journal_refresh, refresh_kind,
};
use hoardini_win::{DirEntry, VolumeInfo};

use super::*;

fn jopts<'a>(fs: &'a FakeFs) -> RefreshOptions<'a> {
    RefreshOptions {
        workers: 3,
        journal: Some(fs),
        ..RefreshOptions::default()
    }
}

fn full(e: &Env, fs: &FakeFs) -> RefreshOutcome {
    let out = refresh(&e.db.conn().unwrap(), fs, fs.root(), &jopts(fs)).unwrap();
    assert!(out.completed);
    out
}

fn try_journal(e: &Env, fs: &FakeFs) -> JournalOutcome {
    journal_refresh(
        &e.db.conn().unwrap(),
        fs,
        fs,
        fs.root(),
        &RefreshOptions::default(),
    )
    .unwrap()
}

fn journal(e: &Env, fs: &FakeFs) -> JournalRefresh {
    match try_journal(e, fs) {
        JournalOutcome::Done(j) => {
            assert!(j.outcome.completed);
            *j
        }
        other => panic!("expected a journal refresh, got {other:?}"),
    }
}

fn listed_lower(fs: &FakeFs) -> Vec<String> {
    fs.listed().iter().map(|p| p.to_lowercase()).collect()
}

fn swap_dirs(fs: &FakeFs, a: &str, b: &str) {
    let tmp = format!("{a}-swap");
    fs.rename(a, &tmp);
    fs.rename(b, a);
    fs.rename(&tmp, b);
}

// Traces: INDEX-JOURNAL-EQUIV
#[test]
fn journal_refresh_after_changes_equals_a_fresh_build() {
    let fs = sample_tree();
    fs.file(r"C:\swap\one\x.bin", 2 * MIB);
    fs.file(r"C:\swap\two\y.bin", 3 * MIB);
    fs.file(r"C:\swap\two\deeper\z.bin", 4 * MIB);
    fs.set_journal_page(7);
    fs.track_dir_times(true);
    let e = env();
    let conn = e.db.conn().unwrap();
    full(&e, &fs);

    mutate_round_one(&fs);
    fs.rename(r"C:\code\project\target", r"C:\Users\josha\target-moved");
    fs.rename(r"C:\Users\josha\.cache", r"C:\archive\cache");
    swap_dirs(&fs, r"C:\swap\one", r"C:\swap\two");
    fs.file(r"C:\fresh\a\b\c\deep.bin", 5 * MIB);
    fs.file(r"C:\fresh\a\b\note.txt", 2 * KIB);
    fs.file(r"C:\shared\new-small.txt", 5 * KIB);
    fs.clear_log();
    let j = journal(&e, &fs);
    let listed = listed_lower(&fs);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));
    assert_consistent(&e);
    assert!(j.pages > 1 && j.records > 0);
    assert!(j.moved_dirs >= 3, "{j:?}");
    for p in [
        r"c:\users\josha\target-moved",
        r"c:\users\josha\target-moved\debug",
        r"c:\swap\one\deeper",
        r"c:\users\josha\onedrive",
    ] {
        assert!(
            !listed.contains(&p.to_string()),
            "{p}: moved subtrees are re-parented and untouched dirs skipped: {listed:?}"
        );
    }
    assert!(listed.contains(&r"c:\archive\cache\pip".to_string()));
    let unique: BTreeSet<&String> = listed.iter().collect();
    assert_eq!(unique.len(), listed.len(), "each dir is listed once");

    mutate_round_two(&fs);
    fs.touch(r"C:\code\new", 1_810_000_000_000);
    fs.clear_log();
    journal(&e, &fs);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));
    assert_consistent(&e);

    fs.deny(r"C:\fresh\a");
    journal(&e, &fs);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));
    fs.allow(r"C:\fresh\a");
    let j = journal(&e, &fs);
    assert_eq!(j.outcome.denied, 0);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));

    fs.clear_log();
    let idle = journal(&e, &fs);
    assert_eq!((idle.records, idle.outcome.dirs), (0, 0));
    assert!(fs.listed().is_empty(), "{:?}", fs.listed());
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));
    assert_consistent(&e);
    let vol = index::index_status(&conn, Some(r"C:\"))
        .unwrap()
        .unwrap()
        .volume_id;
    let kinds: Vec<String> = hoardini_core::history::scans(&conn, vol)
        .unwrap()
        .into_iter()
        .map(|s| s.kind)
        .collect();
    assert_eq!(kinds, ["journal", "full"]);
}

// Traces: INDEX-JOURNAL-EQUIV
#[test]
fn a_link_folded_elsewhere_is_not_counted_again_when_its_dir_is_relisted() {
    let fs = FakeFs::new(r"C:\");
    fs.file(r"C:\c\small.txt", 100 * KIB);
    fs.hard_link(r"C:\c\small.txt", r"C:\d\small2.txt");
    fs.file(r"C:\c\other.txt", 12 * KIB);
    fs.file(r"C:\d\other.txt", 20 * KIB);
    let e = env();
    let conn = e.db.conn().unwrap();
    full(&e, &fs);
    let small = |p: &str| node_by_path(&conn, p).unwrap().unwrap().small_bytes;
    let (c0, d0) = (small(r"C:\c"), small(r"C:\d"));
    assert_eq!(c0 + d0, 100 * KIB + 32 * KIB);

    fs.file(r"C:\c\big.bin", 5 * MIB);
    fs.file(r"C:\d\big.bin", 6 * MIB);
    fs.dir(r"C:\d\sub");
    fs.rename(r"C:\d\big.bin", r"C:\d\bigger.bin");
    fs.file(r"C:\d\added.txt", 8 * KIB);
    fs.clear_log();
    journal(&e, &fs);
    assert!(fs.listed().iter().any(|p| p == r"C:\d"));
    assert_eq!((small(r"C:\c"), small(r"C:\d")), (c0, d0 + 8 * KIB));
    let want = fresh(&fs, None);
    let root = |rows: &[SnapRow]| rows.iter().find(|r| r.path == r"C:\").unwrap().clone();
    assert_eq!(root(&snapshot(&conn, r"C:\").unwrap()), root(&want));
    assert_consistent(&e);
}

// Traces: INDEX-JOURNAL-EQUIV
#[test]
fn a_reset_wrapped_or_missing_journal_falls_back_to_a_full_refresh() {
    let fs = sample_tree();
    let e = env();
    let conn = e.db.conn().unwrap();
    let auto = |kind| refresh_kind(&conn, &fs, fs.root(), kind, &jopts(&fs)).unwrap();
    full(&e, &fs);

    fs.file(r"C:\code\after-reset.bin", 3 * MIB);
    fs.journal_reset();
    let before = snapshot(&conn, r"C:\").unwrap();
    assert_eq!(
        try_journal(&e, &fs),
        JournalOutcome::NeedsFull(NeedsFull::JournalChanged)
    );
    assert_eq!(snapshot(&conn, r"C:\").unwrap(), before, "untouched");
    match auto(RefreshKind::Auto) {
        RefreshRun::Full { outcome, fallback } => {
            assert!(outcome.completed);
            assert_eq!(fallback, Some(NeedsFull::JournalChanged));
        }
        other => panic!("{other:?}"),
    }
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));

    fs.file(r"C:\code\after-wrap.bin", 3 * MIB);
    fs.journal_wrap();
    assert_eq!(
        auto(RefreshKind::Journal),
        RefreshRun::NeedsFull(NeedsFull::Wrapped)
    );
    assert!(
        node_by_path(&conn, r"C:\code\after-wrap.bin")
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        auto(RefreshKind::Auto),
        RefreshRun::Full {
            fallback: Some(NeedsFull::Wrapped),
            ..
        }
    ));
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));
    assert!(matches!(auto(RefreshKind::Auto), RefreshRun::Journal(_)));

    fs.set_journal_enabled(false);
    assert_eq!(
        try_journal(&e, &fs),
        JournalOutcome::NeedsFull(NeedsFull::NoJournal)
    );
    fs.set_journal_enabled(true);
    run(&e, &fs, r"C:\", 2);
    assert_eq!(
        try_journal(&e, &fs),
        JournalOutcome::NeedsFull(NeedsFull::NoPosition),
        "a full refresh without a journal stores no position"
    );

    let small = RefreshOptions {
        threshold: Some(64 * KIB),
        ..jopts(&fs)
    };
    assert!(refresh(&conn, &fs, fs.root(), &small).unwrap().completed);
    assert!(matches!(auto(RefreshKind::Journal), RefreshRun::Journal(_)));
    assert!(matches!(
        auto(RefreshKind::Auto),
        RefreshRun::Full {
            fallback: Some(NeedsFull::ThresholdChanged),
            ..
        }
    ));
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));
}

// Traces: INDEX-JOURNAL-EQUIV
#[test]
fn an_interrupted_journal_refresh_stays_consistent_and_forces_a_full_one() {
    let fs = sample_tree();
    fs.track_dir_times(true);
    let e = env();
    let conn = e.db.conn().unwrap();
    full(&e, &fs);
    mutate_round_one(&fs);
    fs.file(r"C:\fresh\a\b\c\deep.bin", 5 * MIB);
    let cancel = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(AtomicUsize::new(0));
    let (c, s) = (cancel.clone(), seen.clone());
    fs.on_list(move |_| {
        if s.fetch_add(1, Ordering::SeqCst) + 1 >= 3 {
            c.store(true, Ordering::SeqCst);
        }
    });
    let out = journal_refresh(
        &conn,
        &fs,
        &fs,
        r"C:\",
        &RefreshOptions {
            cancel: Some(cancel),
            ..RefreshOptions::default()
        },
    )
    .unwrap();
    fs.clear_hook();
    match out {
        JournalOutcome::Done(j) => {
            assert!(!j.outcome.completed);
            assert_eq!(j.outcome.scan_id, None);
        }
        other => panic!("{other:?}"),
    }
    assert_consistent(&e);
    assert_eq!(
        try_journal(&e, &fs),
        JournalOutcome::NeedsFull(NeedsFull::NoPosition)
    );
    let ran = refresh_kind(&conn, &fs, r"C:\", RefreshKind::Auto, &jopts(&fs)).unwrap();
    assert!(matches!(
        ran,
        RefreshRun::Full {
            fallback: Some(NeedsFull::NoPosition),
            ..
        }
    ));
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &fresh(&fs, None));
}

struct Logged {
    listed: Mutex<Vec<String>>,
}

impl DirLister for Logged {
    fn list_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        self.listed
            .lock()
            .unwrap()
            .push(path.to_string_lossy().to_lowercase());
        WinLister.list_dir(path)
    }

    fn alloc_size_by_open(&self, path: &Path) -> io::Result<u64> {
        WinLister.alloc_size_by_open(path)
    }

    fn volume_info(&self, root: &Path) -> io::Result<VolumeInfo> {
        WinLister.volume_info(root)
    }
}

// Listing updates a dir's access time, and NTFS updates a dir's times in its parent's entry lazily.
fn settled(rows: Vec<SnapRow>) -> Vec<SnapRow> {
    rows.into_iter()
        .map(|mut r| {
            r.atime = None;
            if r.kind == 0 {
                r.mtime = None;
                r.ctime = None;
            }
            r
        })
        .collect()
}

fn write(p: &Path, bytes: usize) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, vec![5u8; bytes]).unwrap();
}

// Traces: INDEX-JOURNAL-EQUIV
#[test]
fn real_journal_refresh_matches_a_fresh_build() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("tree");
    let mb = MIB as usize;
    write(&root.join(r"a\keep.bin"), 2 * mb);
    write(&root.join(r"a\linked.bin"), 2 * mb);
    write(&root.join(r"a\sub\s.txt"), 300);
    write(&root.join(r"a\sub\inner\deep.bin"), 3 * mb);
    write(&root.join(r"b\x.txt"), 10);
    write(&root.join(r"b\grow.bin"), 500 * KIB as usize);
    write(&root.join(r"c\del.bin"), 2 * mb);
    write(&root.join(r"c\gone\dir\f.txt"), 10);
    let root_s = root.to_string_lossy().into_owned();
    let src = WinJournal::for_path(&root_s).unwrap();
    let lister = Logged {
        listed: Mutex::new(Vec::new()),
    };
    let e = env();
    let conn = e.db.conn().unwrap();
    let opts = RefreshOptions {
        workers: 2,
        journal: Some(&src as &dyn JournalSource),
        ..RefreshOptions::default()
    };
    assert!(refresh(&conn, &lister, &root_s, &opts).unwrap().completed);

    write(&root.join(r"b\new.bin"), 3 * mb);
    write(&root.join(r"b\grow.bin"), 2 * mb);
    std::fs::rename(root.join(r"a\keep.bin"), root.join(r"a\kept.bin")).unwrap();
    std::fs::rename(root.join(r"a\sub"), root.join(r"b\sub-moved")).unwrap();
    std::fs::remove_file(root.join(r"c\del.bin")).unwrap();
    std::fs::remove_dir_all(root.join(r"c\gone")).unwrap();
    write(&root.join(r"d\e\f\new.bin"), 2 * mb);
    write(&root.join(r"d\e\g.txt"), 20);
    std::fs::hard_link(root.join(r"a\linked.bin"), root.join(r"c\linked-too.bin")).unwrap();
    lister.listed.lock().unwrap().clear();

    let j = match journal_refresh(&conn, &lister, &src, &root_s, &opts).unwrap() {
        JournalOutcome::Done(j) => j,
        other => panic!("expected a journal refresh, got {other:?}"),
    };
    assert!(j.outcome.completed);
    assert!(j.moved_dirs >= 1, "{j:?}");
    let listed = lister.listed.lock().unwrap().clone();
    let inner = format!(r"{root_s}\b\sub-moved\inner").to_lowercase();
    assert!(!listed.contains(&inner), "{listed:?}");

    let want = {
        let f = env();
        run(&f, &WinLister, &root_s, 2);
        snapshot(&f.db.conn().unwrap(), &root_s).unwrap()
    };
    assert_same(&settled(snapshot(&conn, &root_s).unwrap()), &settled(want));
    assert_consistent(&e);
    let n = |p: &str| node_by_path(&conn, &format!(r"{root_s}\{p}")).unwrap();
    assert!(n(r"b\grow.bin").is_some());
    assert!(n(r"b\sub-moved\inner\deep.bin").is_some());
    assert!(n(r"c\gone").is_none());
}

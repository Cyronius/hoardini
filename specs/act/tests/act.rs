mod fake_rc;
mod rclone;
#[path = "../../policy/tests/support.rs"]
mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hoardini_core::actions::{ArchiveEntry, archive_entries};
use hoardini_core::index::node_by_path;
use hoardini_core::proposals::ProposalKind;
use support::*;

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn entries(fx: &Fx) -> Vec<ArchiveEntry> {
    archive_entries(&fx.db.conn().unwrap(), None, 100).unwrap()
}

fn entry_for(fx: &Fx, original: &Path) -> ArchiveEntry {
    let want = s(original);
    entries(fx)
        .into_iter()
        .find(|e| e.original_path.eq_ignore_ascii_case(&want))
        .unwrap_or_else(|| panic!("no archive entry for {want}"))
}

fn entry_blake3(fx: &Fx, id: i64) -> String {
    fx.db
        .conn()
        .unwrap()
        .query_row(
            "SELECT blake3 FROM archive_entry WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            let rel = s(p.strip_prefix(root).unwrap());
            if e.file_type().unwrap().is_dir() {
                out.insert(format!("{rel}\\"), Vec::new());
                stack.push(p);
            } else {
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

fn make_tree(fx: &Fx, rel: &str) -> PathBuf {
    fx.file(&format!(r"{rel}\a.bin"), &data(300_000, 1));
    fx.file(&format!(r"{rel}\sub\b.bin"), &data(70_000, 2));
    fx.file(&format!(r"{rel}\sub\deeper\c.txt"), b"hello");
    std::fs::create_dir_all(fx.work.join(rel).join("empty")).unwrap();
    fx.work.join(rel)
}

// Traces: ACT-ARCHIVE-VERIFY
#[test]
fn archive_deletes_the_source_only_after_size_and_blake3_match() {
    let fx = Fx::new();
    let (dest, dest_dir) = fx.local_destination("nas");
    let original = data(3 * 1024 * 1024 + 17, 7);
    let file = fx.file(r"docs\model.gguf", &original);
    let dir = make_tree(&fx, "project");
    let dir_before = tree(&dir);
    let ex = fx.executor();

    let p = fx
        .propose(ProposalKind::Archive, &[&file, &dir], Some(&dest.id))
        .unwrap();
    let r = ex.approve(&p.id, &ids(&p)).unwrap();
    assert_eq!(r.items.len(), 2);
    let job = wait(&ex, &r.job_id);
    assert_eq!(job.status, "done", "{:?}", job.error);
    assert_eq!(fx.get(&p.id).status, "done");
    assert!(!file.exists() && !dir.exists());

    let fe = entry_for(&fx, &file);
    assert!(
        fe.dest_uri
            .to_lowercase()
            .starts_with(&s(&dest_dir).to_lowercase())
    );
    assert!(
        fe.dest_uri.ends_with(r"\work\docs\model.gguf"),
        "{}",
        fe.dest_uri
    );
    assert_eq!(std::fs::read(&fe.dest_uri).unwrap(), original);
    assert_eq!(fe.size, original.len() as i64);
    assert_eq!(entry_blake3(&fx, fe.id), blake3_hex(&original));
    let de = entry_for(&fx, &dir);
    assert_eq!(tree(Path::new(&de.dest_uri)), dir_before);

    // A copy that does not verify leaves the source alone, for a file and for a folder.
    let victim = data(500_000, 9);
    let file2 = fx.file(r"docs\second.bin", &victim);
    let dir2 = make_tree(&fx, "project2");
    let dir2_before = tree(&dir2);
    ex.set_copy_hook(Some(Arc::new(|tmp: &Path| {
        let mut bytes = std::fs::read(tmp).unwrap();
        if let Some(b) = bytes.last_mut() {
            *b ^= 0xFF;
        }
        std::fs::write(tmp, bytes).unwrap();
    })));
    let before = entries(&fx).len();
    let p2 = fx
        .propose(ProposalKind::Archive, &[&file2, &dir2], Some(&dest.id))
        .unwrap();
    let r2 = ex.approve(&p2.id, &ids(&p2)).unwrap();
    let job2 = wait(&ex, &r2.job_id);
    assert_eq!(job2.status, "failed");
    let after = fx.get(&p2.id);
    assert_eq!(after.status, "failed");
    for it in &after.items {
        assert_eq!(it.status, "failed");
        assert!(
            it.detail
                .as_deref()
                .unwrap()
                .contains("verification failed"),
            "{:?}",
            it.detail
        );
    }
    assert_eq!(std::fs::read(&file2).unwrap(), victim);
    assert_eq!(tree(&dir2), dir2_before);
    assert_eq!(
        entries(&fx).len(),
        before,
        "no archive entry for an unverified copy"
    );
    assert_eq!(job2.freed_bytes, 0);
}

// Traces: ACT-ARCHIVE-VERIFY
#[test]
fn a_rerun_reuses_copies_that_already_verify() {
    let fx = Fx::new();
    let (dest, _) = fx.local_destination("nas");
    let dir = make_tree(&fx, "resume");
    let before = tree(&dir);
    let ex = fx.executor();

    // First attempt: the second file's copy is damaged, so nothing is deleted.
    let n = std::sync::atomic::AtomicUsize::new(0);
    let n = Arc::new(n);
    let hook_n = n.clone();
    ex.set_copy_hook(Some(Arc::new(move |tmp: &Path| {
        if hook_n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            std::fs::write(tmp, b"garbage").unwrap();
        }
    })));
    let p = fx
        .propose(ProposalKind::Archive, &[&dir], Some(&dest.id))
        .unwrap();
    let r = ex.approve(&p.id, &ids(&p)).unwrap();
    assert_eq!(wait(&ex, &r.job_id).status, "failed");
    assert_eq!(tree(&dir), before);

    // Second attempt copies only what is missing; the first file's verified copy is reused.
    n.store(0, std::sync::atomic::Ordering::SeqCst);
    ex.set_copy_hook(Some(Arc::new({
        let n = n.clone();
        move |_tmp: &Path| {
            n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    })));
    let p2 = fx
        .propose(ProposalKind::Archive, &[&dir], Some(&dest.id))
        .unwrap();
    let r2 = ex.approve(&p2.id, &ids(&p2)).unwrap();
    assert_eq!(wait(&ex, &r2.job_id).status, "done");
    assert_eq!(
        n.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "3 files, 1 reused"
    );
    assert!(!dir.exists());
    assert_eq!(tree(Path::new(&entry_for(&fx, &dir).dest_uri)), before);
}

fn action_rows(fx: &Fx, proposal_id: &str) -> Vec<(i64, String, i64, Option<String>)> {
    let conn = fx.db.conn().unwrap();
    let mut st = conn
        .prepare("SELECT item_id, status, freed_bytes, undo_json FROM action WHERE proposal_id = ?1 ORDER BY id")
        .unwrap();
    st.query_map([proposal_id], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap()
}

// Traces: ACT-JOURNAL
#[test]
fn every_item_is_journaled_with_undo_info_before_it_reports_done() {
    let fx = Fx::new();
    let (dest, _) = fx.local_destination("nas");
    let d1 = fx.file("del1.bin", &data(200_000, 1));
    let d2 = fx.file(r"sub\del2.bin", &data(100_000, 2));
    let a1 = fx.file(r"keep\arch1.bin", &data(150_000, 3));
    let ex = fx.executor();
    let mut rx = ex.subscribe();

    let del = fx.propose(ProposalKind::Delete, &[&d1, &d2], None).unwrap();
    let arc = fx
        .propose(ProposalKind::Archive, &[&a1], Some(&dest.id))
        .unwrap();
    let r1 = ex.approve(&del.id, &ids(&del)).unwrap();
    let r2 = ex.approve(&arc.id, &ids(&arc)).unwrap();

    let mut finished = 0;
    while finished < 2 {
        let job = rx.blocking_recv().unwrap();
        // Whatever a report says is done must already be in the journal.
        for pid in [&del.id, &arc.id] {
            let p = fx.get(pid);
            let rows = action_rows(&fx, pid);
            for it in p.items.iter().filter(|i| i.status == "done") {
                let row = rows
                    .iter()
                    .find(|r| r.0 == it.id)
                    .expect("journal row for a done item");
                assert_eq!(row.1, "done");
                assert!(row.3.is_some(), "undo info for item {}", it.id);
            }
        }
        if matches!(job.status.as_str(), "done" | "failed" | "cancelled") {
            finished += 1;
        }
    }
    wait(&ex, &r1.job_id);
    wait(&ex, &r2.job_id);

    let del_rows = action_rows(&fx, &del.id);
    assert_eq!(del_rows.len(), 2);
    for (item_id, status, freed, undo) in &del_rows {
        assert_eq!(status, "done");
        let it = fx
            .get(&del.id)
            .items
            .into_iter()
            .find(|i| i.id == *item_id)
            .unwrap();
        assert_eq!(*freed, it.alloc);
        let undo: serde_json::Value = serde_json::from_str(undo.as_deref().unwrap()).unwrap();
        assert_eq!(undo["kind"], "none");
    }
    let arc_rows = action_rows(&fx, &arc.id);
    assert_eq!(arc_rows.len(), 1);
    let undo: serde_json::Value = serde_json::from_str(arc_rows[0].3.as_deref().unwrap()).unwrap();
    let entry = entry_for(&fx, &a1);
    assert_eq!(undo["kind"], "restore");
    assert_eq!(undo["archive_entry_id"], entry.id);
    assert_eq!(undo["dest_uri"], entry.dest_uri.as_str());

    let total: i64 = del_rows.iter().chain(&arc_rows).map(|r| r.2).sum();
    assert!(total > 0);
    assert_eq!(ex.vanished_total(), total);
    drop(ex);
    assert_eq!(fx.executor().vanished_total(), total, "survives a restart");
}

// Traces: INDEX-ACTION-PATCH (canonical spec: specs/index/spec.md)
#[test]
fn executed_items_are_patched_out_of_the_index() {
    let fx = Fx::new();
    let big = fx.file(r"a\b\big.bin", &data(400_000, 1));
    let small = fx.file(r"a\small.bin", &data(5_000, 2));
    fx.file(r"a\b\stay.bin", &data(20_000, 3));
    fx.index_work();
    let conn = fx.db.conn().unwrap();
    let size = |p: &Path| node_by_path(&conn, &s(p)).unwrap().unwrap().size;
    let (ab, a, work) = (fx.work.join(r"a\b"), fx.work.join("a"), fx.work.clone());
    let before = [size(&ab), size(&a), size(&work)];
    let big_size = size(&big);
    let small_size = size(&small);

    let ex = fx.executor();
    let p = fx
        .propose(ProposalKind::Delete, &[&big, &small], None)
        .unwrap();
    let r = ex.approve(&p.id, &ids(&p)).unwrap();
    assert_eq!(wait(&ex, &r.job_id).status, "done");
    assert!(node_by_path(&conn, &s(&big)).unwrap().is_none());
    assert_eq!(size(&ab), before[0] - big_size);
    assert_eq!(size(&a), before[1] - big_size - small_size);
    assert_eq!(size(&work), before[2] - big_size - small_size);
}

// Traces: ACT-RESTORE
#[test]
fn restore_puts_back_the_original_path_and_content() {
    let fx = Fx::new();
    let (dest, _) = fx.local_destination("nas");
    let original = data(1_000_000, 5);
    let file = fx.file(r"docs\report.pdf", &original);
    let mtime = std::fs::metadata(&file).unwrap().modified().unwrap();
    let dir = make_tree(&fx, "photos");
    let dir_before = tree(&dir);
    let ex = fx.executor();
    let p = fx
        .propose(ProposalKind::Archive, &[&file, &dir], Some(&dest.id))
        .unwrap();
    let r = ex.approve(&p.id, &ids(&p)).unwrap();
    assert_eq!(wait(&ex, &r.job_id).status, "done");
    let stub = PathBuf::from(format!("{}.hoardini.txt", s(&file)));
    assert!(!file.exists() && stub.exists(), "user files leave a stub");
    assert!(
        std::fs::read_to_string(&stub)
            .unwrap()
            .contains(&entry_for(&fx, &file).dest_uri)
    );

    let fe = entry_for(&fx, &file);
    let job = wait(&ex, &ex.restore(fe.id).unwrap());
    assert_eq!(job.status, "done", "{:?}", job.error);
    assert_eq!(std::fs::read(&file).unwrap(), original);
    assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), mtime);
    assert!(!stub.exists());
    assert!(
        Path::new(&fe.dest_uri).exists(),
        "the archived copy is kept"
    );
    assert!(entry_for(&fx, &file).restored_ms.is_some());

    let de = entry_for(&fx, &dir);
    assert_eq!(wait(&ex, &ex.restore(de.id).unwrap()).status, "done");
    assert_eq!(tree(&dir), dir_before);

    let err = ex.restore(fe.id).unwrap_err().to_string();
    assert!(err.contains("already exists"), "{err}");

    // A damaged archive copy is refused and leaves nothing at the original path.
    let other = fx.file(r"docs\notes.txt", &data(50_000, 6));
    let p2 = fx
        .propose(ProposalKind::Archive, &[&other], Some(&dest.id))
        .unwrap();
    let r2 = ex.approve(&p2.id, &ids(&p2)).unwrap();
    assert_eq!(wait(&ex, &r2.job_id).status, "done");
    let oe = entry_for(&fx, &other);
    let mut bytes = std::fs::read(&oe.dest_uri).unwrap();
    bytes[10] ^= 0x55;
    std::fs::write(&oe.dest_uri, bytes).unwrap();
    let bad = wait(&ex, &ex.restore(oe.id).unwrap());
    assert_eq!(bad.status, "failed");
    assert!(bad.error.unwrap().contains("BLAKE3"));
    assert!(!other.exists());
    assert!(entry_for(&fx, &other).restored_ms.is_none());
}

fn text_blob(n: usize) -> Vec<u8> {
    "Hoardini compressible line 0123456789\n"
        .as_bytes()
        .iter()
        .copied()
        .cycle()
        .take(n)
        .collect()
}

fn compress_request(
    fx: &Fx,
    paths: &[&Path],
    format: &str,
) -> hoardini_core::proposals::NewProposal {
    let mut req = fx.request(ProposalKind::Compress, paths, None);
    req.format = Some(format.into());
    req
}

fn compress_proposal(fx: &Fx, paths: &[&Path], format: &str) -> hoardini_core::proposals::Proposal {
    hoardini_core::proposals::create_with(&fx.db, &fx.env, compress_request(fx, paths, format))
        .unwrap()
}

fn archive_beside(original: &Path, ext: &str) -> PathBuf {
    PathBuf::from(format!("{}.{ext}", s(original)))
}

fn leftover_partials(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".hoardini-partial")
        })
        .count()
}

// Traces: ACT-COMPRESS-ESTIMATE
#[test]
fn estimate_says_random_data_does_not_shrink_and_zeros_do() {
    let fx = Fx::new();
    let random = fx.file("random.dat", &data(4 << 20, 21));
    let zeros = fx.file("zeros.dat", &vec![0u8; 4 << 20]);

    let r = hoardini_core::compress::estimate(&random).unwrap();
    assert_eq!(r.logical_bytes, 4 << 20);
    assert_eq!(r.formats.len(), 5);
    for f in &r.formats {
        assert!((0.97..=1.0).contains(&f.ratio), "{}: {}", f.format, f.ratio);
        assert!(f.saved_bytes < 150_000, "{}: {}", f.format, f.saved_bytes);
    }

    let z = hoardini_core::compress::estimate(&zeros).unwrap();
    for f in &z.formats {
        assert!(f.ratio < 0.1, "{}: {}", f.format, f.ratio);
        assert!(f.saved_bytes > 3 << 20, "{}: {}", f.format, f.saved_bytes);
    }
}

// Traces: ACT-COMPRESS-ESTIMATE
#[test]
fn estimate_flags_already_compressed_data_without_sampling_it() {
    let fx = Fx::new();
    let clip = fx.file("clip.mp4", &data(3 << 20, 22));
    let e = hoardini_core::compress::estimate(&clip).unwrap();
    assert!(e.already_compressed);
    assert_eq!(e.sampled_bytes, 0);
    assert!(
        e.formats
            .iter()
            .all(|f| f.ratio == 1.0 && f.saved_bytes == 0)
    );
}

// Traces: ACT-COMPRESS-VERIFY
#[test]
fn archive_formats_delete_originals_only_after_the_archive_tests_clean() {
    for ext in ["zip", "7z", "tar.zst"] {
        let fx = Fx::new();
        let file = fx.file(r"big\log.txt", &text_blob(600_000));
        let dir = make_tree(&fx, "project");
        let dir_before = tree(&dir);
        let ex = fx.executor();
        let p = compress_proposal(&fx, &[&file, &dir], ext);
        let job = wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id);
        assert_eq!(job.status, "done", "{ext}: {:?}", job.error);
        assert!(
            !file.exists() && !dir.exists(),
            "{ext}: originals stay only on failure"
        );
        let fa = archive_beside(&file, ext);
        let da = archive_beside(&dir, ext);
        assert!(fa.is_file() && da.is_file(), "{ext}");
        assert!(job.freed_bytes > 0, "{ext}: text must free space");
        let done = fx.get(&p.id);
        assert!(done.items.iter().all(|i| i.status == "done"), "{ext}");

        if ext == "zip" {
            let mut z = zip::ZipArchive::new(std::fs::File::open(&da).unwrap()).unwrap();
            let mut got = BTreeMap::new();
            for i in 0..z.len() {
                let mut f = z.by_index(i).unwrap();
                if f.is_dir() {
                    continue;
                }
                let mut b = Vec::new();
                std::io::Read::read_to_end(&mut f, &mut b).unwrap();
                got.insert(f.name().to_string(), b);
            }
            for (rel, bytes) in dir_before.iter().filter(|(k, _)| !k.ends_with('\\')) {
                let name = format!("project/{}", rel.replace('\\', "/"));
                assert_eq!(got.get(&name), Some(bytes), "{name}");
            }
        }
    }
}

// Traces: ACT-COMPRESS-VERIFY
#[test]
fn a_damaged_archive_keeps_the_originals_and_leaves_no_archive() {
    for ext in ["zip", "7z", "tar.zst"] {
        let fx = Fx::new();
        let file = fx.file(r"big\log.txt", &text_blob(400_000));
        let dir = make_tree(&fx, "project");
        let dir_before = tree(&dir);
        let ex = fx.executor();
        ex.set_copy_hook(Some(Arc::new(|p: &Path| {
            let mut b = std::fs::read(p).unwrap();
            let mid = b.len() / 3;
            for x in &mut b[mid..mid + 32] {
                *x ^= 0xFF;
            }
            std::fs::write(p, b).unwrap();
        })));
        let p = compress_proposal(&fx, &[&file, &dir], ext);
        let job = wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id);
        assert_eq!(job.status, "failed", "{ext}");
        assert_eq!(job.freed_bytes, 0, "{ext}");
        assert_eq!(std::fs::read(&file).unwrap(), text_blob(400_000), "{ext}");
        assert_eq!(tree(&dir), dir_before, "{ext}");
        assert!(!archive_beside(&file, ext).exists() && !archive_beside(&dir, ext).exists());
        assert_eq!(leftover_partials(file.parent().unwrap()), 0, "{ext}");
        assert_eq!(leftover_partials(dir.parent().unwrap()), 0, "{ext}");
        assert!(
            fx.get(&p.id).items.iter().all(|i| i.status == "failed"),
            "{ext}"
        );
    }
}

// Traces: ACT-COMPRESS-VERIFY
#[test]
fn in_place_compression_reduces_allocation_and_keeps_content() {
    for (format, name) in [("lzx", "wof.txt"), ("lznt1", "ntfs.txt")] {
        let fx = Fx::new();
        let bytes = text_blob(4 << 20);
        let file = fx.file(name, &bytes);
        let before = hoardini_win::alloc_size_by_open(&file).unwrap();
        let ex = fx.executor();
        let p = compress_proposal(&fx, &[&file], format);
        let job = wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id);
        assert_eq!(job.status, "done", "{format}: {:?}", job.error);
        let after = hoardini_win::alloc_size_by_open(&file).unwrap();
        assert!(after < before / 2, "{format}: {before} -> {after}");
        assert_eq!(std::fs::read(&file).unwrap(), bytes, "{format}");
        assert_eq!(job.freed_bytes as u64, before - after, "{format}");
        let undo: String = fx
            .db
            .conn()
            .unwrap()
            .query_row(
                "SELECT undo_json FROM action WHERE kind = 'compress'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(undo.contains("/u"), "{undo}");
    }
}

struct DupFx {
    keep: PathBuf,
    twin: PathBuf,
    lookalike: PathBuf,
    group: hoardini_core::dupes::DupGroup,
}

fn dup_fixture(fx: &Fx) -> DupFx {
    let bytes = data(600_000, 31);
    let keep = fx.file(r"a\keep.bin", &bytes);
    let twin = fx.file(r"b\twin.bin", &bytes);
    let lookalike = fx.file(r"c\lookalike.bin", &bytes);
    fx.index_work();
    let opts = hoardini_core::dupes::DupOptions {
        min_size: 1000,
        ..Default::default()
    };
    let r = hoardini_core::dupes::find(
        &fx.db.conn().unwrap(),
        &opts,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(r.groups.len(), 1);
    assert_eq!(r.groups[0].paths.len(), 3);
    DupFx {
        keep,
        twin,
        lookalike,
        group: r.groups[0].clone(),
    }
}

fn dedupe_proposal(
    fx: &Fx,
    d: &DupFx,
    mode: &str,
    extras: &[&Path],
) -> hoardini_core::proposals::Proposal {
    let mut req = fx.request(ProposalKind::Dedupe, extras, None);
    req.dedupe = Some(hoardini_core::proposals::DedupeParams {
        group_id: d.group.id,
        keep: s(&d.keep),
        mode: mode.into(),
    });
    hoardini_core::proposals::create_with(&fx.db, &fx.env, req).unwrap()
}

// Same size and mtime, different bytes: only a fresh full hash can tell.
fn silently_alter(path: &Path) {
    let mtime = std::fs::metadata(path).unwrap().modified().unwrap();
    let mut b = std::fs::read(path).unwrap();
    b[300_000] ^= 0xFF;
    std::fs::write(path, b).unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
}

// Traces: ACT-DEDUPE-VERIFY
#[test]
fn hardlink_dedupe_relinks_only_copies_that_still_match_and_never_touches_keep() {
    let fx = Fx::new();
    let d = dup_fixture(&fx);
    let keep_bytes = std::fs::read(&d.keep).unwrap();
    let keep_id = hoardini_win::file_id_of(&d.keep).unwrap();
    let lookalike_id = hoardini_win::file_id_of(&d.lookalike).unwrap();
    silently_alter(&d.lookalike);
    let altered = std::fs::read(&d.lookalike).unwrap();
    let ex = fx.executor();
    let p = dedupe_proposal(&fx, &d, "hardlink", &[&d.twin, &d.lookalike]);
    let job = wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id);
    assert_eq!(job.status, "done", "{:?}", job.error);

    let done = fx.get(&p.id);
    assert_eq!(item(&done, &d.twin).status, "done");
    assert_eq!(item(&done, &d.lookalike).status, "failed");
    assert!(
        item(&done, &d.lookalike)
            .detail
            .as_deref()
            .unwrap()
            .contains("differs")
    );
    assert_eq!(hoardini_win::file_id_of(&d.twin).unwrap(), keep_id);
    assert_eq!(std::fs::read(&d.twin).unwrap(), keep_bytes);
    assert_eq!(std::fs::read(&d.keep).unwrap(), keep_bytes);
    assert_eq!(hoardini_win::file_id_of(&d.keep).unwrap(), keep_id);
    assert_eq!(
        hoardini_win::file_id_of(&d.lookalike).unwrap(),
        lookalike_id
    );
    assert_eq!(std::fs::read(&d.lookalike).unwrap(), altered);
    assert!(job.freed_bytes >= 600_000, "{}", job.freed_bytes);
    let undo: String = fx
        .db
        .conn()
        .unwrap()
        .query_row(
            "SELECT undo_json FROM action WHERE path = ?1",
            [s(&d.twin)],
            |r| r.get(0),
        )
        .unwrap();
    assert!(undo.contains("break the link"), "{undo}");
}

// Traces: ACT-DEDUPE-VERIFY
#[test]
fn delete_dedupe_removes_verified_extras_and_refuses_changed_ones() {
    let fx = Fx::new();
    let d = dup_fixture(&fx);
    let keep_bytes = std::fs::read(&d.keep).unwrap();
    silently_alter(&d.lookalike);
    let ex = fx.executor();
    let p = dedupe_proposal(&fx, &d, "delete", &[&d.twin, &d.lookalike]);
    let job = wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id);
    assert_eq!(job.status, "done", "{:?}", job.error);
    assert!(!d.twin.exists());
    assert!(
        d.lookalike.exists(),
        "a copy that no longer matches is never deleted"
    );
    assert_eq!(std::fs::read(&d.keep).unwrap(), keep_bytes);
    assert!(job.freed_bytes >= 600_000);
}

// Traces: ACT-DEDUPE-VERIFY
#[test]
fn a_dedupe_that_lists_the_kept_copy_for_removal_changes_nothing() {
    let fx = Fx::new();
    let d = dup_fixture(&fx);
    let ex = fx.executor();
    let p = dedupe_proposal(&fx, &d, "delete", &[&d.keep, &d.twin]);
    let job = wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id);
    assert_eq!(job.status, "failed");
    assert!(d.keep.exists() && d.twin.exists());
}

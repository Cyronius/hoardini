use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use hoardini_core::index::{RefreshOptions, WinLister, refresh};
use hoardini_core::proposals::ProposalKind;

use super::support::*;
use super::{compress_proposal, entries, entry_for, make_tree, text_blob, tree};

fn alloc(p: &Path) -> u64 {
    hoardini_win::alloc_size_by_open(p).unwrap()
}

// Traces: ACT-COMPRESS-VERIFY
#[test]
fn in_place_compression_of_a_folder_never_reaches_through_a_link_inside() {
    for format in ["lzx", "lznt1"] {
        let fx = Fx::new();
        let outside = fx.file(r"elsewhere\big.txt", &text_blob(2 << 20));
        let inside = fx.file(r"project\sub\a.txt", &text_blob(2 << 20));
        let dir = fx.work.join("project");
        hoardini_win::create_junction(&dir.join("sub").join("jn"), outside.parent().unwrap())
            .unwrap();
        let (out_before, in_before) = (alloc(&outside), alloc(&inside));
        let ex = fx.executor();
        let p = compress_proposal(&fx, &[&dir], format);
        let job = wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id);
        let it = &fx.get(&p.id).items[0];
        assert_eq!(it.status, "failed", "{format}: {job:?}");
        assert!(
            it.detail.as_deref().unwrap_or("").contains("link"),
            "{format}: {:?}",
            it.detail
        );
        assert_eq!(
            alloc(&outside),
            out_before,
            "{format}: the link's target was compressed"
        );
        assert_eq!(alloc(&inside), in_before, "{format}");
        assert_eq!(job.freed_bytes, 0, "{format}");
    }
}

// Traces: ACT-COMPRESS-VERIFY
#[test]
fn in_place_compression_of_a_folder_reports_the_allocation_it_saved() {
    let fx = Fx::new();
    let a = fx.file(r"logs\a.txt", &text_blob(2 << 20));
    let b = fx.file(r"logs\sub\b.txt", &text_blob(3 << 20));
    let dir = fx.work.join("logs");
    let before = alloc(&a) + alloc(&b);
    let ex = fx.executor();
    let p = compress_proposal(&fx, &[&dir], "lzx");
    let job = wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id);
    assert_eq!(job.status, "done", "{:?}", job.error);
    let after = alloc(&a) + alloc(&b);
    assert!(after < before / 2, "{before} -> {after}");
    assert_eq!(job.freed_bytes as u64, before - after);
}

// Traces: ACT-ARCHIVE-VERIFY
#[test]
fn a_folder_that_gains_a_file_during_the_copy_is_kept_whole() {
    let fx = Fx::new();
    let (dest, _) = fx.local_destination("nas");
    let dir = make_tree(&fx, "inbox");
    let late = dir.join("arrived-during-copy.bin");
    let ex = fx.executor();
    let n = Arc::new(AtomicUsize::new(0));
    ex.set_copy_hook(Some(Arc::new({
        let (late, n) = (late.clone(), n.clone());
        move |_tmp: &Path| {
            if n.fetch_add(1, Ordering::SeqCst) == 0 {
                std::fs::write(&late, b"never archived").unwrap();
            }
        }
    })));
    let p = fx
        .propose(ProposalKind::Archive, &[&dir], Some(&dest.id))
        .unwrap();
    let job = wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id);
    assert_eq!(job.status, "failed", "{job:?}");
    assert_eq!(std::fs::read(&late).unwrap(), b"never archived");
    assert!(dir.join(r"sub\deeper\c.txt").exists());
    assert!(
        entries(&fx).is_empty(),
        "no archive entry for a folder that changed"
    );
    assert_eq!(job.freed_bytes, 0);
}

// Traces: ACT-ARCHIVE-VERIFY
#[test]
fn a_file_written_after_it_was_copied_is_kept() {
    let fx = Fx::new();
    let (dest, _) = fx.local_destination("nas");
    let file = fx.file(r"docs\live.log", &data(400_000, 3));
    let ex = fx.executor();
    ex.set_copy_hook(Some(Arc::new({
        let file = file.clone();
        move |_tmp: &Path| {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file)
                .unwrap();
            std::io::Write::write_all(&mut f, b"one more line").unwrap();
        }
    })));
    let p = fx
        .propose(ProposalKind::Archive, &[&file], Some(&dest.id))
        .unwrap();
    let job = wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id);
    assert_eq!(job.status, "failed", "{job:?}");
    assert!(std::fs::read(&file).unwrap().ends_with(b"one more line"));
    assert!(entries(&fx).is_empty());
}

// Traces: ACT-ARCHIVE-VERIFY
#[test]
fn a_second_archive_of_the_same_path_never_overwrites_the_first_copy() {
    let fx = Fx::new();
    let (dest, _) = fx.local_destination("nas");
    let ex = fx.executor();
    let v1 = data(300_000, 1);
    let v2 = data(200_000, 2);
    let file = fx.file(r"dl\setup.exe", &v1);
    let dir_v1 = make_tree(&fx, "proj");
    let dir_v1_tree = tree(&dir_v1);
    let p = fx
        .propose(ProposalKind::Archive, &[&file, &dir_v1], Some(&dest.id))
        .unwrap();
    assert_eq!(
        wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id).status,
        "done"
    );
    let first_file = entry_for(&fx, &file);
    let first_dir = entry_for(&fx, &dir_v1);

    fx.file(r"dl\setup.exe", &v2);
    fx.file(r"proj\a.bin", &data(123_456, 9));
    let p2 = fx
        .propose(ProposalKind::Archive, &[&file, &dir_v1], Some(&dest.id))
        .unwrap();
    assert_eq!(
        wait(&ex, &ex.approve(&p2.id, &ids(&p2)).unwrap().job_id).status,
        "done"
    );
    let all = entries(&fx);
    assert_eq!(all.len(), 4);
    let uris: std::collections::HashSet<String> =
        all.iter().map(|e| e.dest_uri.to_lowercase()).collect();
    assert_eq!(uris.len(), 4, "each archive keeps its own copy: {all:#?}");

    let job = wait(&ex, &ex.restore(first_file.id).unwrap());
    assert_eq!(job.status, "done", "{:?}", job.error);
    assert_eq!(std::fs::read(&file).unwrap(), v1);
    let job = wait(&ex, &ex.restore(first_dir.id).unwrap());
    assert_eq!(job.status, "done", "{:?}", job.error);
    assert_eq!(tree(&dir_v1), dir_v1_tree);
}

fn index_with_default_threshold(fx: &Fx) {
    let opts = RefreshOptions {
        threshold: Some(1 << 20),
        hotspots_first: false,
        ..RefreshOptions::default()
    };
    let out = refresh(&fx.db.conn().unwrap(), &WinLister, &s(&fx.work), &opts).unwrap();
    assert!(out.completed);
}

// Traces: POLICY-IN-USE (canonical spec: specs/policy/spec.md)
#[test]
fn a_folder_delete_that_meets_an_open_small_file_deletes_nothing() {
    let fx = Fx::new();
    let names = [r"a\1.txt", r"a\2.txt", r"m\big.bin", r"z\held.txt"];
    let paths: Vec<PathBuf> = names
        .iter()
        .map(|n| {
            let size = if n.ends_with("big.bin") {
                1_500_000
            } else {
                2000
            };
            fx.file(&format!(r"cache\{n}"), &data(size, 4))
        })
        .collect();
    // Only the 1.5 MB file gets an index row, so the open-file check cannot see the small one.
    index_with_default_threshold(&fx);
    let dir = fx.work.join("cache");
    let p = fx.propose(ProposalKind::Delete, &[&dir], None).unwrap();
    assert_eq!(p.items[0].status, "pending");
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&paths[3])
        .unwrap();
    let ex = fx.executor();
    let job = wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id);
    let it = fx.get(&p.id).items[0].clone();
    drop(held);
    assert_ne!(it.status, "done", "{job:?}");
    for f in &paths {
        assert!(
            f.exists(),
            "{} was deleted from a folder that could not go",
            f.display()
        );
    }
    assert_eq!(job.freed_bytes, 0);
}

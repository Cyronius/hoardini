use std::io::{Read, Write};
use std::path::Path;

use hoardini_win::*;

const JAN_2020_MS: i64 = 1_577_836_800_000;
const JUN_2021_MS: i64 = 1_622_505_600_000;

fn find(dir: &Path, name: &str) -> DirEntry {
    list_dir(dir)
        .unwrap()
        .into_iter()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("{name} not listed"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[test]
fn list_dir_reports_files_dirs_sizes_times_and_ids() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("data.bin");
    std::fs::write(&f, vec![7u8; 10_000]).unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub").join("inner.txt"), b"x").unwrap();
    set_file_times(&f, Some(JAN_2020_MS), Some(JUN_2021_MS)).unwrap();

    let entries = list_dir(dir.path()).unwrap();
    let mut names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["data.bin", "sub"]);

    let e = find(dir.path(), "data.bin");
    assert!(!e.is_dir() && !e.is_reparse() && !e.is_link());
    assert_eq!(e.logical, 10_000);
    assert_eq!(e.alloc, 12_288);
    assert_eq!(e.atime_ms, JAN_2020_MS);
    assert_eq!(e.mtime_ms, JUN_2021_MS);
    assert!((e.ctime_ms - now_ms()).abs() < 60_000);
    assert_eq!(e.file_id, file_id_of(&f).unwrap());
    assert_ne!(e.file_id, [0u8; 16]);
    assert_eq!(alloc_size_by_open(&f).unwrap(), e.alloc);

    let d = find(dir.path(), "sub");
    assert!(d.is_dir());
    assert_eq!(d.file_id, file_id_of(&dir.path().join("sub")).unwrap());
}

#[test]
fn list_dir_each_streams_the_same_entries() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..2000 {
        std::fs::write(
            dir.path()
                .join(format!("file-with-a-longish-name-{i:05}.txt")),
            b"",
        )
        .unwrap();
    }
    let mut streamed = Vec::new();
    list_dir_each(dir.path(), |e| streamed.push(e)).unwrap();
    assert_eq!(streamed.len(), 2000);
    assert_eq!(streamed, list_dir(dir.path()).unwrap());
}

#[test]
fn empty_dir_lists_nothing_and_missing_dir_errors() {
    let dir = tempfile::tempdir().unwrap();
    assert!(list_dir(dir.path()).unwrap().is_empty());
    let err = list_dir(&dir.path().join("nope")).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    let file = dir.path().join("plain.txt");
    std::fs::write(&file, b"x").unwrap();
    assert!(list_dir(&file).is_err());
}

#[test]
fn hard_links_share_one_file_id() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.bin");
    std::fs::write(&a, vec![1u8; 5000]).unwrap();
    std::fs::hard_link(&a, dir.path().join("b.bin")).unwrap();
    let ea = find(dir.path(), "a.bin");
    let eb = find(dir.path(), "b.bin");
    assert_eq!(ea.file_id, eb.file_id);
    assert_eq!(ea.alloc, eb.alloc);
}

#[test]
fn junction_is_listed_as_a_link_and_never_followed() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("secret.txt"), b"inside").unwrap();
    let link = dir.path().join("link");
    create_junction(&link, &target).unwrap();

    assert_eq!(std::fs::read(link.join("secret.txt")).unwrap(), b"inside");
    let e = find(dir.path(), "link");
    assert!(e.is_dir() && e.is_reparse() && e.is_link());
    assert_eq!(e.reparse_tag, IO_REPARSE_TAG_MOUNT_POINT);
    assert_ne!(e.file_id, find(dir.path(), "target").file_id);
    assert_eq!(e.file_id, file_id_of(&link).unwrap());

    // The handle opens the reparse point itself, so the target's entries never show.
    assert!(list_dir(&link).unwrap().is_empty());
}

#[test]
fn sparse_file_alloc_is_below_logical() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("sparse.bin");
    std::fs::write(&f, vec![9u8; 4 << 20]).unwrap();
    set_sparse_and_zero_range(&f, 0, 3 << 20).unwrap();
    let e = find(dir.path(), "sparse.bin");
    assert_ne!(e.attrs & FILE_ATTRIBUTE_SPARSE_FILE, 0);
    assert_eq!(e.logical, 4 << 20);
    assert_eq!(e.alloc, 1 << 20);
    assert!(!e.needs_alloc_by_open());
}

#[test]
fn long_paths_list_and_resolve() {
    let dir = tempfile::tempdir().unwrap();
    let mut deep = dir.path().to_path_buf();
    for i in 0..6 {
        deep.push(format!("{i}-{}", "d".repeat(60)));
    }
    std::fs::create_dir_all(&deep).unwrap();
    assert!(deep.as_os_str().len() > 300);
    std::fs::write(deep.join("f.txt"), b"abc").unwrap();
    let e = find(&deep, "f.txt");
    assert_eq!(e.logical, 3);
    assert_eq!(file_id_of(&deep.join("f.txt")).unwrap(), e.file_id);
}

#[test]
fn open_read_preserving_atime_keeps_old_atime() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("old.bin");
    let data: Vec<u8> = (0..300_000u32).map(|i| i as u8).collect();
    std::fs::write(&f, &data).unwrap();
    set_file_times(&f, Some(JAN_2020_MS), Some(JAN_2020_MS)).unwrap();

    let mut buf = Vec::new();
    open_read_preserving_atime(&f)
        .unwrap()
        .read_to_end(&mut buf)
        .unwrap();
    assert_eq!(buf, data);
    let e = find(dir.path(), "old.bin");
    assert_eq!(e.atime_ms, JAN_2020_MS);
    assert_eq!(e.mtime_ms, JAN_2020_MS);
}

#[test]
fn plain_read_moves_atime_when_updates_are_enabled() {
    if !last_access_setting().unwrap().enabled {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("control.bin");
    std::fs::write(&f, vec![3u8; 100_000]).unwrap();
    set_file_times(&f, Some(JAN_2020_MS), None).unwrap();
    let _ = std::fs::read(&f).unwrap();
    assert!(find(dir.path(), "control.bin").atime_ms > JAN_2020_MS);
}

#[test]
fn open_read_preserving_atime_refuses_offline_files() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("offline.bin");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .attributes(FILE_ATTRIBUTE_OFFLINE)
        .open(&f)
        .unwrap()
        .write_all(b"pretend remote")
        .unwrap();
    assert!(find(dir.path(), "offline.bin").is_placeholder());
    let err = open_read_preserving_atime(&f).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(err.to_string().contains("cloud placeholder"));
}

#[test]
fn processes_using_finds_this_process() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("held.txt");
    std::fs::write(&f, b"held").unwrap();
    let held = std::fs::File::open(&f).unwrap();
    let users = processes_using(std::slice::from_ref(&f)).unwrap();
    let me = users
        .iter()
        .find(|p| p.pid == std::process::id())
        .expect("own pid listed");
    let exe = std::env::current_exe().unwrap();
    assert!(
        me.exe
            .as_deref()
            .unwrap()
            .eq_ignore_ascii_case(exe.to_str().unwrap())
    );
    drop(held);
    let users = processes_using(&[f]).unwrap();
    assert!(users.iter().all(|p| p.pid != std::process::id()));
}

#[test]
fn processes_using_rejects_directories() {
    let dir = tempfile::tempdir().unwrap();
    let err = processes_using(&[dir.path().to_path_buf()]).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
}

#[test]
fn dpapi_round_trips_and_checks_entropy() {
    let secret = b"sk-test-0123456789 {\"oauth\": true}";
    let sealed = dpapi_protect(secret, b"hoardini-vault").unwrap();
    assert!(!sealed.windows(secret.len()).any(|w| w == secret));
    assert_eq!(dpapi_unprotect(&sealed, b"hoardini-vault").unwrap(), secret);
    assert!(dpapi_unprotect(&sealed, b"other-entropy").is_err());
    assert!(dpapi_unprotect(&sealed[..sealed.len() - 1], b"hoardini-vault").is_err());
    let empty = dpapi_protect(b"", b"").unwrap();
    assert_eq!(dpapi_unprotect(&empty, b"").unwrap(), b"");
}

#[test]
fn volume_queries_on_c() {
    let v = volume_info(Path::new("C:\\")).unwrap();
    assert_eq!(v.root, "C:\\");
    assert_eq!(v.fs_name, "NTFS");
    assert_ne!(v.serial, 0);
    assert!(v.total_bytes > v.free_bytes && v.free_bytes > 0);
    assert_eq!(volume_info(Path::new("C:")).unwrap().serial, v.serial);

    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        volume_root_of(dir.path()).unwrap(),
        Path::new("C:\\").to_path_buf()
    );
    assert!(
        fixed_volumes()
            .unwrap()
            .contains(&Path::new("C:\\").to_path_buf())
    );
}

#[test]
fn last_access_setting_reads_registry() {
    let s = last_access_setting().unwrap();
    assert!(s.raw <= 1 || (0x8000_0000..=0x8000_0003).contains(&s.raw));
    assert_eq!(s.enabled, s.raw & 1 == 0);
}

#[test]
fn installed_programs_and_user_assist_read() {
    let progs = installed_programs().unwrap();
    assert!(!progs.is_empty());
    assert!(progs.iter().all(|p| !p.name.is_empty()));
    assert!(progs.iter().all(|p| p.uninstall_key.starts_with("HK")));
    for e in user_assist().unwrap() {
        assert!(!e.name.is_empty());
        if let Some(ms) = e.last_run_ms {
            assert!(ms > 946_684_800_000 && ms <= now_ms() + 86_400_000);
        }
    }
}

#[test]
fn usn_journal_query_read_and_resolve() {
    let root = Path::new("C:\\");
    let j = usn_query(root).unwrap();
    assert!(j.next_usn >= j.first_usn && j.journal_id != 0);

    let (first_page, next) = usn_read(root, j.journal_id, 0).unwrap();
    assert!(!first_page.is_empty());
    assert!(next > first_page[0].usn);

    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("usn-probe.txt");
    std::fs::write(&f, b"journal me").unwrap();
    let id = file_id_of(&f).unwrap();
    let end = usn_query(root).unwrap().next_usn;
    let mut at = j.next_usn;
    let mut seen = None;
    while at < end && seen.is_none() {
        let (recs, n) = usn_read(root, j.journal_id, at).unwrap();
        seen = recs.into_iter().find(|r| r.file_id == id);
        at = n;
    }
    let rec = seen.expect("record for the new file");
    assert_eq!(rec.parent_id, file_id_of(dir.path()).unwrap());
    assert_ne!(rec.reason, 0);

    let resolved = path_by_file_id(root, id).unwrap();
    assert_eq!(
        resolved.to_string_lossy().to_lowercase(),
        std::fs::canonicalize(&f)
            .unwrap()
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_lowercase()
    );
}

#[test]
fn smb_connect_rejects_non_unc() {
    let err = smb_connect(r"C:\not\a\share", None, None).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

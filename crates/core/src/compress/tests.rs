use std::path::Path;
use std::sync::atomic::AtomicBool;

use super::pack::{Format, pack};
use super::*;

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

fn tree(root: &Path) {
    std::fs::create_dir_all(root.join("sub").join("empty")).unwrap();
    std::fs::write(root.join("a.txt"), "hello hello hello ".repeat(5000)).unwrap();
    std::fs::write(root.join("sub").join("b.bin"), noise(70_000, 2)).unwrap();
    std::fs::write(root.join("sub").join("zero.dat"), b"").unwrap();
}

#[test]
fn every_archive_format_round_trips_through_its_own_verifier() {
    for fmt in [Format::Zip, Format::SevenZ, Format::TarZst] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        tree(&root);
        let cancel = AtomicBool::new(false);
        let packed = pack(&root, fmt, &cancel, &mut |_| {}, None).unwrap();
        assert!(packed.archive.is_file(), "{fmt:?}");
        assert!(packed.archive.to_string_lossy().ends_with(fmt.ext()));
        assert_eq!(packed.source_bytes, 90_000 + 70_000);
        assert!(root.join("a.txt").exists(), "pack never deletes sources");
    }
}

#[test]
fn a_corrupted_archive_is_rejected_and_leaves_no_final_file() {
    for fmt in [Format::Zip, Format::SevenZ, Format::TarZst] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        tree(&root);
        let cancel = AtomicBool::new(false);
        let corrupt = |p: &Path| {
            let mut b = std::fs::read(p).unwrap();
            let mid = b.len() / 3;
            for x in &mut b[mid..mid + 64] {
                *x ^= 0xFF;
            }
            std::fs::write(p, b).unwrap();
        };
        let r = pack(&root, fmt, &cancel, &mut |_| {}, Some(&corrupt));
        assert!(r.is_err(), "{fmt:?} accepted a corrupted archive");
        assert!(!super::pack::archive_path(&root, fmt).exists(), "{fmt:?}");
        assert!(root.join("a.txt").exists());
    }
}

#[test]
fn random_data_estimates_near_one_and_zeros_estimate_high() {
    let tmp = tempfile::tempdir().unwrap();
    let rand = tmp.path().join("rand.dat");
    std::fs::write(&rand, noise(4 << 20, 11)).unwrap();
    let zero = tmp.path().join("zero.dat");
    std::fs::write(&zero, vec![0u8; 4 << 20]).unwrap();
    let r = estimate(&rand).unwrap();
    for f in &r.formats {
        assert!((0.97..=1.0).contains(&f.ratio), "{} {}", f.format, f.ratio);
    }
    let z = estimate(&zero).unwrap();
    for f in &z.formats {
        assert!(f.ratio < 0.1, "{} {}", f.format, f.ratio);
        assert!(f.saved_bytes > 3 << 20);
    }
}

#[test]
fn already_compressed_files_skip_the_codecs() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("clip.mp4");
    std::fs::write(&p, noise(2 << 20, 3)).unwrap();
    let e = estimate(&p).unwrap();
    assert!(e.already_compressed);
    assert_eq!(e.sampled_bytes, 0);
    assert!(
        e.formats
            .iter()
            .all(|f| f.ratio == 1.0 && f.saved_bytes == 0)
    );
}

#[test]
fn every_archive_format_keeps_each_files_mtime() {
    let want_ms = 1_700_000_000_000_i64;
    for fmt in [Format::Zip, Format::SevenZ, Format::TarZst] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        tree(&root);
        hoardini_win::set_file_times(&root.join("a.txt"), None, Some(want_ms)).unwrap();
        let cancel = AtomicBool::new(false);
        let packed = pack(&root, fmt, &cancel, &mut |_| {}, None).unwrap();
        match fmt {
            Format::Zip => {
                let mut z =
                    zip::ZipArchive::new(std::fs::File::open(&packed.archive).unwrap()).unwrap();
                let name = z
                    .file_names()
                    .find(|n| n.ends_with("a.txt"))
                    .unwrap()
                    .to_string();
                let got = z.by_name(&name).unwrap().last_modified().unwrap();
                assert_eq!(Some(got), super::pack::zip_time(want_ms));
                assert_eq!(got.year(), 2023);
            }
            Format::SevenZ => {
                let r = sevenz_rust2::ArchiveReader::open(
                    &packed.archive,
                    sevenz_rust2::Password::empty(),
                )
                .unwrap();
                let f = r
                    .archive()
                    .files
                    .iter()
                    .find(|f| f.name().ends_with("a.txt"))
                    .unwrap();
                assert!(f.has_last_modified_date);
                let got = std::time::SystemTime::from(f.last_modified_date);
                let secs = got.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
                assert_eq!(secs, want_ms / 1000);
            }
            Format::TarZst => {
                let dec =
                    zstd::stream::read::Decoder::new(std::fs::File::open(&packed.archive).unwrap())
                        .unwrap();
                let mut ar = tar::Archive::new(dec);
                let mtime = ar
                    .entries()
                    .unwrap()
                    .map(|e| e.unwrap())
                    .find(|e| e.path().unwrap().ends_with("a.txt"))
                    .map(|e| e.header().mtime().unwrap());
                assert_eq!(mtime, Some((want_ms / 1000) as u64));
            }
        }
    }
}

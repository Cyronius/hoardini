use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use anyhow::{Result, bail};

const PARTIAL_SUFFIX: &str = ".hoardini-partial";
const ZSTD_LEVEL: i32 = 6;

#[derive(Debug, thiserror::Error)]
#[error("cancelled")]
pub struct Cancelled;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    SevenZ,
    TarZst,
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        match s.trim().to_ascii_lowercase().as_str() {
            "zip" => Some(Format::Zip),
            "7z" => Some(Format::SevenZ),
            "tar.zst" | "tzst" => Some(Format::TarZst),
            _ => None,
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::SevenZ => "7z",
            Format::TarZst => "tar.zst",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub rel: String,
    pub src: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub mtime_ms: Option<i64>,
}

pub struct Packed {
    pub archive: PathBuf,
    pub entries: Vec<Entry>,
    pub source_bytes: u64,
}

fn mtime_ms(md: &std::fs::Metadata) -> Option<i64> {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
}

pub fn archive_path(root: &Path, fmt: Format) -> PathBuf {
    let mut s = root.as_os_str().to_os_string();
    s.push(".");
    s.push(fmt.ext());
    PathBuf::from(s)
}

// Links and cloud-only files are refused: an archive would silently drop or download them.
pub fn collect(root: &Path) -> Result<Vec<Entry>> {
    use std::os::windows::fs::MetadataExt;
    let md = std::fs::symlink_metadata(root)?;
    let base = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| anyhow::anyhow!("{} has no name", root.display()))?;
    if !md.is_dir() {
        if md.file_type().is_symlink() || hoardini_win::is_cloud_placeholder(md.file_attributes()) {
            bail!("{} is a link or cloud-only file", root.display());
        }
        return Ok(vec![Entry {
            rel: base,
            src: root.to_path_buf(),
            is_dir: false,
            size: md.len(),
            mtime_ms: mtime_ms(&md),
        }]);
    }
    let mut out = vec![Entry {
        rel: base.clone(),
        src: root.to_path_buf(),
        is_dir: true,
        size: 0,
        mtime_ms: mtime_ms(&md),
    }];
    let mut stack = vec![(root.to_path_buf(), base)];
    while let Some((dir, rel)) = stack.pop() {
        let mut names: Vec<_> = std::fs::read_dir(&dir)?.collect::<std::io::Result<_>>()?;
        names.sort_by_key(|e| e.file_name());
        for e in names {
            let p = e.path();
            let md = std::fs::symlink_metadata(&p)?;
            let r = format!("{rel}/{}", e.file_name().to_string_lossy());
            if md.file_type().is_symlink()
                || md.file_attributes() & hoardini_win::FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                bail!(
                    "{} is a link or reparse point; archiving would drop it",
                    p.display()
                );
            }
            if md.is_dir() {
                out.push(Entry {
                    rel: r.clone(),
                    src: p.clone(),
                    is_dir: true,
                    size: 0,
                    mtime_ms: mtime_ms(&md),
                });
                stack.push((p, r));
            } else {
                if hoardini_win::is_cloud_placeholder(md.file_attributes()) {
                    bail!("{} is cloud-only; archiving would download it", p.display());
                }
                out.push(Entry {
                    rel: r,
                    src: p,
                    is_dir: false,
                    size: md.len(),
                    mtime_ms: mtime_ms(&md),
                });
            }
        }
    }
    Ok(out)
}

struct Source<'a> {
    file: std::fs::File,
    hasher: blake3::Hasher,
    len: u64,
    cancel: &'a AtomicBool,
    progress: &'a mut dyn FnMut(i64),
}

impl Read for Source<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other(Cancelled));
        }
        let n = self.file.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.len += n as u64;
        (self.progress)(n as i64);
        Ok(n)
    }
}

type Expect = HashMap<String, (u64, String)>;

fn open_source<'a>(
    e: &Entry,
    cancel: &'a AtomicBool,
    progress: &'a mut dyn FnMut(i64),
) -> Result<Source<'a>> {
    Ok(Source {
        file: hoardini_win::open_read_preserving_atime(&e.src)?,
        hasher: blake3::Hasher::new(),
        len: 0,
        cancel,
        progress,
    })
}

fn record(expect: &mut Expect, e: &Entry, s: Source<'_>) -> Result<()> {
    if s.len != e.size {
        bail!("{} changed while it was being archived", e.src.display());
    }
    expect.insert(
        e.rel.clone(),
        (s.len, s.hasher.finalize().to_hex().to_string()),
    );
    Ok(())
}

// Zip stores wall-clock local time with 2 s resolution and no zone.
pub(crate) fn zip_time(ms: i64) -> Option<zip::DateTime> {
    let local = ms + hoardini_win::local_offset_ms(ms);
    let (y, mo, d) = crate::history::civil_from_unix_ms(local);
    let secs = local.div_euclid(1000).rem_euclid(86_400);
    zip::DateTime::from_date_and_time(
        u16::try_from(y).ok()?,
        mo as u8,
        d as u8,
        (secs / 3600) as u8,
        (secs % 3600 / 60) as u8,
        (secs % 60) as u8,
    )
    .ok()
}

fn write_zip(
    out: &Path,
    entries: &[Entry],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(i64),
) -> Result<Expect> {
    let mut zw = zip::ZipWriter::new(std::fs::File::create(out)?);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let mut expect = Expect::new();
    for e in entries {
        let opts = match e.mtime_ms.and_then(zip_time) {
            Some(t) => opts.last_modified_time(t),
            None => opts,
        };
        if e.is_dir {
            zw.add_directory(format!("{}/", e.rel), opts)?;
            continue;
        }
        zw.start_file(&e.rel, opts)?;
        let mut s = open_source(e, cancel, progress)?;
        std::io::copy(&mut s, &mut zw)?;
        record(&mut expect, e, s)?;
    }
    zw.finish()?.sync_all()?;
    Ok(expect)
}

fn write_7z(
    out: &Path,
    entries: &[Entry],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(i64),
) -> Result<Expect> {
    let mut sw = sevenz_rust2::ArchiveWriter::create(out)?;
    let mut expect = Expect::new();
    for e in entries {
        if e.is_dir {
            sw.push_archive_entry::<std::fs::File>(
                sevenz_rust2::ArchiveEntry::new_directory(&e.rel),
                None,
            )?;
            continue;
        }
        let mut s = open_source(e, cancel, progress)?;
        sw.push_archive_entry(
            sevenz_rust2::ArchiveEntry::from_path(&e.src, e.rel.clone()),
            Some(&mut s),
        )?;
        record(&mut expect, e, s)?;
    }
    sw.finish()?.sync_all()?;
    Ok(expect)
}

fn write_tar_zst(
    out: &Path,
    entries: &[Entry],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(i64),
) -> Result<Expect> {
    let enc = zstd::stream::write::Encoder::new(std::fs::File::create(out)?, ZSTD_LEVEL)?;
    let mut tw = tar::Builder::new(enc);
    let mut expect = Expect::new();
    for e in entries {
        let mut h = tar::Header::new_gnu();
        h.set_mtime(e.mtime_ms.unwrap_or(0).max(0) as u64 / 1000);
        if e.is_dir {
            h.set_entry_type(tar::EntryType::Directory);
            h.set_mode(0o755);
            h.set_size(0);
            tw.append_data(&mut h, format!("{}/", e.rel), std::io::empty())?;
            continue;
        }
        h.set_entry_type(tar::EntryType::Regular);
        h.set_mode(0o644);
        h.set_size(e.size);
        let mut s = open_source(e, cancel, progress)?;
        tw.append_data(&mut h, &e.rel, &mut s)?;
        record(&mut expect, e, s)?;
    }
    let enc = tw.into_inner()?;
    enc.finish()?.sync_all()?;
    Ok(expect)
}

fn hash_stream(r: &mut dyn Read, cancel: &AtomicBool) -> Result<(u64, String)> {
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        total += n as u64;
    }
    Ok((total, h.finalize().to_hex().to_string()))
}

fn check(seen: &mut Expect, name: &str, got: (u64, String)) -> Result<()> {
    let name = name.trim_end_matches('/').replace('\\', "/");
    match seen.remove(&name) {
        Some(want) if want == got => Ok(()),
        Some(_) => bail!(
            "verification failed: {name} in the archive does not match the source (size or BLAKE3)"
        ),
        None => bail!("verification failed: unexpected entry {name} in the archive"),
    }
}

fn verify_zip(path: &Path, mut want: Expect, cancel: &AtomicBool) -> Result<()> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().to_string();
        let got = hash_stream(&mut f, cancel)?;
        check(&mut want, &name, got)?;
    }
    missing(want)
}

fn verify_7z(path: &Path, mut want: Expect, cancel: &AtomicBool) -> Result<()> {
    let mut r = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty())?;
    let mut failure: Option<anyhow::Error> = None;
    r.for_each_entries(|entry, reader| {
        if entry.is_directory() {
            return Ok(true);
        }
        let res = hash_stream(reader, cancel).and_then(|got| check(&mut want, entry.name(), got));
        match res {
            Ok(()) => Ok(true),
            Err(e) => {
                failure = Some(e);
                Ok(false)
            }
        }
    })?;
    if let Some(e) = failure {
        return Err(e);
    }
    missing(want)
}

fn verify_tar_zst(path: &Path, mut want: Expect, cancel: &AtomicBool) -> Result<()> {
    let dec = zstd::stream::read::Decoder::new(std::fs::File::open(path)?)?;
    let mut ar = tar::Archive::new(dec);
    for entry in ar.entries()? {
        let mut entry = entry?;
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let name = entry.path()?.to_string_lossy().into_owned();
        let got = hash_stream(&mut entry, cancel)?;
        check(&mut want, &name, got)?;
    }
    missing(want)
}

fn missing(want: Expect) -> Result<()> {
    if let Some(name) = want.keys().next() {
        bail!("verification failed: {name} is missing from the archive");
    }
    Ok(())
}

fn unchanged(entries: &[Entry]) -> Result<()> {
    for e in entries.iter().filter(|e| !e.is_dir) {
        let md = std::fs::symlink_metadata(&e.src)?;
        if md.len() != e.size || mtime_ms(&md) != e.mtime_ms {
            bail!("{} changed while it was being archived", e.src.display());
        }
    }
    Ok(())
}

// The final name only ever holds an archive that was re-read and matched; originals stay the caller's to delete.
pub fn pack(
    root: &Path,
    fmt: Format,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(i64),
    before_verify: Option<&dyn Fn(&Path)>,
) -> Result<Packed> {
    let entries = collect(root)?;
    let dest = archive_path(root, fmt);
    if std::fs::symlink_metadata(&dest).is_ok() {
        bail!("{} already exists; move it away first", dest.display());
    }
    let mut tmp = dest.as_os_str().to_os_string();
    tmp.push(PARTIAL_SUFFIX);
    let tmp = PathBuf::from(tmp);
    let result = (|| -> Result<()> {
        let expect = match fmt {
            Format::Zip => write_zip(&tmp, &entries, cancel, progress)?,
            Format::SevenZ => write_7z(&tmp, &entries, cancel, progress)?,
            Format::TarZst => write_tar_zst(&tmp, &entries, cancel, progress)?,
        };
        if let Some(h) = before_verify {
            h(&tmp);
        }
        match fmt {
            Format::Zip => verify_zip(&tmp, expect, cancel)?,
            Format::SevenZ => verify_7z(&tmp, expect, cancel)?,
            Format::TarZst => verify_tar_zst(&tmp, expect, cancel)?,
        }
        unchanged(&entries)?;
        std::fs::rename(&tmp, &dest)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        // Writers wrap io errors in their own types, so cancellation is read off the flag.
        return Err(if cancel.load(Ordering::Relaxed) {
            Cancelled.into()
        } else {
            e
        });
    }
    let source_bytes = entries.iter().map(|e| e.size).sum();
    Ok(Packed {
        archive: dest,
        entries,
        source_bytes,
    })
}

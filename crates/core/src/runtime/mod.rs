mod catalog;

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use catalog::{LLAMA_SERVER, MODELS, PINS, RCLONE, find};

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Exe,
    // Entry 0 is the main executable; entries are zip paths, written flat by file name.
    Zip { extract: &'static [&'static str] },
    File,
}

#[derive(Clone, Copy, Debug)]
pub struct Pinned {
    pub id: &'static str,
    pub version: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    pub kind: Kind,
    pub license_url: &'static str,
}

#[derive(Serialize, Deserialize)]
struct Marker {
    pin_sha256: String,
    files: Vec<MarkerFile>,
}

#[derive(Serialize, Deserialize)]
struct MarkerFile {
    name: String,
    size: u64,
    mtime_ms: u128,
    sha256: String,
}

fn url_file_name(url: &str) -> &str {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/').next().unwrap_or(path)
}

fn entry_name(entry: &str) -> &str {
    entry.rsplit(['/', '\\']).next().unwrap_or(entry)
}

pub fn install_dir(pin: &Pinned, root: &Path) -> PathBuf {
    match pin.kind {
        Kind::File => root.join("models"),
        _ => root
            .join("runtime")
            .join(format!("{}-{}", pin.id, pin.version)),
    }
}

pub fn entry_path(pin: &Pinned, root: &Path) -> PathBuf {
    let name = match pin.kind {
        Kind::Zip { extract } => entry_name(extract[0]),
        _ => url_file_name(pin.url),
    };
    install_dir(pin, root).join(name)
}

fn marker_for(path: &Path, pin: &Pinned) -> PathBuf {
    match pin.kind {
        Kind::File => {
            let mut p = path.as_os_str().to_owned();
            p.push(".verified");
            PathBuf::from(p)
        }
        _ => path.parent().unwrap_or(Path::new("")).join(".verified"),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn mtime_ms(meta: &fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn marker_file(path: &Path, sha256: String) -> Result<MarkerFile> {
    let meta = fs::metadata(path)?;
    Ok(MarkerFile {
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size: meta.len(),
        mtime_ms: mtime_ms(&meta),
        sha256,
    })
}

fn write_marker(path: &Path, marker: &Marker) -> Result<()> {
    fs::write(path, serde_json::to_vec(marker)?)?;
    Ok(())
}

pub fn is_installed(pin: &Pinned, root: &Path) -> bool {
    let entry = entry_path(pin, root);
    entry.is_file() && marker_for(&entry, pin).is_file()
}

pub fn remove(pin: &Pinned, root: &Path) -> Result<()> {
    match pin.kind {
        Kind::File => {
            let entry = entry_path(pin, root);
            for p in [marker_for(&entry, pin), entry] {
                if p.exists() {
                    fs::remove_file(&p)?;
                }
            }
        }
        _ => {
            let dir = install_dir(pin, root);
            if dir.exists() {
                fs::remove_dir_all(&dir)?;
            }
        }
    }
    Ok(())
}

pub type ProgressCb<'a> = &'a (dyn Fn(u64, u64) + Send + Sync);

struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = if self.0.is_dir() {
            fs::remove_dir_all(&self.0)
        } else {
            fs::remove_file(&self.0)
        };
    }
}

async fn download(
    pin: &Pinned,
    dest: &Path,
    progress: ProgressCb<'_>,
    cancel: &AtomicBool,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent("hoardini")
        .connect_timeout(std::time::Duration::from_secs(20))
        .read_timeout(std::time::Duration::from_secs(60))
        .build()?;
    let mut resp = client.get(pin.url).send().await?.error_for_status()?;
    let mut out = fs::File::create(dest)?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    progress(0, pin.size);
    while let Some(chunk) = resp.chunk().await? {
        if cancel.load(Ordering::Relaxed) {
            bail!("download cancelled");
        }
        done += chunk.len() as u64;
        if done > pin.size {
            bail!("{} exceeds its pinned size of {} bytes", pin.id, pin.size);
        }
        hasher.update(&chunk);
        out.write_all(&chunk)?;
        progress(done, pin.size);
    }
    out.flush()?;
    drop(out);
    let got = hex(&hasher.finalize());
    if !got.eq_ignore_ascii_case(pin.sha256) {
        bail!(
            "{} failed its SHA-256 check (expected {}, got {got}); file deleted",
            pin.id,
            pin.sha256
        );
    }
    Ok(())
}

fn extract_zip(pin: &Pinned, extract: &[&str], zip_path: &Path, stage: &Path) -> Result<Marker> {
    let mut archive = zip::ZipArchive::new(fs::File::open(zip_path)?)?;
    fs::create_dir_all(stage)?;
    let mut files = Vec::new();
    for want in extract {
        let mut entry = archive
            .by_name(want)
            .with_context(|| format!("{want} is missing from the {} archive", pin.id))?;
        let out_path = stage.join(entry_name(want));
        let mut out = fs::File::create(&out_path)?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            out.write_all(&buf[..n])?;
        }
        drop(out);
        files.push(marker_file(&out_path, hex(&h.finalize()))?);
    }
    Ok(Marker {
        pin_sha256: pin.sha256.to_string(),
        files,
    })
}

pub async fn ensure(
    pin: &Pinned,
    root: &Path,
    progress: ProgressCb<'_>,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    let entry = entry_path(pin, root);
    if verify_before_run(&entry, pin).is_ok() {
        return Ok(entry);
    }
    let dir = install_dir(pin, root);
    let parent = dir.parent().unwrap_or(root).to_path_buf();
    fs::create_dir_all(&parent)?;
    let tag = uuid::Uuid::now_v7().simple().to_string();
    let part = parent.join(format!(".{}-{}.{tag}.part", pin.id, pin.version));
    let _part_guard = Cleanup(part.clone());
    download(pin, &part, progress, cancel).await?;

    match pin.kind {
        Kind::File | Kind::Exe => {
            fs::create_dir_all(&dir)?;
            if entry.exists() {
                fs::remove_file(&entry)?;
            }
            fs::rename(&part, &entry)?;
            let marker = Marker {
                pin_sha256: pin.sha256.to_string(),
                files: vec![marker_file(&entry, pin.sha256.to_ascii_lowercase())?],
            };
            write_marker(&marker_for(&entry, pin), &marker)?;
        }
        Kind::Zip { extract } => {
            let stage = parent.join(format!(".{}-{}.{tag}.stage", pin.id, pin.version));
            let _stage_guard = Cleanup(stage.clone());
            let marker = extract_zip(pin, extract, &part, &stage)?;
            write_marker(&stage.join(".verified"), &marker)?;
            if dir.exists() {
                fs::remove_dir_all(&dir)?;
            }
            // A directory rename keeps member mtimes, so the marker stays valid.
            fs::rename(&stage, &dir)?;
        }
    }
    Ok(entry)
}

pub fn verify_before_run(path: &Path, pin: &Pinned) -> Result<()> {
    let meta =
        fs::metadata(path).with_context(|| format!("{} is not installed", path.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let marker_p = marker_for(path, pin);
    let marker: Option<Marker> = fs::read(&marker_p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let fresh = marker.as_ref().is_some_and(|m| {
        m.pin_sha256.eq_ignore_ascii_case(pin.sha256)
            && m.files
                .iter()
                .any(|f| f.name == name && f.size == meta.len() && f.mtime_ms == mtime_ms(&meta))
    });
    if fresh {
        return Ok(());
    }
    // Zip members have no pinned hash of their own, so a stale marker means reinstall.
    if matches!(pin.kind, Kind::Zip { .. }) {
        bail!("{name} no longer matches its install record; reinstall it");
    }
    let got = hash_file(path)?;
    if !got.eq_ignore_ascii_case(pin.sha256) {
        let _ = fs::remove_file(&marker_p);
        bail!("{name} does not match its pinned SHA-256; refusing to run it");
    }
    let marker = Marker {
        pin_sha256: pin.sha256.to_string(),
        files: vec![marker_file(path, got)?],
    };
    write_marker(&marker_p, &marker)?;
    Ok(())
}

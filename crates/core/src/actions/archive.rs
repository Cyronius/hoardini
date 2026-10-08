use std::io::{Read, Write};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use rusqlite::{Connection, params};

use super::delete::{remaining_alloc, remove_item};
use super::handler::{Cancelled, CopyHook, Handler, ItemCx, JobCx, Outcome, Runner};
use super::journal;
use crate::catalog::Category;
use crate::destinations;
use crate::policy::{ItemKind, PolicyEnv};
use crate::proposals::StoredItem;
use crate::store::{get_meta, now_ms};

const CHUNK: usize = 1 << 20;
pub(crate) const STUB_SUFFIX: &str = ".hoardini.txt";
const STUB_MARKER: &str = "Hoardini archived this item.";
const PARTIAL_SUFFIX: &str = ".hoardini-partial";

pub(crate) struct Copied {
    pub size: u64,
    pub hash: String,
    pub mtime: Option<SystemTime>,
}

// What a source file looked like when its copy started; the source may go only while it still looks so.
pub(crate) struct Seen {
    pub rel: String,
    pub size: u64,
    pub mtime: Option<SystemTime>,
}

pub(crate) fn unchanged(src: &Path, kind: ItemKind, files: &[Seen]) -> anyhow::Result<()> {
    for f in files {
        let p = if kind == ItemKind::File {
            src.to_path_buf()
        } else {
            src.join(&f.rel)
        };
        let md = std::fs::metadata(&p)?;
        if md.len() != f.size || md.modified().ok() != f.mtime {
            anyhow::bail!(
                "{} changed while it was being archived; the source was kept",
                p.display()
            );
        }
    }
    if kind == ItemKind::Dir && list_tree(src)?.files.len() != files.len() {
        anyhow::bail!(
            "files were added to {} while it was being archived; the source was kept",
            src.display()
        );
    }
    Ok(())
}

fn uri_key(s: &str) -> String {
    s.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

pub(crate) fn day_name(day: &str, n: u32) -> String {
    if n <= 1 {
        day.to_string()
    } else {
        format!("{day}-{n}")
    }
}

// A location an earlier entry holds (or a folder above or inside it) is never written into; the next <day>-N is used.
pub(crate) fn free_location<T>(
    conn: &Connection,
    day: &str,
    at: impl Fn(&str) -> (T, String),
) -> anyhow::Result<T> {
    let held: Vec<String> = conn
        .prepare_cached("SELECT dest_uri FROM archive_entry")?
        .query_map([], |r| r.get::<_, String>(0))?
        .map(|r| r.map(|u| uri_key(&u)))
        .collect::<rusqlite::Result<_>>()?;
    for n in 1..=1000 {
        let (target, uri) = at(&day_name(day, n));
        let want = uri_key(&uri);
        let clash = held.iter().any(|h| {
            *h == want || want.starts_with(&format!("{h}/")) || h.starts_with(&format!("{want}/"))
        });
        if !clash {
            return Ok(target);
        }
    }
    anyhow::bail!("no free archive location under {day}")
}

pub(crate) struct Copier<'a> {
    pub cancel: &'a AtomicBool,
    pub progress: &'a mut dyn FnMut(i64),
    pub hook: Option<&'a CopyHook>,
}

impl Copier<'_> {
    fn check(&self) -> anyhow::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        Ok(())
    }
}

fn hash_file(path: &Path, c: Option<&mut Copier<'_>>) -> anyhow::Result<(u64, String)> {
    let mut f = std::fs::File::open(path)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    let mut c = c;
    loop {
        if let Some(c) = c.as_deref() {
            c.check()?;
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        total += n as u64;
        if let Some(c) = c.as_deref_mut() {
            (c.progress)(n as i64);
        }
    }
    Ok((total, h.finalize().to_hex().to_string()))
}

pub(crate) fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

// The final name only ever holds a verified copy, so an interrupted run never leaves a half file that looks done.
pub(crate) fn copy_verified(src: &Path, dst: &Path, c: &mut Copier<'_>) -> anyhow::Result<Copied> {
    let md = std::fs::metadata(src)?;
    if hoardini_win::is_cloud_placeholder(md.file_attributes()) {
        anyhow::bail!(
            "{} is cloud-only; copying it would download it",
            src.display()
        );
    }
    let src_len = md.len();
    let mtime = md.modified().ok();
    if let Ok(existing) = std::fs::metadata(dst)
        && existing.is_file()
        && existing.len() == src_len
    {
        // Resume: a verified copy from an earlier, interrupted run is reused when it still matches.
        let (_, sh) = hash_file(src, Some(c))?;
        let (_, dh) = hash_file(dst, None)?;
        if sh == dh {
            return Ok(Copied {
                size: src_len,
                hash: sh,
                mtime,
            });
        }
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = with_suffix(dst, PARTIAL_SUFFIX);
    let result = (|| -> anyhow::Result<Copied> {
        let mut r = std::fs::File::open(src)?;
        let mut w = std::fs::File::create(&tmp)?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; CHUNK];
        let mut total = 0u64;
        loop {
            c.check()?;
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            w.write_all(&buf[..n])?;
            total += n as u64;
            (c.progress)(n as i64);
        }
        if let Ok(t) = md.modified() {
            let _ = w.set_modified(t);
        }
        w.sync_all()?;
        drop(w);
        if total != src_len {
            anyhow::bail!("{} changed while it was being copied", src.display());
        }
        let src_hash = hasher.finalize().to_hex().to_string();
        if let Some(hook) = c.hook {
            hook(&tmp);
        }
        let (dst_len, dst_hash) = hash_file(&tmp, None)?;
        if dst_len != total || dst_hash != src_hash {
            anyhow::bail!(
                "verification failed: the copy at {} does not match the source (size or BLAKE3)",
                dst.display()
            );
        }
        std::fs::rename(&tmp, dst)?;
        Ok(Copied {
            size: total,
            hash: src_hash,
            mtime,
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

pub(crate) struct Tree {
    pub dirs: Vec<String>,
    pub files: Vec<String>,
    pub links: usize,
}

pub(crate) fn list_tree(root: &Path) -> anyhow::Result<Tree> {
    let mut t = Tree {
        dirs: Vec::new(),
        files: Vec::new(),
        links: 0,
    };
    let mut stack = vec![String::new()];
    while let Some(rel) = stack.pop() {
        let dir = if rel.is_empty() {
            root.to_path_buf()
        } else {
            root.join(&rel)
        };
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let ft = entry.file_type()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let child = if rel.is_empty() {
                name
            } else {
                format!("{rel}\\{name}")
            };
            if ft.is_symlink() {
                t.links += 1;
            } else if ft.is_dir() {
                t.dirs.push(child.clone());
                stack.push(child);
            } else {
                t.files.push(child);
            }
        }
    }
    t.dirs.sort();
    t.files.sort();
    Ok(t)
}

pub(crate) fn manifest_hash(entries: &[(String, u64, String)]) -> String {
    let mut h = blake3::Hasher::new();
    for (rel, size, hash) in entries {
        h.update(format!("{rel}\t{size}\t{hash}\n").as_bytes());
    }
    h.finalize().to_hex().to_string()
}

pub(crate) struct TreeCopy {
    pub size: u64,
    pub hash: String,
    pub links: usize,
    pub files: Vec<Seen>,
}

// archive_entry has one hash column, so a folder stores BLAKE3 over its sorted (path, size, file hash) list.
pub(crate) fn copy_tree(src: &Path, dst: &Path, c: &mut Copier<'_>) -> anyhow::Result<TreeCopy> {
    let tree = list_tree(src)?;
    std::fs::create_dir_all(dst)?;
    for d in &tree.dirs {
        std::fs::create_dir_all(dst.join(d))?;
    }
    let mut entries = Vec::with_capacity(tree.files.len());
    let mut files = Vec::with_capacity(tree.files.len());
    let mut total = 0u64;
    for f in &tree.files {
        c.check()?;
        let copied = copy_verified(&src.join(f), &dst.join(f), c)?;
        total += copied.size;
        files.push(Seen {
            rel: f.clone(),
            size: copied.size,
            mtime: copied.mtime,
        });
        entries.push((f.clone(), copied.size, copied.hash));
    }
    Ok(TreeCopy {
        size: total,
        hash: manifest_hash(&entries),
        links: tree.links,
        files,
    })
}

pub(crate) fn layout_rel(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\") {
        return format!("UNC\\{rest}");
    }
    let b = path.as_bytes();
    if b.len() >= 2 && b[1] == b':' {
        return format!("{}{}", &path[..1], &path[2..]);
    }
    path.to_string()
}

pub(crate) fn local_date(ms: i64) -> String {
    ymd(ms + hoardini_win::local_offset_ms(ms))
}

fn ymd(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn stub_text(original: &str, dest_uri: &str, date: &str) -> String {
    format!(
        "{STUB_MARKER}\r\n\r\nThis note stands in for something Hoardini moved off this PC to free up space.\r\n\r\nWhere it was: {original}\r\nWhere it is now: {dest_uri}\r\nMoved on: {date}\r\n\r\nTo get it back: open Hoardini, go to the Archive list, find this item, and choose Restore. Hoardini copies it back to where it was and deletes this note.\r\n"
    )
}

// Stubs help where a person looks for their files, not inside caches an app manages.
fn wants_stub(category: Option<Category>) -> bool {
    matches!(
        category,
        None | Some(Category::UserData) | Some(Category::Downloads)
    )
}

pub(crate) struct ArchiveHandler;

impl Handler for ArchiveHandler {
    fn weight(&self, item: &StoredItem) -> i64 {
        item.logical
    }

    fn begin(&self, job: &JobCx<'_>) -> anyhow::Result<Box<dyn Runner>> {
        let dref = job
            .proposal
            .destination
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("archive proposal has no destination"))?;
        let dest = destinations::get(job.conn, &dref.id)?
            .ok_or_else(|| anyhow::anyhow!("destination {} was removed", dref.name))?;
        if dest.kind == "rclone" {
            return super::rclone::begin(job, &dest);
        }
        let root = destinations::connect(&dest, job.secrets)?;
        if !root.is_dir() {
            anyhow::bail!("destination folder {} not found", root.display());
        }
        let need: i64 = job.items.iter().map(|i| i.logical).sum();
        if let Ok(free) = destinations::free_bytes(&root)
            && crate::index::to_i64(free) < need
        {
            anyhow::bail!(
                "{} has {} free; this archive needs {}",
                dest.name,
                crate::paths::humanize_bytes(free),
                crate::paths::humanize_bytes(crate::index::to_u64(need))
            );
        }
        let stubs = get_meta(job.conn, "archive_stubs")?.is_none_or(|v| v.trim() != "false");
        Ok(Box::new(ArchiveRunner {
            dest_id: dest.id,
            root,
            day: local_date(now_ms()),
            stubs,
            env: job.env.clone(),
            hook: job.hook.clone(),
        }))
    }
}

struct ArchiveRunner {
    dest_id: String,
    root: PathBuf,
    day: String,
    stubs: bool,
    env: PolicyEnv,
    hook: Option<CopyHook>,
}

impl Runner for ArchiveRunner {
    fn run(&mut self, cx: &mut ItemCx<'_>) -> anyhow::Result<Outcome> {
        let item = cx.item;
        let src = PathBuf::from(&item.path);
        let rel = layout_rel(&item.path);
        let target = free_location(cx.conn, &self.day, |day| {
            let t = self.root.join(day).join(&rel);
            let uri = t.to_string_lossy().into_owned();
            (t, uri)
        })?;
        let mut copier = Copier {
            cancel: cx.cancel,
            progress: &mut *cx.progress,
            hook: self.hook.as_ref(),
        };
        let (size, hash, links, seen) = match item.kind {
            ItemKind::File => {
                let c = copy_verified(&src, &target, &mut copier)?;
                let seen = Seen {
                    rel: String::new(),
                    size: c.size,
                    mtime: c.mtime,
                };
                (c.size, c.hash, 0, vec![seen])
            }
            ItemKind::Dir => {
                let t = copy_tree(&src, &target, &mut copier)?;
                (t.size, t.hash, t.links, t.files)
            }
            ItemKind::Link => anyhow::bail!("a link holds no data; delete it instead"),
        };
        cx.check_cancel()?;
        unchanged(&src, item.kind, &seen)?;
        let dest_uri = target.to_string_lossy().into_owned();
        let to = Committed {
            dest_id: &self.dest_id,
            dest_uri: &dest_uri,
            size,
            hash: &hash,
            links,
        };
        commit_archive(cx, &to, self.stubs, &self.env)
    }
}

pub(crate) struct Committed<'a> {
    pub dest_id: &'a str,
    pub dest_uri: &'a str,
    pub size: u64,
    pub hash: &'a str,
    pub links: usize,
}

// The journal learns where the copy is before the source goes away.
pub(crate) fn commit_archive(
    cx: &mut ItemCx<'_>,
    to: &Committed<'_>,
    stubs: bool,
    env: &PolicyEnv,
) -> anyhow::Result<Outcome> {
    {
        let item = cx.item;
        let src = PathBuf::from(&item.path);
        let Committed {
            dest_id,
            dest_uri,
            size,
            hash,
            links,
        } = *to;
        let category = env.catalog().lookup_match(&item.path).map(|m| m.category);
        let stub = (stubs && wants_stub(category)).then(|| format!("{}{STUB_SUFFIX}", item.path));
        cx.conn.execute(
            "INSERT INTO archive_entry(action_id, original_path, destination_id, dest_uri, kind, size, blake3, stub_path, archived_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                cx.action_id,
                item.path,
                dest_id,
                dest_uri,
                item.kind.to_db(),
                crate::index::to_i64(size),
                hash,
                stub,
                now_ms()
            ],
        )?;
        let entry_id = cx.conn.last_insert_rowid();
        let undo = serde_json::json!({
            "kind": "restore",
            "archive_entry_id": entry_id,
            "destination_id": dest_id,
            "dest_uri": dest_uri,
            "blake3": hash,
        });

        journal::set_undo(cx.conn, cx.action_id, &undo)?;
        if let Err(e) = remove_item(&src, item.kind) {
            if item.kind == ItemKind::Dir && src.exists() {
                cx.freed = (item.alloc - remaining_alloc(&src)).max(0);
                cx.rescan = true;
            }
            anyhow::bail!("copied and verified to {dest_uri}, but removing the source failed: {e}");
        }
        cx.freed = item.alloc;
        if let Some(stub) = &stub {
            let text = stub_text(&item.path, dest_uri, &local_date(now_ms()));
            if std::fs::write(stub, text).is_ok() {
                cx.rescan = true;
            } else {
                cx.conn.execute(
                    "UPDATE archive_entry SET stub_path = NULL WHERE id = ?1",
                    [entry_id],
                )?;
            }
        }
        let mut detail = format!("Archived to {dest_uri}");
        if links > 0 {
            detail.push_str(&format!(" ({links} links inside were not archived)"));
        }
        Ok(Outcome {
            undo,
            detail: Some(detail),
        })
    }
}

pub(crate) struct EntryRow {
    pub id: i64,
    pub original_path: String,
    pub destination_id: String,
    pub dest_uri: String,
    pub kind: ItemKind,
    pub size: i64,
    pub blake3: Option<String>,
    pub stub_path: Option<String>,
}

pub(crate) fn load_entry(conn: &Connection, id: i64) -> anyhow::Result<Option<EntryRow>> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .query_row(
            "SELECT id, original_path, destination_id, dest_uri, kind, size, blake3, stub_path FROM archive_entry WHERE id = ?1",
            [id],
            |r| {
                Ok(EntryRow {
                    id: r.get(0)?,
                    original_path: r.get(1)?,
                    destination_id: r.get(2)?,
                    dest_uri: r.get(3)?,
                    kind: ItemKind::from_db(r.get(4)?),
                    size: r.get(5)?,
                    blake3: r.get(6)?,
                    stub_path: r.get(7)?,
                })
            },
        )
        .optional()?)
}

pub(crate) fn remove_tmp(p: &Path) {
    if let Ok(md) = std::fs::symlink_metadata(p) {
        let _ = if md.is_dir() {
            std::fs::remove_dir_all(p)
        } else {
            std::fs::remove_file(p)
        };
    }
}

// A failed restore must leave nothing at the original path, hence the sibling temp name.
pub(crate) fn restore_entry(
    conn: &Connection,
    e: &EntryRow,
    c: &mut Copier<'_>,
) -> anyhow::Result<()> {
    let original = PathBuf::from(&e.original_path);
    if std::fs::symlink_metadata(&original).is_ok() {
        anyhow::bail!("something already exists at {}", e.original_path);
    }
    let src = PathBuf::from(&e.dest_uri);
    let tmp = with_suffix(&original, ".hoardini-restore");
    remove_tmp(&tmp);
    if let Some(parent) = original.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let result = (|| -> anyhow::Result<()> {
        let hash = match e.kind {
            ItemKind::Dir => copy_tree(&src, &tmp, c)?.hash,
            _ => copy_verified(&src, &tmp, c)?.hash,
        };
        if let Some(expected) = &e.blake3
            && !expected.eq_ignore_ascii_case(&hash)
        {
            anyhow::bail!(
                "the archived copy at {} no longer matches the BLAKE3 recorded when it was archived",
                e.dest_uri
            );
        }
        std::fs::rename(&tmp, &original)?;
        Ok(())
    })();
    if result.is_err() {
        remove_tmp(&tmp);
    }
    result?;
    finish_restore(conn, e)
}

pub(crate) fn finish_restore(conn: &Connection, e: &EntryRow) -> anyhow::Result<()> {
    if let Some(stub) = &e.stub_path
        && std::fs::read_to_string(stub).is_ok_and(|t| t.starts_with(STUB_MARKER))
    {
        let _ = std::fs::remove_file(stub);
    }
    conn.execute(
        "UPDATE archive_entry SET restored_ms = ?2 WHERE id = ?1",
        params![e.id, now_ms()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stubs_say_where_it_went_and_how_to_restore_and_skip_caches() {
        let t = stub_text(
            r"C:\x\a.zip",
            r"D:\store\2026-10-08\C\x\a.zip",
            "2026-10-08",
        );
        assert!(t.starts_with(STUB_MARKER));
        assert!(t.contains(r"C:\x\a.zip") && t.contains(r"D:\store\2026-10-08"));
        assert!(t.contains("Archive list") && t.contains("Restore"));
        for c in [
            Category::Cache,
            Category::PackageCache,
            Category::BrowserCache,
            Category::Temp,
        ] {
            assert!(!wants_stub(Some(c)), "{c:?}");
        }
        assert!(wants_stub(None) && wants_stub(Some(Category::UserData)));
    }

    #[test]
    fn dates_and_layout() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(1_791_417_600_000), "2026-10-08");
        assert_eq!(ymd(951_782_400_000), "2000-02-29");
        let off = hoardini_win::local_offset_ms(1_791_417_600_000);
        assert!(off.abs() <= 14 * 3_600_000 && off % 900_000 == 0);
        assert_eq!(local_date(1_791_417_600_000), ymd(1_791_417_600_000 + off));
        assert_eq!(layout_rel(r"C:\Users\x\a.bin"), r"C\Users\x\a.bin");
        assert_eq!(layout_rel(r"\\nas\share\a"), r"UNC\nas\share\a");
    }
}

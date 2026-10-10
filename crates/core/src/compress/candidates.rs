use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::catalog::Category;
use crate::index::{self, FLAG_HARDLINK_DUP, FLAG_PLACEHOLDER, KIND_DIR, KIND_FILE, PathCache};
use crate::policy::{PolicyEnv, deny_reason};
use crate::store::{Db, now_ms};

pub const MIN_FOLDER: u64 = 2 << 30;
pub const MIN_SAVED: u64 = 1 << 30;
pub const MIN_SAVED_PERCENT: u64 = 20;
pub const QUIET_DAYS: i64 = 30;
pub const ESTIMATE_TOP: usize = 10;
const DAY_MS: i64 = 86_400_000;
const MAX_EVALUATED: usize = 400;
const FILE_ATTRIBUTE_SPARSE: i64 = 0x200;
const FILE_ATTRIBUTE_COMPRESSED: i64 = 0x800;

// Media, archives, packages and quantized weights: recompressing them gains about nothing.
const PACKED_TYPES: &[&str] = &[
    "mp4",
    "m4v",
    "mkv",
    "webm",
    "avi",
    "mov",
    "wmv",
    "flv",
    "mpg",
    "mpeg",
    "m2ts",
    "bk2",
    "mp3",
    "aac",
    "ogg",
    "opus",
    "flac",
    "m4a",
    "wma",
    "jpg",
    "jpeg",
    "png",
    "webp",
    "gif",
    "heic",
    "heif",
    "avif",
    "jxl",
    "zip",
    "7z",
    "rar",
    "gz",
    "tgz",
    "xz",
    "txz",
    "bz2",
    "zst",
    "lz4",
    "lzma",
    "cab",
    "msi",
    "msix",
    "appx",
    "appxbundle",
    "msixbundle",
    "nupkg",
    "whl",
    "jar",
    "apk",
    "docx",
    "xlsx",
    "pptx",
    "epub",
    "gguf",
];
// WSL and Hyper-V refuse a compressed virtual disk, so a folder holding one is never a candidate.
const VDISK_TYPES: &[&str] = &["vhdx", "vhd", "avhdx", "vmdk", "vdi", "qcow2"];
// These get delete cards; compressing them would only list the same bytes twice.
const SKIP_CATEGORIES: &[Category] = &[
    Category::Cache,
    Category::BrowserCache,
    Category::PackageCache,
    Category::BuildOutput,
    Category::Temp,
    Category::CrashDumps,
    Category::WindowsLeftover,
    Category::System,
    Category::VmDisk,
    Category::InstallerCache,
    Category::Downloads,
];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CachedEstimate {
    pub logical: u64,
    pub saved_bytes: u64,
    pub ratio: f64,
    pub estimated_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CompressCandidate {
    pub path: String,
    pub size: u64,
    pub logical: u64,
    pub files: u64,
    pub newest_ms: Option<i64>,
    pub estimate: Option<CachedEstimate>,
}

impl CompressCandidate {
    pub fn proposable(&self) -> bool {
        self.estimate
            .as_ref()
            .is_some_and(|e| proposable(e.saved_bytes, e.logical))
    }
}

pub fn proposable(saved: u64, logical: u64) -> bool {
    saved >= MIN_SAVED && saved.saturating_mul(100) >= logical.saturating_mul(MIN_SAVED_PERCENT)
}

struct Big {
    parent: Option<i64>,
    size: u64,
    logical: u64,
    files: u64,
}

#[derive(Default)]
struct Subtree {
    newest_ms: Option<i64>,
    file_logical: u64,
    packed_logical: u64,
    links: bool,
    placeholders: bool,
    root_compressed: bool,
}

fn subtree(conn: &Connection, id: i64) -> anyhow::Result<Subtree> {
    let mut st = conn.prepare_cached(
        "WITH RECURSIVE d(id) AS (SELECT ?1 UNION ALL \
         SELECT n.id FROM node n JOIN d ON n.parent_id = d.id WHERE +n.kind = 0) \
         SELECT n.id, n.kind, n.mtime, n.alloc, n.logical, n.attrs, n.flags FROM node n JOIN d ON n.id = d.id \
         UNION ALL SELECT n.id, n.kind, n.mtime, n.alloc, n.logical, n.attrs, n.flags \
         FROM node n JOIN d ON n.parent_id = d.id WHERE +n.kind != 0",
    )?;
    let mut s = Subtree::default();
    let mut rows = st.query([id])?;
    while let Some(r) = rows.next()? {
        let (nid, kind, mtime): (i64, i64, Option<i64>) = (r.get(0)?, r.get(1)?, r.get(2)?);
        let (alloc, logical, attrs, flags): (i64, i64, i64, i64) =
            (r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?);
        s.newest_ms = s.newest_ms.max(mtime);
        if kind == KIND_DIR {
            if nid == id {
                s.root_compressed = attrs & FILE_ATTRIBUTE_COMPRESSED != 0;
            }
            continue;
        }
        if kind != KIND_FILE {
            s.links = true;
            continue;
        }
        s.placeholders |= flags & FLAG_PLACEHOLDER != 0;
        if flags & FLAG_HARDLINK_DUP != 0 {
            continue;
        }
        let logical = index::to_u64(logical);
        s.file_logical += logical;
        // WOF hides its tag from listings, so an allocation well under the size is the sign.
        let shrunk = attrs & FILE_ATTRIBUTE_SPARSE == 0
            && flags & FLAG_PLACEHOLDER == 0
            && index::to_u64(alloc).saturating_mul(10) < logical.saturating_mul(9);
        if attrs & FILE_ATTRIBUTE_COMPRESSED != 0 || shrunk {
            s.packed_logical += logical;
        }
    }
    Ok(s)
}

fn qualifies(conn: &Connection, id: i64, now: i64) -> anyhow::Result<Option<Option<i64>>> {
    let s = subtree(conn, id)?;
    if s.newest_ms.is_some_and(|m| m >= now - QUIET_DAYS * DAY_MS)
        || s.links
        || s.placeholders
        || s.root_compressed
        || (s.file_logical > 0 && s.packed_logical.saturating_mul(2) >= s.file_logical)
    {
        return Ok(None);
    }
    let mix = index::ext_mix(conn, id)?;
    if mix
        .types
        .iter()
        .any(|t| VDISK_TYPES.contains(&t.ext.as_str()))
    {
        return Ok(None);
    }
    let packed: u64 = mix
        .types
        .iter()
        .filter(|t| PACKED_TYPES.contains(&t.ext.as_str()))
        .map(|t| t.bytes)
        .sum();
    if mix.total_bytes > 0 && packed.saturating_mul(2) >= mix.total_bytes {
        return Ok(None);
    }
    Ok(Some(s.newest_ms))
}

// Top-down from `root`: a container (two big children, or one holding 90%) is opened, never listed.
pub fn candidates(
    conn: &Connection,
    env: &PolicyEnv,
    root: &str,
    now: i64,
) -> anyhow::Result<Vec<CompressCandidate>> {
    let Some(root_id) = index::node_id_by_path(conn, root)? else {
        return Ok(Vec::new());
    };
    let volume: i64 =
        conn.query_row("SELECT volume_id FROM node WHERE id = ?1", [root_id], |r| {
            r.get(0)
        })?;
    let mut big: HashMap<i64, Big> = HashMap::new();
    let mut kids: HashMap<i64, Vec<i64>> = HashMap::new();
    {
        let mut st = conn.prepare(
            "SELECT id, parent_id, sub_alloc, sub_logical, sub_count FROM node \
             WHERE kind = 0 AND sub_alloc >= ?1 AND volume_id = ?2",
        )?;
        let rows = st.query_map(params![index::to_i64(MIN_FOLDER), volume], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                Big {
                    parent: r.get(1)?,
                    size: index::to_u64(r.get(2)?),
                    logical: index::to_u64(r.get(3)?),
                    files: index::to_u64(r.get(4)?),
                },
            ))
        })?;
        for row in rows {
            let (id, b) = row?;
            if let Some(p) = b.parent {
                kids.entry(p).or_default().push(id);
            }
            big.insert(id, b);
        }
    }
    let mut paths = PathCache::default();
    let mut out = Vec::new();
    let mut stack = if big.contains_key(&root_id) {
        vec![root_id]
    } else {
        Vec::new()
    };
    let mut evaluated = 0usize;
    while let Some(id) = stack.pop() {
        let Some(b) = big.get(&id) else { continue };
        let Some(path) = paths.get(conn, id)? else {
            continue;
        };
        let children = kids.get(&id).cloned().unwrap_or_default();
        if env
            .catalog()
            .lookup_match(&path)
            .is_some_and(|m| SKIP_CATEGORIES.contains(&m.category))
        {
            continue;
        }
        let container = children.len() >= 2
            || children.iter().any(|c| {
                big.get(c)
                    .is_some_and(|k| k.size.saturating_mul(10) >= b.size * 9)
            });
        if container || deny_reason(conn, env, &path)?.is_some() || evaluated >= MAX_EVALUATED {
            stack.extend(children);
            continue;
        }
        evaluated += 1;
        match qualifies(conn, id, now)? {
            Some(newest_ms) => out.push(CompressCandidate {
                estimate: cached(conn, &path, b.size, newest_ms)?,
                path,
                size: b.size,
                logical: b.logical,
                files: b.files,
                newest_ms,
            }),
            None => stack.extend(children),
        }
    }
    out.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    Ok(out)
}

fn cached(
    conn: &Connection,
    path: &str,
    size: u64,
    newest_ms: Option<i64>,
) -> anyhow::Result<Option<CachedEstimate>> {
    Ok(conn
        .query_row(
            "SELECT logical, saved_bytes, ratio, estimated_ms FROM compress_estimate \
             WHERE path = ?1 AND size = ?2 AND newest_ms IS ?3",
            params![path, index::to_i64(size), newest_ms],
            |r| {
                Ok(CachedEstimate {
                    logical: index::to_u64(r.get(0)?),
                    saved_bytes: index::to_u64(r.get(1)?),
                    ratio: r.get(2)?,
                    estimated_ms: r.get(3)?,
                })
            },
        )
        .optional()?)
}

// What compressing the folder would save, by its last estimate; a compress item counts this, not its size.
pub fn saved_bytes(conn: &Connection, path: &str) -> anyhow::Result<Option<u64>> {
    Ok(conn
        .query_row(
            "SELECT saved_bytes FROM compress_estimate WHERE path = ?1",
            [path],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .map(index::to_u64))
}

pub fn estimate_top(db: &Db, cands: &[CompressCandidate], limit: usize) -> anyhow::Result<usize> {
    let conn = db.conn()?;
    let mut ran = 0;
    for c in cands.iter().take(limit) {
        if cached(&conn, &c.path, c.size, c.newest_ms)?.is_some() {
            continue;
        }
        let est = match super::estimate(Path::new(&c.path)) {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!(path = %c.path, error = %e, "compression estimate skipped");
                continue;
            }
        };
        let Some(lzx) = est.formats.iter().find(|f| f.format == "lzx") else {
            continue;
        };
        conn.execute(
            "INSERT OR REPLACE INTO compress_estimate(path, size, newest_ms, logical, saved_bytes, ratio, estimated_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                c.path,
                index::to_i64(c.size),
                c.newest_ms,
                index::to_i64(est.logical_bytes),
                index::to_i64(lzx.saved_bytes),
                lzx.ratio,
                now_ms()
            ],
        )?;
        ran += 1;
    }
    Ok(ran)
}

pub fn spawn_estimates(
    db: Db,
    cands: Vec<CompressCandidate>,
) -> std::io::Result<std::thread::JoinHandle<usize>> {
    std::thread::Builder::new()
        .name("hoardini-compress-estimates".into())
        .spawn(move || {
            estimate_top(&db, &cands, ESTIMATE_TOP).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "compression estimates failed");
                0
            })
        })
}

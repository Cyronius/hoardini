use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::{PolicyEnv, Verdict, cap_for_with, deny_reason};
use crate::catalog::{Category, UnitKind};
use crate::index::{self, TopFilesQuery, normalize_path};

pub const IN_USE_FILE_CAP: usize = 500;
// Restart Manager cost grows with file count (500 cold files took up to 76 s); execution re-checks with the full cap.
pub const PROPOSE_IN_USE_FILE_CAP: usize = 64;
const SCAN_BUDGET: usize = 250_000;
const NAME_SURROGATE: u32 = 0x2000_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Dir,
    File,
    Link,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemKind::Dir => "dir",
            ItemKind::File => "file",
            ItemKind::Link => "link",
        }
    }

    pub fn to_db(self) -> i64 {
        match self {
            ItemKind::Dir => index::KIND_DIR,
            ItemKind::File => index::KIND_FILE,
            ItemKind::Link => index::KIND_LINK,
        }
    }

    pub fn from_db(k: i64) -> ItemKind {
        match k {
            index::KIND_DIR => ItemKind::Dir,
            index::KIND_FILE => ItemKind::File,
            _ => ItemKind::Link,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Delete,
    Archive,
    Compress,
    Dedupe,
    #[serde(rename = "system_cleanup")]
    SystemCleanup,
    #[serde(rename = "online_only")]
    OnlineOnly,
}

impl Action {
    pub const ALL: [Action; 4] = [
        Action::Delete,
        Action::Dedupe,
        Action::Archive,
        Action::Compress,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Action::Delete => "delete",
            Action::Archive => "archive",
            Action::Compress => "compress",
            Action::Dedupe => "dedupe",
            Action::SystemCleanup => "system_cleanup",
            Action::OnlineOnly => "online_only",
        }
    }

    fn reads_data(self) -> bool {
        matches!(self, Action::Archive | Action::Compress)
    }
}

// What the fs says right now; revalidation compares a fresh one against the stored one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsState {
    pub kind: ItemKind,
    pub logical: i64,
    pub mtime: Option<i64>,
    pub file_id: Option<[u8; 16]>,
    pub attrs: u32,
}

pub fn fs_state(path: &Path) -> std::io::Result<FsState> {
    let md = std::fs::symlink_metadata(path)?;
    let ft = md.file_type();
    let kind = if ft.is_symlink() {
        ItemKind::Link
    } else if ft.is_dir() {
        ItemKind::Dir
    } else {
        ItemKind::File
    };
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    Ok(FsState {
        kind,
        logical: if kind == ItemKind::File {
            index::to_i64(md.len())
        } else {
            0
        },
        mtime,
        file_id: hoardini_win::file_id_of(path).ok(),
        attrs: md.file_attributes(),
    })
}

fn strip_verbatim(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    }
}

fn lexical(p: &str) -> String {
    let p = p.replace('/', "\\");
    let (prefix, rest) = if let Some(r) = p.strip_prefix(r"\\") {
        let mut parts = r.splitn(3, '\\');
        let server = parts.next().unwrap_or("");
        let share = parts.next().unwrap_or("");
        (
            format!(r"\\{server}\{share}"),
            parts.next().unwrap_or("").to_string(),
        )
    } else {
        let (d, r) = p.split_at(p.find('\\').unwrap_or(p.len()));
        (d.to_string(), r.to_string())
    };
    let mut out: Vec<&str> = Vec::new();
    for c in rest.split('\\') {
        match c {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    normalize_path(&format!("{prefix}\\{}", out.join("\\")))
}

fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        || p.starts_with(r"\\")
        || p.starts_with("//")
}

// A trailing dot or an 8.3 alias opens the same entry but would dodge rules keyed by its real name.
fn on_disk_name(path: &Path) -> Option<String> {
    let id = hoardini_win::file_id_of(path).ok()?;
    hoardini_win::list_dir(path.parent()?)
        .ok()?
        .into_iter()
        .find(|e| e.file_id == id)
        .map(|e| e.name)
}

// Short names and junctions in the parent chain resolve to the real location; a final link stays itself.
pub fn resolve(env: &PolicyEnv, raw: &str) -> Result<String, String> {
    let expanded = env.expand(raw);
    let expanded = expanded
        .strip_prefix(r"\\?\")
        .map(|s| s.to_string())
        .unwrap_or(expanded);
    if !is_absolute(&expanded) {
        return Err(format!("{raw} is not an absolute path"));
    }
    let lex = lexical(&expanded);
    let path = Path::new(&lex);
    let Ok(md) = std::fs::symlink_metadata(path) else {
        return Ok(lex);
    };
    // Opening a cloud placeholder can trigger a recall, so only its parent is resolved.
    let real = if md.file_type().is_symlink()
        || hoardini_win::is_cloud_placeholder(md.file_attributes())
    {
        match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => {
                let name = on_disk_name(path).unwrap_or_else(|| name.to_string_lossy().into());
                std::fs::canonicalize(parent)
                    .map(|p| strip_verbatim(&p.join(name)))
                    .unwrap_or_else(|_| lex.clone())
            }
            _ => lex.clone(),
        }
    } else {
        std::fs::canonicalize(path)
            .map(|p| strip_verbatim(&p))
            .unwrap_or_else(|_| lex.clone())
    };
    Ok(normalize_path(&real))
}

#[derive(Clone, Debug)]
pub struct Examined {
    pub path: String,
    pub state: Option<FsState>,
    pub alloc: i64,
    pub logical: i64,
    pub node_id: Option<i64>,
    pub block: Option<String>,
    pub cap: Option<Verdict>,
    pub category: Option<Category>,
    pub copy: Option<crate::copycheck::CopyResult>,
}

#[derive(Default)]
struct TreeScan {
    alloc: u64,
    logical: u64,
    files: Vec<(u64, PathBuf)>,
    placeholder: bool,
}

// The review judged a part by what the index held; anything written or created inside it since must still stop it.
fn fresh_inside(root: &Path, kind: ItemKind, hours: u32) -> Option<String> {
    let cutoff_ms = crate::store::now_ms() - i64::from(hours) * 3_600_000;
    let fresh = || {
        Some(format!(
            "Changed since the proposal: something in it was written or created in the last {hours} hours"
        ))
    };
    let unchecked = || Some(UNCHECKED_INSIDE_REASON.to_string());
    let md = std::fs::symlink_metadata(root).ok()?;
    let ms = |t: std::io::Result<std::time::SystemTime>| {
        t.ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(i64::MAX, |d| d.as_millis() as i64)
    };
    if ms(md.created()).max(ms(md.modified())) > cutoff_ms {
        return fresh();
    }
    if kind != ItemKind::Dir {
        return None;
    }
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = hoardini_win::list_dir(&dir) else {
            return unchecked();
        };
        for e in entries {
            seen += 1;
            if seen > SCAN_BUDGET {
                return unchecked();
            }
            if e.ctime_ms.max(e.mtime_ms) > cutoff_ms {
                return fresh();
            }
            if e.is_dir() && (!e.is_reparse() || e.reparse_tag & NAME_SURROGATE == 0) {
                stack.push(dir.join(&e.name));
            }
        }
    }
    None
}

// Name surrogates are links (junctions, symlinks); other reparse folders, like cloud folders, are real.
fn scan_tree(root: &Path) -> TreeScan {
    let mut out = TreeScan::default();
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = hoardini_win::list_dir(&dir) else {
            continue;
        };
        for e in entries {
            seen += 1;
            if seen > SCAN_BUDGET {
                return out;
            }
            let path = dir.join(&e.name);
            if e.is_dir() {
                if !e.is_reparse() || e.reparse_tag & NAME_SURROGATE == 0 {
                    stack.push(path);
                }
                continue;
            }
            if e.is_link() {
                continue;
            }
            out.placeholder |= e.is_placeholder();
            let alloc = if e.needs_alloc_by_open() {
                e.logical
            } else {
                e.alloc
            };
            out.alloc += alloc;
            out.logical += e.logical;
            out.files.push((alloc, path));
        }
    }
    out
}

fn largest_files(conn: &Connection, dir: &str, cap: usize) -> Vec<PathBuf> {
    let q = TopFilesQuery {
        under: Some(dir.to_string()),
        min_size: 0,
        ext: None,
        not_used_for_days: None,
        limit: cap,
    };
    match index::top_files(conn, &q) {
        Ok(r) => r.items.into_iter().map(|f| PathBuf::from(f.path)).collect(),
        Err(_) => {
            let mut files = scan_tree(Path::new(dir)).files;
            files.sort_by(|a, b| b.0.cmp(&a.0));
            files.truncate(cap);
            files.into_iter().map(|(_, p)| p).collect()
        }
    }
}

fn describe_users(users: &[hoardini_win::ProcessUse]) -> Option<String> {
    let mut seen: Vec<(u32, String)> = Vec::new();
    for u in users {
        if !seen.iter().any(|(pid, _)| *pid == u.pid) {
            let name = if u.app_name.trim().is_empty() {
                u.exe
                    .as_deref()
                    .and_then(|e| e.rsplit('\\').next())
                    .unwrap_or("another program")
                    .to_string()
            } else {
                u.app_name.clone()
            };
            seen.push((u.pid, name));
        }
    }
    let first = seen
        .iter()
        .take(3)
        .map(|(pid, name)| format!("{name} (pid {pid})"))
        .collect::<Vec<_>>()
        .join(", ");
    match seen.len() {
        0 => None,
        n if n > 3 => Some(format!("Open in {first} +{} more", n - 3)),
        _ => Some(format!("Open in {first}")),
    }
}

fn processes_using_files(files: &[PathBuf]) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let users = match hoardini_win::processes_using(files) {
        Ok(u) => u,
        Err(_) => {
            // A stale index row can name a path that is now a folder, which Restart Manager rejects.
            let only_files: Vec<PathBuf> = files.iter().filter(|f| f.is_file()).cloned().collect();
            match hoardini_win::processes_using(&only_files) {
                Ok(u) => u,
                Err(err) => {
                    tracing::warn!(error = %err, "open-file check failed");
                    return None;
                }
            }
        }
    };
    describe_users(&users)
}

pub fn in_use_reason(conn: &Connection, path: &str, kind: ItemKind) -> Option<String> {
    in_use_reason_capped(conn, path, kind, IN_USE_FILE_CAP)
}

pub fn in_use_reason_capped(
    conn: &Connection,
    path: &str,
    kind: ItemKind,
    cap: usize,
) -> Option<String> {
    match kind {
        ItemKind::File => processes_using_files(&[PathBuf::from(path)]),
        ItemKind::Dir => processes_using_files(&largest_files(conn, path, cap)),
        ItemKind::Link => None,
    }
}

fn dir_has_placeholder(conn: &Connection, node_id: i64) -> anyhow::Result<bool> {
    Ok(conn.query_row(
        "WITH RECURSIVE d(id) AS (SELECT ?1 UNION ALL SELECT n.id FROM node n JOIN d ON n.parent_id = d.id WHERE +n.kind = 0) \
         SELECT EXISTS(SELECT 1 FROM node f WHERE f.parent_id IN (SELECT id FROM d) AND (f.flags & ?2) != 0)",
        rusqlite::params![node_id, index::FLAG_PLACEHOLDER],
        |r| r.get(0),
    )?)
}

const NOT_SYNCED_REASON: &str = "Not in a cloud-synced folder (OneDrive, Dropbox, iCloud, ...); there is no cloud copy to fall back on";
const ALREADY_ONLINE_REASON: &str = "Already online-only; nothing is stored on this PC";

fn online_only_block(state: &FsState, alloc: i64) -> Option<String> {
    if alloc == 0
        || (state.kind == ItemKind::File && hoardini_win::is_cloud_placeholder(state.attrs))
    {
        return Some(ALREADY_ONLINE_REASON.into());
    }
    None
}

const COPY_GONE_REASON: &str = "Its copy no longer counts: gone, changed, in the Recycle Bin, in a temp or cache folder, or set to be removed too";

const UNCHECKED_INSIDE_REASON: &str =
    "Too many files inside, or one unreadable, to confirm nothing in it is new";

const PLACEHOLDER_REASON: &str =
    "Cloud-only (online-only) file; archiving or compressing would download it first";

pub fn examine(
    conn: &Connection,
    env: &PolicyEnv,
    path: &str,
    action: Action,
) -> anyhow::Result<Examined> {
    let catalog = env.catalog().lookup_match(path);
    let category = catalog.as_ref().map(|m| m.category);
    let catalog_cap = catalog.as_ref().and_then(|m| m.cap);
    let mut ex = Examined {
        path: path.to_string(),
        state: None,
        alloc: 0,
        logical: 0,
        node_id: None,
        block: None,
        cap: None,
        category,
        copy: None,
    };
    let state = match fs_state(Path::new(path)) {
        Ok(s) => s,
        Err(_) => {
            ex.block = Some("Not found".into());
            ex.cap = cap_for_with(conn, env, Path::new(path), catalog_cap, false)?;
            return Ok(ex);
        }
    };
    let node = index::node_by_path(conn, path)?;
    let mut placeholder = hoardini_win::is_cloud_placeholder(state.attrs);
    match state.kind {
        ItemKind::File => {
            ex.logical = state.logical;
            ex.alloc = match &node {
                Some(n)
                    if n.kind == index::NodeKind::File
                        && index::to_i64(n.logical) == state.logical =>
                {
                    index::to_i64(n.size)
                }
                _ => hoardini_win::alloc_size_by_open(Path::new(path))
                    .map(index::to_i64)
                    .unwrap_or(state.logical),
            };
        }
        ItemKind::Dir => match &node {
            Some(n) if n.kind == index::NodeKind::Dir => {
                ex.alloc = index::to_i64(n.size);
                ex.logical = index::to_i64(n.logical);
                if action.reads_data() {
                    placeholder |= dir_has_placeholder(conn, n.id)?;
                }
            }
            _ => {
                let scan = scan_tree(Path::new(path));
                ex.alloc = index::to_i64(scan.alloc);
                ex.logical = index::to_i64(scan.logical);
                placeholder |= scan.placeholder;
            }
        },
        ItemKind::Link => {}
    }
    ex.node_id = node.map(|n| n.id);
    let kept = crate::memory::location::keep_block(conn, env, path, action)?;
    let in_use = in_use_reason_capped(conn, path, state.kind, PROPOSE_IN_USE_FILE_CAP);
    let not_synced = action == Action::OnlineOnly
        && crate::cloud::root_for(&env.cloud().roots(), path).is_none();
    ex.block = if let Some(r) = kept {
        Some(r)
    } else if let Some(r) = &in_use {
        Some(r.clone())
    } else if not_synced {
        Some(NOT_SYNCED_REASON.into())
    } else if action == Action::OnlineOnly {
        online_only_block(&state, ex.alloc)
    } else if action.reads_data() && placeholder {
        Some(PLACEHOLDER_REASON.into())
    } else if action.reads_data() && state.kind == ItemKind::Link {
        Some("A link holds no data; delete it instead".into())
    } else {
        None
    };
    (ex.cap, ex.copy) = super::cap_and_copy(
        conn,
        env,
        Path::new(path),
        catalog_cap,
        in_use.is_some(),
        action == Action::OnlineOnly,
    )?;
    ex.state = Some(state);
    Ok(ex)
}

#[cfg(test)]
thread_local! {
    static AFTER_IN_USE: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

// None = still the item the user approved; Some(detail) = skip it.
fn note_slow(path: &str, what: &str, since: std::time::Instant) {
    let ms = since.elapsed().as_millis() as u64;
    if ms >= 500 {
        tracing::info!(%path, what, ms, "slow revalidation step");
    }
}

pub fn revalidate(
    conn: &Connection,
    env: &PolicyEnv,
    action: Action,
    path: &str,
    expected: &FsState,
    cap: Option<Verdict>,
    from_review: bool,
) -> anyhow::Result<Option<String>> {
    // The open-file check is slow, so every cheap check runs after it, as close to acting as possible.
    let t = std::time::Instant::now();
    let in_use = in_use_reason(conn, path, expected.kind);
    note_slow(path, "open-file check", t);
    if let Some(r) = in_use {
        return Ok(Some(r));
    }
    #[cfg(test)]
    AFTER_IN_USE.with(|h| {
        if let Some(f) = h.borrow().as_ref() {
            f()
        }
    });
    if let Some(r) = deny_reason(conn, env, path)? {
        return Ok(Some(format!("Denied: {r}")));
    }
    if let Some(r) = crate::memory::location::keep_block(conn, env, path, action)? {
        return Ok(Some(r));
    }
    let now = match fs_state(Path::new(path)) {
        Ok(s) => s,
        Err(_) => return Ok(Some("Not found anymore".into())),
    };
    if now.kind != expected.kind {
        return Ok(Some(format!(
            "Changed since the proposal: now a {}",
            now.kind.as_str()
        )));
    }
    if now.logical != expected.logical {
        return Ok(Some(format!(
            "Changed since the proposal: size {} -> {} bytes",
            expected.logical, now.logical
        )));
    }
    if now.mtime != expected.mtime {
        return Ok(Some("Changed since the proposal: modified".into()));
    }
    if expected.file_id.is_some() && now.file_id.is_some() && now.file_id != expected.file_id {
        return Ok(Some(
            "Changed since the proposal: replaced by another file".into(),
        ));
    }
    if action.reads_data()
        && now.kind == ItemKind::File
        && hoardini_win::is_cloud_placeholder(now.attrs)
    {
        return Ok(Some(PLACEHOLDER_REASON.into()));
    }
    // A review card's part was old when proposed; a chat request the user made for a new temp file is theirs to make.
    if from_review
        && let Some(e) = env.catalog().lookup(path)
        && e.unit == UnitKind::Contents
        && let Some(hours) = e.min_age_hours
    {
        let t = std::time::Instant::now();
        let fresh = fresh_inside(Path::new(path), now.kind, hours);
        note_slow(path, "freshness walk", t);
        if let Some(r) = fresh {
            return Ok(Some(r));
        }
    }
    // A `redownload` cap on a path no catalog entry knows came from a copy, which must still count now.
    if action == Action::Delete
        && cap == Some(Verdict::Redownload)
        && env.catalog().lookup(path).is_none()
        && crate::copycheck::identical_for(conn, env, path, false).is_none()
    {
        return Ok(Some(COPY_GONE_REASON.into()));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Db;

    #[test]
    fn a_swap_during_the_open_file_check_is_caught() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Db::open(tmp.path().join("h.db")).unwrap();
        let conn = db.conn().unwrap();
        let env = PolicyEnv::with_paths(&tmp.path().join("profile"), &tmp.path().join("data"));
        let victim = tmp.path().join("work").join("cache");
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let path = victim.to_string_lossy().into_owned();
        let expected = fs_state(&victim).unwrap();
        assert_eq!(expected.kind, ItemKind::Dir);
        assert_eq!(
            revalidate(&conn, &env, Action::Delete, &path, &expected, None, false).unwrap(),
            None
        );

        let (v, e) = (victim.clone(), elsewhere.clone());
        AFTER_IN_USE.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                std::fs::remove_dir(&v).unwrap();
                hoardini_win::create_junction(&v, &e).unwrap();
            }));
        });
        let got = revalidate(&conn, &env, Action::Delete, &path, &expected, None, false).unwrap();
        AFTER_IN_USE.with(|h| *h.borrow_mut() = None);
        let detail = got.expect("swap must be caught");
        assert!(detail.starts_with("Changed since the proposal"), "{detail}");
    }
}

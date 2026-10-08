use std::fs::Metadata;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rusqlite::Connection;

use super::{
    DuplicateRef, Evidence, Family, Identity, PathKind, cache, identify, inuse, owner, summary,
    usage,
};
use crate::catalog::Catalog;
use crate::index::{self, FileItem, NodeInfo, NodeKind, TopFilesQuery};
use crate::store::{Db, now_ms};

const MAX_DUPES: usize = 20;

fn catalog() -> &'static Catalog {
    static C: OnceLock<&'static Catalog> = OnceLock::new();
    C.get_or_init(Catalog::embedded)
}

// Cloud placeholders must never be opened: a read would download them.
pub(crate) fn may_read(attrs: Option<u32>, indexed_placeholder: bool) -> bool {
    !indexed_placeholder && !attrs.is_some_and(hoardini_win::is_cloud_placeholder)
}

fn placeholder_identity() -> Identity {
    let mut id = Identity::unknown_no_access();
    id.kind = "cloud placeholder".into();
    id.note = Some("cloud placeholder, not read (it would download)".into());
    id
}

fn read_identity(path: &Path, len: u64) -> Identity {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    match hoardini_win::open_read_preserving_atime(path) {
        Ok(f) => identify(f, len, &name),
        Err(_) => Identity::unknown_no_access(),
    }
}

fn kind_of(meta: &std::io::Result<Metadata>, node: Option<&NodeInfo>) -> PathKind {
    match meta {
        Ok(m) if m.is_symlink() => PathKind::Link,
        Ok(m) if m.is_dir() => PathKind::Dir,
        Ok(_) if node.is_some_and(|n| n.kind == NodeKind::Link) => PathKind::Link,
        Ok(_) => PathKind::File,
        Err(_) => PathKind::Missing,
    }
}

fn duplicates_of(conn: &Connection, path: &str) -> Vec<DuplicateRef> {
    let Ok(mut st) = conn.prepare_cached(
        "SELECT m.path, g.size FROM dup_member m JOIN dup_group g ON g.id = m.group_id \
         WHERE m.group_id IN (SELECT group_id FROM dup_member WHERE path = ?1 COLLATE NOCASE) \
         AND m.path <> ?1 COLLATE NOCASE ORDER BY m.path LIMIT ?2",
    ) else {
        return Vec::new();
    };
    st.query_map(rusqlite::params![path, MAX_DUPES as i64], |r| {
        Ok(DuplicateRef {
            path: r.get(0)?,
            size: u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
        })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

fn largest_files(conn: &Connection, dir: &str) -> Vec<FileItem> {
    index::top_files(
        conn,
        &TopFilesQuery {
            under: Some(dir.to_string()),
            limit: inuse::DIR_FILE_CAP,
            ..Default::default()
        },
    )
    .map(|r| r.items)
    .unwrap_or_default()
}

pub fn inspect(db: &Db, path: &Path) -> anyhow::Result<Evidence> {
    let conn = db.conn()?;
    let path_s = path.to_string_lossy().into_owned();
    let meta = std::fs::symlink_metadata(path);
    let node = index::node_by_path(&conn, &path_s).ok().flatten();
    let kind = kind_of(&meta, node.as_ref());
    let attrs = meta.as_ref().ok().map(|m| m.file_attributes());
    let placeholder = !may_read(attrs, node.as_ref().is_some_and(|n| n.placeholder));
    let mtime_ms = meta
        .as_ref()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(usage::system_time_ms);

    let mut ev = Evidence::new(path_s.clone(), kind);
    match (&node, &meta) {
        (Some(n), _) if n.kind != NodeKind::Link => {
            ev.alloc_bytes = Some(n.size);
            ev.logical_bytes = Some(n.logical);
            if n.kind == NodeKind::Dir {
                ev.file_count = Some(n.files);
            }
        }
        (_, Ok(m)) if kind == PathKind::File => {
            ev.logical_bytes = Some(m.len());
            if !placeholder {
                ev.alloc_bytes = hoardini_win::alloc_size_by_open(path).ok();
            }
        }
        _ => {}
    }

    ev.catalog = catalog().lookup_match(&path_s);
    ev.owner = owner::resolve(&path_s, ev.catalog.as_ref());

    let dir_files = if kind == PathKind::Dir {
        largest_files(&conn, &path_s)
    } else {
        Vec::new()
    };

    let key = node.as_ref().map(|n| cache::key_for(&conn, n.id, mtime_ms));
    let cached = key
        .as_ref()
        .and_then(|k| cache::load(&conn, k))
        .filter(|c| c.kind == kind && c.alloc_bytes == ev.alloc_bytes);
    ev.identity = match cached {
        Some(c) => c.identity,
        None => compute_identity(path, kind, placeholder, &meta, &dir_files, ev.alloc_bytes),
    };

    ev.in_use = match kind {
        PathKind::File => inuse::holders(&[path.to_path_buf()]),
        PathKind::Dir => {
            let files: Vec<PathBuf> = dir_files
                .iter()
                .filter(|f| !f.placeholder)
                .map(|f| PathBuf::from(&f.path))
                .collect();
            inuse::holders(&files)
        }
        _ => Vec::new(),
    };

    if let Some(k) = &key {
        let mut snapshot = ev.clone();
        snapshot.in_use.clear();
        cache::store(&conn, k, &snapshot);
    }

    let (atime_ms, atime_note) = match kind {
        PathKind::File | PathKind::Link => (
            meta.as_ref()
                .ok()
                .and_then(|m| m.accessed().ok())
                .and_then(usage::system_time_ms),
            None,
        ),
        PathKind::Dir => (
            dir_files.iter().filter_map(|f| f.atime_ms).max(),
            Some(format!(
                "newest of the {} largest files, from the index (may be stale)",
                dir_files.len()
            )),
        ),
        PathKind::Missing => (None, None),
    };
    ev.last_used = usage::signals(
        &conn,
        &usage::Inputs {
            path: &path_s,
            is_dir: kind == PathKind::Dir,
            atime_ms,
            atime_note: atime_note.as_deref().filter(|_| atime_ms.is_some()),
            mtime_ms,
            owner: ev.owner.as_ref(),
        },
    );

    ev.duplicates = duplicates_of(&conn, &path_s);
    ev.memories = crate::memory::recall_path(&conn, &path_s).unwrap_or_default();
    ev.policy_cap = crate::policy::cap_for(
        &conn,
        path,
        ev.catalog.as_ref().and_then(|c| c.cap),
        !ev.in_use.is_empty(),
    )?;
    ev.summary = summary::build(&ev, now_ms());
    Ok(ev)
}

fn compute_identity(
    path: &Path,
    kind: PathKind,
    placeholder: bool,
    meta: &std::io::Result<Metadata>,
    dir_files: &[FileItem],
    total: Option<u64>,
) -> Option<Identity> {
    match kind {
        PathKind::File if placeholder => Some(placeholder_identity()),
        PathKind::File => {
            let len = meta.as_ref().map_or(0, Metadata::len);
            Some(read_identity(path, len))
        }
        // Only a folder dominated (>= half its bytes) by one model file takes that file's identity.
        PathKind::Dir => {
            let biggest = dir_files.iter().find(|f| !f.placeholder)?;
            if total.is_none_or(|t| biggest.size.saturating_mul(2) < t) {
                return None;
            }
            let p = Path::new(&biggest.path);
            let attrs = std::fs::symlink_metadata(p).ok()?.file_attributes();
            if !may_read(Some(attrs), false) {
                return None;
            }
            let id = read_identity(p, biggest.logical);
            (id.family == Family::Model).then_some(id)
        }
        PathKind::Link | PathKind::Missing => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_never_readable() {
        let offline = 0x1000;
        let recall = 0x40_0000;
        assert!(!may_read(Some(offline), false));
        assert!(!may_read(Some(recall), false));
        assert!(!may_read(Some(0x20), true));
        assert!(may_read(Some(0x20), false));
        assert!(may_read(None, false));
    }
}

mod child;
mod dedupe;
mod fake;
mod journal;
mod lister;
mod patch;
mod prof;
mod query;
mod verify;
mod walk;
mod writer;

pub use child::run_child;
pub use fake::FakeFs;
pub use journal::{
    JournalOutcome, JournalRefresh, JournalSource, NeedsFull, RefreshKind, RefreshRun, WinJournal,
    journal_refresh, refresh_kind,
};
pub use lister::{DirLister, WinLister};
pub use patch::{patch_removed, patch_rescan};
pub use query::{
    DirItem, FileItem, IndexStatus, NodeInfo, NodeKind, Ranked, TopFilesQuery, children,
    humanize_age, index_status, node_by_path, node_id_by_path, path_of, summary_for_prompt,
    top_dirs, top_files,
};
pub use verify::{SnapRow, snapshot, verify_totals};
pub use walk::{Event, Mode, Progress, RefreshOptions, RefreshOutcome, refresh};

pub(crate) use query::PathCache;

use anyhow::Result;
use rusqlite::Connection;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

pub const KIND_DIR: i64 = 0;
pub const KIND_FILE: i64 = 1;
pub const KIND_LINK: i64 = 2;

pub const FLAG_DENIED: i64 = 1;
pub const FLAG_PLACEHOLDER: i64 = 2;
pub const FLAG_HARDLINK_DUP: i64 = 4;

// Tuned by measurement (C: indexes to ~132 MiB); only `hoardini index --threshold` overrides it.
pub const DEFAULT_THRESHOLD: u64 = 1 << 20;

// A first build inserts in random key order, so it builds these lookup indexes sorted at its end.
const BULK_INDEXES: &str = "DROP INDEX IF EXISTS node_file_id; \
     DROP INDEX IF EXISTS node_parent_name; \
     CREATE INDEX IF NOT EXISTS node_parent_bulk ON node(parent_id);";
const LOOKUP_INDEXES: &str = "DROP INDEX IF EXISTS node_parent_bulk; \
     CREATE INDEX IF NOT EXISTS node_parent_name ON node(parent_id, name COLLATE NOCASE); \
     CREATE INDEX IF NOT EXISTS node_file_id ON node(volume_id, file_id);";

pub(crate) fn bulk_indexes(conn: &Connection, bulk: bool) -> Result<()> {
    conn.execute_batch(if bulk { BULK_INDEXES } else { LOOKUP_INDEXES })?;
    Ok(())
}

// NTFS ids fit in 8 bytes and are stored as an INTEGER; wider ids (ReFS) stay a 16-byte BLOB.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileId(pub [u8; 16]);

impl ToSql for FileId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let (low, high) = self.0.split_at(8);
        Ok(if high == [0; 8] {
            let mut b = [0u8; 8];
            b.copy_from_slice(low);
            ToSqlOutput::from(i64::from_le_bytes(b))
        } else {
            ToSqlOutput::from(self.0.to_vec())
        })
    }
}

impl FromSql for FileId {
    fn column_result(v: ValueRef<'_>) -> FromSqlResult<Self> {
        match v {
            ValueRef::Integer(i) => {
                let mut id = [0u8; 16];
                id[..8].copy_from_slice(&i.to_le_bytes());
                Ok(FileId(id))
            }
            ValueRef::Blob(b) => {
                <[u8; 16]>::try_from(b)
                    .map(FileId)
                    .map_err(|_| FromSqlError::InvalidBlobSize {
                        expected_size: 16,
                        blob_size: b.len(),
                    })
            }
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

// Drive roots keep their separator ("C:\"); every other path drops a trailing one.
pub fn normalize_path(p: &str) -> String {
    let mut s = p.trim().replace('/', "\\");
    if let Some(rest) = s.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        s = rest.to_string();
    }
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        let mut out = String::with_capacity(s.len() + 1);
        out.push(b[0].to_ascii_uppercase() as char);
        out.push_str(&s[1..]);
        if out.len() == 2 {
            out.push('\\');
        }
        s = out;
    }
    s
}

pub(crate) fn join_path(parent: &str, name: &str) -> String {
    if parent.ends_with('\\') {
        format!("{parent}{name}")
    } else {
        format!("{parent}\\{name}")
    }
}

pub(crate) fn split_parent(p: &str) -> Option<(String, &str)> {
    let idx = p.rfind('\\')?;
    let name = &p[idx + 1..];
    if name.is_empty() {
        return None;
    }
    let parent = &p[..idx];
    let parent = if parent.len() == 2 && parent.ends_with(':') {
        format!("{parent}\\")
    } else {
        parent.to_string()
    };
    Some((parent, name))
}

pub(crate) fn is_ancestor_path(a: &str, b: &str) -> bool {
    if b.len() <= a.len() || !b.is_char_boundary(a.len()) || !b[..a.len()].eq_ignore_ascii_case(a) {
        return false;
    }
    a.ends_with('\\') || b.as_bytes()[a.len()] == b'\\'
}

pub(crate) fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

pub(crate) fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_roots_and_trailing_separators() {
        assert_eq!(normalize_path("c:"), r"C:\");
        assert_eq!(normalize_path(r"c:\"), r"C:\");
        assert_eq!(normalize_path("c:/Users/x/"), r"C:\Users\x");
        assert_eq!(normalize_path(r"\\?\C:\a"), r"C:\a");
    }

    #[test]
    fn splits_and_joins() {
        assert_eq!(
            split_parent(r"C:\Users"),
            Some((r"C:\".to_string(), "Users"))
        );
        assert_eq!(split_parent(r"C:\a\b"), Some((r"C:\a".to_string(), "b")));
        assert_eq!(split_parent(r"C:\"), None);
        assert_eq!(join_path(r"C:\", "a"), r"C:\a");
        assert_eq!(join_path(r"C:\a", "b"), r"C:\a\b");
        assert!(is_ancestor_path(r"C:\", r"C:\a"));
        assert!(is_ancestor_path(r"c:\A", r"C:\a\b"));
        assert!(!is_ancestor_path(r"C:\a", r"C:\ab"));
        assert!(!is_ancestor_path(r"C:\a", r"C:\a"));
    }
}

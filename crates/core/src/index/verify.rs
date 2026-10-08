use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use super::{PathCache, normalize_path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SnapRow {
    pub path: String,
    pub kind: i64,
    pub alloc: i64,
    pub logical: i64,
    pub small_count: i64,
    pub small_bytes: i64,
    pub small_logical: i64,
    pub sub_alloc: i64,
    pub sub_logical: i64,
    pub sub_count: i64,
    pub mtime: Option<i64>,
    pub atime: Option<i64>,
    pub ctime: Option<i64>,
    pub attrs: i64,
    pub reparse_tag: Option<i64>,
    pub file_id: Option<Vec<u8>>,
    pub flags: i64,
}

// Row ids and generation marks differ between equal indexes, so they are left out.
pub fn snapshot(conn: &Connection, root: &str) -> Result<Vec<SnapRow>> {
    let root = normalize_path(root);
    let mut st = conn.prepare(
        "SELECT n.id, n.kind, n.alloc, n.logical, n.small_count, n.small_bytes, n.small_logical, \
         n.sub_alloc, n.sub_logical, n.sub_count, n.mtime, n.atime, n.ctime, n.attrs, n.reparse_tag, \
         n.file_id, n.flags FROM node n JOIN volume v ON v.id = n.volume_id WHERE v.root = ?1",
    )?;
    let raw = st
        .query_map([&root], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                SnapRow {
                    path: String::new(),
                    kind: r.get(1)?,
                    alloc: r.get(2)?,
                    logical: r.get(3)?,
                    small_count: r.get(4)?,
                    small_bytes: r.get(5)?,
                    small_logical: r.get(6)?,
                    sub_alloc: r.get(7)?,
                    sub_logical: r.get(8)?,
                    sub_count: r.get(9)?,
                    mtime: r.get(10)?,
                    atime: r.get(11)?,
                    ctime: r.get(12)?,
                    attrs: r.get(13)?,
                    reparse_tag: r.get(14)?,
                    file_id: r.get(15)?,
                    flags: r.get(16)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut paths = PathCache::default();
    let mut out = Vec::with_capacity(raw.len());
    for (id, mut row) in raw {
        row.path = paths
            .get(conn, id)?
            .unwrap_or_else(|| format!("<orphan {id}>"));
        out.push(row);
    }
    out.sort_by(|a, b| {
        a.path
            .to_lowercase()
            .cmp(&b.path.to_lowercase())
            .then(a.kind.cmp(&b.kind))
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(out)
}

pub fn verify_totals(conn: &Connection) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    let mut st = conn.prepare(
        "SELECT p.id, p.sub_alloc, p.sub_logical, p.sub_count, \
         p.small_bytes + COALESCE(SUM(CASE WHEN c.kind = 0 THEN c.sub_alloc \
           WHEN c.kind = 1 AND (c.flags & 4) = 0 THEN c.alloc ELSE 0 END), 0), \
         p.small_logical + COALESCE(SUM(CASE WHEN c.kind = 0 THEN c.sub_logical \
           WHEN c.kind = 1 AND (c.flags & 4) = 0 THEN c.logical ELSE 0 END), 0), \
         p.small_count + COALESCE(SUM(CASE WHEN c.kind = 0 THEN c.sub_count \
           WHEN c.kind = 1 AND (c.flags & 4) = 0 THEN 1 ELSE 0 END), 0) \
         FROM node p LEFT JOIN node c ON c.parent_id = p.id WHERE p.kind = 0 GROUP BY p.id",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            [r.get::<_, i64>(1)?, r.get(2)?, r.get(3)?],
            [r.get::<_, i64>(4)?, r.get(5)?, r.get(6)?],
        ))
    })?;
    for row in rows {
        let (id, have, want) = row?;
        if have != want {
            problems.push(format!(
                "dir {id}: sub (alloc, logical, count) = {have:?}, parts sum to {want:?}"
            ));
        }
    }
    let mut orphans = conn.prepare(
        "SELECT n.id FROM node n LEFT JOIN node p ON p.id = n.parent_id \
         WHERE n.parent_id IS NOT NULL AND p.id IS NULL",
    )?;
    for id in orphans.query_map([], |r| r.get::<_, i64>(0))? {
        problems.push(format!("node {} has no parent row", id?));
    }
    let mut non_dir_parents = conn
        .prepare("SELECT n.id FROM node n JOIN node p ON p.id = n.parent_id WHERE p.kind <> 0")?;
    for id in non_dir_parents.query_map([], |r| r.get::<_, i64>(0))? {
        problems.push(format!("node {} sits under a non-directory", id?));
    }
    Ok(problems)
}

use std::collections::HashMap;
use std::fmt::Write as _;

use anyhow::{Result, anyhow, bail};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde::{Deserialize, Serialize};

use super::{
    FLAG_DENIED, FLAG_HARDLINK_DUP, FLAG_PLACEHOLDER, KIND_DIR, KIND_FILE, is_ancestor_path,
    join_path, normalize_path, to_u64,
};
use crate::history::{self, Since};
use crate::paths::humanize_bytes;
use crate::store::now_ms;

const DAY_MS: i64 = 86_400_000;
const PROMPT_MAX_CHARS: usize = 4000;

#[derive(Default)]
pub(crate) struct PathCache {
    map: HashMap<i64, String>,
}

impl PathCache {
    pub fn get(&mut self, conn: &Connection, id: i64) -> Result<Option<String>> {
        if let Some(p) = self.map.get(&id) {
            return Ok(Some(p.clone()));
        }
        let mut st = conn.prepare_cached("SELECT parent_id, name FROM node WHERE id = ?1")?;
        let mut pending: Vec<(i64, String)> = Vec::new();
        let mut base: Option<String> = None;
        let mut cur = Some(id);
        while let Some(c) = cur {
            if let Some(p) = self.map.get(&c) {
                base = Some(p.clone());
                break;
            }
            let Some((parent, name)) = st
                .query_row([c], |r| {
                    Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, String>(1)?))
                })
                .optional()?
            else {
                return Ok(None);
            };
            pending.push((c, name));
            if pending.len() > 4096 {
                bail!("node {id}: parent chain does not end");
            }
            cur = parent;
        }
        let mut path = base;
        for (c, name) in pending.into_iter().rev() {
            let p = match &path {
                Some(b) => join_path(b, &name),
                None => name,
            };
            self.map.insert(c, p.clone());
            path = Some(p);
        }
        Ok(path)
    }
}

pub fn path_of(conn: &Connection, id: i64) -> Result<Option<String>> {
    PathCache::default().get(conn, id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Dir,
    File,
    Link,
}

impl NodeKind {
    fn from_db(k: i64) -> NodeKind {
        match k {
            KIND_DIR => NodeKind::Dir,
            KIND_FILE => NodeKind::File,
            _ => NodeKind::Link,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NodeInfo {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub path: String,
    pub name: String,
    pub kind: NodeKind,
    pub size: u64,
    pub logical: u64,
    pub files: u64,
    pub small_count: u64,
    pub small_bytes: u64,
    pub mtime_ms: Option<i64>,
    pub atime_ms: Option<i64>,
    pub ctime_ms: Option<i64>,
    pub attrs: u32,
    pub denied: bool,
    pub placeholder: bool,
    pub hardlink_dup: bool,
}

const NODE_COLS: &str = "id, parent_id, name, kind, alloc, logical, small_count, small_bytes, \
     sub_alloc, sub_logical, sub_count, mtime, atime, ctime, attrs, flags";

fn node_from_row(r: &Row, path: Option<String>) -> rusqlite::Result<NodeInfo> {
    let kind = NodeKind::from_db(r.get(3)?);
    let flags: i64 = r.get(15)?;
    let name: String = r.get(2)?;
    let (size, logical, files) = match kind {
        NodeKind::Dir => (r.get(8)?, r.get(9)?, r.get(10)?),
        NodeKind::File => (r.get(4)?, r.get(5)?, 1),
        NodeKind::Link => (0, 0, 0),
    };
    Ok(NodeInfo {
        id: r.get(0)?,
        parent_id: r.get(1)?,
        path: path.unwrap_or_else(|| name.clone()),
        name,
        kind,
        size: to_u64(size),
        logical: to_u64(logical),
        files: to_u64(files),
        small_count: to_u64(r.get(6)?),
        small_bytes: to_u64(r.get(7)?),
        mtime_ms: r.get(11)?,
        atime_ms: r.get(12)?,
        ctime_ms: r.get(13)?,
        attrs: u32::try_from(r.get::<_, i64>(14)?).unwrap_or(0),
        denied: flags & FLAG_DENIED != 0,
        placeholder: flags & FLAG_PLACEHOLDER != 0,
        hardlink_dup: flags & FLAG_HARDLINK_DUP != 0,
    })
}

fn node_info(conn: &Connection, id: i64) -> Result<Option<NodeInfo>> {
    let path = path_of(conn, id)?;
    Ok(conn
        .query_row(
            &format!("SELECT {NODE_COLS} FROM node WHERE id = ?1"),
            [id],
            |r| node_from_row(r, path.clone()),
        )
        .optional()?)
}

pub fn node_id_by_path(conn: &Connection, path: &str) -> Result<Option<i64>> {
    let path = normalize_path(path);
    let mut best: Option<(usize, i64)> = None;
    {
        let mut st = conn.prepare_cached(
            "SELECT root, root_node_id FROM volume WHERE root_node_id IS NOT NULL",
        )?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (root, id) = row?;
            let matches = root.eq_ignore_ascii_case(&path) || is_ancestor_path(&root, &path);
            if matches && best.is_none_or(|(len, _)| root.len() > len) {
                best = Some((root.len(), id));
            }
        }
    }
    let Some((len, mut id)) = best else {
        return Ok(None);
    };
    let mut st = conn.prepare_cached(
        "SELECT id FROM node WHERE parent_id = ?1 AND name = ?2 COLLATE NOCASE \
         ORDER BY name = ?2 DESC, seen_gen DESC, id DESC LIMIT 1",
    )?;
    for comp in path[len..].split('\\').filter(|c| !c.is_empty()) {
        match st.query_row(params![id, comp], |r| r.get(0)).optional()? {
            Some(c) => id = c,
            None => return Ok(None),
        }
    }
    Ok(Some(id))
}

pub fn node_by_path(conn: &Connection, path: &str) -> Result<Option<NodeInfo>> {
    match node_id_by_path(conn, path)? {
        Some(id) => node_info(conn, id),
        None => Ok(None),
    }
}

pub fn children(conn: &Connection, node_id: i64) -> Result<Vec<NodeInfo>> {
    let base = path_of(conn, node_id)?.ok_or_else(|| anyhow!("node {node_id} not found"))?;
    let mut st = conn.prepare_cached(&format!(
        "SELECT {NODE_COLS} FROM node WHERE parent_id = ?1"
    ))?;
    let mut out = st
        .query_map([node_id], |r| {
            let name: String = r.get(2)?;
            node_from_row(r, Some(join_path(&base, &name)))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    out.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

#[derive(Clone, Debug, Serialize)]
pub struct Ranked<T> {
    pub items: Vec<T>,
    pub more_count: u64,
    pub more_bytes: u64,
}

impl<T> Ranked<T> {
    pub fn more_line(&self) -> Option<String> {
        match (self.more_count, self.more_bytes) {
            (0, _) => None,
            (n, 0) => Some(format!("+{n} more")),
            (n, b) => Some(format!("+{n} more ({})", humanize_bytes(b))),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TopFilesQuery {
    pub under: Option<String>,
    pub min_size: u64,
    pub ext: Option<String>,
    pub not_used_for_days: Option<u32>,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FileItem {
    pub path: String,
    pub size: u64,
    pub logical: u64,
    pub mtime_ms: Option<i64>,
    pub atime_ms: Option<i64>,
    pub ctime_ms: Option<i64>,
    pub last_used_ms: Option<i64>,
    pub placeholder: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DirItem {
    pub path: String,
    pub size: u64,
    pub logical: u64,
    pub files: u64,
    pub depth: u32,
    pub mtime_ms: Option<i64>,
    pub denied: bool,
}

enum Scope {
    All,
    Volume(i64),
    Subtree(i64),
}

fn scope_of(conn: &Connection, under: Option<&str>) -> Result<Scope> {
    let Some(under) = under.filter(|u| !u.trim().is_empty()) else {
        return Ok(Scope::All);
    };
    let id = node_id_by_path(conn, under)?.ok_or_else(|| anyhow!("{under} is not in the index"))?;
    let (parent, kind, volume): (Option<i64>, i64, i64) = conn.query_row(
        "SELECT parent_id, kind, volume_id FROM node WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if kind != KIND_DIR {
        bail!("{under} is not a directory");
    }
    Ok(if parent.is_none() {
        Scope::Volume(volume)
    } else {
        Scope::Subtree(id)
    })
}

fn like_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

pub fn top_files(conn: &Connection, q: &TopFilesQuery) -> Result<Ranked<FileItem>> {
    let mut args: Vec<Value> = Vec::new();
    let mut prefix = String::new();
    let mut filters = vec![
        "f.kind = 1".to_string(),
        "(f.flags & 4) = 0".to_string(),
        format!("f.alloc >= {}", super::to_i64(q.min_size)),
    ];
    match scope_of(conn, q.under.as_deref())? {
        Scope::All => {}
        // Unary + keeps the planner on node_file_alloc instead of node_file_id.
        Scope::Volume(v) => filters.push(format!("+f.volume_id = {v}")),
        Scope::Subtree(id) => {
            // `+n.kind` keeps SQLite on the parent_id index; the kind index made each step scan every dir.
            prefix = format!(
                "WITH RECURSIVE d(id) AS (SELECT {id} UNION ALL \
                 SELECT n.id FROM node n JOIN d ON n.parent_id = d.id WHERE +n.kind = 0) "
            );
            filters.push("f.parent_id IN (SELECT id FROM d)".into());
        }
    }
    if let Some(ext) = q.ext.as_deref() {
        let exts: Vec<String> = ext
            .split([',', ' ', ';'])
            .map(|e| e.trim().trim_start_matches('.').to_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        if !exts.is_empty() {
            let mut ors = Vec::new();
            for e in exts {
                ors.push("f.name LIKE ? ESCAPE '\\'".to_string());
                args.push(Value::Text(format!("%.{}", like_escape(&e))));
            }
            filters.push(format!("({})", ors.join(" OR ")));
        }
    }
    if let Some(days) = q.not_used_for_days {
        filters.push("MAX(COALESCE(f.atime, 0), COALESCE(f.mtime, 0)) < ?".into());
        args.push(Value::Integer(now_ms() - i64::from(days) * DAY_MS));
    }
    let wh = filters.join(" AND ");
    let limit = q.limit.max(1);
    let (total, total_bytes): (i64, i64) = conn.query_row(
        &format!("{prefix}SELECT COUNT(*), COALESCE(SUM(f.alloc), 0) FROM node f WHERE {wh}"),
        params_from_iter(args.iter()),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut st = conn.prepare(&format!(
        "{prefix}SELECT f.parent_id, f.name, f.alloc, f.logical, f.mtime, f.atime, f.ctime, f.flags \
         FROM node f WHERE {wh} ORDER BY f.alloc DESC, f.id LIMIT {limit}"
    ))?;
    let rows = st
        .query_map(params_from_iter(args.iter()), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<i64>>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut paths = PathCache::default();
    let mut items = Vec::with_capacity(rows.len());
    let mut shown_bytes = 0u64;
    for (parent, name, alloc, logical, mtime, atime, ctime, flags) in rows {
        let base = paths.get(conn, parent)?.unwrap_or_default();
        shown_bytes += to_u64(alloc);
        items.push(FileItem {
            path: join_path(&base, &name),
            size: to_u64(alloc),
            logical: to_u64(logical),
            mtime_ms: mtime,
            atime_ms: atime,
            ctime_ms: ctime,
            last_used_ms: atime.max(mtime),
            placeholder: flags & FLAG_PLACEHOLDER != 0,
        });
    }
    Ok(Ranked {
        more_count: to_u64(total).saturating_sub(items.len() as u64),
        more_bytes: to_u64(total_bytes).saturating_sub(shown_bytes),
        items,
    })
}

pub fn top_dirs(
    conn: &Connection,
    under: Option<&str>,
    depth: u32,
    limit: usize,
) -> Result<Ranked<DirItem>> {
    let depth = depth.clamp(1, 16);
    let limit = limit.max(1);
    let seed = match scope_of(conn, under)? {
        Scope::All => {
            "SELECT root_node_id, 0 FROM volume WHERE root_node_id IS NOT NULL".to_string()
        }
        Scope::Volume(v) => format!("SELECT root_node_id, 0 FROM volume WHERE id = {v}"),
        Scope::Subtree(id) => format!("SELECT {id}, 0"),
    };
    let cte = format!(
        "WITH RECURSIVE d(id, depth) AS ({seed} UNION ALL \
         SELECT n.id, d.depth + 1 FROM node n JOIN d ON n.parent_id = d.id \
         WHERE +n.kind = 0 AND d.depth < {depth}) "
    );
    let (total, depth1_bytes): (i64, i64) = conn.query_row(
        &format!(
            "{cte}SELECT COUNT(*), COALESCE(SUM(CASE WHEN d.depth = 1 THEN n.sub_alloc ELSE 0 END), 0) \
             FROM d JOIN node n ON n.id = d.id WHERE d.depth >= 1"
        ),
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut st = conn.prepare(&format!(
        "{cte}SELECT n.parent_id, n.name, n.sub_alloc, n.sub_logical, n.sub_count, n.mtime, n.flags, d.depth \
         FROM d JOIN node n ON n.id = d.id WHERE d.depth >= 1 \
         ORDER BY n.sub_alloc DESC, n.id LIMIT {limit}"
    ))?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut paths = PathCache::default();
    let mut items = Vec::with_capacity(rows.len());
    let mut shown_depth1 = 0u64;
    for (parent, name, alloc, logical, count, mtime, flags, d) in rows {
        let base = paths.get(conn, parent)?.unwrap_or_default();
        if d == 1 {
            shown_depth1 += to_u64(alloc);
        }
        items.push(DirItem {
            path: join_path(&base, &name),
            size: to_u64(alloc),
            logical: to_u64(logical),
            files: to_u64(count),
            depth: u32::try_from(d).unwrap_or(0),
            mtime_ms: mtime,
            denied: flags & FLAG_DENIED != 0,
        });
    }
    // Nested dirs overlap, so a byte total for the rest only makes sense one level deep.
    let more_bytes = if depth == 1 {
        to_u64(depth1_bytes).saturating_sub(shown_depth1)
    } else {
        0
    };
    Ok(Ranked {
        more_count: to_u64(total).saturating_sub(items.len() as u64),
        more_bytes,
        items,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IndexStatus {
    pub volume: String,
    pub volume_id: i64,
    pub generation: i64,
    pub last_refresh_ms: Option<i64>,
    pub age_ms: Option<i64>,
    pub refresh_incomplete: bool,
    pub threshold_bytes: Option<u64>,
    pub indexed_bytes: u64,
    pub indexed_logical: u64,
    pub file_count: u64,
    pub dir_count: Option<u64>,
    pub denied_count: Option<u64>,
    pub total_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
    pub used_bytes: Option<u64>,
    pub unaccounted_bytes: Option<i64>,
    pub last_scan_id: Option<i64>,
}

type LastScan = (i64, i64, Option<i64>, Option<i64>, Option<i64>);
type VolumeRow = (
    i64,
    String,
    Option<i64>,
    i64,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

fn pick_volume(conn: &Connection, volume: Option<&str>) -> Result<Option<VolumeRow>> {
    let cols = "id, root, root_node_id, gen, threshold, last_refresh_ms, total_bytes, free_bytes";
    let map = |r: &Row| -> rusqlite::Result<VolumeRow> {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
        ))
    };
    Ok(match volume {
        Some(v) => conn
            .query_row(
                &format!("SELECT {cols} FROM volume WHERE root = ?1 COLLATE NOCASE"),
                [normalize_path(v)],
                map,
            )
            .optional()?,
        None => conn
            .query_row(
                &format!(
                    "SELECT {cols} FROM volume WHERE root_node_id IS NOT NULL \
                     ORDER BY last_refresh_ms IS NULL, last_refresh_ms DESC, id LIMIT 1"
                ),
                [],
                map,
            )
            .optional()?,
    })
}

pub fn index_status(conn: &Connection, volume: Option<&str>) -> Result<Option<IndexStatus>> {
    let Some((volume_id, root, root_node, generation, threshold, last_refresh, total, free)) =
        pick_volume(conn, volume)?
    else {
        return Ok(None);
    };
    let (indexed, indexed_logical, files): (i64, i64, i64) = match root_node {
        Some(id) => conn
            .query_row(
                "SELECT sub_alloc, sub_logical, sub_count FROM node WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .unwrap_or((0, 0, 0)),
        None => (0, 0, 0),
    };
    let last_scan: Option<LastScan> = conn
        .query_row(
            "SELECT id, gen, dir_count, denied_count, unaccounted_bytes FROM scan WHERE volume_id = ?1 \
             AND finished_ms IS NOT NULL ORDER BY finished_ms DESC, id DESC LIMIT 1",
            [volume_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let total = total.map(to_u64);
    let free = free.map(to_u64);
    Ok(Some(IndexStatus {
        volume: root,
        volume_id,
        generation,
        last_refresh_ms: last_refresh,
        age_ms: last_refresh.map(|t| (now_ms() - t).max(0)),
        refresh_incomplete: last_scan
            .as_ref()
            .map_or(generation > 0, |s| generation > s.1),
        threshold_bytes: threshold.map(to_u64),
        indexed_bytes: to_u64(indexed),
        indexed_logical: to_u64(indexed_logical),
        file_count: to_u64(files),
        dir_count: last_scan.as_ref().and_then(|s| s.2).map(to_u64),
        denied_count: last_scan.as_ref().and_then(|s| s.3).map(to_u64),
        total_bytes: total,
        free_bytes: free,
        used_bytes: total.zip(free).map(|(t, f)| t.saturating_sub(f)),
        unaccounted_bytes: last_scan.as_ref().and_then(|s| s.4),
        last_scan_id: last_scan.map(|s| s.0),
    }))
}

pub fn humanize_age(age_ms: i64) -> String {
    let s = age_ms.max(0) / 1000;
    let plural = |n: i64, unit: &str| {
        if n == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{n} {unit}s ago")
        }
    };
    match s {
        0..60 => "just now".into(),
        60..3600 => plural(s / 60, "minute"),
        3600..172_800 => plural(s / 3600, "hour"),
        _ => plural(s / 86_400, "day"),
    }
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

// A dir holding >= 90% of its parent explains it, so it replaces the parent in the list.
fn prompt_dirs(conn: &Connection, root: &str, n: usize) -> Result<Ranked<DirItem>> {
    let all = top_dirs(conn, Some(root), 4, n * 5)?;
    let mut picked: Vec<DirItem> = Vec::new();
    for c in all.items {
        if let Some(i) = picked
            .iter()
            .position(|p| is_ancestor_path(&p.path, &c.path) && c.size * 10 >= p.size * 9)
        {
            picked[i] = c;
        } else if picked.len() < n {
            picked.push(c);
        }
    }
    picked.sort_by(|a, b| b.size.cmp(&a.size));
    let shown = picked.len() as u64;
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM node n JOIN volume v ON v.id = n.volume_id \
         WHERE v.root = ?1 AND n.kind = 0 AND n.sub_alloc >= ?2",
        params![root, 1i64 << 30],
        |r| r.get(0),
    )?;
    Ok(Ranked {
        items: picked,
        more_count: to_u64(total).saturating_sub(shown),
        more_bytes: 0,
    })
}

pub fn summary_for_prompt(conn: &Connection, volume: Option<&str>) -> Result<String> {
    summary_with(conn, volume, 15, true)
}

// For watched drives other than the selected one: no change list and only the top folders.
pub fn compact_summary_for_prompt(conn: &Connection, volume: &str) -> Result<String> {
    summary_with(conn, Some(volume), 5, false)
}

fn summary_with(
    conn: &Connection,
    volume: Option<&str>,
    dir_count: usize,
    with_changes: bool,
) -> Result<String> {
    let Some(st) = index_status(conn, volume)? else {
        return Ok("No index yet: the first refresh has not finished.".into());
    };
    let mut s = String::new();
    let age = st
        .age_ms
        .map(humanize_age)
        .unwrap_or_else(|| "an unfinished first refresh".into());
    let _ = write!(
        s,
        "Index of {} as of {age} (refresh {}",
        st.volume, st.generation
    );
    if st.refresh_incomplete {
        s.push_str("; a newer refresh has not finished");
    }
    s.push_str(").\n");
    if let (Some(total), Some(free)) = (st.total_bytes, st.free_bytes) {
        let pct = if total > 0 { free * 100 / total } else { 0 };
        let _ = writeln!(
            s,
            "Volume {}: {} free ({pct}%).",
            humanize_bytes(total),
            humanize_bytes(free)
        );
    }
    let _ = write!(
        s,
        "Indexed {} in {} files",
        humanize_bytes(st.indexed_bytes),
        group_digits(st.file_count)
    );
    if let Some(u) = st.unaccounted_bytes.filter(|u| *u > 0) {
        let _ = write!(
            s,
            "; unaccounted {} (restore points, shadow copies, unreadable dirs)",
            humanize_bytes(to_u64(u))
        );
    }
    s.push_str(".\nBiggest folders:\n");
    let dirs = prompt_dirs(conn, &st.volume, dir_count)?;
    for d in &dirs.items {
        let _ = writeln!(
            s,
            "- {} {} ({} files)",
            d.path,
            humanize_bytes(d.size),
            group_digits(d.files)
        );
    }
    if let Some(more) = dirs.more_line() {
        let _ = writeln!(s, "{more} folders over 1 GB");
    }
    let changes = if with_changes {
        history::changes(conn, Some(st.volume_id), Since::Previous, 5)?
    } else {
        None
    };
    if let Some(ch) = changes {
        let ago = humanize_age(ch.to.finished_ms - ch.from.finished_ms).replace(" ago", "");
        let _ = write!(s, "Since the refresh before ({ago} earlier):");
        if let Some(fd) = ch.free_delta {
            let sign = if fd < 0 { "-" } else { "+" };
            let _ = write!(s, " free {sign}{}", humanize_bytes(fd.unsigned_abs()));
        }
        s.push('\n');
        for c in ch.grew.iter().chain(ch.new.iter()).take(5) {
            let _ = writeln!(
                s,
                "- {} +{}{}",
                c.path,
                humanize_bytes(c.delta.unsigned_abs()),
                if c.before.is_none() { " (new)" } else { "" }
            );
        }
        for c in ch.shrank.iter().chain(ch.removed.iter()).take(3) {
            let _ = writeln!(
                s,
                "- {} -{}{}",
                c.path,
                humanize_bytes(c.delta.unsigned_abs()),
                if c.after.is_none() { " (removed)" } else { "" }
            );
        }
    }
    if s.len() > PROMPT_MAX_CHARS {
        let mut cut = PROMPT_MAX_CHARS;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("\n…");
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanizes_ages() {
        assert_eq!(humanize_age(5_000), "just now");
        assert_eq!(humanize_age(60_000), "1 minute ago");
        assert_eq!(humanize_age(3 * 3_600_000), "3 hours ago");
        assert_eq!(humanize_age(3 * DAY_MS), "3 days ago");
    }

    #[test]
    fn groups_digits() {
        assert_eq!(group_digits(5_812_345), "5,812,345");
        assert_eq!(group_digits(999), "999");
    }

    fn indexed(fs: &crate::index::FakeFs) -> (tempfile::TempDir, crate::store::Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::store::Db::open(dir.path().join("q.db")).unwrap();
        let out = crate::index::refresh(
            &db.conn().unwrap(),
            fs,
            fs.root(),
            &crate::index::RefreshOptions::default(),
        )
        .unwrap();
        assert!(out.completed);
        (dir, db)
    }

    #[test]
    fn top_files_filters_and_counts_the_rest() {
        let fs = crate::index::FakeFs::new(r"C:\");
        let mib = 1u64 << 20;
        fs.file(r"C:\m\a.gguf", 500 * mib);
        fs.file(r"C:\m\b.GGUF", 300 * mib);
        fs.file(r"C:\m\c.safetensors", 200 * mib);
        fs.file(r"C:\x\d.gguf", 100 * mib);
        fs.file(r"C:\x\e_.bin", 2 * mib);
        fs.touch(r"C:\m\b.GGUF", now_ms());
        let (_d, db) = indexed(&fs);
        let conn = db.conn().unwrap();
        let q = |under: Option<&str>, ext: Option<&str>, days: Option<u32>, limit| {
            top_files(
                &conn,
                &TopFilesQuery {
                    under: under.map(String::from),
                    min_size: 50 * mib,
                    ext: ext.map(String::from),
                    not_used_for_days: days,
                    limit,
                },
            )
            .unwrap()
        };
        let all = q(None, None, None, 2);
        assert_eq!(all.items[0].path, r"C:\m\a.gguf");
        assert_eq!((all.more_count, all.more_bytes), (2, 300 * mib));
        let gguf = q(Some(r"c:\M"), Some(".gguf"), None, 10);
        assert_eq!(gguf.items.len(), 2);
        let stale = q(None, Some("gguf, safetensors"), Some(30), 10);
        let names: Vec<&str> = stale.items.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            names,
            vec![r"C:\m\a.gguf", r"C:\m\c.safetensors", r"C:\x\d.gguf"]
        );
        let escaped = top_files(
            &conn,
            &TopFilesQuery {
                ext: Some("_".into()),
                limit: 10,
                ..TopFilesQuery::default()
            },
        )
        .unwrap();
        assert!(escaped.items.is_empty());
        assert!(
            top_files(
                &conn,
                &TopFilesQuery {
                    under: Some(r"C:\nope".into()),
                    limit: 5,
                    ..TopFilesQuery::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn top_dirs_status_and_prompt_summary() {
        let fs = crate::index::FakeFs::new(r"C:\");
        let gib = 1u64 << 30;
        fs.file(r"C:\Users\josha\.lmstudio\models\q.gguf", 30 * gib);
        fs.file(r"C:\Users\josha\Downloads\iso.iso", 5 * gib);
        fs.file(r"C:\Windows\big.dll", 10 * gib);
        let (_d, db) = indexed(&fs);
        let conn = db.conn().unwrap();
        let d1 = top_dirs(&conn, None, 1, 1).unwrap();
        assert_eq!(d1.items[0].path, r"C:\Users");
        assert_eq!((d1.more_count, d1.more_bytes), (1, 10 * gib));
        let d3 = top_dirs(&conn, Some(r"C:\Users"), 3, 10).unwrap();
        assert_eq!(d3.items[0].path, r"C:\Users\josha");
        assert!(
            d3.items
                .iter()
                .any(|d| d.path == r"C:\Users\josha\.lmstudio\models")
        );
        let st = index_status(&conn, None).unwrap().unwrap();
        assert_eq!(st.indexed_bytes, 45 * gib);
        assert_eq!(st.file_count, 3);
        assert!(!st.refresh_incomplete);
        assert!(st.age_ms.unwrap() < 60_000);
        let s = summary_for_prompt(&conn, Some("c:")).unwrap();
        assert!(s.starts_with(r"Index of C:\ as of just now"), "{s}");
        assert!(s.contains(r"C:\Users\josha\.lmstudio\models\q.gguf") || s.contains(r".lmstudio"));
        assert!(s.len() <= PROMPT_MAX_CHARS + 4);
        let kids = children(
            &conn,
            node_id_by_path(&conn, r"C:\Users\josha").unwrap().unwrap(),
        )
        .unwrap();
        assert_eq!(kids[0].path, r"C:\Users\josha\.lmstudio");
    }

    #[test]
    fn hotspots_are_listed_first() {
        let fs = crate::index::FakeFs::new(r"C:\");
        let gib = 1u64 << 30;
        fs.file(r"C:\a\x.bin", gib);
        fs.file(r"C:\z\grow\y.bin", gib);
        fs.file(r"C:\z\grow\deeper\w.bin", gib);
        let (_d, db) = indexed(&fs);
        let conn = db.conn().unwrap();
        fs.file(r"C:\z\grow\y2.bin", gib);
        let one = crate::index::RefreshOptions {
            workers: 1,
            ..crate::index::RefreshOptions::default()
        };
        crate::index::refresh(&conn, &fs, r"C:\", &one).unwrap();
        fs.clear_log();
        crate::index::refresh(&conn, &fs, r"C:\", &one).unwrap();
        let listed = fs.listed();
        assert_eq!(listed[0], r"C:\z\grow");
        assert_eq!(listed[1], r"C:\z\grow\deeper");
        assert_eq!(listed[2], r"C:\");
        assert_eq!(listed.iter().filter(|p| *p == r"C:\z\grow").count(), 1);
    }

    #[test]
    fn ranked_more_line() {
        let r: Ranked<u8> = Ranked {
            items: vec![],
            more_count: 3,
            more_bytes: 2048,
        };
        assert_eq!(r.more_line().as_deref(), Some("+3 more (2.0 KB)"));
    }
}

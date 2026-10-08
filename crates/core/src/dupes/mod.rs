use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::index::{
    FLAG_DENIED, FLAG_HARDLINK_DUP, FLAG_PLACEHOLDER, KIND_FILE, PathCache, is_ancestor_path,
    normalize_path, to_i64, to_u64,
};
use crate::store::now_ms;

pub const DEFAULT_MIN_SIZE: u64 = 100 << 20;
pub const DEFAULT_BUDGET: Duration = Duration::from_secs(30);
const SAMPLE: u64 = 64 << 10;
const CHUNK: usize = 1 << 20;

#[derive(Clone, Debug)]
pub struct DupOptions {
    pub under: Option<String>,
    pub min_size: u64,
    pub budget: Duration,
}

impl Default for DupOptions {
    fn default() -> Self {
        DupOptions {
            under: None,
            min_size: DEFAULT_MIN_SIZE,
            budget: DEFAULT_BUDGET,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DupGroup {
    pub id: i64,
    pub size: u64,
    pub hash: String,
    pub paths: Vec<String>,
}

impl DupGroup {
    pub fn wasted_bytes(&self) -> u64 {
        self.size * (self.paths.len() as u64).saturating_sub(1)
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DupReport {
    pub groups: Vec<DupGroup>,
    // Candidates whose bucket was not fully hashed before the budget ran out.
    pub pending: usize,
    pub cancelled: bool,
    pub hashed_bytes: u64,
    pub candidates: usize,
}

struct Cand {
    path: String,
    size: u64,
    mtime: Option<i64>,
    file_id: Option<Vec<u8>>,
}

struct Known {
    size: u64,
    hash: String,
    mtime: Option<i64>,
    file_id: Option<Vec<u8>>,
}

fn candidates(conn: &Connection, opts: &DupOptions) -> Result<Vec<Cand>> {
    let under = opts
        .under
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(normalize_path);
    let mut st = conn.prepare(
        "SELECT id, logical, mtime, file_id FROM node
         WHERE kind = ?1 AND logical >= ?2 AND (flags & ?3) = 0 AND (attrs & 0x400) = 0
           AND logical IN (SELECT logical FROM node WHERE kind = ?1 AND logical >= ?2 AND (flags & ?3) = 0 GROUP BY logical HAVING COUNT(*) > 1)",
    )?;
    let mask = FLAG_DENIED | FLAG_PLACEHOLDER | FLAG_HARDLINK_DUP;
    let rows = st
        .query_map(
            params![KIND_FILE, to_i64(opts.min_size.max(1)), mask],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<crate::index::FileId>>(3)?
                        .map(|f| f.0.to_vec()),
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut cache = PathCache::default();
    let mut out = Vec::with_capacity(rows.len());
    for (id, size, mtime, file_id) in rows {
        let Some(path) = cache.get(conn, id)? else {
            continue;
        };
        if let Some(u) = &under
            && !(path.eq_ignore_ascii_case(u) || is_ancestor_path(u, &path))
        {
            continue;
        }
        out.push(Cand {
            path,
            size: to_u64(size),
            mtime,
            file_id,
        });
    }
    Ok(out)
}

fn load_known(conn: &Connection) -> Result<HashMap<String, Known>> {
    let mut st = conn.prepare(
        "SELECT lower(m.path), g.size, g.hash, m.mtime, m.file_id FROM dup_member m JOIN dup_group g ON g.id = m.group_id",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            Known {
                size: to_u64(r.get::<_, i64>(1)?),
                hash: r.get(2)?,
                mtime: r.get(3)?,
                file_id: r.get(4)?,
            },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn open(path: &str) -> std::io::Result<std::fs::File> {
    hoardini_win::open_read_preserving_atime(Path::new(path))
}

fn sample_hash(path: &str, size: u64) -> std::io::Result<String> {
    let mut f = open(path)?;
    if f.metadata()?.len() != size {
        return Err(std::io::Error::other("size changed since indexing"));
    }
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; (SAMPLE * 2).min(size) as usize];
    if size <= SAMPLE * 2 {
        f.read_exact(&mut buf)?;
        h.update(&buf);
    } else {
        for off in [0, size - SAMPLE] {
            f.seek(SeekFrom::Start(off))?;
            f.read_exact(&mut buf[..SAMPLE as usize])?;
            h.update(&buf[..SAMPLE as usize]);
        }
    }
    Ok(h.finalize().to_hex().to_string())
}

pub fn full_hash(path: &Path, cancel: &AtomicBool) -> std::io::Result<(u64, String)> {
    let mut f = hoardini_win::open_read_preserving_atime(path)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "cancelled",
            ));
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        total += n as u64;
    }
    Ok((total, h.finalize().to_hex().to_string()))
}

fn same_file_id(a: &Option<Vec<u8>>, b: &Option<Vec<u8>>) -> bool {
    matches!((a, b), (Some(x), Some(y)) if x == y)
}

// Two paths with one file_id are hard links of the same data, not copies worth removing.
fn distinct_files(mut members: Vec<Cand>) -> Vec<Cand> {
    let mut seen: Vec<Option<Vec<u8>>> = Vec::new();
    members.retain(|m| {
        if m.file_id.is_some() && seen.iter().any(|s| same_file_id(s, &m.file_id)) {
            return false;
        }
        seen.push(m.file_id.clone());
        true
    });
    members
}

pub fn find(conn: &Connection, opts: &DupOptions, cancel: &AtomicBool) -> Result<DupReport> {
    let started = Instant::now();
    let all = candidates(conn, opts)?;
    let mut report = DupReport {
        candidates: all.len(),
        ..DupReport::default()
    };
    let known = load_known(conn)?;

    let mut by_size: HashMap<u64, Vec<Cand>> = HashMap::new();
    for c in all {
        by_size.entry(c.size).or_default().push(c);
    }
    let mut buckets: Vec<(u64, Vec<Cand>)> = by_size
        .into_iter()
        .map(|(s, v)| (s, distinct_files(v)))
        .filter(|(_, v)| v.len() > 1)
        .collect();
    buckets.sort_by_key(|(s, _)| std::cmp::Reverse(*s));

    // (size, hash) -> members with their fs state at hash time
    let mut found: HashMap<(u64, String), Vec<Cand>> = HashMap::new();
    let mut unsettled: HashSet<u64> = HashSet::new();
    let mut stop = false;

    'buckets: for (size, members) in buckets {
        if stop || cancel.load(Ordering::Relaxed) {
            stop = true;
            report.cancelled = cancel.load(Ordering::Relaxed);
            report.pending += members.len();
            unsettled.insert(size);
            continue;
        }
        let mut by_sample: HashMap<String, Vec<Cand>> = HashMap::new();
        for m in members {
            match sample_hash(&m.path, m.size) {
                Ok(s) => by_sample.entry(s).or_default().push(m),
                Err(e) => tracing::debug!(path = %m.path, error = %e, "skipped: not sampled"),
            }
        }
        let mut left: Vec<Vec<Cand>> = by_sample.into_values().filter(|v| v.len() > 1).collect();
        left.sort_by(|a, b| a[0].path.cmp(&b[0].path));
        for (gi, group) in left.iter().enumerate() {
            let mut by_full: HashMap<String, Vec<usize>> = HashMap::new();
            for (i, m) in group.iter().enumerate() {
                let reuse = known.get(&m.path.to_lowercase()).filter(|k| {
                    k.size == m.size && k.mtime == m.mtime && same_file_id(&k.file_id, &m.file_id)
                });
                let hash = match reuse {
                    Some(k) => k.hash.clone(),
                    None => {
                        if started.elapsed() >= opts.budget || cancel.load(Ordering::Relaxed) {
                            stop = true;
                            report.cancelled = cancel.load(Ordering::Relaxed);
                            report.pending += group.len() - i;
                            report.pending += left[gi + 1..].iter().map(Vec::len).sum::<usize>();
                            unsettled.insert(size);
                            continue 'buckets;
                        }
                        match full_hash(Path::new(&m.path), cancel) {
                            Ok((n, h)) if n == m.size => {
                                report.hashed_bytes += n;
                                h
                            }
                            Ok(_) => continue,
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                                stop = true;
                                report.cancelled = true;
                                report.pending += group.len() - i;
                                report.pending +=
                                    left[gi + 1..].iter().map(Vec::len).sum::<usize>();
                                unsettled.insert(size);
                                continue 'buckets;
                            }
                            Err(e) => {
                                tracing::debug!(path = %m.path, error = %e, "skipped: not hashed");
                                continue;
                            }
                        }
                    }
                };
                by_full.entry(hash).or_default().push(i);
            }
            for (hash, idx) in by_full {
                if idx.len() < 2 {
                    continue;
                }
                let e = found.entry((size, hash)).or_default();
                for i in idx {
                    let m = &group[i];
                    e.push(Cand {
                        path: m.path.clone(),
                        size: m.size,
                        mtime: m.mtime,
                        file_id: m.file_id.clone(),
                    });
                }
            }
        }
    }

    report.groups = persist(conn, opts, found, &unsettled)?;
    Ok(report)
}

fn persist(
    conn: &Connection,
    opts: &DupOptions,
    found: HashMap<(u64, String), Vec<Cand>>,
    unsettled: &HashSet<u64>,
) -> Result<Vec<DupGroup>> {
    let tx = conn.unchecked_transaction()?;
    let now = now_ms();
    let mut kept_paths: HashSet<String> = HashSet::new();
    let mut result = Vec::new();
    for ((size, hash), mut members) in found {
        members.sort_by(|a, b| a.path.cmp(&b.path));
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM dup_group WHERE size = ?1 AND hash = ?2",
                params![to_i64(size), hash],
                |r| r.get(0),
            )
            .ok();
        let id = match existing {
            Some(id) => id,
            None => {
                tx.execute(
                    "INSERT INTO dup_group(size, hash, created_ms) VALUES (?1, ?2, ?3)",
                    params![to_i64(size), hash, now],
                )?;
                tx.last_insert_rowid()
            }
        };
        // Replace the member list: paths from earlier runs that moved or changed fall out.
        tx.execute("DELETE FROM dup_member WHERE group_id = ?1", [id])?;
        for m in &members {
            tx.execute(
                "INSERT OR REPLACE INTO dup_member(group_id, path, node_id, file_id, mtime)
                 VALUES (?1, ?2, NULL, ?3, ?4)",
                params![id, m.path, m.file_id, m.mtime],
            )?;
            kept_paths.insert(m.path.to_lowercase());
        }
        result.push(DupGroup {
            id,
            size,
            hash,
            paths: members.into_iter().map(|m| m.path).collect(),
        });
    }
    // Rows in scope that this run settled but did not re-confirm are stale.
    let under = opts
        .under
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(normalize_path);
    let stale: Vec<(i64, String)> = {
        let mut st = tx.prepare(
            "SELECT m.group_id, m.path, g.size FROM dup_member m JOIN dup_group g ON g.id = m.group_id WHERE g.size >= ?1",
        )?;
        let rows = st.query_map([to_i64(opts.min_size.max(1))], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        rows.filter_map(Result::ok)
            .filter(|(_, p, s)| {
                let lp = p.to_lowercase();
                let in_scope = under
                    .as_deref()
                    .is_none_or(|u| lp == u.to_lowercase() || is_ancestor_path(u, p));
                in_scope && !kept_paths.contains(&lp) && !unsettled.contains(&to_u64(*s))
            })
            .map(|(g, p, _)| (g, p))
            .collect()
    };
    for (g, p) in stale {
        tx.execute(
            "DELETE FROM dup_member WHERE group_id = ?1 AND path = ?2",
            params![g, p],
        )?;
    }
    tx.execute(
        "DELETE FROM dup_group WHERE (SELECT COUNT(*) FROM dup_member m WHERE m.group_id = dup_group.id) < 2",
        [],
    )?;
    tx.execute(
        "DELETE FROM dup_member WHERE group_id NOT IN (SELECT id FROM dup_group)",
        [],
    )?;
    tx.commit()?;
    result.sort_by_key(|g| std::cmp::Reverse(g.wasted_bytes()));
    Ok(result)
}

// Persisted groups, members that still exist, biggest waste first.
pub fn groups(
    conn: &Connection,
    under: Option<&str>,
    min_size: u64,
    limit: usize,
) -> Result<Vec<DupGroup>> {
    let under = under
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(normalize_path);
    let mut st = conn.prepare(
        "SELECT g.id, g.size, g.hash, m.path FROM dup_group g JOIN dup_member m ON m.group_id = g.id
         WHERE g.size >= ?1 ORDER BY g.id, m.path",
    )?;
    let rows = st
        .query_map([to_i64(min_size)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                to_u64(r.get::<_, i64>(1)?),
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut by_id: Vec<DupGroup> = Vec::new();
    for (id, size, hash, path) in rows {
        if let Some(u) = &under
            && !(path.eq_ignore_ascii_case(u) || is_ancestor_path(u, &path))
        {
            continue;
        }
        if std::fs::symlink_metadata(&path).is_err() {
            continue;
        }
        match by_id.last_mut() {
            Some(g) if g.id == id => g.paths.push(path),
            _ => by_id.push(DupGroup {
                id,
                size,
                hash,
                paths: vec![path],
            }),
        }
    }
    by_id.retain(|g| g.paths.len() > 1);
    by_id.sort_by_key(|g| std::cmp::Reverse(g.wasted_bytes()));
    by_id.truncate(limit);
    Ok(by_id)
}

// (wasted bytes, extra copies) across all persisted groups; None when nothing is known.
pub fn hint(conn: &Connection) -> Option<(u64, u64)> {
    let (bytes, count): (i64, i64) = conn
        .query_row(
            "SELECT COALESCE(SUM(g.size * (c.n - 1)), 0), COALESCE(SUM(c.n - 1), 0)
             FROM dup_group g JOIN (SELECT group_id, COUNT(*) n FROM dup_member GROUP BY group_id) c ON c.group_id = g.id
             WHERE c.n > 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    (count > 0).then(|| (to_u64(bytes), to_u64(count)))
}

#[cfg(test)]
mod tests;

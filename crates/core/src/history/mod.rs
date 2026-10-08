use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use crate::index::{PathCache, is_ancestor_path, split_parent, to_i64, to_u64};
use crate::store::now_ms;

pub const DIR_MIN_BYTES: u64 = 256 << 20;
pub const FILE_MIN_BYTES: u64 = 100 << 20;
pub const KEEP_RECENT: usize = 20;
pub const HOTSPOT_WINDOW: usize = 10;
pub const HOTSPOT_TOP_K: usize = 10;
const MIN_CHANGE: i64 = 1 << 20;
const DAY_MS: i64 = 86_400_000;
const GIB: f64 = (1u64 << 30) as f64;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ScanRecord {
    pub volume_id: i64,
    pub generation: i64,
    pub kind: String,
    pub mode: String,
    pub started_ms: i64,
    pub finished_ms: i64,
    pub total_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
    pub indexed_bytes: u64,
    pub unaccounted_bytes: Option<i64>,
    pub file_count: u64,
    pub dir_count: u64,
    pub denied_count: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScanInfo {
    pub id: i64,
    pub volume_id: i64,
    pub generation: i64,
    pub kind: String,
    pub mode: String,
    pub started_ms: i64,
    pub finished_ms: i64,
    pub total_bytes: Option<u64>,
    pub used_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
    pub indexed_bytes: u64,
    pub unaccounted_bytes: Option<i64>,
    pub file_count: u64,
    pub dir_count: u64,
    pub denied_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SummaryDir {
    pub path: String,
    pub alloc: u64,
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SummaryFile {
    pub path: String,
    pub alloc: u64,
    pub mtime_ms: Option<i64>,
}

const SCAN_COLS: &str = "id, volume_id, gen, kind, mode, started_ms, finished_ms, total_bytes, \
     used_bytes, free_bytes, indexed_bytes, unaccounted_bytes, file_count, dir_count, denied_count";

fn scan_from_row(r: &Row) -> rusqlite::Result<ScanInfo> {
    let opt = |v: Option<i64>| v.map(to_u64);
    Ok(ScanInfo {
        id: r.get(0)?,
        volume_id: r.get(1)?,
        generation: r.get(2)?,
        kind: r.get(3)?,
        mode: r.get(4)?,
        started_ms: r.get(5)?,
        finished_ms: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
        total_bytes: opt(r.get(7)?),
        used_bytes: opt(r.get(8)?),
        free_bytes: opt(r.get(9)?),
        indexed_bytes: to_u64(r.get::<_, Option<i64>>(10)?.unwrap_or(0)),
        unaccounted_bytes: r.get(11)?,
        file_count: to_u64(r.get::<_, Option<i64>>(12)?.unwrap_or(0)),
        dir_count: to_u64(r.get::<_, Option<i64>>(13)?.unwrap_or(0)),
        denied_count: to_u64(r.get::<_, Option<i64>>(14)?.unwrap_or(0)),
    })
}

pub fn scan_by_id(conn: &Connection, id: i64) -> Result<Option<ScanInfo>> {
    Ok(conn
        .query_row(
            &format!("SELECT {SCAN_COLS} FROM scan WHERE id = ?1"),
            [id],
            scan_from_row,
        )
        .optional()?)
}

pub fn scans(conn: &Connection, volume_id: i64) -> Result<Vec<ScanInfo>> {
    let mut st = conn.prepare(&format!(
        "SELECT {SCAN_COLS} FROM scan WHERE volume_id = ?1 AND finished_ms IS NOT NULL \
         ORDER BY finished_ms DESC, id DESC"
    ))?;
    Ok(st
        .query_map([volume_id], scan_from_row)?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn summary_dirs(conn: &Connection, scan_id: i64) -> Result<Vec<SummaryDir>> {
    let mut st = conn.prepare(
        "SELECT path, alloc, count FROM scan_dir WHERE scan_id = ?1 ORDER BY alloc DESC",
    )?;
    Ok(st
        .query_map([scan_id], |r| {
            Ok(SummaryDir {
                path: r.get(0)?,
                alloc: to_u64(r.get(1)?),
                count: to_u64(r.get(2)?),
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn summary_files(conn: &Connection, scan_id: i64) -> Result<Vec<SummaryFile>> {
    let mut st = conn.prepare(
        "SELECT path, alloc, mtime FROM scan_file WHERE scan_id = ?1 ORDER BY alloc DESC",
    )?;
    Ok(st
        .query_map([scan_id], |r| {
            Ok(SummaryFile {
                path: r.get(0)?,
                alloc: to_u64(r.get(1)?),
                mtime_ms: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

fn collect_summary(
    conn: &Connection,
    volume_id: i64,
) -> Result<(Vec<SummaryDir>, Vec<SummaryFile>)> {
    let mut paths = PathCache::default();
    let dir_rows: Vec<(i64, i64, i64)> = conn
        .prepare(
            "SELECT id, sub_alloc, sub_count FROM node WHERE kind = 0 AND sub_alloc >= ?1 AND volume_id = ?2",
        )?
        .query_map(params![to_i64(DIR_MIN_BYTES), volume_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut dirs = Vec::with_capacity(dir_rows.len());
    for (id, alloc, count) in dir_rows {
        if let Some(path) = paths.get(conn, id)? {
            dirs.push(SummaryDir {
                path,
                alloc: to_u64(alloc),
                count: to_u64(count),
            });
        }
    }
    let file_rows: Vec<(i64, i64, Option<i64>)> = conn
        .prepare(
            "SELECT id, alloc, mtime FROM node WHERE kind = 1 AND alloc >= ?1 AND volume_id = ?2 \
             AND (flags & 4) = 0",
        )?
        .query_map(params![to_i64(FILE_MIN_BYTES), volume_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut files = Vec::with_capacity(file_rows.len());
    for (id, alloc, mtime) in file_rows {
        if let Some(path) = paths.get(conn, id)? {
            files.push(SummaryFile {
                path,
                alloc: to_u64(alloc),
                mtime_ms: mtime,
            });
        }
    }
    Ok((dirs, files))
}

pub fn record_scan(conn: &Connection, rec: &ScanRecord) -> Result<i64> {
    let (dirs, files) = collect_summary(conn, rec.volume_id)?;
    let id = record_scan_rows(conn, rec, &dirs, &files)?;
    apply_retention(conn, rec.volume_id)?;
    Ok(id)
}

pub fn record_scan_rows(
    conn: &Connection,
    rec: &ScanRecord,
    dirs: &[SummaryDir],
    files: &[SummaryFile],
) -> Result<i64> {
    let used = rec
        .total_bytes
        .zip(rec.free_bytes)
        .map(|(t, f)| to_i64(t.saturating_sub(f)));
    conn.execute(
        "INSERT INTO scan(volume_id, gen, kind, mode, started_ms, finished_ms, total_bytes, used_bytes, \
         free_bytes, indexed_bytes, unaccounted_bytes, file_count, dir_count, denied_count) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            rec.volume_id,
            rec.generation,
            rec.kind,
            rec.mode,
            rec.started_ms,
            rec.finished_ms,
            rec.total_bytes.map(to_i64),
            used,
            rec.free_bytes.map(to_i64),
            to_i64(rec.indexed_bytes),
            rec.unaccounted_bytes,
            to_i64(rec.file_count),
            to_i64(rec.dir_count),
            to_i64(rec.denied_count)
        ],
    )?;
    let id = conn.last_insert_rowid();
    let mut d = conn.prepare_cached(
        "INSERT OR REPLACE INTO scan_dir(scan_id, path, alloc, count) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for x in dirs {
        d.execute(params![id, x.path, to_i64(x.alloc), to_i64(x.count)])?;
    }
    let mut f = conn.prepare_cached(
        "INSERT OR REPLACE INTO scan_file(scan_id, path, alloc, mtime) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for x in files {
        f.execute(params![id, x.path, to_i64(x.alloc), x.mtime_ms])?;
    }
    Ok(id)
}

pub fn civil_from_unix_ms(ms: i64) -> (i64, u32, u32) {
    let z = ms.div_euclid(DAY_MS) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

pub fn fmt_date(ms: i64) -> String {
    let (y, m, d) = civil_from_unix_ms(ms);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn apply_retention(conn: &Connection, volume_id: i64) -> Result<u64> {
    let all: Vec<(i64, i64)> = conn
        .prepare(
            "SELECT id, finished_ms FROM scan WHERE volume_id = ?1 AND finished_ms IS NOT NULL \
             ORDER BY finished_ms DESC, id DESC",
        )?
        .query_map([volume_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut months = HashSet::new();
    let mut drop = Vec::new();
    let mut monthly = Vec::new();
    for (i, (id, finished)) in all.iter().enumerate() {
        let (y, m, _) = civil_from_unix_ms(*finished);
        let newest_of_month = months.insert((y, m));
        if newest_of_month {
            monthly.push(*id);
        } else if i >= KEEP_RECENT {
            drop.push(*id);
        }
    }
    for id in &monthly {
        conn.execute("UPDATE scan SET keep_monthly = 1 WHERE id = ?1", [id])?;
    }
    for id in &drop {
        conn.execute("DELETE FROM scan_dir WHERE scan_id = ?1", [id])?;
        conn.execute("DELETE FROM scan_file WHERE scan_id = ?1", [id])?;
        conn.execute("DELETE FROM scan WHERE id = ?1", [id])?;
    }
    Ok(drop.len() as u64)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Since {
    Previous,
    Scan(i64),
    Days(u32),
}

impl FromStr for Since {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Since> {
        let s = s.trim().to_ascii_lowercase();
        if s.is_empty() || s == "prev" || s == "previous" {
            return Ok(Since::Previous);
        }
        if let Some(d) = s.strip_suffix('d') {
            return Ok(Since::Days(d.trim().parse()?));
        }
        match s.trim_start_matches('#').parse() {
            Ok(id) => Ok(Since::Scan(id)),
            Err(_) => bail!("expected a scan id, Nd (days), or \"previous\": {s:?}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Change {
    pub path: String,
    pub kind: &'static str,
    pub before: Option<u64>,
    pub after: Option<u64>,
    pub delta: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Omitted {
    pub grew: usize,
    pub shrank: usize,
    pub new: usize,
    pub removed: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Changes {
    pub volume: String,
    pub from: ScanInfo,
    pub to: ScanInfo,
    pub free_delta: Option<i64>,
    pub indexed_delta: i64,
    pub grew: Vec<Change>,
    pub shrank: Vec<Change>,
    pub new: Vec<Change>,
    pub removed: Vec<Change>,
    pub omitted: Omitted,
}

fn latest_volume(conn: &Connection) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT volume_id FROM scan WHERE finished_ms IS NOT NULL ORDER BY finished_ms DESC, id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?)
}

fn parent_key(path_lower: &str) -> Option<String> {
    split_parent(path_lower).map(|(p, _)| p)
}

type SizeMap = HashMap<String, (String, u64)>;

fn size_map<T>(items: Vec<T>, f: impl Fn(T) -> (String, u64)) -> SizeMap {
    items
        .into_iter()
        .map(|x| {
            let (p, a) = f(x);
            (p.to_lowercase(), (p, a))
        })
        .collect()
}

fn diff_into(before: &SizeMap, after: &SizeMap, kind: &'static str, lists: &mut [Vec<Change>; 4]) {
    for (k, (p, a)) in after {
        match before.get(k) {
            Some((_, b)) => {
                let delta = to_i64(*a) - to_i64(*b);
                let change = Change {
                    path: p.clone(),
                    kind,
                    before: Some(*b),
                    after: Some(*a),
                    delta,
                };
                if delta >= MIN_CHANGE {
                    lists[0].push(change);
                } else if delta <= -MIN_CHANGE {
                    lists[1].push(change);
                }
            }
            None => lists[2].push(Change {
                path: p.clone(),
                kind,
                before: None,
                after: Some(*a),
                delta: to_i64(*a),
            }),
        }
    }
    for (k, (p, b)) in before {
        if !after.contains_key(k) {
            lists[3].push(Change {
                path: p.clone(),
                kind,
                before: Some(*b),
                after: None,
                delta: -to_i64(*b),
            });
        }
    }
}

// A child that explains >= 90% of a dir's change says more than the dir, so the dir is dropped.
fn collapse(lists: &mut [Vec<Change>; 4]) {
    let mut explain_up: HashMap<String, i64> = HashMap::new();
    let mut explain_down: HashMap<String, i64> = HashMap::new();
    for c in lists[0].iter().chain(lists[2].iter()) {
        if let Some(p) = parent_key(&c.path.to_lowercase()) {
            let e = explain_up.entry(p).or_default();
            *e = (*e).max(c.delta);
        }
    }
    for c in lists[1].iter().chain(lists[3].iter()) {
        if let Some(p) = parent_key(&c.path.to_lowercase()) {
            let e = explain_down.entry(p).or_default();
            *e = (*e).max(-c.delta);
        }
    }
    let new_dirs: HashSet<String> = lists[2]
        .iter()
        .filter(|c| c.kind == "dir")
        .map(|c| c.path.to_lowercase())
        .collect();
    let removed_dirs: HashSet<String> = lists[3]
        .iter()
        .filter(|c| c.kind == "dir")
        .map(|c| c.path.to_lowercase())
        .collect();
    let explained = |map: &HashMap<String, i64>, c: &Change| {
        c.kind == "dir"
            && map
                .get(&c.path.to_lowercase())
                .is_some_and(|e| e * 10 >= c.delta.abs() * 9)
    };
    lists[0].retain(|c| !explained(&explain_up, c));
    lists[1].retain(|c| !explained(&explain_down, c));
    lists[2].retain(|c| {
        c.kind != "dir"
            || !parent_key(&c.path.to_lowercase()).is_some_and(|p| new_dirs.contains(&p))
    });
    lists[3].retain(|c| {
        c.kind != "dir"
            || !parent_key(&c.path.to_lowercase()).is_some_and(|p| removed_dirs.contains(&p))
    });
}

pub fn changes(
    conn: &Connection,
    volume_id: Option<i64>,
    since: Since,
    limit: usize,
) -> Result<Option<Changes>> {
    let Some(volume_id) = (match volume_id {
        Some(v) => Some(v),
        None => latest_volume(conn)?,
    }) else {
        return Ok(None);
    };
    let all = scans(conn, volume_id)?;
    let Some(to) = all.first().cloned() else {
        return Ok(None);
    };
    let from = match since {
        Since::Previous => all.get(1).cloned(),
        Since::Scan(id) => match all.iter().find(|s| s.id == id) {
            Some(s) => Some(s.clone()),
            None => bail!("scan {id} is not in the history of this volume"),
        },
        Since::Days(n) => {
            let cutoff = now_ms() - i64::from(n) * DAY_MS;
            all.iter()
                .skip(1)
                .find(|s| s.finished_ms <= cutoff)
                .or_else(|| all.last().filter(|s| s.id != to.id))
                .cloned()
        }
    };
    let Some(from) = from.filter(|f| f.id != to.id) else {
        return Ok(None);
    };
    let volume: String =
        conn.query_row("SELECT root FROM volume WHERE id = ?1", [volume_id], |r| {
            r.get(0)
        })?;
    let mut lists: [Vec<Change>; 4] = Default::default();
    let dirs_before = size_map(summary_dirs(conn, from.id)?, |d| (d.path, d.alloc));
    let dirs_after = size_map(summary_dirs(conn, to.id)?, |d| (d.path, d.alloc));
    diff_into(&dirs_before, &dirs_after, "dir", &mut lists);
    let files_before = size_map(summary_files(conn, from.id)?, |f| (f.path, f.alloc));
    let files_after = size_map(summary_files(conn, to.id)?, |f| (f.path, f.alloc));
    diff_into(&files_before, &files_after, "file", &mut lists);
    collapse(&mut lists);
    for l in lists.iter_mut() {
        l.sort_by(|a, b| {
            b.delta
                .abs()
                .cmp(&a.delta.abs())
                .then_with(|| a.path.cmp(&b.path))
        });
    }
    let limit = limit.max(1);
    let omitted = Omitted {
        grew: lists[0].len().saturating_sub(limit),
        shrank: lists[1].len().saturating_sub(limit),
        new: lists[2].len().saturating_sub(limit),
        removed: lists[3].len().saturating_sub(limit),
    };
    for l in lists.iter_mut() {
        l.truncate(limit);
    }
    let [grew, shrank, new, removed] = lists;
    Ok(Some(Changes {
        volume,
        free_delta: to
            .free_bytes
            .zip(from.free_bytes)
            .map(|(a, b)| to_i64(a) - to_i64(b)),
        indexed_delta: to_i64(to.indexed_bytes) - to_i64(from.indexed_bytes),
        from,
        to,
        grew,
        shrank,
        new,
        removed,
        omitted,
    }))
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Hotspot {
    pub path: String,
    pub score: f64,
    pub size: u64,
    pub growth: u64,
    pub recurrence: u32,
    pub refilled: bool,
}

// Growth and recurrence outweigh size: the point is to revisit where things change.
pub fn hotspot_score(size: u64, growth: u64, recurrence: u32, refilled: bool) -> f64 {
    let gib = |b: u64| b as f64 / GIB;
    (1.0 + gib(size)).ln()
        + 3.0 * (1.0 + gib(growth)).ln()
        + f64::from(recurrence)
        + if refilled { 2.0 } else { 0.0 }
}

fn refilled(series: &[u64]) -> bool {
    for i in 1..series.len() {
        let (prev, cur) = (series[i - 1], series[i]);
        if prev >= DIR_MIN_BYTES && cur <= prev / 2 {
            let dropped = prev - cur;
            let regrowth: u64 = series[i..]
                .windows(2)
                .map(|w| w[1].saturating_sub(w[0]))
                .sum();
            if regrowth.saturating_mul(2) >= dropped {
                return true;
            }
        }
    }
    false
}

fn rank_hotspots(conn: &Connection, volume_id: i64) -> Result<Vec<Hotspot>> {
    let root: Option<String> = conn
        .query_row("SELECT root FROM volume WHERE id = ?1", [volume_id], |r| {
            r.get(0)
        })
        .optional()?;
    let mut ids: Vec<i64> = conn
        .prepare(
            "SELECT id FROM scan WHERE volume_id = ?1 AND finished_ms IS NOT NULL \
             ORDER BY finished_ms DESC, id DESC LIMIT ?2",
        )?
        .query_map(params![volume_id, HOTSPOT_WINDOW as i64], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    ids.reverse();
    let n = ids.len();
    let mut series: HashMap<String, (String, Vec<u64>)> = HashMap::new();
    for (i, id) in ids.iter().enumerate() {
        for d in summary_dirs(conn, *id)? {
            let e = series
                .entry(d.path.to_lowercase())
                .or_insert_with(|| (d.path.clone(), vec![0; n]));
            e.0 = d.path;
            e.1[i] = d.alloc;
        }
    }
    let mut recurrence: HashMap<&str, u32> = HashMap::new();
    for i in 1..n {
        let mut growers: Vec<(u64, &str)> = series
            .iter()
            .filter_map(|(k, (_, s))| {
                let g = s[i].saturating_sub(s[i - 1]);
                (g > 0).then_some((g, k.as_str()))
            })
            .collect();
        growers.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        for (_, k) in growers.into_iter().take(HOTSPOT_TOP_K) {
            *recurrence.entry(k).or_default() += 1;
        }
    }
    let mut out: Vec<Hotspot> = series
        .iter()
        .filter(|(_, (p, _))| root.as_deref().is_none_or(|r| !r.eq_ignore_ascii_case(p)))
        .map(|(k, (p, s))| {
            let growth: u64 = s.windows(2).map(|w| w[1].saturating_sub(w[0])).sum();
            let rec = recurrence.get(k.as_str()).copied().unwrap_or(0);
            let refill = refilled(s);
            let size = s.last().copied().unwrap_or(0);
            Hotspot {
                path: p.clone(),
                score: hotspot_score(size, growth, rec, refill),
                size,
                growth,
                recurrence: rec,
                refilled: refill,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(out)
}

// Among the top candidates, an ancestor of another candidate is dropped: the deeper dir is the hotspot.
fn collapse_hotspots(ranked: Vec<Hotspot>, limit: usize) -> Vec<Hotspot> {
    let cands: Vec<Hotspot> = ranked.into_iter().take(limit.saturating_mul(3)).collect();
    let keep: Vec<bool> = cands
        .iter()
        .map(|a| !cands.iter().any(|b| is_ancestor_path(&a.path, &b.path)))
        .collect();
    cands
        .into_iter()
        .zip(keep)
        .filter_map(|(h, k)| k.then_some(h))
        .take(limit)
        .collect()
}

pub fn hotspots(conn: &Connection, volume_id: i64, limit: usize) -> Result<Vec<Hotspot>> {
    Ok(collapse_hotspots(rank_hotspots(conn, volume_id)?, limit))
}

pub fn hotspot_dirs(conn: &Connection, volume_id: i64, limit: usize) -> Result<Vec<String>> {
    let ranked: Vec<Hotspot> = rank_hotspots(conn, volume_id)?
        .into_iter()
        .filter(|h| h.growth > 0)
        .collect();
    Ok(collapse_hotspots(ranked, limit)
        .into_iter()
        .map(|h| h.path)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_unix_ms(0), (1970, 1, 1));
        assert_eq!(fmt_date(1_791_417_600_000), "2026-10-08");
        assert_eq!(fmt_date(951_782_400_000), "2000-02-29");
        assert_eq!(fmt_date(-86_400_000), "1969-12-31");
    }

    #[test]
    fn parses_since() {
        assert_eq!("7d".parse::<Since>().unwrap(), Since::Days(7));
        assert_eq!("12".parse::<Since>().unwrap(), Since::Scan(12));
        assert_eq!("#12".parse::<Since>().unwrap(), Since::Scan(12));
        assert_eq!("previous".parse::<Since>().unwrap(), Since::Previous);
        assert!("soon".parse::<Since>().is_err());
    }

    #[test]
    fn detects_refill() {
        let g = 1u64 << 30;
        assert!(refilled(&[4 * g, 0, 3 * g]));
        assert!(!refilled(&[4 * g, 0, g / 2]));
        assert!(!refilled(&[4 * g, 5 * g, 6 * g]));
    }

    #[test]
    fn score_is_monotone_in_each_signal() {
        let g = 1u64 << 30;
        let base = hotspot_score(2 * g, g, 1, false);
        assert!(hotspot_score(4 * g, g, 1, false) > base);
        assert!(hotspot_score(2 * g, 2 * g, 1, false) > base);
        assert!(hotspot_score(2 * g, g, 2, false) > base);
        assert!(hotspot_score(2 * g, g, 1, true) > base);
    }
}

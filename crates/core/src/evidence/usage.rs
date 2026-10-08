use rusqlite::Connection;

use super::owner;
use super::{Confidence, LastUsedSignal, OwnerApp};

const DAY_MS: i64 = 86_400_000;

pub fn atime_facts(is_dir: bool) -> (Confidence, &'static str) {
    if is_dir {
        return (Confidence::Low, "folder access time is rarely updated");
    }
    match hoardini_win::last_access_setting() {
        Ok(s) if s.enabled => (
            Confidence::Medium,
            "1-hour resolution, may lag up to an hour; AV/indexers can touch it",
        ),
        Ok(_) => (
            Confidence::Low,
            "not tracked: last-access updates are disabled on this machine",
        ),
        Err(_) => (Confidence::Low, "last-access setting unreadable"),
    }
}

pub fn system_time_ms(t: std::time::SystemTime) -> Option<i64> {
    let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(d.as_millis()).ok()
}

// First appearance and the start of the current unchanged run, from stored scan summaries.
pub fn scan_history(conn: &Connection, path: &str, is_dir: bool) -> Option<(i64, Option<i64>)> {
    let sql = if is_dir {
        "SELECT s.finished_ms, f.alloc, f.count FROM scan_dir f JOIN scan s ON s.id = f.scan_id \
         WHERE f.path = ?1 AND s.finished_ms IS NOT NULL ORDER BY s.finished_ms DESC"
    } else {
        "SELECT s.finished_ms, f.alloc, f.mtime FROM scan_file f JOIN scan s ON s.id = f.scan_id \
         WHERE f.path = ?1 AND s.finished_ms IS NOT NULL ORDER BY s.finished_ms DESC"
    };
    let mut st = conn.prepare_cached(sql).ok()?;
    let rows: Vec<(i64, i64, Option<i64>)> = st
        .query_map([path], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .ok()?
        .filter_map(Result::ok)
        .collect();
    let (latest, oldest) = (rows.first()?, rows.last()?);
    let run = rows
        .iter()
        .take_while(|r| (r.1, r.2) == (latest.1, latest.2))
        .count();
    let since = (run >= 2).then(|| rows[run - 1].0);
    Some((oldest.0, since))
}

pub struct Inputs<'a> {
    pub path: &'a str,
    pub is_dir: bool,
    pub atime_ms: Option<i64>,
    pub atime_note: Option<&'a str>,
    pub mtime_ms: Option<i64>,
    pub owner: Option<&'a OwnerApp>,
}

pub fn signals(conn: &Connection, i: &Inputs) -> Vec<LastUsedSignal> {
    let mut out = Vec::new();
    if let Some(ms) = i.atime_ms {
        let (confidence, note) = atime_facts(i.is_dir && i.atime_note.is_none());
        out.push(LastUsedSignal {
            source: "atime".into(),
            value_ms: Some(ms),
            confidence,
            note: Some(match i.atime_note {
                Some(n) => format!("{n}; {note}"),
                None => note.to_string(),
            }),
        });
    }
    if let Some(ms) = i.mtime_ms {
        out.push(LastUsedSignal {
            source: "mtime".into(),
            value_ms: Some(ms),
            confidence: Confidence::High,
            note: Some("last modified, not last opened".into()),
        });
    }
    let entries = owner::assist();
    if let Some((ms, exe)) = owner::last_run_under(&entries, i.path) {
        out.push(LastUsedSignal {
            source: "userassist".into(),
            value_ms: Some(ms),
            confidence: Confidence::High,
            note: Some(format!("last run of {exe}")),
        });
    }
    if let Some(loc) = i.owner.and_then(|o| o.install_location.as_deref())
        && let Some((ms, exe)) = owner::last_run_under(&entries, loc)
    {
        out.push(LastUsedSignal {
            source: "userassist (owner app)".into(),
            value_ms: Some(ms),
            confidence: Confidence::Medium,
            note: Some(format!("last run of {exe}")),
        });
    }
    if let Some((first, since)) = scan_history(conn, i.path, i.is_dir) {
        out.push(LastUsedSignal {
            source: "hoardini first-seen".into(),
            value_ms: Some(first),
            confidence: Confidence::Medium,
            note: Some("first scan whose summary listed it; not its creation time".into()),
        });
        if let Some(ms) = since {
            out.push(LastUsedSignal {
                source: "hoardini unchanged-since".into(),
                value_ms: Some(ms),
                confidence: Confidence::Medium,
                note: Some("size and mtime identical in every scan since".into()),
            });
        }
    }
    out
}

pub fn days_ago(now_ms: i64, then_ms: i64) -> i64 {
    (now_ms - then_ms).max(0) / DAY_MS
}

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use super::{
    Event, JournalSource, Mode, RefreshKind, RefreshOptions, RefreshRun, WinJournal, WinLister,
    refresh_kind,
};
use crate::store::{Db, get_meta};

const USAGE: &str = "usage: hoardini index [--db <path>] [--volume C:\\] [--mode background|fast] \
[--kind auto|full|journal] [--threshold <bytes>] [--workers N] [--memory-cap-mb N]";

#[derive(Debug, PartialEq)]
struct ChildArgs {
    db: PathBuf,
    volume: String,
    mode: Mode,
    kind: RefreshKind,
    threshold: Option<u64>,
    workers: Option<usize>,
    memory_cap_mb: Option<u64>,
}

fn default_volume() -> String {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    format!("{}\\", drive.trim_end_matches('\\'))
}

fn parse_args(args: &[String]) -> Result<Option<ChildArgs>> {
    let mut out = ChildArgs {
        db: crate::paths::db_path(),
        volume: default_volume(),
        mode: Mode::Background,
        kind: RefreshKind::Auto,
        threshold: None,
        workers: None,
        memory_cap_mb: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        if matches!(flag, "-h" | "--help") {
            return Ok(None);
        }
        let mut value = || -> Result<String> {
            match &inline {
                Some(v) => Ok(v.clone()),
                None => it
                    .next()
                    .cloned()
                    .ok_or_else(|| anyhow!("{flag} needs a value")),
            }
        };
        match flag {
            "--db" => out.db = PathBuf::from(value()?),
            "--volume" => out.volume = value()?,
            "--mode" => {
                out.mode = match value()?.as_str() {
                    "background" => Mode::Background,
                    "fast" => Mode::Fast,
                    other => bail!("unknown mode {other:?}"),
                }
            }
            "--kind" => out.kind = value()?.parse()?,
            "--threshold" => out.threshold = Some(value()?.parse().context("--threshold")?),
            "--workers" => out.workers = Some(value()?.parse().context("--workers")?),
            "--memory-cap-mb" => {
                out.memory_cap_mb = Some(value()?.parse().context("--memory-cap-mb")?)
            }
            other => bail!("unknown argument {other:?}"),
        }
    }
    Ok(Some(out))
}

fn emit(v: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

fn meta_num(conn: &rusqlite::Connection, key: &str) -> Option<u64> {
    get_meta(conn, key).ok().flatten()?.trim().parse().ok()
}

fn run(a: ChildArgs) -> Result<i32> {
    let db = Db::open(&a.db)?;
    let conn = db.conn()?;
    if a.mode == Mode::Background
        && let Err(e) = hoardini_win::enter_background_mode()
    {
        tracing::warn!(error = %e, "background mode unavailable; indexing at normal priority");
    }
    let cap_mb = a
        .memory_cap_mb
        .or_else(|| meta_num(&conn, "indexer_memory_cap_mb"))
        .unwrap_or(256);
    let _job = match hoardini_win::cap_process_memory(cap_mb << 20) {
        Ok(g) => Some(g),
        Err(e) => {
            tracing::warn!(error = %e, "memory cap unavailable; indexing without one");
            None
        }
    };
    let workers = match (a.workers, a.mode) {
        (Some(n), _) => n,
        (None, Mode::Background) => meta_num(&conn, "indexer_workers")
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(2),
        (None, Mode::Fast) => std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4),
    };
    let on_event = |ev: Event<'_>| match ev {
        Event::Start {
            volume,
            generation,
            mode,
            kind,
        } => emit(json!({
            "type": "start", "volume": volume, "mode": mode.as_str(), "gen": generation, "kind": kind
        })),
        Event::Progress(p) => emit(json!({
            "type": "progress", "phase": p.phase, "dirs": p.dirs, "files": p.files,
            "bytes": p.bytes, "fraction": p.fraction
        })),
    };
    let journal = match WinJournal::for_path(&a.volume) {
        Ok(j) => Some(j),
        Err(e) => {
            tracing::warn!(error = %e, "no change journal for this volume");
            None
        }
    };
    let ran = refresh_kind(
        &conn,
        &WinLister,
        &a.volume,
        a.kind,
        &RefreshOptions {
            mode: a.mode,
            workers,
            threshold: a.threshold,
            on_event: Some(&on_event),
            journal: journal.as_ref().map(|j| j as &dyn JournalSource),
            ..RefreshOptions::default()
        },
    )?;
    let (out, extra) = match ran {
        RefreshRun::NeedsFull(why) => {
            emit(json!({"type": "needs_full", "reason": why.as_str()}));
            return Ok(3);
        }
        RefreshRun::Full { outcome, fallback } => (
            outcome,
            json!({"kind": "full", "fallback": fallback.map(|f| f.as_str())}),
        ),
        RefreshRun::Journal(j) => (
            j.outcome,
            json!({
                "kind": "journal", "records": j.records, "pages": j.pages, "dirty_dirs": j.dirty_dirs,
                "moved_dirs": j.moved_dirs, "new_dirs": j.new_dirs, "resolved_ids": j.resolved_ids,
                "next_usn": j.next_usn
            }),
        ),
    };
    let mut done = json!({
        "type": "done",
        "scan_id": out.scan_id,
        "duration_ms": out.duration_ms,
        "indexed_bytes": out.indexed_bytes,
        "unaccounted_bytes": out.unaccounted_bytes,
        "dirs": out.dirs,
        "files": out.files,
        "denied": out.denied,
    });
    if let (Some(d), Value::Object(x)) = (done.as_object_mut(), extra) {
        d.extend(x);
    }
    emit(done);
    Ok(0)
}

// Exit codes: 0 done, 1 refresh failed, 2 bad arguments, 3 `--kind journal` needs a full refresh.
pub fn run_child(args: &[String]) -> i32 {
    let a = match parse_args(args) {
        Ok(Some(a)) => a,
        Ok(None) => {
            eprintln!("{USAGE}");
            return 0;
        }
        Err(e) => {
            emit(json!({"type": "error", "message": format!("{e:#}")}));
            eprintln!("{USAGE}");
            return 2;
        }
    };
    match run(a) {
        Ok(code) => code,
        Err(e) => {
            emit(json!({"type": "error", "message": format!("{e:#}")}));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_protocol_args() {
        let a = parse_args(&s(&[
            "--db",
            r"D:\t\h.db",
            "--volume=D:\\",
            "--mode",
            "fast",
            "--threshold",
            "65536",
            "--workers",
            "3",
            "--memory-cap-mb",
            "128",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(a.db, PathBuf::from(r"D:\t\h.db"));
        assert_eq!(a.volume, "D:\\");
        assert_eq!(a.mode, Mode::Fast);
        assert_eq!(a.threshold, Some(65536));
        assert_eq!(a.workers, Some(3));
        assert_eq!(a.memory_cap_mb, Some(128));
        assert_eq!(a.kind, RefreshKind::Auto);
        let j = parse_args(&s(&["--kind", "journal"])).unwrap().unwrap();
        assert_eq!(j.kind, RefreshKind::Journal);
        assert!(parse_args(&s(&["--kind=full"])).unwrap().unwrap().kind == RefreshKind::Full);
        assert!(parse_args(&s(&["--kind", "partial"])).is_err());
        assert!(parse_args(&s(&["--help"])).unwrap().is_none());
        assert!(parse_args(&s(&["--mode", "turbo"])).is_err());
        assert!(parse_args(&s(&["--bogus"])).is_err());
        assert!(parse_args(&s(&["--db"])).is_err());
    }
}

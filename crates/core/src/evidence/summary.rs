use super::usage::days_ago;
use super::{Confidence, Evidence, PathKind};
use crate::catalog::CatalogMatch;
use crate::memory::Kind;
use crate::paths::humanize_bytes;
use crate::policy::Verdict;

fn conf(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

fn last_used_phrase(e: &Evidence, now_ms: i64) -> Option<String> {
    let pick = |source: &str| e.last_used.iter().find(|s| s.source == source);
    let dated = |verb: &str, src: &str, ms: i64, c: Confidence| {
        let d = days_ago(now_ms, ms);
        if d == 0 {
            format!("{verb} today ({src}, {})", conf(c))
        } else {
            format!("{verb} ~{d} days ago ({src}, {})", conf(c))
        }
    };
    if let Some(s) = pick("atime").filter(|s| s.confidence != Confidence::Low)
        && let Some(ms) = s.value_ms
    {
        let d = days_ago(now_ms, ms);
        return Some(if d == 0 {
            format!("opened today (atime, {})", conf(s.confidence))
        } else {
            format!("not opened in ~{d} days (atime, {})", conf(s.confidence))
        });
    }
    if let Some(s) = pick("userassist")
        && let Some(ms) = s.value_ms
    {
        return Some(dated("last run", "userassist", ms, s.confidence));
    }
    let s = pick("mtime")?;
    Some(dated("last modified", "mtime", s.value_ms?, s.confidence))
}

fn catalog_phrase(c: &CatalogMatch) -> String {
    if c.regenerable {
        format!("{}, regenerable", c.name)
    } else if c.verdict_hint == Verdict::Redownload {
        format!("{}, redownloadable", c.name)
    } else {
        c.name.clone()
    }
}

pub fn build(e: &Evidence, now_ms: i64) -> String {
    let head = match (&e.identity, e.kind) {
        (_, PathKind::Missing) => return "missing".into(),
        (Some(i), PathKind::Dir) if i.kind != "unknown" => format!("folder of {}", i.describe()),
        (Some(i), _) if i.kind != "unknown" => i.describe(),
        (_, PathKind::File) => "file".into(),
        (_, PathKind::Dir) => "folder".into(),
        (_, PathKind::Link) => "link".into(),
    };
    let mut parts = vec![head];
    if let Some(b) = e.alloc_bytes {
        parts.push(humanize_bytes(b));
    }
    if e.kind == PathKind::Dir
        && let Some(n) = e.file_count
    {
        parts.push(format!("{n} files"));
    }
    if let Some(c) = &e.catalog {
        parts.push(catalog_phrase(c));
        if c.shared_blobs {
            parts.push("shared blobs: delete only whole units".into());
        }
    } else if let Some(o) = &e.owner {
        parts.push(format!("owner {}", o.name));
    }
    if let Some(p) = last_used_phrase(e, now_ms) {
        parts.push(p);
    }
    if !e.in_use.is_empty() {
        let names: Vec<&str> = e.in_use.iter().take(3).map(|p| p.name.as_str()).collect();
        parts.push(format!("IN USE by {}", names.join(", ")));
    }
    if !e.duplicates.is_empty() {
        parts.push(format!("{} duplicate(s)", e.duplicates.len()));
    }
    if let Some(m) = e.memories.iter().find(|m| m.kind == Kind::Protect) {
        parts.push(format!("PROTECTED by user note: {}", m.text));
    } else if !e.memories.is_empty() {
        parts.push(format!("{} saved note(s)", e.memories.len()));
    }
    if let Some(cap) = e.policy_cap {
        parts.push(format!("verdict capped at {}", cap.as_str()));
    }
    parts.join(", ")
}

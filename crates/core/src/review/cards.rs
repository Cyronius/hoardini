use std::path::Path;

use rusqlite::Connection;
use serde_json::Value;

use super::units::{Unit, Why};
use crate::catalog::{Category, Entry, UnitKind};
use crate::destinations::Destination;
use crate::index::{self, DirLister, NodeKind, is_ancestor_path};
use crate::memory::Memory;
use crate::memory::location::{capitalized, first_sentence};
use crate::policy::{Action, PolicyEnv, Verdict};
use crate::proposals::ProposalKind;

const DAY_MS: i64 = 86_400_000;
pub const ARCHIVE_MIN_BYTES: u64 = 5 << 30;
pub const ARCHIVE_COLD_DAYS: i64 = 180;
// Per-project folders that the catalog files under other categories.
const BUILD_IDS: &[&str] = &["project-venv", "pycache"];

pub struct Cx<'a> {
    pub conn: &'a Connection,
    pub env: &'a PolicyEnv,
    pub lister: &'a dyn DirLister,
    pub now: i64,
    pub destination: Option<Destination>,
    pub locations: Vec<(String, Memory)>,
}

impl Cx<'_> {
    pub fn location(&self, path: &str) -> Option<&Memory> {
        self.locations
            .iter()
            .filter(|(p, _)| p.eq_ignore_ascii_case(path) || is_ancestor_path(p, path))
            .max_by_key(|(p, _)| p.len())
            .map(|(_, m)| m)
    }

    pub fn location_at(&self, path: &str) -> Option<&Memory> {
        self.locations
            .iter()
            .find(|(p, _)| p.eq_ignore_ascii_case(path))
            .map(|(_, m)| m)
    }
}

pub struct Seen<'a> {
    pub unit: &'a Unit,
    pub entry: Option<&'a Entry>,
    pub admin: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub path: String,
    pub size: u64,
    pub verdict: Option<Verdict>,
    pub reason: String,
}

// Order matters: each unit goes to the first card whose `take` returns items.
pub struct Card {
    pub id: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub kind: ProposalKind,
    pub ticked: bool,
    pub take: fn(&Cx, &Seen) -> anyhow::Result<Vec<Pick>>,
}

pub const CARDS: &[Card] = &[
    Card {
        id: "temp",
        title: "Temp and Windows leftovers",
        summary: "Temporary files, crash dumps and logs that nothing has used for a day.",
        kind: ProposalKind::Delete,
        ticked: true,
        take: temp,
    },
    Card {
        id: "caches",
        title: "Caches",
        summary: "Browser, shader, app and package caches; their apps rebuild or download them again.",
        kind: ProposalKind::Delete,
        ticked: true,
        take: caches,
    },
    Card {
        id: "build",
        title: "Build output",
        summary: "Build output and dependency folders; each project rebuilds them.",
        kind: ProposalKind::Delete,
        ticked: true,
        take: build,
    },
    Card {
        id: "downloads",
        title: "Installers and archives in Downloads",
        summary: "Installers, disk images and archives you downloaded; most can be downloaded again.",
        kind: ProposalKind::Delete,
        ticked: false,
        take: downloads,
    },
    Card {
        id: "archive",
        title: "Send elsewhere",
        summary: "Photos, videos and documents nobody has opened for 180 days: copied to the destination, checked, then removed here.",
        kind: ProposalKind::Archive,
        ticked: false,
        take: archive,
    },
];

pub fn card(id: &str) -> Option<&'static Card> {
    CARDS.iter().find(|c| c.id == id)
}

fn catalog_unit<'a>(s: &Seen<'a>) -> Option<&'a Entry> {
    if s.unit.why == Why::Catalog && !s.admin {
        s.entry
    } else {
        None
    }
}

fn temp(cx: &Cx, s: &Seen) -> anyhow::Result<Vec<Pick>> {
    Ok(match catalog_unit(s) {
        Some(e)
            if matches!(
                e.category,
                Category::Temp | Category::CrashDumps | Category::Logs | Category::WindowsLeftover
            ) =>
        {
            picks(cx, s.unit, e)
        }
        _ => Vec::new(),
    })
}

fn caches(cx: &Cx, s: &Seen) -> anyhow::Result<Vec<Pick>> {
    Ok(match catalog_unit(s) {
        Some(e)
            if !BUILD_IDS.contains(&e.id.as_str())
                && matches!(
                    e.category,
                    Category::Cache
                        | Category::BrowserCache
                        | Category::PackageCache
                        | Category::InstallerCache
                ) =>
        {
            picks(cx, s.unit, e)
        }
        _ => Vec::new(),
    })
}

fn build(cx: &Cx, s: &Seen) -> anyhow::Result<Vec<Pick>> {
    Ok(match catalog_unit(s) {
        Some(e)
            if (e.category == Category::BuildOutput || BUILD_IDS.contains(&e.id.as_str()))
                && is_build_output(cx, s.unit, e) =>
        {
            picks(cx, s.unit, e)
        }
        _ => Vec::new(),
    })
}

// The rust-target note says a `target` with no Cargo.toml or pom.xml beside it is unknown.
pub fn is_build_output(cx: &Cx, unit: &Unit, e: &Entry) -> bool {
    if e.id != "rust-target" {
        return true;
    }
    let Some(parent) = Path::new(&unit.path).parent() else {
        return false;
    };
    cx.lister.list_dir(parent).is_ok_and(|es| {
        es.iter().any(|x| {
            x.name.eq_ignore_ascii_case("Cargo.toml") || x.name.eq_ignore_ascii_case("pom.xml")
        })
    })
}

fn downloads(cx: &Cx, s: &Seen) -> anyhow::Result<Vec<Pick>> {
    if catalog_unit(s).is_none_or(|e| e.id != "downloads") {
        return Ok(Vec::new());
    }
    let Some(own) = cx.env.catalog().lookup_exact(&s.unit.path) else {
        return Ok(Vec::new());
    };
    if own.category != Category::Downloads || own.id == "downloads" {
        return Ok(Vec::new());
    }
    Ok(vec![Pick {
        path: s.unit.path.clone(),
        size: s.unit.size,
        verdict: Some(own.verdict()),
        reason: with_cleaned(cx, &s.unit.path, knowledge(own)),
    }])
}

fn archive(cx: &Cx, s: &Seen) -> anyhow::Result<Vec<Pick>> {
    if cx.destination.is_none() || s.unit.why != Why::Kept || !s.unit.is_dir {
        return Ok(Vec::new());
    }
    let Some(loc) = cx.location(&s.unit.path) else {
        return Ok(Vec::new());
    };
    let Some(d) = &loc.detail else {
        return Ok(Vec::new());
    };
    if d.verdict != Some(Verdict::Keep) || !d.actions.contains(&Action::Archive) {
        return Ok(Vec::new());
    }
    let label = capitalized(d.label.as_deref().unwrap_or(&d.category));
    if loc.source == "user" && d.label.as_deref() == Some("move elsewhere") {
        return Ok(vec![Pick {
            path: s.unit.path.clone(),
            size: s.unit.size,
            verdict: Some(Verdict::Review),
            reason: loc.text.clone(),
        }]);
    }
    if !matches!(d.category.as_str(), "photos" | "documents") {
        return Ok(Vec::new());
    }
    let cutoff = cx.now - ARCHIVE_COLD_DAYS * DAY_MS;
    let cold = |path: &str, size: u64| -> anyhow::Result<Option<Pick>> {
        if size < ARCHIVE_MIN_BYTES {
            return Ok(None);
        }
        let last = last_change(cx.conn, path)?;
        Ok(last.filter(|t| *t <= cutoff).map(|t| Pick {
            path: path.to_string(),
            size,
            verdict: Some(Verdict::Review),
            reason: format!(
                "{label}: nothing in it opened or changed for {}.",
                days(cx.now - t)
            ),
        }))
    };
    if let Some(p) = cold(&s.unit.path, s.unit.size)? {
        return Ok(vec![p]);
    }
    let Some(node) = index::node_by_path(cx.conn, &s.unit.path)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for c in index::children(cx.conn, node.id)? {
        if c.kind != NodeKind::Link
            && let Some(p) = cold(&c.path, c.size)?
        {
            out.push(p);
        }
    }
    Ok(out)
}

pub fn last_change(conn: &Connection, path: &str) -> anyhow::Result<Option<i64>> {
    let Some(id) = index::node_id_by_path(conn, path)? else {
        return Ok(None);
    };
    Ok(conn.query_row(
        "WITH RECURSIVE d(id) AS (SELECT ?1 UNION ALL SELECT n.id FROM node n JOIN d ON n.parent_id = d.id) \
         SELECT MAX(MAX(COALESCE(n.mtime, 0), COALESCE(n.atime, 0))) FROM node n JOIN d ON n.id = d.id",
        [id],
        |r| r.get::<_, Option<i64>>(0),
    )?)
}

fn picks(cx: &Cx, unit: &Unit, e: &Entry) -> Vec<Pick> {
    if unit.kind == UnitKind::Contents {
        return unit
            .parts
            .iter()
            .map(|p| Pick {
                path: p.path.clone(),
                size: p.size,
                verdict: Some(e.verdict()),
                reason: with_cleaned(
                    cx,
                    &p.path,
                    match p.mtime_ms {
                        Some(t) => format!("{}: untouched for {}.", e.name, days(cx.now - t)),
                        None => knowledge(e),
                    },
                ),
            })
            .collect();
    }
    vec![Pick {
        path: unit.path.clone(),
        size: unit.size,
        verdict: Some(e.verdict()),
        reason: with_cleaned(cx, &unit.path, knowledge(e)),
    }]
}

pub fn knowledge(e: &Entry) -> String {
    format!("{}: {}", e.name, first_sentence(&e.notes))
}

fn days(ms: i64) -> String {
    match (ms / DAY_MS).max(0) {
        1 => "1 day".into(),
        n => format!("{n} days"),
    }
}

// Cleaned entries belong to job learning (MEM-LEARN-JOB); each carries the time it ran as `ms`.
fn with_cleaned(cx: &Cx, path: &str, reason: String) -> String {
    let last = cx.location(path).and_then(|m| {
        m.detail
            .as_ref()?
            .cleaned
            .iter()
            .filter_map(|v| v.get("ms").and_then(Value::as_i64))
            .max()
    });
    match last {
        Some(ms) => {
            let (_, m, d) = crate::history::civil_from_unix_ms(ms);
            format!("{reason} You cleaned this on {m:02}-{d:02}.")
        }
        None => reason,
    }
}

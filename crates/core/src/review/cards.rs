use std::cell::OnceCell;
use std::path::Path;

use rusqlite::Connection;

use super::units::{Unit, Why};
use crate::catalog::{Category, Entry, UnitKind};
use crate::cloud::SyncRoot;
use crate::compress::CompressCandidate;
use crate::copycheck::{self, Confidence, CopyResult};
use crate::destinations::Destination;
use crate::index::{self, DirLister, NodeKind, is_ancestor_path};
use crate::memory::Memory;
use crate::memory::location::{capitalized, first_sentence};
use crate::paths::humanize_bytes;
use crate::policy::{Action, PolicyEnv, Verdict};
use crate::proposals::ProposalKind;

const DAY_MS: i64 = 86_400_000;
pub const ARCHIVE_MIN_BYTES: u64 = 5 << 30;
pub const ARCHIVE_COLD_DAYS: i64 = 180;
pub const ONLINE_COLD_DAYS: i64 = 90;
// Checks whose identical answer means the bytes can be fetched again; a duplicate, git or sync answer does not.
pub const REDOWNLOAD_CHECKS: &[&str] = &["huggingface", "winget", "archive"];
const CHILD_FILE_MIN: u64 = copycheck::FILE_MIN;
const CHILD_DIR_MIN: u64 = copycheck::DIR_MIN;
const MAX_SYNC_CHECKS: usize = 50;
const ONLINE_DEPTH: usize = 3;
// The catalog files these under other categories, but each belongs to one project.
const BUILD_IDS: &[&str] = &["project-venv", "pycache"];

pub struct Cx<'a> {
    pub conn: &'a Connection,
    pub env: &'a PolicyEnv,
    pub lister: &'a dyn DirLister,
    pub now: i64,
    pub destination: Option<Destination>,
    pub locations: Vec<(String, Memory)>,
    pub sync_roots: Vec<SyncRoot>,
    pub compress: Vec<CompressCandidate>,
    pub leftovers: Vec<String>,
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

    pub fn synced(&self, path: &str) -> bool {
        crate::cloud::root_for(&self.sync_roots, path).is_some()
    }
}

pub struct Seen<'a> {
    pub unit: &'a Unit,
    pub entry: Option<&'a Entry>,
    pub admin: bool,
    copies: OnceCell<Vec<CopyResult>>,
}

impl<'a> Seen<'a> {
    pub fn new(unit: &'a Unit, entry: Option<&'a Entry>, admin: bool) -> Seen<'a> {
        Seen {
            unit,
            entry,
            admin,
            copies: OnceCell::new(),
        }
    }

    pub fn copies(&self, cx: &Cx) -> &[CopyResult] {
        self.copies
            .get_or_init(|| copycheck::results_for(cx.conn, &self.unit.path))
    }
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
        id: "disposable",
        title: "Folders you said can go",
        summary: "You answered Disposable for these folders.",
        kind: ProposalKind::Delete,
        ticked: true,
        take: disposable,
    },
    Card {
        id: "redownload",
        title: "Re-downloadable, verified",
        summary: "Files with an identical copy online or in your archive, checked byte for byte; they can be fetched again.",
        kind: ProposalKind::Delete,
        ticked: true,
        take: redownload,
    },
    Card {
        id: "online_only",
        title: "Online-only",
        summary: "Synced files nobody has opened for 90 days: the copy on this PC is freed, the files stay in the cloud and download again when opened.",
        kind: ProposalKind::OnlineOnly,
        ticked: true,
        take: online_only,
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
        id: "compress",
        title: "Compress in place",
        summary: "Folders Windows can compress in place (LZX): the files stay where they are and open as before.",
        kind: ProposalKind::Compress,
        ticked: false,
        take: compress,
    },
    Card {
        id: "archive",
        title: "Send elsewhere",
        summary: "Photos, videos and documents nobody has opened for 180 days: copied to the destination, checked, then removed here.",
        kind: ProposalKind::Archive,
        ticked: false,
        take: archive,
    },
    Card {
        id: "windows",
        title: "Cleanups only Windows can do",
        summary: "Space only Windows' own tools or an administrator can free; approving asks for administrator rights once.",
        kind: ProposalKind::SystemCleanup,
        ticked: true,
        take: windows,
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

fn disposable(cx: &Cx, s: &Seen) -> anyhow::Result<Vec<Pick>> {
    if s.unit.why != Why::Disposable {
        return Ok(Vec::new());
    }
    let text = cx
        .location_at(&s.unit.path)
        .map_or("You said this folder can go.", |m| m.text.as_str());
    Ok(vec![Pick {
        path: s.unit.path.clone(),
        size: s.unit.size,
        verdict: Some(Verdict::Safe),
        reason: text.to_string(),
    }])
}

fn redownload(cx: &Cx, s: &Seen) -> anyhow::Result<Vec<Pick>> {
    if s.admin || matches!(s.unit.why, Why::Kept | Why::Disposable) {
        return Ok(Vec::new());
    }
    let Some(r) = s
        .copies(cx)
        .iter()
        .filter(|r| {
            r.confidence == Confidence::Identical
                && r.pending().is_none()
                && REDOWNLOAD_CHECKS.contains(&r.check.as_str())
        })
        .min_by_key(|r| REDOWNLOAD_CHECKS.iter().position(|c| *c == r.check))
    else {
        return Ok(Vec::new());
    };
    let note = r.note();
    let reason = match s.entry.filter(|e| e.unit != UnitKind::Container) {
        Some(e) => format!("{}: {note}.", e.name),
        None => format!("{}.", capitalized(&note)),
    };
    Ok(vec![Pick {
        path: s.unit.path.clone(),
        size: s.unit.size,
        verdict: copycheck::verdict_for(r),
        reason,
    }])
}

fn online_only(cx: &Cx, s: &Seen) -> anyhow::Result<Vec<Pick>> {
    if s.admin || !cx.synced(&s.unit.path) {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut checks = MAX_SYNC_CHECKS;
    cold_synced(
        cx,
        (&s.unit.path, s.unit.size, s.unit.is_dir),
        0,
        &mut checks,
        &mut out,
    )?;
    Ok(out)
}

// A folder opened or changed lately may still hold cold folders and files, so the walk goes on into it.
fn cold_synced(
    cx: &Cx,
    (path, size, dir): (&str, u64, bool),
    depth: usize,
    checks: &mut usize,
    out: &mut Vec<Pick>,
) -> anyhow::Result<()> {
    if *checks == 0 {
        return Ok(());
    }
    let cutoff = cx.now - ONLINE_COLD_DAYS * DAY_MS;
    if let Some(last) = last_change(cx.conn, path)?.filter(|t| *t <= cutoff) {
        *checks -= 1;
        if let Some(r) =
            copycheck::sync_result(cx.conn, cx.env.cloud().clone(), &cx.env.profile, path)
        {
            // Pinned files are the user's choice, and nothing local means already online-only.
            let local = r
                .detail
                .get("local_bytes")
                .and_then(serde_json::Value::as_u64);
            if r.confidence == Confidence::Identical
                && !copycheck::is_pinned(&r.detail)
                && local != Some(0)
            {
                out.push(Pick {
                    path: path.to_string(),
                    size,
                    verdict: Some(Verdict::Synced),
                    reason: format!(
                        "{}; nothing in it opened or changed for {}.",
                        capitalized(&r.note()),
                        days(cx.now - last)
                    ),
                });
                return Ok(());
            }
        }
    }
    if !dir || depth >= ONLINE_DEPTH {
        return Ok(());
    }
    let Some(node) = index::node_by_path(cx.conn, path)? else {
        return Ok(());
    };
    for c in index::children(cx.conn, node.id)?
        .into_iter()
        .filter(|c| match c.kind {
            NodeKind::File => c.size >= CHILD_FILE_MIN,
            NodeKind::Dir => c.size >= CHILD_DIR_MIN,
            NodeKind::Link => false,
        })
    {
        cold_synced(
            cx,
            (&c.path, c.size, c.kind == NodeKind::Dir),
            depth + 1,
            checks,
            out,
        )?;
    }
    Ok(())
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
        reason: knowledge(own),
    }])
}

fn compress(cx: &Cx, s: &Seen) -> anyhow::Result<Vec<Pick>> {
    if s.admin || s.unit.why == Why::Disposable {
        return Ok(Vec::new());
    }
    let u = &s.unit.path;
    Ok(cx
        .compress
        .iter()
        .filter(|c| {
            c.path.eq_ignore_ascii_case(u)
                || is_ancestor_path(&c.path, u)
                || is_ancestor_path(u, &c.path)
        })
        .filter_map(|c| {
            let e = c.estimate.as_ref()?;
            Some(Pick {
                path: c.path.clone(),
                size: c.size,
                verdict: Some(Verdict::Review),
                reason: format!(
                    "Windows compression would save about {} ({}% of it); the files stay usable.",
                    humanize_bytes(e.saved_bytes),
                    e.saved_bytes * 100 / e.logical.max(1)
                ),
            })
        })
        .collect())
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

// Machine-wide leftovers the review cannot delete as the user; the elevated helper clears the same list.
fn windows(cx: &Cx, s: &Seen) -> anyhow::Result<Vec<Pick>> {
    let Some(e) = s.entry.filter(|_| s.admin && s.unit.why == Why::Catalog) else {
        return Ok(Vec::new());
    };
    if !cx
        .leftovers
        .iter()
        .any(|p| p.eq_ignore_ascii_case(&s.unit.path))
    {
        return Ok(Vec::new());
    }
    Ok(picks(cx, s.unit, e))
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
                reason: match p.mtime_ms {
                    Some(t) => format!("{}: untouched for {}.", e.name, days(cx.now - t)),
                    None => knowledge(e),
                },
            })
            .collect();
    }
    vec![Pick {
        path: unit.path.clone(),
        size: unit.size,
        verdict: Some(e.verdict()),
        reason: knowledge(e),
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

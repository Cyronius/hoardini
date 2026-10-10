use serde::{Deserialize, Serialize};

use super::cards::{Cx, knowledge, last_change};
use super::units::{Unit, Why};
use crate::catalog::{Category, Entry, UnitKind};
use crate::index::{self, is_ancestor_path};
use crate::memory::Memory;
use crate::memory::knowledge::is_classifier as location_classifier;
use crate::memory::location;
use crate::policy::{Action, Area as PolicyArea, Verdict, area_of};

const DAY_MS: i64 = 86_400_000;
pub const AREA_MIN_BYTES: u64 = 1 << 30;
pub const MAX_AREAS: usize = 30;
pub const RECENT_DAYS: i64 = 30;
pub const ADMIN_HINT: &str = "Needs admin; Hoardini will offer this with Windows cleanups.";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Area {
    pub path: String,
    pub size: u64,
    pub label: String,
    pub category: String,
    pub verdict: Option<Verdict>,
    pub why_not: String,
    pub hint: Option<String>,
    pub unknown: bool,
    pub actions: Vec<Action>,
}

pub struct Left<'a> {
    pub unit: &'a Unit,
    pub entry: Option<&'a Entry>,
    pub admin: bool,
    pub not_build: bool,
    pub claimed: u64,
}

struct Row<'a> {
    path: String,
    size: u64,
    entry: Option<&'a Entry>,
    loc: Option<&'a Memory>,
    admin: bool,
    not_build: bool,
    model: bool,
}

fn same_or_under(a: &str, p: &str) -> bool {
    a.eq_ignore_ascii_case(p) || is_ancestor_path(a, p)
}

fn category_str(c: Category) -> String {
    serde_json::to_value(c)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

pub fn collect<'a>(
    cx: &'a Cx,
    left: &[Left<'a>],
    all: &[Unit],
    root: &str,
    volume: &str,
) -> anyhow::Result<Vec<Area>> {
    let mut rows: Vec<Row<'a>> = Vec::new();
    let catalog = cx.env.catalog();
    for l in left {
        let u = l.unit;
        let mut own = u.size.saturating_sub(l.claimed);
        if u.why == Why::Kept {
            for inner in all.iter().filter(|x| is_ancestor_path(&u.path, &x.path)) {
                own = own.saturating_sub(inner.size);
            }
            for (p, e) in catalog.literal_paths_under(&u.path) {
                if e.verdict() != Verdict::Keep || e.unit == UnitKind::Container {
                    continue;
                }
                if let Some(n) = index::node_by_path(cx.conn, &p)? {
                    own = own.saturating_sub(n.size);
                    rows.push(Row {
                        loc: cx.location_at(&n.path),
                        path: n.path,
                        size: n.size,
                        entry: Some(e),
                        admin: false,
                        not_build: false,
                        model: false,
                    });
                }
            }
        }
        let system = matches!(area_of(cx.env, &u.path), Some(PolicyArea::System { .. }));
        let own_row =
            l.admin || l.not_build || (u.why == Why::Kept && (l.entry.is_some() || system));
        let group = if own_row {
            None
        } else {
            cx.locations
                .iter()
                .filter(|(p, m)| {
                    same_or_under(p, &u.path)
                        && same_or_under(root, p)
                        && !p.eq_ignore_ascii_case(volume)
                        && m.detail.as_ref().is_some_and(|d| d.category != "container")
                })
                .max_by_key(|(p, _)| p.len())
        };
        let (path, loc, entry) = match group {
            Some((p, m)) => (p.clone(), Some(m), None),
            None => (u.path.clone(), cx.location_at(&u.path), l.entry),
        };
        let model = u.why == Why::ModelStore;
        match rows.iter_mut().find(|r| r.path.eq_ignore_ascii_case(&path)) {
            Some(r) => {
                r.size += own;
                r.model |= model;
            }
            None => rows.push(Row {
                path,
                size: own,
                entry,
                loc,
                admin: l.admin,
                not_build: l.not_build,
                model,
            }),
        }
    }
    rows.retain(|r| r.size >= AREA_MIN_BYTES);
    rows.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    rows.truncate(MAX_AREAS);
    rows.iter().map(|r| describe(cx, r, volume)).collect()
}

fn describe(cx: &Cx, r: &Row, volume: &str) -> anyhow::Result<Area> {
    // The user's or a job's word beats the catalog entry; classifier rows only restate it.
    let told = r
        .loc
        .filter(|m| !location_classifier(&m.source))
        .and_then(|m| m.detail.as_ref().map(|d| (m, d)));
    let entry = r.entry.filter(|e| e.unit != UnitKind::Container);
    let (label, category, verdict, actions, text) = if r.not_build {
        (
            "unknown".to_string(),
            "unknown".to_string(),
            None,
            Vec::new(),
            "Named target, but no Cargo.toml or pom.xml sits beside it, so it may not be build output."
                .to_string(),
        )
    } else if let (Some(e), None) = (entry, told) {
        let v = e.verdict();
        let actions = if v == Verdict::Keep || r.admin {
            Vec::new()
        } else {
            Action::ALL.to_vec()
        };
        (
            e.name.clone(),
            category_str(e.category),
            Some(v),
            actions,
            knowledge(e),
        )
    } else if let Some((m, d)) =
        told.or_else(|| r.loc.and_then(|m| m.detail.as_ref().map(|d| (m, d))))
    {
        (
            d.label.clone().unwrap_or_else(|| d.category.clone()),
            d.category.clone(),
            d.verdict,
            d.actions.clone(),
            m.text.clone(),
        )
    } else {
        let id = index::node_id_by_path(cx.conn, &r.path)?.unwrap_or_default();
        let a = location::classify(cx.conn, cx.env, &r.path, id)?;
        (a.label, a.category, a.verdict, a.actions, a.text)
    };
    let unknown = category == "unknown";
    let mut why_not = if r.model || matches!(category.as_str(), "model-store" | "model-weights") {
        "Model files: Hoardini proposes them once it can show where else a copy exists.".to_string()
    } else if category == "vm-disk" {
        "A virtual machine's disk: deleting it deletes the machine inside.".to_string()
    } else if category == "game-library" {
        "Installed games: remove them through their launcher.".to_string()
    } else {
        text
    };
    if unknown && last_change(cx.conn, &r.path)?.is_some_and(|t| cx.now - t < RECENT_DAYS * DAY_MS)
    {
        why_not.push_str(" Something in it changed in the last 30 days.");
    }
    Ok(Area {
        hint: hint(cx, r, &category, volume),
        path: r.path.clone(),
        size: r.size,
        label,
        category,
        verdict,
        why_not,
        unknown,
        actions,
    })
}

fn hint(cx: &Cx, r: &Row, category: &str, volume: &str) -> Option<String> {
    if r.admin {
        let windows = r.entry.is_some_and(|e| {
            matches!(
                e.category,
                Category::Temp | Category::CrashDumps | Category::Logs | Category::WindowsLeftover
            )
        });
        return Some(if windows {
            ADMIN_HINT.into()
        } else {
            "Needs admin.".into()
        });
    }
    match r.entry.map(|e| e.id.as_str()) {
        Some("pagefile") => return Some("Windows manages its size.".into()),
        Some("winsxs") => return Some("Windows can trim this itself.".into()),
        Some("hiberfil") => {
            return Some(
                "Windows recreates it at boot; turning hibernation off removes it.".into(),
            );
        }
        _ => {}
    }
    if matches!(category, "photos" | "documents") && cx.destination.is_none() {
        let drive = volume.trim_end_matches('\\');
        return Some(format!("Add a destination to move them off {drive}."));
    }
    None
}

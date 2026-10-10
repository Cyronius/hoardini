use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::cards::{Cx, knowledge, last_change};
use super::units::{Unit, Why};
use crate::catalog::{Category, Entry, UnitKind};
use crate::copycheck;
use crate::index::{self, is_ancestor_path};
use crate::memory::Memory;
use crate::memory::knowledge::is_classifier as location_classifier;
use crate::memory::location;
use crate::policy::{Action, Area as PolicyArea, Verdict, area_of};

const DAY_MS: i64 = 86_400_000;
pub const AREA_MIN_BYTES: u64 = 1 << 30;
pub const MAX_AREAS: usize = 30;
pub const RECENT_DAYS: i64 = 30;
pub const ADMIN_HINT: &str = "Needs admin.";
const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "CMakeLists.txt",
];

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
    #[serde(default)]
    pub only_copy: bool,
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
    only_copy: bool,
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
    let mut projects: HashMap<String, bool> = HashMap::new();
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
                        only_copy: false,
                    });
                }
            }
        }
        let system = matches!(area_of(cx.env, &u.path), Some(PolicyArea::System { .. }));
        // Every check that applies came up empty, so it is listed as it is, not folded into its project.
        // A virtual disk or an installed game is always its own only copy; the catalog says more about it.
        let only_copy = !l.admin
            && u.why != Why::Kept
            && own >= AREA_MIN_BYTES
            && l.entry
                .is_none_or(|e| !matches!(e.category, Category::VmDisk | Category::GameLibrary))
            && copycheck::only_copy(cx.conn, &u.path);
        let own_row = l.admin
            || l.not_build
            || only_copy
            || (u.why == Why::Kept && (l.entry.is_some() || system));
        let rows_above: Vec<&(String, Memory)> = if own_row {
            Vec::new()
        } else {
            cx.locations
                .iter()
                .filter(|(p, m)| {
                    same_or_under(p, &u.path)
                        && same_or_under(root, p)
                        && !p.eq_ignore_ascii_case(volume)
                        && m.detail.as_ref().is_some_and(|d| d.category != "container")
                })
                .collect()
        };
        let category = |m: &Memory| m.detail.as_ref().map(|d| d.category.clone());
        let deepest = rows_above.iter().max_by_key(|(p, _)| p.len()).copied();
        // A subfolder that is unknown, or of the project's own kind, says less than the project does.
        let project = deepest.and_then(|(_, dm)| {
            let inner = category(dm);
            rows_above
                .iter()
                .filter(|(p, m)| {
                    (inner.as_deref() == Some("unknown") || category(m) == inner)
                        && *projects
                            .entry(p.to_lowercase())
                            .or_insert_with(|| is_project(cx, p))
                })
                .min_by_key(|(p, _)| p.len())
                .copied()
        });
        let (path, loc, entry) = match project.or(deepest) {
            Some((p, m)) => (p.clone(), Some(m), None),
            None if !own_row && u.why == Why::Leaf => {
                match project_folder(cx, root, &u.path, &mut projects) {
                    Some(p) => (p, None, None),
                    None => (u.path.clone(), cx.location_at(&u.path), l.entry),
                }
            }
            None => (u.path.clone(), cx.location_at(&u.path), l.entry),
        };
        match rows.iter_mut().find(|r| r.path.eq_ignore_ascii_case(&path)) {
            Some(r) => r.size += own,
            None => rows.push(Row {
                path,
                size: own,
                entry,
                loc,
                admin: l.admin,
                not_build: l.not_build,
                only_copy,
            }),
        }
    }
    rows.retain(|r| r.size >= AREA_MIN_BYTES);
    rows.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    rows.truncate(MAX_AREAS);
    rows.iter().map(|r| describe(cx, r, volume)).collect()
}

fn project_folder(
    cx: &Cx,
    root: &str,
    path: &str,
    known: &mut HashMap<String, bool>,
) -> Option<String> {
    let mut chain = Vec::new();
    let mut cur = path;
    while let Some(i) = cur.rfind('\\') {
        cur = &cur[..i];
        if cur.len() <= root.len() {
            break;
        }
        chain.push(cur.to_string());
    }
    chain.into_iter().rev().find(|p| {
        *known
            .entry(p.to_lowercase())
            .or_insert_with(|| is_project(cx, p))
    })
}

fn is_project(cx: &Cx, path: &str) -> bool {
    cx.lister.list_dir(Path::new(path)).is_ok_and(|es| {
        es.iter().any(|e| {
            PROJECT_MARKERS
                .iter()
                .any(|m| e.name.eq_ignore_ascii_case(m))
                || e.name.to_ascii_lowercase().ends_with(".sln")
        })
    })
}

fn describe(cx: &Cx, r: &Row, volume: &str) -> anyhow::Result<Area> {
    // A Disposable answer the folder has outgrown says nothing any more, so the area is asked about again.
    let loc = r.loc.filter(|m| {
        !(location::is_disposable_answer(m) && location::answer_outgrown(cx.conn, cx.env, m))
    });
    // The user's or a job's word beats the catalog entry; classifier rows only restate it.
    let told = loc
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
        told.or_else(|| loc.and_then(|m| m.detail.as_ref().map(|d| (m, d))))
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
    let mut why_not = if matches!(category.as_str(), "model-store" | "model-weights") {
        "Model files: Hoardini proposes them once it can show where else a copy exists.".to_string()
    } else if category == "vm-disk" {
        "A virtual machine's disk: deleting it deletes the machine inside.".to_string()
    } else if category == "game-library" {
        "Installed games: remove them through their launcher.".to_string()
    } else {
        text
    };
    if r.only_copy {
        let notes: Vec<String> = copycheck::results_for(cx.conn, &r.path)
            .iter()
            .map(|c| c.note())
            .collect();
        why_not = format!("Only copy we know of: {}.", notes.join("; "));
    } else if unknown
        && last_change(cx.conn, &r.path)?.is_some_and(|t| cx.now - t < RECENT_DAYS * DAY_MS)
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
        only_copy: r.only_copy,
    })
}

fn hint(cx: &Cx, r: &Row, category: &str, volume: &str) -> Option<String> {
    if r.admin {
        return Some(ADMIN_HINT.into());
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
        let drive = volume
            .get(..2)
            .filter(|d| d.ends_with(':'))
            .unwrap_or(volume);
        return Some(format!("Add a destination to move them off {drive}."));
    }
    None
}

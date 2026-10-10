use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::knowledge::{
    self, Detail, Knowledge, MODEL_WEIGHTS, is_classifier, knowledge_at, set_detail,
};
use super::{
    COLS, Kind, Memory, NewMemory, Scope, covering, from_row, normalize_key_with,
    path_and_ancestors, remember_with,
};
use crate::catalog::{Category, UnitKind};
use crate::index::{self, PathCache, is_ancestor_path, normalize_path, to_u64};
use crate::paths;
use crate::policy::{Action, Area, PolicyEnv, Verdict, area_of};

const GIB: u64 = 1 << 30;
const DAY_MS: i64 = 86_400_000;
pub const LARGE_BYTES: u64 = 5 * GIB;
pub const LARGE_USED_PCT: u64 = 1;
// The index ranks dirs by size only from 256 MiB up (node_sub_alloc), so smaller ones never count as large.
const LARGE_FLOOR: u64 = 256 << 20;
pub const MAX_PER_VOLUME: usize = 40;
pub const DOMINANT_CHILD_PCT: u64 = 90;
pub const MIX_SHARE_PCT: u64 = 80;
pub const RESIZE_PCT: u64 = 25;
pub const REASSESS_MS: i64 = 90 * DAY_MS;

pub const PHOTO_VIDEO_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "heic", "heif", "webp", "tif", "tiff", "bmp", "psd", "cr2", "cr3",
    "nef", "arw", "dng", "orf", "rw2", "raf", "srw", "pef", "mp4", "mov", "m4v", "avi", "mkv",
    "wmv", "mts", "m2ts", "3gp", "webm", "mpg", "mpeg", "vob",
];
pub const DOCUMENT_EXTS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "md",
    "csv", "epub", "pages", "numbers", "key", "one", "msg", "pst",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Large {
    pub node_id: i64,
    pub path: String,
    pub size: u64,
}

pub fn large_folders(conn: &Connection, volume: &str) -> anyhow::Result<Vec<Large>> {
    let root = normalize_path(volume);
    let Some((vid, root_node, total, free)) = conn
        .query_row(
            "SELECT id, root_node_id, total_bytes, free_bytes FROM volume WHERE root = ?1 COLLATE NOCASE",
            [&root],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(Vec::new());
    };
    let Some(root_node) = root_node else {
        return Ok(Vec::new());
    };
    let indexed: i64 = conn.query_row(
        "SELECT sub_alloc FROM node WHERE id = ?1",
        [root_node],
        |r| r.get(0),
    )?;
    let used = match (total, free) {
        (Some(t), Some(f)) if t > f => to_u64(t - f),
        _ => to_u64(indexed),
    };
    let min = LARGE_BYTES
        .min(used * LARGE_USED_PCT / 100)
        .max(LARGE_FLOOR);
    let mut st = conn.prepare(
        "SELECT id, sub_alloc FROM node WHERE kind = 0 AND sub_alloc >= 268435456 AND sub_alloc >= ?1 \
         AND volume_id = ?2 AND parent_id IS NOT NULL ORDER BY sub_alloc DESC, id",
    )?;
    let rows = st
        .query_map(params![index::to_i64(min), vid], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut biggest = conn.prepare_cached(
        "SELECT MAX(CASE WHEN kind = 0 THEN sub_alloc ELSE alloc END) FROM node WHERE parent_id = ?1",
    )?;
    let mut paths = PathCache::default();
    let mut out = Vec::new();
    for (id, size) in rows {
        let size = to_u64(size);
        let child = to_u64(
            biggest
                .query_row([id], |r| r.get::<_, Option<i64>>(0))?
                .unwrap_or(0),
        );
        if child * 100 >= size * DOMINANT_CHILD_PCT {
            continue;
        }
        let Some(path) = paths.get(conn, id)? else {
            continue;
        };
        out.push(Large {
            node_id: id,
            path,
            size,
        });
        if out.len() == MAX_PER_VOLUME {
            break;
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Assessment {
    pub source: &'static str,
    pub by: String,
    pub category: String,
    pub label: String,
    pub verdict: Option<Verdict>,
    pub actions: Vec<Action>,
    pub text: String,
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

fn first_sentence(s: &str) -> &str {
    match s.find(". ") {
        Some(i) => &s[..=i],
        None => s,
    }
}

fn allowed(verdict: Option<Verdict>) -> Vec<Action> {
    match verdict {
        Some(Verdict::Keep) | None => Vec::new(),
        Some(_) => Action::ALL.to_vec(),
    }
}

fn mix_assessment(
    conn: &Connection,
    node_id: i64,
    weights: &[String],
    personal_only: bool,
) -> anyhow::Result<Option<Assessment>> {
    let mix = index::ext_mix(conn, node_id)?;
    if mix.total_bytes == 0 {
        return Ok(None);
    }
    let share = |exts: &[&str]| -> u64 {
        let b: u64 = mix
            .types
            .iter()
            .filter(|t| exts.iter().any(|e| e.eq_ignore_ascii_case(&t.ext)))
            .map(|t| t.bytes)
            .sum();
        b * 100 / mix.total_bytes
    };
    let weight_refs: Vec<&str> = weights.iter().map(String::as_str).collect();
    let tables: [(&str, &str, &[&str], &str); 3] = [
        (
            "photos",
            "your photos and videos",
            PHOTO_VIDEO_EXTS,
            "images or video",
        ),
        ("documents", "your documents", DOCUMENT_EXTS, "documents"),
        (MODEL_WEIGHTS, "model weights", &weight_refs, "model files"),
    ];
    for (category, label, exts, what) in tables {
        if personal_only && category == MODEL_WEIGHTS {
            continue;
        }
        let pct = share(exts);
        if pct < MIX_SHARE_PCT {
            continue;
        }
        let verdict = if category == MODEL_WEIGHTS {
            Verdict::Review
        } else {
            Verdict::Keep
        };
        let actions = if verdict == Verdict::Keep {
            vec![Action::Archive]
        } else {
            Action::ALL.to_vec()
        };
        return Ok(Some(Assessment {
            source: "mix",
            by: "mix".into(),
            category: category.into(),
            label: label.into(),
            verdict: Some(verdict),
            actions,
            text: format!("{}: {pct}% of its bytes are {what}.", capitalized(label)),
        }));
    }
    Ok(None)
}

pub fn classify(
    conn: &Connection,
    env: &PolicyEnv,
    path: &str,
    node_id: i64,
) -> anyhow::Result<Assessment> {
    let p = normalize_path(path);
    let area = area_of(env, &p);
    if let Some(Area::System { label }) = &area {
        return Ok(Assessment {
            source: "deny",
            by: "deny".into(),
            category: "system".into(),
            label: label.clone(),
            verdict: Some(Verdict::Keep),
            actions: Vec::new(),
            text: format!(
                "{}: Hoardini never deletes anything here.",
                capitalized(label)
            ),
        });
    }
    if let Some(m) = covering(conn, &p, &env.profile)?
        .into_iter()
        .find(|m| m.kind == Kind::Protect)
    {
        return Ok(Assessment {
            source: "deny",
            by: format!("protect:{}", m.id),
            category: "protected".into(),
            label: "protected".into(),
            verdict: Some(Verdict::Keep),
            actions: Vec::new(),
            text: format!("Protected by your note: {}", m.text),
        });
    }
    let weights = knowledge::weight_extensions(conn)?;
    let cat = env.catalog();
    if let Some(e) = cat.lookup(&p) {
        let exact = cat.lookup_exact(&p).is_some_and(|x| x.id == e.id);
        if e.unit != UnitKind::Container || exact {
            let verdict = e.verdict();
            if e.category == Category::UserData
                && verdict != Verdict::Keep
                && let Some(a) = mix_assessment(conn, node_id, &weights, true)?
            {
                return Ok(a);
            }
            let category = serde_json::to_value(e.category)?
                .as_str()
                .unwrap_or_default()
                .to_string();
            return Ok(Assessment {
                source: "catalog",
                by: format!("catalog:{}", e.id),
                category,
                label: e.name.clone(),
                verdict: Some(verdict),
                actions: allowed(Some(verdict)),
                text: format!("{}: {}", e.name, first_sentence(&e.notes)),
            });
        }
    }
    if let Some(Area::Container { label }) = area {
        return Ok(Assessment {
            source: "deny",
            by: "deny".into(),
            category: "container".into(),
            text: format!(
                "{}: a container; what is inside is judged folder by folder.",
                capitalized(&label)
            ),
            label,
            verdict: None,
            actions: Vec::new(),
        });
    }
    if let Some(a) = mix_assessment(conn, node_id, &weights, false)? {
        return Ok(a);
    }
    Ok(Assessment {
        source: "index",
        by: "index".into(),
        category: "unknown".into(),
        label: "unknown".into(),
        verdict: None,
        actions: Vec::new(),
        text: "Not identified: no catalog entry matches, and no kind of file makes up 80% of it."
            .into(),
    })
}

pub(crate) fn expanded(env: &PolicyEnv, key: &str) -> String {
    normalize_path(&paths::expand_with(key, &env.profile))
}

pub fn locations(conn: &Connection) -> anyhow::Result<Vec<Memory>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM memory WHERE scope = 'path' AND detail_json IS NOT NULL ORDER BY id"
    ))?;
    let rows = st.query_map([], from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn due(m: &Memory, size: u64, now: i64) -> bool {
    if m.source == "user" {
        return false;
    }
    let Some(d) = &m.detail else {
        return true;
    };
    let (Some(at), Some(when)) = (d.size_at, d.assessed_ms) else {
        return true;
    };
    size.abs_diff(at) * 100 >= at * RESIZE_PCT || now - when >= REASSESS_MS
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RefreshSummary {
    pub large: usize,
    pub assessed: usize,
    pub written: usize,
    pub dropped: usize,
}

pub fn refresh_volume(
    conn: &Connection,
    env: &PolicyEnv,
    volume: &str,
    now: i64,
) -> anyhow::Result<RefreshSummary> {
    let root = normalize_path(volume);
    let on_volume = |p: &str| root.eq_ignore_ascii_case(p) || is_ancestor_path(&root, p);
    let rows: Vec<(String, Memory)> = locations(conn)?
        .into_iter()
        .map(|m| (expanded(env, &m.key), m))
        .filter(|(p, _)| on_volume(p))
        .collect();
    let large = large_folders(conn, &root)?;
    let mut sum = RefreshSummary {
        large: large.len(),
        ..RefreshSummary::default()
    };
    for l in &large {
        let existing = rows
            .iter()
            .find(|(p, _)| p.eq_ignore_ascii_case(&l.path))
            .map(|(_, m)| m);
        if existing.is_some_and(|m| !due(m, l.size, now)) {
            continue;
        }
        sum.assessed += 1;
        let a = classify(conn, env, &l.path, l.node_id)?;
        let mut detail = existing.and_then(|m| m.detail.clone()).unwrap_or_default();
        detail.category = a.category;
        detail.label = Some(a.label);
        detail.verdict = a.verdict;
        detail.actions = a.actions;
        detail.size_at = Some(l.size);
        detail.assessed_ms = Some(now);
        detail.by = Some(a.by);
        // A job row only records what was cleaned there; what the folder is stays the sweep's call (MEM-LEARN-JOB).
        if let Some(m) = existing.filter(|m| is_job(&m.source)) {
            conn.execute(
                "UPDATE memory SET text = ?1, detail_json = ?2, updated_ms = ?3 WHERE id = ?4",
                params![a.text, serde_json::to_string(&detail)?, now, m.id],
            )?;
            sum.written += 1;
            continue;
        }
        let (m, written) = knowledge::upsert_with(
            conn,
            Knowledge {
                scope: Scope::Path,
                key: l.path.clone(),
                kind: Kind::Verdict,
                text: a.text,
                source: a.source.into(),
                detail,
            },
            now,
            &env.profile,
        )?;
        if written {
            sum.written += 1;
        } else if m.source != "user" {
            let mut d = m.detail.clone().unwrap_or_default();
            d.size_at = Some(l.size);
            d.assessed_ms = Some(now);
            set_detail(conn, m.id, &d)?;
        }
    }
    let still: HashSet<String> = large.iter().map(|l| l.path.to_lowercase()).collect();
    for (p, m) in &rows {
        let job = m.detail.as_ref().is_some_and(Detail::has_job_content);
        if !still.contains(&p.to_lowercase()) && is_classifier(&m.source) && !m.pinned && !job {
            conn.execute("DELETE FROM memory WHERE id = ?1", [m.id])?;
            sum.dropped += 1;
        }
    }
    Ok(sum)
}

pub fn is_job(source: &str) -> bool {
    source.starts_with("job:")
}

// `rule` is a glob in key syntax (`~\...`, `*` within a folder, `**` across folders, metacharacters bracketed).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cleaned {
    pub ms: i64,
    pub bytes: u64,
    pub path: String,
    pub rule: String,
    pub job: String,
    pub kind: String,
}

impl Cleaned {
    pub fn verb(&self) -> &'static str {
        match self.kind.as_str() {
            "archive" => "archived",
            "compress" => "compressed",
            _ => "cleaned",
        }
    }

    pub fn reason(&self) -> String {
        format!(
            "you {} this on {} ({})",
            self.verb(),
            short_date(self.ms),
            paths::humanize_bytes(self.bytes)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kept {
    pub ms: i64,
    pub path: String,
    pub size: u64,
    pub job: String,
}

pub(crate) fn short_date(ms: i64) -> String {
    let d = crate::history::fmt_date(ms);
    d.get(5..).unwrap_or(&d).to_string()
}

// Entries this build cannot read are skipped, never dropped from the row.
pub fn cleaned_entries(d: &Detail) -> Vec<Cleaned> {
    d.cleaned
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect()
}

pub fn kept_entries(d: &Detail) -> Vec<Kept> {
    d.kept
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect()
}

fn covering_rows(conn: &Connection, key: &str) -> anyhow::Result<Vec<Memory>> {
    let mut out = Vec::new();
    for k in path_and_ancestors(key) {
        if let Some(m) = knowledge_at(conn, Scope::Path, &k)? {
            out.push(m);
        }
    }
    Ok(out)
}

pub fn was_kept(conn: &Connection, path: &str, size: u64) -> bool {
    was_kept_with(conn, &paths::profile_dir().to_string_lossy(), path, size).unwrap_or(false)
}

// Growing by half means data the user has not seen, so it is worth asking about again.
pub fn was_kept_with(
    conn: &Connection,
    profile: &str,
    path: &str,
    size: u64,
) -> anyhow::Result<bool> {
    let key = normalize_key_with(
        Scope::Path,
        path.strip_prefix(r"\\?\").unwrap_or(path),
        profile,
    );
    for m in covering_rows(conn, &key)? {
        let Some(d) = &m.detail else { continue };
        if kept_entries(d).iter().any(|k| {
            k.path.eq_ignore_ascii_case(&key) && u128::from(size) * 2 < u128::from(k.size) * 3
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn cleaned_for(conn: &Connection, path: &str) -> Option<Cleaned> {
    cleaned_for_with(conn, &paths::profile_dir().to_string_lossy(), path)
        .ok()
        .flatten()
}

pub fn cleaned_for_with(
    conn: &Connection,
    profile: &str,
    path: &str,
) -> anyhow::Result<Option<Cleaned>> {
    let key = normalize_key_with(
        Scope::Path,
        path.strip_prefix(r"\\?\").unwrap_or(path),
        profile,
    );
    let chain: Vec<String> = path_and_ancestors(&key)
        .into_iter()
        .map(|k| k.replace('\\', "/"))
        .collect();
    let mut best: Option<Cleaned> = None;
    for m in covering_rows(conn, &key)? {
        let Some(d) = &m.detail else { continue };
        for c in cleaned_entries(d) {
            let hit = c.path.eq_ignore_ascii_case(&key)
                || super::compile_glob(&c.rule).is_ok_and(|g| chain.iter().any(|p| g.is_match(p)));
            if hit && best.as_ref().is_none_or(|b| c.ms > b.ms) {
                best = Some(c);
            }
        }
    }
    Ok(best)
}

pub fn location_at(conn: &Connection, profile: &str, path: &str) -> anyhow::Result<Option<Memory>> {
    let key = normalize_key_with(Scope::Path, path, profile);
    knowledge_at(conn, Scope::Path, &key)
}

pub fn location_covering(
    conn: &Connection,
    profile: &str,
    path: &str,
) -> anyhow::Result<Option<Memory>> {
    let norm = normalize_key_with(
        Scope::Path,
        path.strip_prefix(r"\\?\").unwrap_or(path),
        profile,
    );
    for k in path_and_ancestors(&norm) {
        if let Some(m) = knowledge_at(conn, Scope::Path, &k)? {
            return Ok(Some(m));
        }
    }
    Ok(None)
}

pub fn tag(m: &Memory) -> Option<String> {
    let d = m.detail.as_ref()?;
    let label = d
        .label
        .clone()
        .or_else(|| d.app.clone())
        .unwrap_or_else(|| d.category.clone());
    if label.trim().is_empty() {
        return None;
    }
    Some(match d.verdict {
        Some(v) => format!("{label}, {}", v.as_str()),
        None => label,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Answer {
    Keep,
    Disposable,
    MoveElsewhere,
    NeverTouch,
}

pub fn answer(
    conn: &Connection,
    env: &PolicyEnv,
    path: &str,
    answer: Answer,
    now: i64,
) -> anyhow::Result<Memory> {
    let p = normalize_path(&env.expand(path));
    let prev = location_at(conn, &env.profile, &p)?
        .and_then(|m| m.detail)
        .unwrap_or_default();
    let size = index::node_by_path(conn, &p)?
        .map(|n| n.size)
        .or(prev.size_at);
    let (label, verdict, actions, text) = match answer {
        Answer::Keep => (
            "yours",
            Verdict::Keep,
            vec![Action::Archive, Action::Compress],
            "You keep this folder; it may still be moved or compressed.",
        ),
        Answer::Disposable => (
            "disposable",
            Verdict::Safe,
            Action::ALL.to_vec(),
            "You said this folder can go.",
        ),
        Answer::MoveElsewhere => (
            "move elsewhere",
            Verdict::Keep,
            vec![Action::Archive],
            "You want this folder moved to another drive, not deleted.",
        ),
        Answer::NeverTouch => (
            "never touch",
            Verdict::Keep,
            Vec::new(),
            "Never touch this folder.",
        ),
    };
    let category = if prev.category.is_empty() || prev.category == "unknown" {
        "user".to_string()
    } else {
        prev.category.clone()
    };
    let detail = Detail {
        category,
        label: Some(label.into()),
        verdict: Some(verdict),
        actions,
        size_at: size,
        assessed_ms: Some(now),
        by: Some("user".into()),
        ..prev
    };
    let (m, _) = knowledge::upsert_with(
        conn,
        Knowledge {
            scope: Scope::Path,
            key: p.clone(),
            kind: Kind::Verdict,
            text: text.into(),
            source: "user".into(),
            detail,
        },
        now,
        &env.profile,
    )?;
    if answer == Answer::NeverTouch {
        remember_with(
            conn,
            NewMemory::new(
                Scope::Path,
                &p,
                Kind::Protect,
                "Never touch: your answer to What is this?",
                "user",
            ),
            now,
            &env.profile,
        )?;
    }
    Ok(m)
}

pub fn keep_block(
    conn: &Connection,
    env: &PolicyEnv,
    path: &str,
    action: Action,
) -> anyhow::Result<Option<String>> {
    let p = normalize_path(path);
    for m in locations(conn)? {
        let Some(d) = &m.detail else {
            continue;
        };
        let blocked = match action {
            Action::Delete | Action::Dedupe => true,
            Action::Archive | Action::Compress => !d.actions.contains(&action),
        };
        if d.verdict != Some(Verdict::Keep) || !blocked {
            continue;
        }
        let loc = expanded(env, &m.key);
        if loc.eq_ignore_ascii_case(&p) || is_ancestor_path(&loc, &p) {
            return Ok(Some(format!(
                "{p} is in {loc}, a kept location: {}",
                m.text
            )));
        }
        if is_ancestor_path(&p, &loc) {
            return Ok(Some(format!(
                "{p} contains {loc}, a kept location: {}",
                m.text
            )));
        }
    }
    Ok(None)
}

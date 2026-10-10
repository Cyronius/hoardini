pub mod areas;
pub mod cards;
pub mod trigger;
pub mod units;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::catalog::{Category, Entry};
use crate::cloud::SyncRoot;
use crate::compress::{self, CompressCandidate};
use crate::destinations::{self, Destination};
use crate::index::{self, DirLister, is_ancestor_path, normalize_path};
use crate::memory::{Memory, location};
use crate::paths::humanize_bytes;
use crate::policy::{Area as PolicyArea, PolicyEnv, area_of};
use crate::proposals::{self, NewItem, NewProposal, NewSystemCleanup, ProposalKind, ReviewCard};
use crate::store::Db;
use crate::syscleanup::{self, OpPart};

pub use areas::Area;
pub use cards::{CARDS, Card, Pick, card};
use cards::{Cx, Seen};
pub use trigger::{Builds, Opened};
use units::{Unit, Why};

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;
pub const RERUN_AFTER_MS: i64 = 6 * HOUR_MS;
pub const DECLINED_FOR_MS: i64 = 30 * DAY_MS;
pub const CARD_MIN_BYTES: u64 = 500 << 20;
pub const MAX_ITEMS: usize = 50;
// Review cards live outside the chat; the prompt names them separately (AGENT-PROMPT-CONTEXT).
pub const CONVERSATION: &str = "review";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Volume(i64),
    Folder(String),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CardRef {
    pub card: String,
    pub title: String,
    pub proposal_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Review {
    pub id: String,
    pub volume_id: i64,
    pub volume: String,
    pub scope_path: Option<String>,
    pub scan_id: Option<i64>,
    pub created_ms: i64,
    pub cards: Vec<CardRef>,
    pub areas: Vec<Area>,
}

impl Review {
    pub fn root(&self) -> &str {
        self.scope_path.as_deref().unwrap_or(&self.volume)
    }
}

fn same_or_under(a: &str, p: &str) -> bool {
    a.eq_ignore_ascii_case(p) || is_ancestor_path(a, p)
}

fn overlaps(a: &str, b: &str) -> bool {
    same_or_under(a, b) || is_ancestor_path(b, a)
}

fn volume_root(conn: &Connection, id: i64) -> anyhow::Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT root FROM volume WHERE id = ?1 AND root_node_id IS NOT NULL",
            [id],
            |r| r.get(0),
        )
        .optional()?)
}

pub fn volume_of(conn: &Connection, path: &str) -> anyhow::Result<Option<(i64, String)>> {
    let mut st = conn.prepare("SELECT id, root FROM volume WHERE root_node_id IS NOT NULL")?;
    let rows = st
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(_, root)| same_or_under(root, path))
        .max_by_key(|(_, root)| root.len()))
}

// A folder that is the volume root is the volume.
fn target(
    conn: &Connection,
    env: &PolicyEnv,
    scope: &Scope,
) -> anyhow::Result<(i64, String, Option<String>)> {
    match scope {
        Scope::Volume(id) => {
            let root = volume_root(conn, *id)?
                .ok_or_else(|| anyhow::anyhow!("volume {id} has no index"))?;
            Ok((*id, root, None))
        }
        Scope::Folder(raw) => {
            let p = normalize_path(&env.expand(raw));
            let (id, root) =
                volume_of(conn, &p)?.ok_or_else(|| anyhow::anyhow!("{p} is not in the index"))?;
            if index::node_id_by_path(conn, &p)?.is_none() {
                anyhow::bail!("{p} is not in the index");
            }
            let folder = (!p.eq_ignore_ascii_case(&root)).then_some(p);
            Ok((id, root, folder))
        }
    }
}

fn replaceable(
    conn: &Connection,
    volume_id: i64,
    folder: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let mut st = conn.prepare(
        "SELECT p.id FROM proposal p JOIN review r ON r.id = p.review_id \
         WHERE p.status = 'pending' AND p.touched = 0 AND r.volume_id = ?1 \
         AND ((?2 IS NULL AND r.scope_path IS NULL) OR r.scope_path = ?2 COLLATE NOCASE)",
    )?;
    let ids = st
        .query_map(params![volume_id, folder], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(ids)
}

struct Taken {
    paths: Vec<String>,
    // System cleanup items are operations, so they stay out by operation rather than by the folders they name.
    ops: Vec<String>,
}

fn taken(conn: &Connection, replacing: &[String], now: i64) -> anyhow::Result<Taken> {
    let mut st = conn.prepare(
        "SELECT p.id, p.kind, i.path, p.params_json FROM proposal_item i JOIN proposal p ON p.id = i.proposal_id \
         WHERE p.status = 'pending' OR (p.status = 'declined' AND p.decided_ms >= ?1)",
    )?;
    let rows = st
        .query_map([now - DECLINED_FOR_MS], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Taken {
        paths: Vec::new(),
        ops: Vec::new(),
    };
    for (id, kind, path, params) in rows {
        if replacing.contains(&id) {
            continue;
        }
        if kind != ProposalKind::SystemCleanup.as_str() {
            out.paths.push(path);
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(&params).unwrap_or_default();
        for name in v["ops"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| o["op"].as_str())
        {
            if !out.ops.iter().any(|o| o == name) {
                out.ops.push(name.to_string());
            }
        }
    }
    Ok(out)
}

// Windows-run leftovers and machine-wide folders need elevation; Hoardini deletes as the user.
fn needs_admin(env: &PolicyEnv, unit: &Unit, entry: Option<&Entry>) -> bool {
    matches!(area_of(env, &unit.path), Some(PolicyArea::System { .. }))
        || same_or_under(&env.program_data, &unit.path)
        || (entry.is_some_and(|e| e.category == Category::WindowsLeftover)
            && !same_or_under(&env.profile, &unit.path))
}

// An archive onto the same drive frees nothing.
fn destination_off(conn: &Connection, volume: &str) -> anyhow::Result<Option<Destination>> {
    for d in destinations::list(conn)? {
        let here = destinations::local_root(&d)
            .ok()
            .flatten()
            .map(|p| normalize_path(&p.to_string_lossy()))
            .is_some_and(|p| {
                volume_of(conn, &p)
                    .ok()
                    .flatten()
                    .is_some_and(|(_, r)| r.eq_ignore_ascii_case(volume))
            });
        if !here {
            return Ok(Some(d));
        }
    }
    Ok(None)
}

// Removing a file from a sync folder here removes the cloud copy too, so only Online-only may hold one.
fn touches_sync(roots: &[SyncRoot], path: &str) -> bool {
    roots
        .iter()
        .any(|r| overlaps(&normalize_path(&r.path), path))
}

// The part of a unit its card holds: the picks inside it, or all of it when a pick holds it.
fn claimed_by(u: &Unit, picks: &[Pick]) -> u64 {
    let n: u64 = picks
        .iter()
        .map(|p| {
            if same_or_under(&u.path, &p.path) {
                p.size
            } else if is_ancestor_path(&p.path, &u.path) {
                u.size
            } else {
                0
            }
        })
        .sum();
    n.min(u.size)
}

fn system_volume(env: &PolicyEnv, volume: &str) -> bool {
    env.system_root
        .get(..2)
        .is_some_and(|d| normalize_path(d).eq_ignore_ascii_case(volume))
}

// A folder holding something a delete card takes would be compressed and deleted at once.
fn compressible(units: &[Unit], c: &CompressCandidate) -> bool {
    c.proposable()
        && !units.iter().any(|u| {
            matches!(u.why, Why::Catalog | Why::Disposable) && is_ancestor_path(&c.path, &u.path)
        })
}

fn with_cleaned(conn: &Connection, env: &PolicyEnv, p: &mut Pick) -> anyhow::Result<()> {
    if let Some(c) = location::cleaned_for_with(conn, &env.profile, &p.path)? {
        p.reason = format!("{} {}.", p.reason, location::capitalized(&c.reason()));
    }
    Ok(())
}

pub fn build(
    db: &Db,
    env: &PolicyEnv,
    lister: &dyn DirLister,
    scope: Scope,
    now: i64,
) -> anyhow::Result<Review> {
    let conn = db.conn()?;
    let (volume_id, volume, folder) = target(&conn, env, &scope)?;
    if folder.is_none() {
        location::refresh_volume(&conn, env, &volume, now)?;
    }
    let root = folder.clone().unwrap_or_else(|| volume.clone());
    let scan_id = index::index_status(&conn, Some(&volume))?.and_then(|s| s.last_scan_id);
    let units = units::resolve(&conn, env, lister, &root, now)?;
    let replacing = replaceable(&conn, volume_id, folder.as_deref())?;
    let taken = taken(&conn, &replacing, now)?;
    let is_taken = |p: &str| taken.paths.iter().any(|t| overlaps(t, p));
    let cx = Cx {
        conn: &conn,
        env,
        lister,
        now,
        destination: destination_off(&conn, &volume)?,
        locations: location::locations(&conn)?
            .into_iter()
            .map(|m| (location::expanded(env, &m.key), m))
            .collect::<Vec<(String, Memory)>>(),
        sync_roots: env.cloud().roots(),
        compress: compress::candidates(&conn, env, &root, now)?
            .into_iter()
            .filter(|c| compressible(&units, c))
            .collect(),
        leftovers: syscleanup::leftover_targets(&env.system_root, &env.program_data),
    };
    let catalog = env.catalog();
    let mut chosen: Vec<Vec<Pick>> = CARDS.iter().map(|_| Vec::new()).collect();
    let mut left = Vec::new();
    for u in &units {
        if is_taken(&u.path) {
            continue;
        }
        let entry = u.catalog_id.as_deref().and_then(|id| catalog.get(id));
        let seen = Seen::new(u, entry, needs_admin(env, u, entry));
        let synced = cx.synced(&u.path);
        let mut claimed = 0;
        for (i, c) in CARDS.iter().enumerate() {
            let online = c.kind == ProposalKind::OnlineOnly;
            if synced && !online {
                continue;
            }
            let picks = (c.take)(&cx, &seen)?;
            if picks.is_empty() {
                continue;
            }
            let (gone, picks): (Vec<Pick>, Vec<Pick>) =
                picks.into_iter().partition(|p| is_taken(&p.path));
            let mut keep = Vec::new();
            for mut p in picks {
                if (!online && touches_sync(&cx.sync_roots, &p.path))
                    || location::was_kept_with(&conn, &env.profile, &p.path, p.size)?
                {
                    continue;
                }
                with_cleaned(&conn, env, &mut p)?;
                keep.push(p);
            }
            if keep.is_empty() && gone.is_empty() {
                continue;
            }
            claimed = claimed_by(u, &gone) + claimed_by(u, &keep);
            for p in keep {
                if !chosen[i]
                    .iter()
                    .any(|q| q.path.eq_ignore_ascii_case(&p.path))
                {
                    chosen[i].push(p);
                }
            }
            break;
        }
        left.push(areas::Left {
            unit: u,
            entry,
            admin: seen.admin && u.why != Why::Kept,
            not_build: entry.is_some_and(|e| !cards::is_build_output(&cx, u, e)),
            claimed: claimed.min(u.size),
        });
    }
    let areas = areas::collect(&cx, &left, &units, &root, &volume)?;

    let id = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO review(id, volume_id, scope_path, scan_id, created_ms, areas_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, volume_id, folder, scan_id, now, serde_json::to_string(&areas)?],
    )?;
    let min = if folder.is_some() { 1 } else { CARD_MIN_BYTES };
    for (c, mut picks) in CARDS.iter().zip(chosen) {
        picks.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
        let total: u64 = picks.iter().map(|p| p.size).sum();
        let card = ReviewCard {
            review_id: id.clone(),
            card: c.id.into(),
            ticked: c.ticked,
        };
        if c.kind == ProposalKind::SystemCleanup {
            let parts = (total >= min).then(|| {
                picks
                    .into_iter()
                    .map(|p| OpPart {
                        path: p.path,
                        size: p.size,
                    })
                    .collect()
            });
            let system =
                (folder.is_none() && system_volume(env, &volume)).then_some(volume.as_str());
            if let Err(e) = windows_card(db, env, c, parts, system, &taken.ops, &card) {
                tracing::warn!(card = c.id, error = %e, "review card not created");
            }
            continue;
        }
        if picks.is_empty() || total < min {
            continue;
        }
        let rest = picks.split_off(picks.len().min(MAX_ITEMS));
        let mut summary = c.summary.to_string();
        if !rest.is_empty() {
            let bytes: u64 = rest.iter().map(|p| p.size).sum();
            summary.push_str(&format!(
                " {} smaller items ({}) are not listed.",
                rest.len(),
                humanize_bytes(bytes)
            ));
        }
        let req = NewProposal {
            conversation_id: Some(CONVERSATION.into()),
            tool_call_id: None,
            kind: c.kind,
            destination_id: (c.kind == ProposalKind::Archive)
                .then(|| cx.destination.as_ref().map(|d| d.id.clone()))
                .flatten(),
            format: (c.kind == ProposalKind::Compress).then(|| "lzx".to_string()),
            dedupe: None,
            summary,
            items: picks
                .into_iter()
                .map(|p| NewItem {
                    path: p.path,
                    verdict: p.verdict,
                    reason: Some(p.reason),
                })
                .collect(),
        };
        match proposals::create_review_card(db, env, req, &card) {
            Ok(Ok(_)) => {}
            Ok(Err(dropped)) => {
                tracing::info!(
                    card = c.id,
                    refused = dropped.len(),
                    "review card refused by policy"
                )
            }
            Err(e) => tracing::warn!(card = c.id, error = %e, "review card not created"),
        }
    }
    for p in &replacing {
        conn.execute(
            "UPDATE proposal SET status = 'superseded' WHERE id = ?1 AND status = 'pending' AND touched = 0",
            [p],
        )?;
    }
    get(&conn, &id)?.ok_or_else(|| anyhow::anyhow!("review vanished"))
}

// The leftovers the walk found, plus what Windows' own tools could free on the system drive.
fn windows_card(
    db: &Db,
    env: &PolicyEnv,
    c: &Card,
    leftovers: Option<Vec<OpPart>>,
    system_volume: Option<&str>,
    taken_ops: &[String],
    card: &ReviewCard,
) -> anyhow::Result<()> {
    let mut ops: Vec<syscleanup::CleanupCandidate> = Vec::new();
    ops.extend(leftovers.and_then(syscleanup::leftovers_candidate));
    if let Some(volume) = system_volume {
        let facts = env.facts();
        if facts.system_drive.eq_ignore_ascii_case(volume) {
            ops.extend(syscleanup::candidates(&db.conn()?, &facts)?);
        }
    }
    ops.retain(|o| !taken_ops.iter().any(|t| t == o.op.name()));
    if ops.is_empty() {
        return Ok(());
    }
    proposals::create_system_cleanup_card(
        db,
        NewSystemCleanup {
            conversation_id: Some(CONVERSATION.into()),
            tool_call_id: None,
            summary: c.summary.into(),
            ops,
            dropped: Vec::new(),
        },
        Some(card),
    )?;
    Ok(())
}

// Estimates the scope's biggest compress candidates; true when that made a folder proposable that the review does not hold.
pub fn compress_estimates(
    db: &Db,
    env: &PolicyEnv,
    review: &Review,
    now: i64,
) -> anyhow::Result<bool> {
    let conn = db.conn()?;
    let before = compress::candidates(&conn, env, review.root(), now)?;
    let held: Vec<String> = proposal_ids(&conn, &review.id)?
        .into_iter()
        .filter_map(|id| proposals::get(db, &id).ok().flatten())
        .filter(|p| p.kind == ProposalKind::Compress)
        .flat_map(|p| p.items.into_iter().map(|i| i.path))
        .collect();
    if compress::estimate_top(db, &before, compress::ESTIMATE_TOP)? == 0 {
        return Ok(false);
    }
    Ok(compress::candidates(&conn, env, review.root(), now)?
        .iter()
        .any(|c| c.proposable() && !held.iter().any(|h| h.eq_ignore_ascii_case(&c.path))))
}

// Built again only while nobody has touched it and nothing newer replaced it.
pub fn rebuild_if_current(
    db: &Db,
    env: &PolicyEnv,
    lister: &dyn DirLister,
    review_id: &str,
    now: i64,
) -> anyhow::Result<Option<Review>> {
    let conn = db.conn()?;
    let Some(r) = get(&conn, review_id)? else {
        return Ok(None);
    };
    let latest = latest(&conn, r.volume_id, r.scope_path.as_deref())?;
    if latest.is_none_or(|l| l.id != r.id) || !untouched(&conn, &r.id)? {
        return Ok(None);
    }
    let scope = match r.scope_path {
        Some(p) => Scope::Folder(p),
        None => Scope::Volume(r.volume_id),
    };
    build(db, env, lister, scope, now).map(Some)
}

pub fn get(conn: &Connection, id: &str) -> anyhow::Result<Option<Review>> {
    type Head = (i64, String, Option<String>, Option<i64>, i64, String);
    let head: Option<Head> = conn
        .query_row(
            "SELECT r.volume_id, COALESCE(v.root, ''), r.scope_path, r.scan_id, r.created_ms, r.areas_json \
             FROM review r LEFT JOIN volume v ON v.id = r.volume_id WHERE r.id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((volume_id, volume, scope_path, scan_id, created_ms, areas_json)) = head else {
        return Ok(None);
    };
    let mut st = conn.prepare(
        "SELECT id, card FROM proposal WHERE review_id = ?1 AND parent_id IS NULL \
         ORDER BY total_alloc DESC, created_ms, id",
    )?;
    let cards = st
        .query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|(proposal_id, c)| {
            let c = c.unwrap_or_default();
            CardRef {
                title: card(&c).map_or_else(|| c.clone(), |x| x.title.to_string()),
                card: c,
                proposal_id,
            }
        })
        .collect();
    Ok(Some(Review {
        id: id.to_string(),
        volume_id,
        volume,
        scope_path,
        scan_id,
        created_ms,
        cards,
        areas: serde_json::from_str(&areas_json).unwrap_or_default(),
    }))
}

pub fn latest(
    conn: &Connection,
    volume_id: i64,
    folder: Option<&str>,
) -> anyhow::Result<Option<Review>> {
    let id: Option<String> = conn
        .query_row(
            "SELECT id FROM review WHERE volume_id = ?1 \
             AND ((?2 IS NULL AND scope_path IS NULL) OR scope_path = ?2 COLLATE NOCASE) \
             ORDER BY created_ms DESC, id DESC LIMIT 1",
            params![volume_id, folder],
            |r| r.get(0),
        )
        .optional()?;
    match id {
        Some(id) => get(conn, &id),
        None => Ok(None),
    }
}

fn last_scan(conn: &Connection, volume: &str) -> anyhow::Result<Option<i64>> {
    Ok(index::index_status(conn, Some(volume))?.and_then(|s| s.last_scan_id))
}

pub fn needs_rerun(conn: &Connection, r: &Review, now: i64) -> anyhow::Result<bool> {
    Ok(now - r.created_ms > RERUN_AFTER_MS || last_scan(conn, &r.volume)? != r.scan_id)
}

pub fn untouched(conn: &Connection, review_id: &str) -> anyhow::Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM proposal WHERE review_id = ?1 \
         AND (touched = 1 OR status NOT IN ('pending', 'superseded'))",
        [review_id],
        |r| r.get(0),
    )?;
    Ok(n == 0)
}

pub fn open(
    db: &Db,
    env: &PolicyEnv,
    lister: &dyn DirLister,
    volume_id: i64,
    now: i64,
) -> anyhow::Result<Review> {
    let conn = db.conn()?;
    match latest(&conn, volume_id, None)? {
        Some(r) if !needs_rerun(&conn, &r, now)? => Ok(r),
        _ => build(db, env, lister, Scope::Volume(volume_id), now),
    }
}

pub fn rebuild_if_untouched(
    db: &Db,
    env: &PolicyEnv,
    lister: &dyn DirLister,
    volume_id: i64,
    now: i64,
) -> anyhow::Result<Option<Review>> {
    let conn = db.conn()?;
    let Some(r) = latest(&conn, volume_id, None)? else {
        return Ok(None);
    };
    if last_scan(&conn, &r.volume)? == r.scan_id || !untouched(&conn, &r.id)? {
        return Ok(None);
    }
    build(db, env, lister, Scope::Volume(volume_id), now).map(Some)
}

pub fn ready_bytes(conn: &Connection, review_id: &str) -> anyhow::Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(i.alloc), 0) FROM proposal_item i JOIN proposal p ON p.id = i.proposal_id \
         WHERE p.review_id = ?1 AND p.status = 'pending' AND i.status = 'pending' AND i.selected = 1",
        [review_id],
        |r| r.get(0),
    )?)
}

pub fn proposal_ids(conn: &Connection, review_id: &str) -> anyhow::Result<Vec<String>> {
    let mut st =
        conn.prepare("SELECT id FROM proposal WHERE review_id = ?1 ORDER BY created_ms, id")?;
    let ids = st
        .query_map([review_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(ids)
}

// A "What is this?" answer shows on the stored areas at once, without a new review.
pub fn apply_answer(conn: &Connection, path: &str, m: &Memory) -> anyhow::Result<usize> {
    let Some(d) = &m.detail else {
        return Ok(0);
    };
    let needle = serde_json::to_string(path)?;
    let mut st = conn.prepare(
        "SELECT id, areas_json FROM review WHERE instr(lower(areas_json), lower(?1)) > 0",
    )?;
    let rows = st
        .query_map([&needle], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut changed = 0;
    for (id, json) in rows {
        let mut areas: Vec<Area> = serde_json::from_str(&json).unwrap_or_default();
        for a in areas
            .iter_mut()
            .filter(|a| a.path.eq_ignore_ascii_case(path))
        {
            a.label = d.label.clone().unwrap_or_else(|| d.category.clone());
            a.category = d.category.clone();
            a.verdict = d.verdict;
            a.actions = d.actions.clone();
            a.why_not = m.text.clone();
            a.unknown = false;
            changed += 1;
        }
        conn.execute(
            "UPDATE review SET areas_json = ?2 WHERE id = ?1",
            params![id, serde_json::to_string(&areas)?],
        )?;
    }
    Ok(changed)
}

pub mod areas;
pub mod cards;
pub mod trigger;
pub mod units;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::catalog::{Category, Entry, UnitKind};
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
    pub carried: bool,
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

// A volume review replaces the drive's folder reviews too, so their untouched cards never hide items from it.
fn replaceable(
    conn: &Connection,
    volume_id: i64,
    folder: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let mut st = conn.prepare(
        "SELECT p.id FROM proposal p JOIN review r ON r.id = p.review_id \
         WHERE p.status = 'pending' AND p.touched = 0 AND r.volume_id = ?1 \
         AND (?2 IS NULL OR r.scope_path = ?2 COLLATE NOCASE)",
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
         WHERE p.status IN ('pending', 'approved', 'running') \
         OR (p.status = 'declined' AND p.decided_ms >= ?1)",
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

struct Assigned<'a> {
    unit: &'a Unit,
    entry: Option<&'a Entry>,
    admin: bool,
    not_build: bool,
    card: Option<usize>,
    gone: Vec<Pick>,
}

pub fn build(
    db: &Db,
    env: &PolicyEnv,
    lister: &dyn DirLister,
    scope: Scope,
    now: i64,
) -> anyhow::Result<Review> {
    let mut conn = db.conn()?;
    let (volume_id, volume, folder) = target(&conn, env, &scope)?;
    drop_orphans(&conn)?;
    if folder.is_none() {
        location::refresh_volume(&conn, env, &volume, now)?;
    }
    let root = folder.clone().unwrap_or_else(|| volume.clone());
    let scan_id = index::index_status(&conn, Some(&volume))?.and_then(|s| s.last_scan_id);
    let units = units::resolve(&conn, env, lister, &root, now)?;
    let replacing = replaceable(&conn, volume_id, folder.as_deref())?;
    let taken = taken(&conn, &replacing, now)?;
    let is_taken = |p: &str| taken.paths.iter().any(|t| overlaps(t, p));
    // A contents unit's parts are split one by one below, so one taken part leaves the others in.
    let unit_taken = |u: &Unit| match u.kind {
        UnitKind::Contents => taken.paths.iter().any(|t| same_or_under(t, &u.path)),
        _ => is_taken(&u.path),
    };
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
    let mut assigned = Vec::new();
    for u in &units {
        if unit_taken(u) {
            continue;
        }
        let entry = u.catalog_id.as_deref().and_then(|id| catalog.get(id));
        let seen = Seen::new(u, entry, needs_admin(env, u, entry));
        let synced = cx.synced(&u.path);
        let mut a = Assigned {
            unit: u,
            entry,
            admin: seen.admin && u.why != Why::Kept,
            not_build: entry.is_some_and(|e| !cards::is_build_output(&cx, u, e)),
            card: None,
            gone: Vec::new(),
        };
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
            a.card = Some(i);
            a.gone = gone;
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
        assigned.push(a);
    }
    // A folder holding or inside another card's item would be compressed and then deleted or moved.
    if let Some(ci) = CARDS.iter().position(|c| c.kind == ProposalKind::Compress) {
        let others: Vec<String> = chosen
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != ci)
            .flat_map(|(_, ps)| ps.iter().map(|p| p.path.clone()))
            .collect();
        chosen[ci].retain(|p| !others.iter().any(|o| overlaps(o, &p.path)));
    }
    let left: Vec<areas::Left> = assigned
        .iter()
        .map(|a| areas::Left {
            unit: a.unit,
            entry: a.entry,
            admin: a.admin,
            not_build: a.not_build,
            claimed: a
                .card
                .map_or(0, |i| {
                    claimed_by(a.unit, &a.gone) + claimed_by(a.unit, &chosen[i])
                })
                .min(a.unit.size),
        })
        .collect();
    let areas = areas::collect(&cx, &left, &units, &root, &volume)?;

    let id = uuid::Uuid::now_v7().to_string();
    let mut created = Vec::new();
    let made = make_cards(
        db,
        env,
        &cx,
        chosen,
        &Target {
            review_id: &id,
            volume: &volume,
            folder: folder.as_deref(),
        },
        &taken.ops,
        &mut created,
    );
    drop(cx);
    let row = NewRow {
        id: &id,
        volume_id,
        folder: folder.as_deref(),
        scan_id,
        now,
        areas_json: serde_json::to_string(&areas)?,
    };
    if let Err(e) = made.and_then(|()| publish(&mut conn, &row, &replacing, &created, now)) {
        discard(&conn, &created);
        return Err(e);
    }
    get(&conn, &id)?.ok_or_else(|| anyhow::anyhow!("review vanished"))
}

struct Target<'a> {
    review_id: &'a str,
    volume: &'a str,
    folder: Option<&'a str>,
}

fn make_cards(
    db: &Db,
    env: &PolicyEnv,
    cx: &Cx,
    chosen: Vec<Vec<Pick>>,
    t: &Target,
    taken_ops: &[String],
    created: &mut Vec<String>,
) -> anyhow::Result<()> {
    let min = if t.folder.is_some() {
        1
    } else {
        CARD_MIN_BYTES
    };
    for (c, mut picks) in CARDS.iter().zip(chosen) {
        picks.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
        let total: u64 = picks.iter().map(|p| p.size).sum();
        let card = ReviewCard {
            review_id: t.review_id.to_string(),
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
            let system = (t.folder.is_none() && system_volume(env, t.volume)).then_some(t.volume);
            match windows_card(db, env, c, parts, system, taken_ops, &card) {
                Ok(Some(p)) => created.push(p),
                Ok(None) => {}
                Err(e) => tracing::warn!(card = c.id, error = %e, "review card not created"),
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
            Ok(Ok(p)) => created.push(p.id),
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
    Ok(())
}

struct NewRow<'a> {
    id: &'a str,
    volume_id: i64,
    folder: Option<&'a str>,
    scan_id: Option<i64>,
    now: i64,
    areas_json: String,
}

// The review becomes visible here, at once with its cards and the supersede, so nothing reads it half built.
fn publish(
    conn: &mut Connection,
    row: &NewRow,
    replacing: &[String],
    created: &[String],
    now: i64,
) -> anyhow::Result<()> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for p in replacing {
        tx.execute(
            "UPDATE proposal SET status = 'superseded' WHERE id = ?1 AND status = 'pending' AND touched = 0",
            [p],
        )?;
    }
    // A card ticked, decided or made while this one was built keeps its items; the new cards give them up.
    let live = taken(&tx, created, now)?;
    for id in created {
        strip(&tx, id, &live)?;
    }
    tx.execute(
        "INSERT INTO review(id, volume_id, scope_path, scan_id, created_ms, areas_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![row.id, row.volume_id, row.folder, row.scan_id, row.now, row.areas_json],
    )?;
    tx.commit()?;
    Ok(())
}

fn strip(conn: &Connection, id: &str, live: &Taken) -> anyhow::Result<()> {
    let (kind, params): (String, String) = conn.query_row(
        "SELECT kind, params_json FROM proposal WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if kind == ProposalKind::SystemCleanup.as_str() {
        let v: serde_json::Value = serde_json::from_str(&params).unwrap_or_default();
        let clash = v["ops"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| o["op"].as_str())
            .any(|op| live.ops.iter().any(|t| t == op));
        if clash {
            discard(conn, &[id.to_string()]);
        }
        return Ok(());
    }
    let mut st = conn.prepare("SELECT id, path FROM proposal_item WHERE proposal_id = ?1")?;
    let items = st
        .query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let gone: Vec<i64> = items
        .iter()
        .filter(|(_, p)| live.paths.iter().any(|t| overlaps(t, p)))
        .map(|(i, _)| *i)
        .collect();
    if gone.is_empty() {
        return Ok(());
    }
    if gone.len() == items.len() {
        discard(conn, &[id.to_string()]);
        return Ok(());
    }
    for i in gone {
        conn.execute("DELETE FROM proposal_item WHERE id = ?1", [i])?;
    }
    conn.execute(
        "UPDATE proposal SET total_alloc = (SELECT COALESCE(SUM(alloc), 0) FROM proposal_item \
         WHERE proposal_id = ?1 AND status != 'blocked') WHERE id = ?1",
        [id],
    )?;
    Ok(())
}

// Cards of a build that never published belong to no review; they are dropped, not left holding their items.
fn discard(conn: &Connection, ids: &[String]) {
    for id in ids {
        let gone = conn.execute(
            "DELETE FROM proposal WHERE id = ?1 AND status = 'pending' AND touched = 0",
            [id],
        );
        if gone.is_ok_and(|n| n > 0) {
            let _ = conn.execute("DELETE FROM proposal_item WHERE proposal_id = ?1", [id]);
        }
    }
}

// A build the process never finished left cards with no review row; they would hold their items for good.
fn drop_orphans(conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE proposal SET status = 'superseded' WHERE review_id IS NOT NULL AND status = 'pending' \
         AND touched = 0 AND NOT EXISTS (SELECT 1 FROM review r WHERE r.id = proposal.review_id)",
        [],
    )?;
    Ok(())
}

fn windows_card(
    db: &Db,
    env: &PolicyEnv,
    c: &Card,
    leftovers: Option<Vec<OpPart>>,
    system_volume: Option<&str>,
    taken_ops: &[String],
    card: &ReviewCard,
) -> anyhow::Result<Option<String>> {
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
        return Ok(None);
    }
    let p = proposals::create_system_cleanup_card(
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
    Ok(Some(p.id))
}

pub fn compress_estimates(
    db: &Db,
    env: &PolicyEnv,
    review: &Review,
    now: i64,
) -> anyhow::Result<bool> {
    let conn = db.conn()?;
    let before = compress::candidates(&conn, env, review.root(), now)?;
    let was: Vec<String> = before
        .iter()
        .filter(|c| c.proposable())
        .map(|c| c.path.clone())
        .collect();
    let held: Vec<String> = proposal_ids(&conn, &review.id)?
        .into_iter()
        .filter_map(|id| proposals::get(db, &id).ok().flatten())
        .filter(|p| p.kind == ProposalKind::Compress)
        .flat_map(|p| p.items.into_iter().map(|i| i.path))
        .collect();
    if compress::estimate_top(db, &before, compress::ESTIMATE_TOP)? == 0 {
        return Ok(false);
    }
    // Only a folder this pass made proposable can change the card, and only one the review would not leave out.
    let replacing = replaceable(&conn, review.volume_id, review.scope_path.as_deref())?;
    let taken = taken(&conn, &replacing, now)?;
    let roots = env.cloud().roots();
    let listed = |list: &[String], p: &str| list.iter().any(|x| x.eq_ignore_ascii_case(p));
    Ok(compress::candidates(&conn, env, review.root(), now)?
        .iter()
        .any(|c| {
            c.proposable()
                && !listed(&was, &c.path)
                && !listed(&held, &c.path)
                && !taken.paths.iter().any(|t| overlaps(t, &c.path))
                && !touches_sync(&roots, &c.path)
        }))
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
    // Touched cards of earlier reviews (for a volume review, folder reviews' too) stay live, so they are listed here.
    let mut st = conn.prepare(
        "SELECT p.id, p.card, p.review_id != ?1 FROM proposal p WHERE p.review_id = ?1 AND p.parent_id IS NULL \
         UNION ALL \
         SELECT p.id, p.card, 1 FROM proposal p JOIN review r ON r.id = p.review_id \
         WHERE r.volume_id = ?2 AND r.id != ?1 AND r.created_ms <= ?3 \
         AND (?4 IS NULL OR r.scope_path = ?4 COLLATE NOCASE) \
         AND p.touched = 1 AND p.status IN ('pending', 'approved', 'running') \
         AND (p.parent_id IS NULL OR NOT EXISTS (SELECT 1 FROM proposal q WHERE q.id = p.parent_id \
              AND q.status IN ('pending', 'approved', 'running'))) \
         ORDER BY 1",
    )?;
    let mut rows = st
        .query_map(params![id, volume_id, created_ms, scope_path], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let size = |pid: &str| -> (i64, i64) {
        conn.query_row(
            "SELECT total_alloc, created_ms FROM proposal WHERE id = ?1",
            [pid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, 0))
    };
    rows.sort_by_cached_key(|(pid, _, _)| {
        let (alloc, created) = size(pid);
        (std::cmp::Reverse(alloc), created, pid.clone())
    });
    let cards = rows
        .into_iter()
        .map(|(proposal_id, c, carried)| {
            let c = c.unwrap_or_default();
            CardRef {
                title: card(&c).map_or_else(|| c.clone(), |x| x.title.to_string()),
                card: c,
                proposal_id,
                carried,
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

// A build that ran while this one was queued may have left a fresh review.
pub fn build_if_stale(
    db: &Db,
    env: &PolicyEnv,
    lister: &dyn DirLister,
    volume_id: i64,
    now: i64,
) -> anyhow::Result<Option<Review>> {
    let conn = db.conn()?;
    if let Some(r) = latest(&conn, volume_id, None)?
        && !needs_rerun(&conn, &r, now)?
    {
        return Ok(None);
    }
    build(db, env, lister, Scope::Volume(volume_id), now).map(Some)
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

// A compress item frees what its estimate saves, not its folder's size.
pub fn ready_bytes(conn: &Connection, review_id: &str) -> anyhow::Result<i64> {
    let mut total = 0;
    for id in proposal_ids(conn, review_id)? {
        total += conn.query_row(
            "SELECT COALESCE(SUM(CASE WHEN p.kind = 'compress' THEN COALESCE(e.saved_bytes, i.alloc) \
             ELSE i.alloc END), 0) FROM proposal_item i JOIN proposal p ON p.id = i.proposal_id \
             LEFT JOIN compress_estimate e ON p.kind = 'compress' AND e.path = i.path \
             WHERE p.id = ?1 AND p.status = 'pending' AND i.status = 'pending' AND i.selected = 1",
            [&id],
            |r| r.get::<_, i64>(0),
        )?;
    }
    Ok(total)
}

pub fn proposal_ids(conn: &Connection, review_id: &str) -> anyhow::Result<Vec<String>> {
    let mut st =
        conn.prepare("SELECT id FROM proposal WHERE review_id = ?1 ORDER BY created_ms, id")?;
    let mut ids = st
        .query_map([review_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    let Some(r) = get(conn, review_id)? else {
        return Ok(ids);
    };
    let mut kids =
        conn.prepare("SELECT id FROM proposal WHERE parent_id = ?1 ORDER BY created_ms, id")?;
    for c in r.cards.iter().filter(|c| c.carried) {
        let mut todo = vec![c.proposal_id.clone()];
        while let Some(id) = todo.pop() {
            if ids.contains(&id) {
                continue;
            }
            todo.extend(
                kids.query_map([&id], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?,
            );
            ids.push(id);
        }
    }
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

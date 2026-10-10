use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::destinations;
use crate::index::is_ancestor_path;
use crate::memory;
use crate::policy::{self, Action, FsState, ItemKind, PolicyEnv, Verdict};
use crate::store::{Db, now_ms};
use crate::syscleanup::{CleanupCandidate, OpView, SystemOp};

pub const SMALL_BYTES: i64 = 50 * 1024 * 1024;

// Both bounds, so a card made only of small files still ticks its biggest ones.
pub fn is_small(alloc: i64, total: i64) -> bool {
    alloc < SMALL_BYTES && alloc.saturating_mul(50) < total
}

pub fn default_tick(blocked: bool, verdict: Option<Verdict>, small: bool) -> bool {
    !blocked && matches!(verdict, Some(Verdict::Safe | Verdict::Redownload)) && !small
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalKind {
    Delete,
    Archive,
    Compress,
    Dedupe,
    SystemCleanup,
}

impl ProposalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProposalKind::Delete => "delete",
            ProposalKind::Archive => "archive",
            ProposalKind::Compress => "compress",
            ProposalKind::Dedupe => "dedupe",
            ProposalKind::SystemCleanup => "system_cleanup",
        }
    }

    pub fn parse(s: &str) -> Option<ProposalKind> {
        match s {
            "delete" => Some(ProposalKind::Delete),
            "archive" => Some(ProposalKind::Archive),
            "compress" => Some(ProposalKind::Compress),
            "dedupe" => Some(ProposalKind::Dedupe),
            "system_cleanup" => Some(ProposalKind::SystemCleanup),
            _ => None,
        }
    }

    pub fn action(self) -> Action {
        match self {
            ProposalKind::Delete => Action::Delete,
            ProposalKind::Archive => Action::Archive,
            ProposalKind::Compress => Action::Compress,
            ProposalKind::Dedupe => Action::Dedupe,
            ProposalKind::SystemCleanup => Action::SystemCleanup,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DedupeParams {
    pub group_id: i64,
    pub keep: String,
    pub mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewItem {
    pub path: String,
    pub verdict: Option<Verdict>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewProposal {
    pub conversation_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub kind: ProposalKind,
    pub destination_id: Option<String>,
    pub format: Option<String>,
    pub dedupe: Option<DedupeParams>,
    pub summary: String,
    pub items: Vec<NewItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DestinationRef {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProposalItem {
    pub id: i64,
    pub path: String,
    pub kind: String,
    pub alloc: i64,
    pub verdict: Option<Verdict>,
    pub cap: Option<Verdict>,
    pub reason: Option<String>,
    pub block_reason: Option<String>,
    pub selected: bool,
    pub status: String,
    pub detail: Option<String>,
    pub small: bool,
    pub memory_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<OpView>,
}

// A requested path the gate refused outright (deny list, protect memory, not absolute, nested).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedItem {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub kind: ProposalKind,
    pub status: String,
    pub total_alloc: i64,
    pub created_ms: i64,
    pub job_id: Option<String>,
    pub destination: Option<DestinationRef>,
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe: Option<DedupeParams>,
    pub summary: String,
    pub conversation_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub items: Vec<ProposalItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped: Vec<DroppedItem>,
    pub parent_id: Option<String>,
    pub follow_up_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Params {
    destination_id: Option<String>,
    format: Option<String>,
    dedupe: Option<DedupeParams>,
    summary: String,
    #[serde(default)]
    dropped: Vec<DroppedItem>,
    // One per item, in item order (system_cleanup only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ops: Vec<OpView>,
}

// The executor's view of an item: the public fields plus the fs state it was proposed against.
#[derive(Clone, Debug)]
pub(crate) struct StoredItem {
    pub id: i64,
    pub path: String,
    pub kind: ItemKind,
    pub alloc: i64,
    pub logical: i64,
    pub mtime: Option<i64>,
    pub file_id: Option<[u8; 16]>,
    pub selected: bool,
    pub status: String,
}

impl StoredItem {
    pub fn baseline(&self) -> FsState {
        FsState {
            kind: self.kind,
            logical: if self.kind == ItemKind::File {
                self.logical
            } else {
                0
            },
            mtime: self.mtime,
            file_id: self.file_id,
            attrs: 0,
        }
    }
}

pub fn create(db: &Db, req: NewProposal) -> anyhow::Result<Proposal> {
    create_with(db, &PolicyEnv::system(), req)
}

struct Row {
    path: String,
    ex: policy::Examined,
    verdict: Option<Verdict>,
    reason: Option<String>,
    note: Option<String>,
    tick: Option<bool>,
}

fn nothing_left(dropped: &[DroppedItem]) -> anyhow::Error {
    let why = dropped
        .iter()
        .map(|d| format!("{}: {}", d.path, d.reason))
        .collect::<Vec<_>>()
        .join("; ");
    anyhow::anyhow!("No proposal created; every path was refused by policy. {why}")
}

pub fn create_with(db: &Db, env: &PolicyEnv, req: NewProposal) -> anyhow::Result<Proposal> {
    build(db, env, req, None)?.map_err(|dropped| nothing_left(&dropped))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewSystemCleanup {
    pub conversation_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub summary: String,
    pub ops: Vec<CleanupCandidate>,
    pub dropped: Vec<DroppedItem>,
}

fn op_refusal(c: &CleanupCandidate) -> Option<String> {
    match &c.op {
        SystemOp::CompactVhdx { path }
            if !path.to_ascii_lowercase().ends_with(".vhdx")
                || !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) =>
        {
            Some(format!("{path} is not a virtual disk file"))
        }
        SystemOp::RestorePoints { volume }
            if !(volume.len() == 2
                && volume.ends_with(':')
                && volume.as_bytes()[0].is_ascii_alphabetic()) =>
        {
            Some(format!("{volume} is not a drive"))
        }
        _ => None,
    }
}

// Items are operations: the path is what each one works on, alloc its estimate (0 when unknown).
pub fn create_system_cleanup(db: &Db, req: NewSystemCleanup) -> anyhow::Result<Proposal> {
    let mut dropped = req.dropped;
    let mut ops: Vec<CleanupCandidate> = Vec::new();
    for c in req.ops {
        if ops.iter().any(|o| o.op == c.op) {
            continue;
        }
        match op_refusal(&c) {
            Some(reason) => dropped.push(DroppedItem {
                path: c.target,
                reason,
            }),
            None => ops.push(c),
        }
    }
    if ops.is_empty() {
        let why = dropped
            .iter()
            .map(|d| format!("{}: {}", d.path, d.reason))
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::bail!("No proposal created; there is nothing Windows can clean up here. {why}");
    }
    let id = uuid::Uuid::now_v7().to_string();
    let to_i64 = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
    let total: i64 = ops.iter().map(|c| to_i64(c.estimate.unwrap_or(0))).sum();
    let params = Params {
        summary: req.summary,
        dropped,
        ops: ops.iter().map(CleanupCandidate::view).collect(),
        ..Params::default()
    };
    let mut conn = db.conn()?;
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO proposal(id, conversation_id, tool_call_id, kind, params_json, status, total_alloc, created_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7)",
        params![
            id,
            req.conversation_id,
            req.tool_call_id,
            ProposalKind::SystemCleanup.as_str(),
            serde_json::to_string(&params)?,
            total,
            now_ms(),
        ],
    )?;
    for (seq, c) in ops.iter().enumerate() {
        let kind = match c.op {
            SystemOp::Hibernation | SystemOp::CompactVhdx { .. } => ItemKind::File,
            _ => ItemKind::Dir,
        };
        let verdict = c.op.verdict();
        tx.execute(
            "INSERT INTO proposal_item(proposal_id, seq, path, kind, alloc, logical, verdict, reason, selected, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending')",
            params![
                id,
                seq as i64,
                c.target,
                kind.to_db(),
                to_i64(c.estimate.unwrap_or(0)),
                to_i64(c.size.unwrap_or(0)),
                verdict.as_str(),
                c.note,
                default_tick(false, Some(verdict), false),
            ],
        )?;
    }
    tx.commit()?;
    load(&conn, &id)?.ok_or_else(|| anyhow::anyhow!("proposal vanished"))
}

// A follow-up keeps each item's approved tick (aligned with the request's items) instead of the default.
struct Carry {
    parent_id: String,
    ticks: Vec<bool>,
}

pub(crate) fn create_follow_up(
    db: &Db,
    env: &PolicyEnv,
    parent_id: &str,
) -> anyhow::Result<Option<Proposal>> {
    let Some(p) = load(&db.conn()?, parent_id)? else {
        return Ok(None);
    };
    if matches!(p.kind, ProposalKind::Dedupe | ProposalKind::SystemCleanup) {
        return Ok(None);
    }
    let (items, ticks): (Vec<NewItem>, Vec<bool>) = p
        .items
        .iter()
        .filter(|i| i.status != "done" && std::fs::symlink_metadata(&i.path).is_ok())
        .map(|i| {
            let item = NewItem {
                path: i.path.clone(),
                verdict: i.verdict,
                reason: i.reason.clone(),
            };
            (item, i.selected)
        })
        .unzip();
    if items.is_empty() {
        return Ok(None);
    }
    let req = NewProposal {
        conversation_id: p.conversation_id,
        tool_call_id: None,
        kind: p.kind,
        destination_id: p.destination.map(|d| d.id),
        format: p.format,
        dedupe: None,
        summary: p.summary,
        items,
    };
    let carry = Carry {
        parent_id: parent_id.to_string(),
        ticks,
    };
    Ok(build(db, env, req, Some(carry))?.ok())
}

// Err(dropped) when the gate refused every path.
fn build(
    db: &Db,
    env: &PolicyEnv,
    req: NewProposal,
    carry: Option<Carry>,
) -> anyhow::Result<Result<Proposal, Vec<DroppedItem>>> {
    if req.items.is_empty() {
        anyhow::bail!("a proposal needs at least one path");
    }
    if !crate::actions::supports(req.kind) {
        anyhow::bail!("{} proposals are not available yet", req.kind.as_str());
    }
    if req.kind == ProposalKind::SystemCleanup {
        anyhow::bail!("system_cleanup proposals hold detected operations, not paths");
    }
    let mut conn = db.conn()?;
    let mut dest_root: Option<String> = None;
    if req.kind == ProposalKind::Archive {
        let id = req.destination_id.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "an archive proposal needs a destination_id; call destinations() to list them"
            )
        })?;
        let dest = destinations::get(&conn, id)?.ok_or_else(|| {
            anyhow::anyhow!("unknown destination {id}; call destinations() to list registered ones")
        })?;
        dest_root = destinations::local_root(&dest)?
            .map(|root| crate::index::normalize_path(&root.to_string_lossy()));
    }

    let mut dropped: Vec<DroppedItem> = Vec::new();
    let mut accepted: Vec<(String, usize)> = Vec::new();
    for (i, item) in req.items.iter().enumerate() {
        let path = match policy::resolve(env, &item.path) {
            Ok(p) => p,
            Err(reason) => {
                dropped.push(DroppedItem {
                    path: item.path.clone(),
                    reason,
                });
                continue;
            }
        };
        if let Some(reason) = policy::deny_reason(&conn, env, &path)? {
            dropped.push(DroppedItem { path, reason });
            continue;
        }
        if let Some(root) = &dest_root
            && (root.eq_ignore_ascii_case(&path)
                || is_ancestor_path(root, &path)
                || is_ancestor_path(&path, root))
        {
            dropped.push(DroppedItem {
                reason: format!("{path} overlaps the archive destination {root}"),
                path,
            });
            continue;
        }
        if accepted.iter().any(|(p, _)| p.eq_ignore_ascii_case(&path)) {
            continue;
        }
        accepted.push((path, i));
    }
    let nested: Vec<(usize, String)> = accepted
        .iter()
        .enumerate()
        .filter_map(|(i, (p, _))| {
            accepted
                .iter()
                .find(|(q, _)| is_ancestor_path(q, p))
                .map(|(q, _)| (i, q.clone()))
        })
        .collect();
    for (i, parent) in nested.iter().rev() {
        let (path, _) = accepted.remove(*i);
        dropped.push(DroppedItem {
            reason: format!("already covered by {parent}"),
            path,
        });
    }

    let mut rows = Vec::new();
    for (path, i) in accepted {
        let item = &req.items[i];
        let ex = policy::examine(&conn, env, &path, req.kind.action())?;
        let note = memory::note_covering_with(&conn, &path, &env.profile)?.map(|m| m.text);
        rows.push(Row {
            verdict: item.verdict.map(|v| v.capped(ex.cap)),
            reason: item.reason.clone(),
            tick: carry.as_ref().and_then(|c| c.ticks.get(i).copied()),
            path,
            ex,
            note,
        });
    }
    if rows.is_empty() {
        return Ok(Err(dropped));
    }

    let id = uuid::Uuid::now_v7().to_string();
    let created = now_ms();
    let total: i64 = rows
        .iter()
        .filter(|r| r.ex.block.is_none())
        .map(|r| r.ex.alloc)
        .sum();
    let params = Params {
        destination_id: req.destination_id.clone(),
        format: req.format.clone(),
        dedupe: req.dedupe.clone(),
        summary: req.summary.clone(),
        dropped,
        ops: Vec::new(),
    };
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO proposal(id, conversation_id, tool_call_id, kind, params_json, status, total_alloc, created_ms, parent_id)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7, ?8)",
        params![
            id,
            req.conversation_id,
            req.tool_call_id,
            req.kind.as_str(),
            serde_json::to_string(&params)?,
            total,
            created,
            carry.as_ref().map(|c| c.parent_id.as_str()),
        ],
    )?;
    for (seq, r) in rows.iter().enumerate() {
        let state = r.ex.state.as_ref();
        let blocked = r.ex.block.is_some();
        let selected = match r.tick {
            Some(tick) => tick && !blocked,
            None => default_tick(blocked, r.verdict, is_small(r.ex.alloc, total)),
        };
        tx.execute(
            "INSERT INTO proposal_item(proposal_id, seq, path, node_id, kind, alloc, logical, mtime, file_id,
                                       verdict, cap, reason, block_reason, selected, status, memory_note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                id,
                seq as i64,
                r.path,
                r.ex.node_id,
                state.map_or(ItemKind::File, |s| s.kind).to_db(),
                r.ex.alloc,
                state.map_or(r.ex.logical, |s| if s.kind == ItemKind::File {
                    s.logical
                } else {
                    r.ex.logical
                }),
                state.and_then(|s| s.mtime),
                state.and_then(|s| s.file_id).map(|f| f.to_vec()),
                r.verdict.map(Verdict::as_str),
                r.ex.cap.map(Verdict::as_str),
                r.reason,
                r.ex.block,
                selected,
                if blocked { "blocked" } else { "pending" },
                r.note,
            ],
        )?;
    }
    tx.commit()?;
    let p = load(&conn, &id)?.ok_or_else(|| anyhow::anyhow!("proposal vanished"))?;
    Ok(Ok(p))
}

pub fn get(db: &Db, id: &str) -> anyhow::Result<Option<Proposal>> {
    load(&db.conn()?, id)
}

pub fn decline(db: &Db, id: &str) -> anyhow::Result<Proposal> {
    let conn = db.conn()?;
    let n = conn.execute(
        "UPDATE proposal SET status = 'declined', decided_ms = ?2 WHERE id = ?1 AND status = 'pending'",
        params![id, now_ms()],
    )?;
    let p = load(&conn, id)?.ok_or_else(|| anyhow::anyhow!("unknown proposal {id}"))?;
    if n == 0 {
        anyhow::bail!("proposal {id} is already {}", p.status);
    }
    Ok(p)
}

pub fn list_for_conversation(db: &Db, conversation_id: &str) -> anyhow::Result<Vec<Proposal>> {
    let conn = db.conn()?;
    let ids: Vec<String> = conn
        .prepare("SELECT id FROM proposal WHERE conversation_id = ?1 ORDER BY created_ms, id")?
        .query_map([conversation_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(p) = load(&conn, &id)? {
            out.push(p);
        }
    }
    Ok(out)
}

pub(crate) fn load(conn: &Connection, id: &str) -> anyhow::Result<Option<Proposal>> {
    type Head = (
        Option<String>,
        Option<String>,
        String,
        String,
        String,
        i64,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let head: Option<Head> = conn
        .query_row(
            "SELECT conversation_id, tool_call_id, kind, params_json, status, total_alloc, created_ms, job_id, parent_id,
                    (SELECT f.id FROM proposal f WHERE f.parent_id = p.id ORDER BY f.created_ms DESC, f.id DESC LIMIT 1)
             FROM proposal p WHERE p.id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                ))
            },
        )
        .optional()?;
    let Some((
        conversation_id,
        tool_call_id,
        kind,
        params_json,
        status,
        total_alloc,
        created_ms,
        job_id,
        parent_id,
        follow_up_id,
    )) = head
    else {
        return Ok(None);
    };
    let kind = ProposalKind::parse(&kind)
        .ok_or_else(|| anyhow::anyhow!("proposal {id} has unknown kind {kind}"))?;
    let params: Params = serde_json::from_str(&params_json).unwrap_or_default();
    let destination = match &params.destination_id {
        Some(did) => Some(DestinationRef {
            id: did.clone(),
            name: destinations::get(conn, did)?
                .map(|d| d.name)
                .unwrap_or_else(|| "(removed destination)".into()),
        }),
        None => None,
    };
    let mut st = conn.prepare_cached(
        "SELECT id, path, kind, alloc, verdict, cap, reason, block_reason, selected, status, detail, memory_note
         FROM proposal_item WHERE proposal_id = ?1 ORDER BY seq",
    )?;
    let items = st
        .query_map([id], |r| {
            let verdict: Option<String> = r.get(4)?;
            let cap: Option<String> = r.get(5)?;
            let alloc: i64 = r.get(3)?;
            Ok(ProposalItem {
                id: r.get(0)?,
                path: r.get(1)?,
                kind: ItemKind::from_db(r.get(2)?).as_str().to_string(),
                alloc,
                verdict: verdict.as_deref().and_then(Verdict::parse),
                cap: cap.as_deref().and_then(Verdict::parse),
                reason: r.get(6)?,
                block_reason: r.get(7)?,
                selected: r.get(8)?,
                status: r.get(9)?,
                detail: r.get(10)?,
                small: is_small(alloc, total_alloc),
                memory_note: r.get(11)?,
                op: None,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut items = items;
    if kind == ProposalKind::SystemCleanup {
        for (it, op) in items.iter_mut().zip(&params.ops) {
            it.op = Some(op.clone());
            it.small = false;
        }
    }
    Ok(Some(Proposal {
        id: id.to_string(),
        kind,
        status,
        total_alloc,
        created_ms,
        job_id,
        destination,
        format: params.format,
        dedupe: params.dedupe,
        summary: params.summary,
        conversation_id,
        tool_call_id,
        items,
        dropped: params.dropped,
        parent_id,
        follow_up_id,
    }))
}

pub(crate) fn stored_items(
    conn: &Connection,
    proposal_id: &str,
) -> anyhow::Result<Vec<StoredItem>> {
    let mut st = conn.prepare_cached(
        "SELECT id, path, kind, alloc, logical, mtime, file_id, selected, status
         FROM proposal_item WHERE proposal_id = ?1 ORDER BY seq",
    )?;
    let rows = st
        .query_map([proposal_id], |r| {
            let fid: Option<Vec<u8>> = r.get(6)?;
            Ok(StoredItem {
                id: r.get(0)?,
                path: r.get(1)?,
                kind: ItemKind::from_db(r.get(2)?),
                alloc: r.get(3)?,
                logical: r.get(4)?,
                mtime: r.get(5)?,
                file_id: fid.and_then(|v| <[u8; 16]>::try_from(v.as_slice()).ok()),
                selected: r.get(7)?,
                status: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(crate) fn set_item(
    conn: &Connection,
    item_id: i64,
    status: &str,
    detail: Option<&str>,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE proposal_item SET status = ?2, detail = COALESCE(?3, detail) WHERE id = ?1",
        params![item_id, status, detail],
    )?;
    Ok(())
}

pub(crate) fn set_status(conn: &Connection, id: &str, status: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE proposal SET status = ?2 WHERE id = ?1",
        params![id, status],
    )?;
    Ok(())
}

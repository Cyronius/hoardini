use std::collections::HashMap;
use std::fmt::Write;

use hoardini_core::memory::knowledge;
use hoardini_core::paths::humanize_bytes;
use hoardini_core::proposals::{self, Proposal};
use hoardini_core::review;
use hoardini_core::store::{Db, get_meta, now_ms, set_meta};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::agui::{AgFunctionCall, AgMessage, AgToolCall};
use crate::provider::{ChatMessage, ToolCallMsg};
use crate::tools::util::{redact_all, redact_value};

#[derive(Clone, Debug)]
pub struct Stored {
    pub id: String,
    pub msg: ChatMessage,
}

pub const OMITTED_PREFIX: &str = "[omitted:";
const MIN_SHED_CHARS: usize = 400;

// The app keeps one conversation; the server uses this ID whatever a client sends.
pub const MAIN: &str = "main";
pub const SESSION_GAP_MS: i64 = 6 * 60 * 60 * 1000;
pub const DEFAULT_CONTEXT_TOKENS: usize = 8_192;
const CHARS_PER_TOKEN: usize = 4;
const TRIM_HIGH_PCT: usize = 75;
const TRIM_LOW_PCT: usize = 40;
const EARLIER_DECISION: &str =
    "(The user answered a proposal card from earlier in the chat, before the part you can see.)";

pub fn ensure_conversation(
    conn: &Connection,
    id: &str,
    provider_id: Option<&str>,
) -> anyhow::Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO conversation(id, provider_id, created_ms, updated_ms) VALUES (?1, ?2, ?3, ?3)
         ON CONFLICT(id) DO UPDATE SET provider_id = COALESCE(excluded.provider_id, conversation.provider_id)",
        params![id, provider_id, now],
    )?;
    Ok(())
}

pub fn exists(conn: &Connection, id: &str) -> anyhow::Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM message WHERE id = ?1", [id], |_| Ok(()))
        .optional()?
        .is_some())
}

pub fn append(conn: &Connection, conv: &str, id: &str, msg: &ChatMessage) -> anyhow::Result<()> {
    append_at(conn, conv, id, msg, now_ms())
}

pub fn append_at(
    conn: &Connection,
    conv: &str,
    id: &str,
    msg: &ChatMessage,
    at_ms: i64,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO message(id, conversation_id, seq, role, json, created_ms)
         VALUES (?1, ?2, (SELECT COALESCE(MAX(seq), 0) + 1 FROM message WHERE conversation_id = ?2), ?3, ?4, ?5)",
        params![id, conv, msg.role, serde_json::to_string(msg)?, at_ms],
    )?;
    conn.execute(
        "UPDATE conversation SET updated_ms = ?2 WHERE id = ?1",
        params![conv, at_ms],
    )?;
    Ok(())
}

pub fn append_all_at(
    conn: &mut Connection,
    conv: &str,
    msgs: &[Stored],
    at_ms: i64,
) -> anyhow::Result<()> {
    let tx = conn.transaction()?;
    for s in msgs {
        append_at(&tx, conv, &s.id, &s.msg, at_ms)?;
    }
    tx.commit()?;
    Ok(())
}

pub fn replace(conn: &Connection, id: &str, msg: &ChatMessage) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE message SET json = ?2 WHERE id = ?1",
        params![id, serde_json::to_string(msg)?],
    )?;
    Ok(())
}

pub fn set_ui(conn: &Connection, id: &str, data: &Value) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE message SET ui_json = ?2 WHERE id = ?1",
        params![id, data.to_string()],
    )?;
    Ok(())
}

fn ui_from(conn: &Connection, conv: &str, from_seq: i64) -> anyhow::Result<HashMap<String, Value>> {
    let mut st = conn.prepare(
        "SELECT id, ui_json FROM message WHERE conversation_id = ?1 AND seq >= ?2 AND ui_json IS NOT NULL",
    )?;
    let rows = st
        .query_map(params![conv, from_seq], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, json)| serde_json::from_str(&json).ok().map(|v| (id, v)))
        .collect())
}

fn query_stored(conn: &Connection, sql: &str, conv: &str) -> anyhow::Result<Vec<Stored>> {
    let mut st = conn.prepare(sql)?;
    let rows = st
        .query_map([conv], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, json)| {
            serde_json::from_str::<ChatMessage>(&json)
                .ok()
                .map(|msg| Stored { id, msg })
        })
        .collect())
}

// The whole scrollback, including what the model no longer sees.
pub fn load(conn: &Connection, conv: &str) -> anyhow::Result<Vec<Stored>> {
    query_stored(
        conn,
        "SELECT id, json FROM message WHERE conversation_id = ?1 ORDER BY seq, rowid",
        conv,
    )
}

pub fn load_from(conn: &Connection, conv: &str, from_seq: i64) -> anyhow::Result<Vec<Stored>> {
    let mut st = conn.prepare(
        "SELECT id, json FROM message WHERE conversation_id = ?1 AND seq >= ?2 ORDER BY seq, rowid",
    )?;
    let rows = st
        .query_map(params![conv, from_seq], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, json)| {
            serde_json::from_str::<ChatMessage>(&json)
                .ok()
                .map(|msg| Stored { id, msg })
        })
        .collect())
}

pub fn load_window(conn: &Connection, conv: &str) -> anyhow::Result<Vec<Stored>> {
    query_stored(
        conn,
        "SELECT id, json FROM message WHERE conversation_id = ?1
           AND seq >= COALESCE((SELECT window_start_seq FROM conversation WHERE id = ?1), 0)
         ORDER BY seq, rowid",
        conv,
    )
}

pub fn window_start(conn: &Connection, conv: &str) -> anyhow::Result<i64> {
    Ok(conn
        .query_row(
            "SELECT window_start_seq FROM conversation WHERE id = ?1",
            [conv],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

pub fn set_window_start_at(conn: &Connection, conv: &str, message_id: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE conversation SET window_start_seq =
           (SELECT seq FROM message WHERE id = ?2 AND conversation_id = ?1)
         WHERE id = ?1 AND EXISTS (SELECT 1 FROM message WHERE id = ?2 AND conversation_id = ?1)",
        params![conv, message_id],
    )?;
    Ok(())
}

pub fn start_fresh(conn: &Connection, conv: &str) -> anyhow::Result<()> {
    ensure_conversation(conn, conv, None)?;
    conn.execute(
        "UPDATE conversation SET window_start_seq =
           (SELECT COALESCE(MAX(seq), 0) + 1 FROM message WHERE conversation_id = ?1)
         WHERE id = ?1",
        [conv],
    )?;
    Ok(())
}

// Messages before this seq stay stored but are off screen until the user asks for them.
const VIEW_START_KEY: &str = "chat_view_start_seq";

pub fn view_start(conn: &Connection) -> i64 {
    get_meta(conn, VIEW_START_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

pub fn clear_screen(conn: &Connection, conv: &str) -> anyhow::Result<()> {
    start_fresh(conn, conv)?;
    set_meta(conn, VIEW_START_KEY, &window_start(conn, conv)?.to_string())?;
    Ok(())
}

pub fn count_before(conn: &Connection, conv: &str, seq: i64) -> anyhow::Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM message WHERE conversation_id = ?1 AND seq < ?2",
        params![conv, seq],
        |r| r.get(0),
    )?)
}

pub fn clear(conn: &mut Connection, conv: &str) -> anyhow::Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM message WHERE conversation_id = ?1", [conv])?;
    // seq restarts at 1, so a leftover window start would hide every new message.
    tx.execute(
        "UPDATE conversation SET window_start_seq = 0 WHERE id = ?1",
        [conv],
    )?;
    set_meta(&tx, VIEW_START_KEY, "0")?;
    tx.commit()?;
    Ok(())
}

fn last_message_ms(conn: &Connection, conv: &str) -> anyhow::Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT created_ms FROM message WHERE conversation_id = ?1 ORDER BY seq DESC, rowid DESC LIMIT 1",
            [conv],
            |r| r.get(0),
        )
        .optional()?)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowView {
    pub window_start_id: Option<String>,
    pub has_hidden: bool,
    pub fresh: bool,
}

pub fn view(conn: &Connection, conv: &str, now: i64) -> anyhow::Result<WindowView> {
    view_from(conn, conv, now, 0)
}

// The divider only matters among the messages on screen, which begin at `from_seq`.
pub fn view_from(
    conn: &Connection,
    conv: &str,
    now: i64,
    from_seq: i64,
) -> anyhow::Result<WindowView> {
    let start = window_start(conn, conv)?;
    let window_start_id: Option<String> = conn
        .query_row(
            "SELECT id FROM message WHERE conversation_id = ?1 AND seq >= ?2 ORDER BY seq, rowid LIMIT 1",
            params![conv, start],
            |r| r.get(0),
        )
        .optional()?;
    let has_hidden = conn
        .query_row(
            "SELECT 1 FROM message WHERE conversation_id = ?1 AND seq < ?2 AND seq >= ?3 LIMIT 1",
            params![conv, start, from_seq],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    let fresh = match last_message_ms(conn, conv)? {
        None => true,
        Some(t) => now - t > SESSION_GAP_MS || window_start_id.is_none(),
    };
    Ok(WindowView {
        window_start_id,
        has_hidden,
        fresh,
    })
}

pub fn context_tokens(context_window: u32) -> usize {
    match context_window {
        0 => DEFAULT_CONTEXT_TOKENS,
        n => n as usize,
    }
}

fn msg_chars(m: &ChatMessage) -> usize {
    serde_json::to_string(m).map_or(0, |s| s.len())
}

pub fn trim_point(
    window: &[Stored],
    overhead_chars: usize,
    context_tokens: usize,
) -> Option<usize> {
    let budget = context_tokens.saturating_mul(CHARS_PER_TOKEN);
    let sizes: Vec<usize> = window.iter().map(|s| msg_chars(&s.msg)).collect();
    let total: usize = sizes.iter().sum();
    if (overhead_chars + total) * 100 <= budget * TRIM_HIGH_PCT {
        return None;
    }
    let mut before = 0;
    let mut last_user = None;
    for (i, s) in window.iter().enumerate() {
        if i > 0 && s.msg.role == "user" {
            if (overhead_chars + total - before) * 100 <= budget * TRIM_LOW_PCT {
                return Some(i);
            }
            last_user = Some(i);
        }
        before += sizes[i];
    }
    last_user
}

pub fn next_turn(window: &[Stored]) -> Option<usize> {
    window
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, s)| s.msg.role == "user")
        .map(|(i, _)| i)
}

pub fn tool_msg(call_id: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: "tool".into(),
        content: Some(content.into()),
        tool_call_id: Some(call_id.into()),
        ..Default::default()
    }
}

fn find_call<'a>(stored: &'a [Stored], call_id: &str) -> Option<(usize, &'a ToolCallMsg)> {
    stored
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, s)| s.msg.role == "assistant")
        .find_map(|(i, s)| {
            s.msg
                .tool_calls
                .as_ref()
                .and_then(|calls| calls.iter().find(|c| c.id == call_id))
                .map(|c| (i, c))
        })
}

fn result_after(stored: &[Stored], pos: usize, call_id: &str) -> Option<usize> {
    stored
        .iter()
        .enumerate()
        .skip(pos + 1)
        .find(|(_, s)| s.msg.role == "tool" && s.msg.tool_call_id.as_deref() == Some(call_id))
        .map(|(i, _)| i)
}

pub fn call_ids(stored: &[Stored]) -> std::collections::HashSet<String> {
    stored
        .iter()
        .flat_map(|s| s.msg.tool_calls.iter().flatten().map(|c| c.id.clone()))
        .collect()
}

pub fn proposal_id_of(call: &ToolCallMsg) -> Option<String> {
    serde_json::from_str::<Value>(&call.function.arguments)
        .ok()?
        .get("proposal_id")?
        .as_str()
        .map(str::to_string)
}

pub fn describe_proposal(p: &Proposal) -> String {
    let kind = serde_json::to_value(p.kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let state = match p.status.as_str() {
        "pending" => "is still pending: the user has not decided. Nothing was changed.",
        "declined" => {
            "was declined by the user. Nothing was changed; do not propose it again unless the user asks."
        }
        "approved" | "running" => {
            "was approved. The job runs in the background; the app shows its progress."
        }
        "done" => "was approved and has finished.",
        "partial" => "was approved and finished only partly; see the items.",
        "failed" => "was approved but the job failed; see the items.",
        _ => "has an unknown status.",
    };
    let mut s = format!("Proposal {} ({kind}) {state}", p.id);
    if !matches!(p.status.as_str(), "pending" | "declined") {
        let sel: Vec<_> = p.items.iter().filter(|i| i.selected).collect();
        let bytes: i64 = sel.iter().map(|i| i.alloc.max(0)).sum();
        let _ = write!(
            s,
            " The user selected {} of {} items ({}).",
            sel.len(),
            p.items.len(),
            humanize_bytes(bytes as u64)
        );
        if let Some(j) = &p.job_id {
            let _ = write!(s, " Job {j}.");
        }
    }
    s.push('\n');
    for i in p.items.iter().take(20) {
        let _ = write!(
            s,
            "- {} | {} | {} | {}",
            i.path,
            humanize_bytes(i.alloc.max(0) as u64),
            if i.selected {
                "selected"
            } else {
                "not selected"
            },
            i.status
        );
        if let Some(b) = &i.block_reason {
            let _ = write!(s, " ({b})");
        }
        if let Some(d) = &i.detail {
            let _ = write!(s, " ({d})");
        }
        s.push('\n');
    }
    if p.items.len() > 20 {
        let _ = writeln!(s, "+{} more", p.items.len() - 20);
    }
    s
}

pub fn describe_open(p: &Proposal) -> String {
    let mut target = p
        .items
        .first()
        .map_or_else(|| "(no items)".to_string(), |i| i.path.clone());
    if p.items.len() > 1 {
        let _ = write!(target, " +{} more", p.items.len() - 1);
    }
    match (&p.destination, &p.format) {
        (Some(d), _) => {
            let _ = write!(target, " to {}", d.name);
        }
        (None, Some(f)) => {
            let _ = write!(target, " as {f}");
        }
        _ => {}
    }
    format!(
        "{} | {} | {target} | {}",
        p.id,
        p.kind.as_str(),
        humanize_bytes(p.total_alloc.max(0) as u64)
    )
}

// Cards can outlive the turn that made them, so the prompt names every one still waiting, review cards too.
pub fn open_proposals(db: &Db, conv: &str) -> anyhow::Result<Vec<String>> {
    let ids: Vec<String> = db
        .conn()?
        .prepare(
            "SELECT id FROM proposal WHERE (conversation_id = ?1 OR review_id IS NOT NULL) AND status = 'pending'              ORDER BY review_id IS NOT NULL, created_ms, id",
        )?
        .query_map([conv], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(p) = proposals::get(db, &id)? {
            let mut line = describe_open(&p);
            if let Some(c) = &p.card {
                let title = review::card(c).map_or(c.as_str(), |x| x.title);
                let _ = write!(line, " | review card: {title}");
            }
            out.push(line);
        }
    }
    Ok(out)
}

pub const NO_DECISION: &str =
    "No decision was recorded for this proposal. Treat it as not approved; nothing was changed.";

// The client's tool-message text is never trusted: the answer comes from the proposal row.
pub fn proposal_result(db: &Db, conv: &str, call: &ToolCallMsg) -> String {
    let by_id = proposal_id_of(call).and_then(|id| proposals::get(db, &id).ok().flatten());
    let found = by_id.or_else(|| {
        proposals::list_for_conversation(db, conv)
            .ok()?
            .into_iter()
            .find(|p| p.tool_call_id.as_deref() == Some(call.id.as_str()))
    });
    match found {
        Some(p) => describe_proposal(&p),
        None => NO_DECISION.into(),
    }
}

pub fn ingest(
    db: &Db,
    conv: &str,
    provider_id: Option<&str>,
    input: &[AgMessage],
    now: i64,
) -> anyhow::Result<()> {
    let conn = db.conn()?;
    ensure_conversation(&conn, conv, provider_id)?;
    for m in input {
        if m.id.trim().is_empty() || exists(&conn, &m.id)? {
            continue;
        }
        match m.role.as_str() {
            "user" => {
                let text = m.content.as_deref().unwrap_or("");
                if !text.trim().is_empty() {
                    let gap =
                        last_message_ms(&conn, conv)?.is_some_and(|t| now - t > SESSION_GAP_MS);
                    append_at(&conn, conv, &m.id, &ChatMessage::text("user", text), now)?;
                    if gap {
                        set_window_start_at(&conn, conv, &m.id)?;
                    }
                }
            }
            "tool" => {
                let Some(call_id) = m.tool_call_id.as_deref() else {
                    continue;
                };
                let stored = load(&conn, conv)?;
                let Some((pos, call)) = find_call(&stored, call_id) else {
                    continue;
                };
                // Read-tool results are server-made; only proposal answers come from the client.
                if !call.function.name.starts_with("propose_")
                    || result_after(&stored, pos, call_id).is_some()
                {
                    continue;
                }
                let content = proposal_result(db, conv, call);
                append_at(&conn, conv, &m.id, &tool_msg(call_id, &content), now)?;
            }
            _ => {}
        }
    }
    Ok(())
}

// Every tool call gets exactly one result right after its assistant message, as providers require.
pub fn context(db: &Db, conv: &str, stored: &[Stored]) -> Vec<ChatMessage> {
    let calls = call_ids(stored);
    let mut used = vec![false; stored.len()];
    let mut out = Vec::with_capacity(stored.len());
    for (i, s) in stored.iter().enumerate() {
        match s.msg.role.as_str() {
            // A card answered after its call left the window; the decision text names the proposal.
            "tool"
                if s.msg
                    .tool_call_id
                    .as_ref()
                    .is_some_and(|id| !calls.contains(id)) =>
            {
                let text = s.msg.content.as_deref().unwrap_or(NO_DECISION);
                out.push(ChatMessage::text(
                    "user",
                    &format!("{EARLIER_DECISION}\n{text}"),
                ));
            }
            "tool" => {}
            "assistant" => {
                out.push(s.msg.clone());
                for c in s.msg.tool_calls.iter().flatten() {
                    let hit = (i + 1..stored.len()).find(|&j| {
                        !used[j]
                            && stored[j].msg.role == "tool"
                            && stored[j].msg.tool_call_id.as_deref() == Some(c.id.as_str())
                    });
                    match hit {
                        Some(j) => {
                            used[j] = true;
                            out.push(stored[j].msg.clone());
                        }
                        None if c.function.name.starts_with("propose_") => {
                            out.push(tool_msg(&c.id, &proposal_result(db, conv, c)));
                        }
                        None => out.push(tool_msg(&c.id, "No result was recorded for this call.")),
                    }
                }
            }
            _ => out.push(s.msg.clone()),
        }
    }
    out
}

fn tool_name(stored: &[Stored], call_id: &str) -> String {
    find_call(stored, call_id)
        .map(|(_, c)| c.function.name.clone())
        .unwrap_or_else(|| "tool".into())
}

// Overflow recovery: stub the largest tool result (older turns first) and heal the stored row.
pub fn shed_largest(conn: &Connection, stored: &mut [Stored]) -> anyhow::Result<Option<usize>> {
    let last_assistant = stored.iter().rposition(|s| s.msg.role == "assistant");
    let size = |s: &Stored| s.msg.content.as_deref().map_or(0, str::len);
    let candidate = |s: &Stored| {
        s.msg.role == "tool"
            && size(s) >= MIN_SHED_CHARS
            && !s
                .msg
                .content
                .as_deref()
                .unwrap_or("")
                .starts_with(OMITTED_PREFIX)
    };
    let pick = |older_only: bool| {
        stored
            .iter()
            .enumerate()
            .filter(|(i, s)| candidate(s) && (!older_only || last_assistant.is_none_or(|a| *i < a)))
            .max_by_key(|(_, s)| size(s))
            .map(|(i, _)| i)
    };
    let Some(i) = pick(true).or_else(|| pick(false)) else {
        return Ok(None);
    };
    let chars = size(&stored[i]);
    let name = stored[i]
        .msg
        .tool_call_id
        .as_deref()
        .map(|id| tool_name(stored, id))
        .unwrap_or_else(|| "tool".into());
    stored[i].msg.content = Some(format!(
        "{OMITTED_PREFIX} this {name} result ({chars} characters) was dropped to fit the model's context window; call the tool again with a narrower query if you need it]"
    ));
    replace(conn, &stored[i].id, &stored[i].msg)?;
    Ok(Some(chars))
}

fn hidden_call(c: &ToolCallMsg) -> bool {
    c.function.name.starts_with("propose_") && proposal_id_of(c).is_none()
}

// A chip confirmed after its run was stored would otherwise offer Confirm again on reload; one whose row is gone
// (ids are reused) is dropped, so its Undo can never reach another row.
fn learned_now(conn: &Connection, profile: Option<&str>, mut v: Value) -> Value {
    if v.get("kind").and_then(Value::as_str) != Some("learned") {
        return v;
    }
    let system = hoardini_core::paths::profile_dir()
        .to_string_lossy()
        .into_owned();
    let profile = profile.unwrap_or(&system);
    if let Some(items) = v.get_mut("items").and_then(Value::as_array_mut) {
        items.retain_mut(|it| {
            let (Some(id), Some(text)) = (
                it.get("memory_id").and_then(Value::as_i64),
                it.get("text").and_then(Value::as_str),
            ) else {
                return false;
            };
            let key = it.get("key").and_then(Value::as_str);
            match knowledge::learned_row(conn, id, key, text, profile) {
                Ok(Some(m)) => {
                    it["confirmed"] = Value::Bool(m.source == "user");
                    true
                }
                _ => false,
            }
        });
    }
    v
}

// The UI's view of a stored thread; failed proposal calls stay LLM-only so no empty card renders.
pub fn agui_messages(db: &Db, conv: &str, profile: Option<&str>) -> anyhow::Result<Vec<AgMessage>> {
    agui_messages_from(db, conv, profile, 0)
}

pub fn agui_messages_from(
    db: &Db,
    conv: &str,
    profile: Option<&str>,
    from_seq: i64,
) -> anyhow::Result<Vec<AgMessage>> {
    let conn = db.conn()?;
    let stored = load_from(&conn, conv, from_seq)?;
    let mut ui = ui_from(&conn, conv, from_seq)?;
    let shown = |s: &str| match profile {
        Some(p) => redact_all(s, p),
        None => s.to_string(),
    };
    let hidden: std::collections::HashSet<String> = stored
        .iter()
        .flat_map(|s| s.msg.tool_calls.iter().flatten())
        .filter(|c| hidden_call(c))
        .map(|c| c.id.clone())
        .collect();
    let mut out = Vec::new();
    for s in &stored {
        let m = &s.msg;
        if m.role == "tool"
            && m.tool_call_id
                .as_ref()
                .is_some_and(|id| hidden.contains(id))
        {
            continue;
        }
        let calls: Vec<AgToolCall> = m
            .tool_calls
            .iter()
            .flatten()
            .filter(|c| !hidden.contains(&c.id))
            .map(|c| AgToolCall {
                id: c.id.clone(),
                kind: "function".into(),
                function: AgFunctionCall {
                    name: c.function.name.clone(),
                    arguments: shown(&c.function.arguments),
                },
            })
            .collect();
        let content = m.content.as_deref().map(shown).filter(|c| !c.is_empty());
        if m.role == "assistant" && content.is_none() && calls.is_empty() {
            continue;
        }
        out.push(AgMessage {
            id: s.id.clone(),
            role: m.role.clone(),
            content,
            tool_calls: (!calls.is_empty()).then_some(calls),
            tool_call_id: m.tool_call_id.clone(),
            data: ui
                .remove(&s.id)
                .map(|v| learned_now(&conn, profile, v))
                .map(|v| match profile {
                    Some(p) => redact_value(&v, p),
                    None => v,
                }),
        });
    }
    Ok(out)
}

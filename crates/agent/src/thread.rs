use hoardini_core::store::{Db, now_ms};
use rusqlite::{Connection, OptionalExtension, params};

use crate::agui::{AgFunctionCall, AgMessage, AgToolCall};
use crate::provider::{ChatMessage, ToolCallMsg};
use crate::tools::util::redact_all;

#[derive(Clone, Debug)]
pub struct Stored {
    pub id: String,
    pub msg: ChatMessage,
}

pub const OMITTED_PREFIX: &str = "[omitted:";
const MIN_SHED_CHARS: usize = 400;

// Questions about one item on Home each get their own `item:` thread (AGENT-ASK).
pub const ITEM_PREFIX: &str = "item:";

// An item thread ID is short and plain, so it can't collide with another conversation.
pub fn is_item_thread(id: &str) -> bool {
    id.strip_prefix(ITEM_PREFIX).is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= 64
            && rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    })
}
pub const DEFAULT_CONTEXT_TOKENS: usize = 8_192;
const CHARS_PER_TOKEN: usize = 4;
const TRIM_HIGH_PCT: usize = 75;
const TRIM_LOW_PCT: usize = 40;

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

fn find_call<'a>(stored: &'a [Stored], call_id: &str) -> Option<&'a ToolCallMsg> {
    stored
        .iter()
        .rev()
        .filter(|s| s.msg.role == "assistant")
        .find_map(|s| {
            s.msg
                .tool_calls
                .as_ref()
                .and_then(|calls| calls.iter().find(|c| c.id == call_id))
        })
}

pub fn call_ids(stored: &[Stored]) -> std::collections::HashSet<String> {
    stored
        .iter()
        .flat_map(|s| s.msg.tool_calls.iter().flatten().map(|c| c.id.clone()))
        .collect()
}

// Only new user messages are taken from the client; tool results and answers are the loop's own.
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
        if m.role != "user" || m.id.trim().is_empty() || exists(&conn, &m.id)? {
            continue;
        }
        let text = m.content.as_deref().unwrap_or("");
        if !text.trim().is_empty() {
            append_at(&conn, conv, &m.id, &ChatMessage::text("user", text), now)?;
        }
    }
    Ok(())
}

// Every tool call gets exactly one result right after its assistant message, as providers require.
pub fn context(stored: &[Stored]) -> Vec<ChatMessage> {
    let mut used = vec![false; stored.len()];
    let mut out = Vec::with_capacity(stored.len());
    for (i, s) in stored.iter().enumerate() {
        match s.msg.role.as_str() {
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
        .map(|c| c.function.name.clone())
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

// The UI's view of a stored thread, every path shown with the profile as `~` when one is given.
pub fn agui_messages(db: &Db, conv: &str, profile: Option<&str>) -> anyhow::Result<Vec<AgMessage>> {
    let stored = load(&db.conn()?, conv)?;
    let shown = |s: &str| match profile {
        Some(p) => redact_all(s, p),
        None => s.to_string(),
    };
    let mut out = Vec::new();
    for s in &stored {
        let m = &s.msg;
        let calls: Vec<AgToolCall> = m
            .tool_calls
            .iter()
            .flatten()
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
        });
    }
    Ok(out)
}

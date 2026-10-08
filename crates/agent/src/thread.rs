use std::fmt::Write;

use hoardini_core::paths::humanize_bytes;
use hoardini_core::proposals::{self, Proposal};
use hoardini_core::store::{Db, now_ms};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

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

pub fn title_from(text: &str) -> String {
    let line = text.trim().lines().next().unwrap_or("").trim();
    let mut t: String = line.chars().take(80).collect();
    if line.chars().count() > 80 {
        t.push_str("...");
    }
    t
}

pub fn ensure_conversation(
    conn: &Connection,
    id: &str,
    title: Option<&str>,
    provider_id: Option<&str>,
) -> anyhow::Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO conversation(id, title, provider_id, created_ms, updated_ms) VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(id) DO UPDATE SET title = COALESCE(conversation.title, excluded.title),
           provider_id = COALESCE(excluded.provider_id, conversation.provider_id)",
        params![id, title, provider_id, now],
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
    let now = now_ms();
    conn.execute(
        "INSERT INTO message(id, conversation_id, seq, role, json, created_ms)
         VALUES (?1, ?2, (SELECT COALESCE(MAX(seq), 0) + 1 FROM message WHERE conversation_id = ?2), ?3, ?4, ?5)",
        params![id, conv, msg.role, serde_json::to_string(msg)?, now],
    )?;
    conn.execute(
        "UPDATE conversation SET updated_ms = ?2 WHERE id = ?1",
        params![conv, now],
    )?;
    Ok(())
}

pub fn append_all(conn: &mut Connection, conv: &str, msgs: &[Stored]) -> anyhow::Result<()> {
    let tx = conn.transaction()?;
    for s in msgs {
        append(&tx, conv, &s.id, &s.msg)?;
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

pub fn load(conn: &Connection, conv: &str) -> anyhow::Result<Vec<Stored>> {
    let mut st = conn
        .prepare("SELECT id, json FROM message WHERE conversation_id = ?1 ORDER BY seq, rowid")?;
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
) -> anyhow::Result<()> {
    let conn = db.conn()?;
    let title = input
        .iter()
        .filter(|m| m.role == "user")
        .find_map(|m| m.content.as_deref().filter(|c| !c.trim().is_empty()))
        .map(title_from);
    ensure_conversation(&conn, conv, title.as_deref(), provider_id)?;
    for m in input {
        if m.id.trim().is_empty() || exists(&conn, &m.id)? {
            continue;
        }
        match m.role.as_str() {
            "user" => {
                let text = m.content.as_deref().unwrap_or("");
                if !text.trim().is_empty() {
                    append(&conn, conv, &m.id, &ChatMessage::text("user", text))?;
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
                append(&conn, conv, &m.id, &tool_msg(call_id, &content))?;
            }
            _ => {}
        }
    }
    Ok(())
}

// Every tool call gets exactly one result right after its assistant message, as providers require.
pub fn context(db: &Db, conv: &str, stored: &[Stored]) -> Vec<ChatMessage> {
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

// The UI's view of a stored thread; failed proposal calls stay LLM-only so no empty card renders.
pub fn agui_messages(db: &Db, conv: &str, profile: Option<&str>) -> anyhow::Result<Vec<AgMessage>> {
    let stored = load(&db.conn()?, conv)?;
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
        });
    }
    Ok(out)
}

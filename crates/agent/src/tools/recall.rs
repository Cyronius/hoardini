use std::fmt::Write;

use hoardini_core::memory::{Memory, recall_path_with, recall_text};
use hoardini_core::store::now_ms;
use serde::Deserialize;
use serde_json::{Value, json};

use super::util::{abs_path, conn, date, failed, opt_string, parse, untrusted};
use super::{Outcome, Tool, ToolCtx, ToolError};

pub struct Recall;

#[derive(Deserialize)]
struct Args {
    #[serde(default, deserialize_with = "opt_string")]
    path: Option<String>,
    #[serde(default, alias = "query", deserialize_with = "opt_string")]
    text: Option<String>,
}

const SHOW: usize = 20;

fn looks_like_path(s: &str) -> bool {
    let t = s.trim();
    t.starts_with('~') || t.contains('\\') || t.as_bytes().get(1) == Some(&b':')
}

pub fn render(rows: &[Memory]) -> String {
    let now = now_ms();
    let mut out = String::new();
    for m in rows.iter().take(SHOW) {
        let kind = serde_json::to_value(m.kind).unwrap_or_default();
        let _ = write!(
            out,
            "- #{} [{}] {}: {} (source {}, {})",
            m.id,
            kind.as_str().unwrap_or("?"),
            m.key,
            m.text,
            m.source,
            date(Some(m.updated_ms))
        );
        if m.is_stale(now) {
            out.push_str(" STALE: re-check before relying on it");
        }
        out.push('\n');
    }
    if rows.len() > SHOW {
        let _ = writeln!(out, "+{} more", rows.len() - SHOW);
    }
    out
}

impl Tool for Recall {
    fn name(&self) -> &'static str {
        "recall"
    }

    fn description(&self) -> &'static str {
        "Saved notes about a path (the path, its parent folders and matching patterns) or matching a text search."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {
            "path": {"type": "string", "description": "File or folder to recall notes for"},
            "text": {"type": "string", "description": "Words to search notes for, when there is no path"}
        }})
    }

    fn small(&self) -> bool {
        true
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: Args = parse(args)?;
        let (path, text) = match (a.path, a.text) {
            (Some(p), _) => (Some(p), None),
            (None, Some(t)) if looks_like_path(&t) => (Some(t), None),
            (None, Some(t)) => (None, Some(t)),
            (None, None) => {
                return Err(ToolError::BadArgs("give a path or text to recall".into()));
            }
        };
        let conn = conn(ctx)?;
        let (rows, what) = match (path, text) {
            (Some(p), _) => {
                let abs = abs_path(ctx, &p)?;
                let rows = recall_path_with(&conn, &abs, &ctx.profile).map_err(failed)?;
                (rows, abs)
            }
            (None, Some(t)) => (recall_text(&conn, &t).map_err(failed)?, format!("{t:?}")),
            (None, None) => (Vec::new(), String::new()),
        };
        if rows.is_empty() {
            return Ok(Outcome::text(format!("No saved notes for {what}.")));
        }
        let body = render(&rows);
        let content = if rows.iter().take(SHOW).any(|m| m.source.starts_with("web:")) {
            format!("Saved notes for {what}:\n{}", untrusted("notes from web pages", &body))
        } else {
            format!("Saved notes for {what}:\n{body}")
        };
        Ok(Outcome::text(content))
    }
}

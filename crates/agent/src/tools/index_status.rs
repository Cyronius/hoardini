use std::fmt::Write;

use hoardini_core::index::{humanize_age, index_status};
use hoardini_core::paths::humanize_bytes;
use serde_json::{Value, json};

use super::util::{conn, digits, failed};
use super::{Outcome, Tool, ToolCtx, ToolError};

pub struct IndexStatus;

impl Tool for IndexStatus {
    fn name(&self) -> &'static str {
        "index_status"
    }

    fn description(&self) -> &'static str {
        "How old the disk index is, free space per volume, and whether a refresh is running."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn run(&self, ctx: &ToolCtx, _args: Value) -> Result<Outcome, ToolError> {
        let conn = conn(ctx)?;
        let mut st = conn
            .prepare("SELECT root FROM volume WHERE root_node_id IS NOT NULL ORDER BY root LIMIT 16")
            .map_err(failed)?;
        let roots = st
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        let mut out = String::new();
        for root in &roots {
            let Some(s) = index_status(&conn, Some(root)).map_err(failed)? else {
                continue;
            };
            let age = s
                .age_ms
                .map(humanize_age)
                .unwrap_or_else(|| "an unfinished first refresh".into());
            let _ = write!(out, "{}: index as of {age} (refresh #{}", s.volume, s.generation);
            if s.refresh_incomplete {
                out.push_str(", a newer refresh did not finish");
            }
            out.push_str(").");
            if let (Some(total), Some(free)) = (s.total_bytes, s.free_bytes) {
                let pct = if total > 0 { free * 100 / total } else { 0 };
                let _ = write!(
                    out,
                    " Volume {}, {} free ({pct}%).",
                    humanize_bytes(total),
                    humanize_bytes(free)
                );
            }
            let _ = write!(
                out,
                " Indexed {} in {} files",
                humanize_bytes(s.indexed_bytes),
                digits(s.file_count)
            );
            if let Some(u) = s.unaccounted_bytes.filter(|u| *u > 0) {
                let _ = write!(
                    out,
                    "; {} unaccounted (restore points, shadow copies, unreadable folders)",
                    humanize_bytes(u as u64)
                );
            }
            if let Some(d) = s.denied_count.filter(|d| *d > 0) {
                let _ = write!(out, "; {} folders could not be read", digits(d));
            }
            out.push_str(".\n");
        }
        if roots.is_empty() {
            out.push_str("No index yet. Call refresh to build one.\n");
        }
        out.push_str(
            &refresh_note(&ctx.index.status()).unwrap_or_else(|| "Refresh: idle.".into()),
        );
        Ok(Outcome::text(out))
    }
}

// Accepts the server's Status or its `index` part; Some only while a refresh runs.
pub fn refresh_note(status: &Value) -> Option<String> {
    let v = status.get("index").unwrap_or(status);
    if v.get("state").and_then(Value::as_str) != Some("refreshing") {
        return None;
    }
    let mut s = String::from("Refresh: running");
    if let Some(mode) = v.get("mode").and_then(Value::as_str) {
        let _ = write!(s, " in {mode} mode");
    }
    let p = v.get("progress");
    if let Some(f) = p.and_then(|p| p.get("fraction")).and_then(Value::as_f64) {
        let _ = write!(s, ", {:.0}%", (f * 100.0).clamp(0.0, 100.0));
    }
    if let Some(ph) = p.and_then(|p| p.get("phase")).and_then(Value::as_str) {
        let _ = write!(s, " ({ph})");
    }
    s.push_str(". Answers use the stored index until it finishes.");
    Some(s)
}

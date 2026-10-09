use std::fmt::Write;

use hoardini_core::history::{
    Change, Since, changes, fmt_date, free_summary, scans_between,
};
use hoardini_core::paths::humanize_bytes;
use serde::Deserialize;
use serde_json::{Value, json};

use super::util::{conn, failed, limit, opt_string, opt_u64, parse};
use super::{Outcome, Tool, ToolCtx, ToolError};

pub struct Changes;

#[derive(Deserialize)]
struct Args {
    #[serde(default, deserialize_with = "opt_string")]
    since: Option<String>,
    #[serde(default, deserialize_with = "opt_u64")]
    limit: Option<u64>,
    #[serde(default, deserialize_with = "opt_string")]
    drive: Option<String>,
}

fn signed(delta: i64) -> String {
    let sign = if delta < 0 { "-" } else { "+" };
    format!("{sign}{}", humanize_bytes(delta.unsigned_abs()))
}

fn section(out: &mut String, title: &str, items: &[Change], omitted: usize) {
    if items.is_empty() {
        return;
    }
    let _ = writeln!(out, "{title}:");
    for c in items {
        let span = match (c.before, c.after) {
            (Some(b), Some(a)) => format!(" ({} -> {})", humanize_bytes(b), humanize_bytes(a)),
            _ => String::new(),
        };
        let _ = writeln!(out, "- {} {} {}{span}", c.kind, c.path, signed(c.delta));
    }
    if omitted > 0 {
        let _ = writeln!(out, "+{omitted} more");
    }
}

impl Tool for Changes {
    fn name(&self) -> &'static str {
        "changes"
    }

    fn description(&self) -> &'static str {
        "What grew, shrank, appeared or disappeared between two index refreshes (folders >= 256 MB, files >= 100 MB)."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {
            "since": {"type": "string", "description": "\"previous\" (default), a number of days like \"30d\", or a refresh id"},
            "limit": {"type": "integer", "description": "Max entries per section (default 10)"},
            "drive": {"type": "string", "description": "Drive letter like D: (default: the drive the user is looking at)"}
        }})
    }

    fn small(&self) -> bool {
        true
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: Args = parse(args)?;
        let since: Since = a
            .since
            .as_deref()
            .unwrap_or("previous")
            .parse()
            .map_err(|e: anyhow::Error| ToolError::BadArgs(e.to_string()))?;
        let conn = conn(ctx)?;
        let drive = a
            .drive
            .map(|d| format!("{}\\", d.trim().trim_end_matches(['\\', '/'])))
            .or_else(|| hoardini_core::index::focus(&conn));
        let volume_id = match drive {
            Some(d) => match hoardini_core::index::index_status(&conn, Some(&d)).map_err(failed)? {
                Some(st) => Some(st.volume_id),
                None => return Ok(Outcome::text(format!("{d} is not indexed."))),
            },
            None => None,
        };
        let Some(ch) = changes(&conn, volume_id, since, limit(a.limit, 10)).map_err(failed)? else {
            return Ok(Outcome::text(
                "No earlier refresh to compare with yet; changes appear after the second refresh.",
            ));
        };
        let mut out = format!(
            "{}: refresh #{} ({}) -> #{} ({}).",
            ch.volume,
            ch.from.id,
            fmt_date(ch.from.finished_ms),
            ch.to.id,
            fmt_date(ch.to.finished_ms)
        );
        if let Some(fd) = ch.free_delta {
            let _ = write!(out, " Free space {}.", signed(fd));
        }
        let _ = writeln!(out, " Indexed {}.", signed(ch.indexed_delta));
        section(&mut out, "Grew", &ch.grew, ch.omitted.grew);
        section(&mut out, "New", &ch.new, ch.omitted.new);
        section(&mut out, "Shrank", &ch.shrank, ch.omitted.shrank);
        section(&mut out, "Removed", &ch.removed, ch.omitted.removed);
        if ch.grew.is_empty() && ch.new.is_empty() && ch.shrank.is_empty() && ch.removed.is_empty() {
            out.push_str("No big changes.\n");
        }
        let range = scans_between(
            &conn,
            ch.from.volume_id,
            Some(ch.from.finished_ms),
            Some(ch.to.finished_ms),
        )
        .map_err(failed)?;
        if let Some(line) = free_summary(&range).map(|f| f.line()) {
            let _ = writeln!(out, "{line}");
        }
        let _ = writeln!(out, "range_ms: {}..{} {}", ch.from.finished_ms, ch.to.finished_ms, ch.volume);
        Ok(Outcome::text(out))
    }
}

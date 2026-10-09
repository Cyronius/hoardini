use std::fmt::Write;

use hoardini_core::index::top_dirs;
use hoardini_core::paths::humanize_bytes;
use serde::Deserialize;
use serde_json::{Value, json};

use super::util::{
    abs_path, as_of_ms, conn, digits, failed, index_age, limit, opt_string, opt_u64, parse,
};
use super::{DATA_VERSION, Outcome, Tool, ToolCtx, ToolError};

pub struct TopDirs;

#[derive(Deserialize)]
struct Args {
    #[serde(default, deserialize_with = "opt_string")]
    under: Option<String>,
    #[serde(default, deserialize_with = "opt_u64")]
    depth: Option<u64>,
    #[serde(default, deserialize_with = "opt_u64")]
    limit: Option<u64>,
}

impl Tool for TopDirs {
    fn name(&self) -> &'static str {
        "top_dirs"
    }

    fn description(&self) -> &'static str {
        "Largest folders from the index, biggest first, with total size and file count."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {
            "under": {"type": "string", "description": "Folder to break down, e.g. ~\\.cache; omit for the volume roots"},
            "depth": {"type": "integer", "description": "How many levels below `under` to rank (default 1, max 4)"},
            "limit": {"type": "integer", "description": "How many to list (default 15, max 50)"}
        }})
    }

    fn small(&self) -> bool {
        true
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: Args = parse(args)?;
        let under = a.under.as_deref().map(|u| abs_path(ctx, u)).transpose()?;
        let depth = a.depth.unwrap_or(1).clamp(1, 4) as u32;
        let conn = conn(ctx)?;
        let r = top_dirs(&conn, under.as_deref(), depth, limit(a.limit, 15)).map_err(failed)?;
        let mut out = format!(
            "Largest folders under {} (depth {depth}; {}):\n",
            under.as_deref().unwrap_or("the indexed volumes"),
            index_age(&conn)
        );
        if r.items.is_empty() {
            out.push_str("No subfolders.\n");
        }
        for d in &r.items {
            let _ = write!(
                out,
                "- {} | {} | {} files",
                d.path,
                humanize_bytes(d.size),
                digits(d.files)
            );
            if d.denied {
                out.push_str(" | some contents unreadable");
            }
            out.push('\n');
        }
        if let Some(more) = r.more_line() {
            let _ = writeln!(out, "{more}");
        }
        let items: Vec<Value> = r
            .items
            .iter()
            .map(|d| json!({"path": d.path, "size": d.size, "files": d.files, "denied": d.denied}))
            .collect();
        let data = json!({
            "v": DATA_VERSION,
            "kind": "folders",
            "under": under,
            "depth": depth,
            "as_of_ms": as_of_ms(&conn, under.as_deref()),
            "items": items,
            "more": {"count": r.more_count, "bytes": r.more_bytes},
        });
        Ok(Outcome::text(out).with_data(data))
    }
}

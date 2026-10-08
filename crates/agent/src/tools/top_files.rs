use std::fmt::Write;

use hoardini_core::index::{TopFilesQuery, top_files};
use hoardini_core::paths::humanize_bytes;
use serde::Deserialize;
use serde_json::{Value, json};

use super::util::{abs_path, conn, date, failed, index_age, limit, opt_days, opt_size, opt_string, opt_u64, parse};
use super::{Outcome, Tool, ToolCtx, ToolError};

pub struct TopFiles;

#[derive(Deserialize)]
struct Args {
    #[serde(default, deserialize_with = "opt_string")]
    under: Option<String>,
    #[serde(default, deserialize_with = "opt_size")]
    min_size: Option<u64>,
    #[serde(default, deserialize_with = "opt_string")]
    ext: Option<String>,
    #[serde(default, deserialize_with = "opt_days")]
    not_used_for: Option<u32>,
    #[serde(default, deserialize_with = "opt_u64")]
    limit: Option<u64>,
}

impl Tool for TopFiles {
    fn name(&self) -> &'static str {
        "top_files"
    }

    fn description(&self) -> &'static str {
        "Largest files from the index, biggest first, with modified and last-access dates."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {
            "under": {"type": "string", "description": "Folder to search, e.g. ~\\Downloads; omit for everything"},
            "min_size": {"type": "string", "description": "Smallest size to list, e.g. \"500 MB\""},
            "ext": {"type": "string", "description": "Extensions, comma separated, e.g. \"gguf,safetensors\""},
            "not_used_for": {"type": "integer", "description": "Only files neither modified nor accessed for this many days"},
            "limit": {"type": "integer", "description": "How many to list (default 15, max 50)"}
        }})
    }

    fn small(&self) -> bool {
        true
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: Args = parse(args)?;
        let under = a.under.as_deref().map(|u| abs_path(ctx, u)).transpose()?;
        let conn = conn(ctx)?;
        let q = TopFilesQuery {
            under: under.clone(),
            min_size: a.min_size.unwrap_or(0),
            ext: a.ext.clone(),
            not_used_for_days: a.not_used_for,
            limit: limit(a.limit, 15),
        };
        let r = top_files(&conn, &q).map_err(failed)?;
        let mut out = format!(
            "Largest files under {}",
            under.as_deref().unwrap_or("all indexed volumes")
        );
        let mut filters = Vec::new();
        if let Some(m) = a.min_size.filter(|m| *m > 0) {
            filters.push(format!("at least {}", humanize_bytes(m)));
        }
        if let Some(e) = &a.ext {
            filters.push(format!("ext {e}"));
        }
        if let Some(d) = a.not_used_for {
            filters.push(format!("not modified or accessed for {d}+ days"));
        }
        if !filters.is_empty() {
            let _ = write!(out, " ({})", filters.join(", "));
        }
        let _ = writeln!(out, "; {}:", index_age(&conn));
        if r.items.is_empty() {
            out.push_str("No files match.\n");
        }
        for f in &r.items {
            let _ = write!(
                out,
                "- {} | {} | modified {} | accessed {}",
                f.path,
                humanize_bytes(f.size),
                date(f.mtime_ms),
                date(f.atime_ms)
            );
            if f.placeholder {
                out.push_str(" | cloud-only placeholder");
            }
            out.push('\n');
        }
        if let Some(more) = r.more_line() {
            let _ = writeln!(out, "{more}");
        }
        Ok(Outcome::text(out))
    }
}

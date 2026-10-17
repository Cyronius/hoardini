use std::fmt::Write;
use std::sync::atomic::AtomicBool;

use hoardini_core::dupes::{self, DupOptions};
use hoardini_core::paths::humanize_bytes;
use serde::Deserialize;
use serde_json::{Value, json};

use super::util::{abs_path, conn, failed, index_age, opt_size, opt_string, parse};
use super::{Outcome, Tool, ToolCtx, ToolError};

pub struct Duplicates;

const MAX_GROUPS: usize = 10;
const MAX_PATHS: usize = 6;

#[derive(Deserialize)]
struct Args {
    #[serde(default, deserialize_with = "opt_string")]
    under: Option<String>,
    #[serde(default, deserialize_with = "opt_size")]
    min_size: Option<u64>,
}

impl Tool for Duplicates {
    fn name(&self) -> &'static str {
        "duplicates"
    }

    fn description(&self) -> &'static str {
        "Find identical large files (same content, not hard links) from the index, hashing candidates as needed. If it says candidates are still being hashed, call it again to continue."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {
            "under": {"type": "string", "description": "Folder to search; omit for everything indexed"},
            "min_size": {"type": "string", "description": "Smallest file to consider (default \"100 MB\")"}
        }})
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: Args = parse(args)?;
        let under = a.under.as_deref().map(|u| abs_path(ctx, u)).transpose()?;
        let conn = conn(ctx)?;
        let opts = DupOptions {
            under: under.clone(),
            min_size: a.min_size.unwrap_or(dupes::DEFAULT_MIN_SIZE),
            ..DupOptions::default()
        };
        let r = dupes::find(&conn, &opts, &AtomicBool::new(false)).map_err(failed)?;
        let total: u64 = r.groups.iter().map(|g| g.wasted_bytes()).sum();
        let mut out = format!(
            "Duplicate groups under {} (files of {}+; {}):\n",
            under.as_deref().unwrap_or("all indexed volumes"),
            humanize_bytes(opts.min_size),
            index_age(&conn)
        );
        if r.groups.is_empty() {
            out.push_str("No duplicates found.\n");
        } else {
            let _ = writeln!(
                out,
                "{} group{} wasting {} in total.",
                r.groups.len(),
                if r.groups.len() == 1 { "" } else { "s" },
                humanize_bytes(total)
            );
        }
        for g in r.groups.iter().take(MAX_GROUPS) {
            let _ = writeln!(
                out,
                "group {} | {} each | {} copies | wasted {}",
                g.id,
                humanize_bytes(g.size),
                g.paths.len(),
                humanize_bytes(g.wasted_bytes())
            );
            for p in g.paths.iter().take(MAX_PATHS) {
                let _ = writeln!(out, "  - {p}");
            }
            if g.paths.len() > MAX_PATHS {
                let _ = writeln!(out, "  +{} more", g.paths.len() - MAX_PATHS);
            }
        }
        if r.groups.len() > MAX_GROUPS {
            let _ = writeln!(out, "+{} more groups", r.groups.len() - MAX_GROUPS);
        }
        if r.pending > 0 {
            let _ = writeln!(
                out,
                "{} candidate file{} still being hashed; call duplicates again to continue (hashes already done are reused).",
                r.pending,
                if r.pending == 1 { " is" } else { "s are" }
            );
        }
        Ok(Outcome::text(out))
    }
}

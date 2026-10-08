use std::fmt::Write;

use hoardini_core::destinations::{list, test};
use hoardini_core::paths::humanize_bytes;
use serde_json::{Value, json};

use super::util::conn;
use super::{Outcome, Tool, ToolCtx, ToolError};

pub struct Destinations;

impl Tool for Destinations {
    fn name(&self) -> &'static str {
        "destinations"
    }

    fn description(&self) -> &'static str {
        "The archive destinations the user registered (name, type, free space). propose_archive can only target one of these."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn run(&self, ctx: &ToolCtx, _args: Value) -> Result<Outcome, ToolError> {
        let conn = conn(ctx)?;
        let all = list(&conn)
            .map_err(|e| ToolError::Failed(format!("could not list destinations: {e}")))?;
        if all.is_empty() {
            return Ok(Outcome::text(
                "No destinations registered. The user can add one in Settings, Destinations; archiving needs one.",
            ));
        }
        let mut out = String::from("Registered destinations:\n");
        // Config holds paths and secret refs; only name, kind and free space ever reach the model.
        for d in all.iter().take(12) {
            let t = test(d, ctx.secrets.as_ref());
            let free = match (t.ok, t.free_bytes) {
                (true, Some(f)) => format!("{} free", humanize_bytes(f.max(0) as u64)),
                (true, None) => "free space unknown".into(),
                (false, _) => "currently unreachable".into(),
            };
            let _ = writeln!(out, "- {} ({}): {free}", d.name, d.kind);
        }
        if all.len() > 12 {
            let _ = writeln!(out, "+{} more", all.len() - 12);
        }
        Ok(Outcome::text(out))
    }
}

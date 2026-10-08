use serde::Deserialize;
use serde_json::{Value, json};

use super::util::{abs_path, opt_string, parse};
use super::{Outcome, Tool, ToolCtx, ToolError};

pub struct Refresh;

#[derive(Deserialize)]
struct Args {
    #[serde(default, deserialize_with = "opt_string")]
    mode: Option<String>,
    #[serde(default, deserialize_with = "opt_string")]
    scope: Option<String>,
}

impl Tool for Refresh {
    fn name(&self) -> &'static str {
        "refresh"
    }

    fn description(&self) -> &'static str {
        "Start updating the disk index. Use only when the user wants current numbers; answers keep using the stored index meanwhile."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {
            "mode": {"type": "string", "enum": ["background", "fast"],
                     "description": "background (default, polite) or fast (all cores, when the user waits)"},
            "scope": {"type": "string", "description": "Optional folder to refresh; omit for the whole index"}
        }})
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: Args = parse(args)?;
        let mode = a.mode.as_deref().map(str::trim).unwrap_or("background");
        if !matches!(mode, "background" | "fast") {
            return Err(ToolError::BadArgs(format!(
                "mode must be \"background\" or \"fast\", got {mode:?}"
            )));
        }
        let scope = a.scope.as_deref().map(|s| abs_path(ctx, s)).transpose()?;
        let started = ctx
            .index
            .refresh_scope(scope.as_deref(), mode)
            .map_err(|e| ToolError::Failed(format!("could not start a refresh: {e}")))?;
        Ok(Outcome::text(if started {
            format!(
                "Started a {mode} refresh. Answers use the stored index until it finishes; call index_status for progress."
            )
        } else {
            "A refresh is already running; call index_status for progress.".into()
        }))
    }
}

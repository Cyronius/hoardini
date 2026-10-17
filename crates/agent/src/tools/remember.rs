use hoardini_core::memory::{Kind, NewMemory, Scope, remember_with};
use hoardini_core::store::now_ms;
use serde::Deserialize;
use serde_json::{Value, json};

use super::util::{abs_path, conn, failed, opt_string, parse};
use super::{Outcome, Tool, ToolCtx, ToolError};

pub struct Remember;

#[derive(Deserialize)]
struct Args {
    #[serde(default, deserialize_with = "opt_string")]
    scope: Option<String>,
    kind: String,
    text: String,
    #[serde(default, deserialize_with = "opt_string")]
    key: Option<String>,
    #[serde(default)]
    user_asked: bool,
}

impl Tool for Remember {
    fn name(&self) -> &'static str {
        "remember"
    }

    fn description(&self) -> &'static str {
        "Save a note for later questions: a user preference, a fact about a folder, a verdict, or a folder the user wants protected. The user can see and remove it in Settings."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {
            "scope": {"type": "string", "description": "A path (~\\.lmstudio\\models), a pattern (~\\code\\**\\target), or \"global\" for a general preference"},
            "kind": {"type": "string", "enum": ["fact", "preference", "protect", "verdict"],
                     "description": "protect = nothing under scope may be removed"},
            "text": {"type": "string", "description": "The note, one sentence"},
            "user_asked": {"type": "boolean", "description": "true only if the user explicitly asked you to remember this"}
        }, "required": ["scope", "kind", "text"]})
    }

    fn small(&self) -> bool {
        true
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: Args = parse(args)?;
        let kind: Kind = serde_json::from_value(Value::String(a.kind.trim().to_lowercase()))
            .map_err(|_| {
                ToolError::BadArgs(format!(
                    "kind must be fact, preference, protect or verdict, got {:?}",
                    a.kind
                ))
            })?;
        let text = a.text.trim();
        if text.is_empty() {
            return Err(ToolError::BadArgs("text is empty".into()));
        }
        let raw_scope = a.scope.as_deref().map(str::trim).unwrap_or("global");
        let (scope, key) = if raw_scope.eq_ignore_ascii_case("global") {
            (Scope::Global, a.key.clone().unwrap_or_else(|| "note".into()))
        } else if raw_scope.contains(['*', '?']) {
            (Scope::Glob, raw_scope.to_string())
        } else {
            (Scope::Path, abs_path(ctx, raw_scope)?)
        };
        let conn = conn(ctx)?;
        let mut new = NewMemory::new(scope, &key, kind, text, "agent");
        new.pinned = a.user_asked;
        let m = remember_with(
            &conn,
            new,
            now_ms(),
            &ctx.profile,
        )
        .map_err(failed)?;
        Ok(Outcome::text(format!(
            "Noted as memory #{} ({}, {}): {}. The user can change or remove it in Settings.",
            m.id, a.kind, m.key, m.text
        )))
    }
}

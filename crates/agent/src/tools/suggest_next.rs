use std::fmt::Write;

use hoardini_core::index::node_by_path;
use serde_json::{Value, json};

use super::util::{abs_path, conn, failed};
use super::{DATA_VERSION, Outcome, Tool, ToolCtx, ToolError};

pub struct SuggestNext;

pub const MAX_PILLS: usize = 3;
// Each kind needs a working UI action; add one here only together with its button.
pub const PILL_KINDS: [&str; 4] = ["ask", "clean_up", "explore", "what_is"];
const NEEDS_PATH: [&str; 3] = ["clean_up", "explore", "what_is"];
const MAX_LABEL_CHARS: usize = 80;

struct Pill {
    label: String,
    kind: String,
    path: Option<String>,
}

fn field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn items_of(args: &Value) -> Result<Vec<Value>, ToolError> {
    match args.get("items") {
        Some(Value::Array(a)) => Ok(a.clone()),
        // Small models double-encode nested arrays as a JSON string.
        Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
            Ok(Value::Array(a)) => Ok(a),
            _ => Err(ToolError::BadArgs("items must be an array".into())),
        },
        _ => Err(ToolError::BadArgs(
            "items must be an array of {label, kind, path}".into(),
        )),
    }
}

impl Tool for SuggestNext {
    fn name(&self) -> &'static str {
        "suggest_next"
    }

    fn description(&self) -> &'static str {
        "Offer up to 3 next steps as buttons under your answer. Call it last. kind \"ask\": the label is sent as the user's next question. kind \"clean_up\": looks for what can go in `path` (a folder from the index). kind \"explore\": opens `path` in the Explore treemap. kind \"what_is\": asks the user what `path` is (keep it, it can go, or move it elsewhere), for a big folder nobody has identified."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {
            "items": {"type": "array", "description": "1 to 3 suggestions, best first", "items": {
                "type": "object",
                "properties": {
                    "label": {"type": "string", "description": "Button text, a few words"},
                    "kind": {"type": "string", "enum": PILL_KINDS},
                    "path": {"type": "string", "description": "Folder or file it is about, e.g. ~\\.cache; required for clean_up, explore and what_is"}
                },
                "required": ["label", "kind"]
            }}
        }, "required": ["items"]})
    }

    fn small(&self) -> bool {
        true
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let items = items_of(&args)?;
        let conn = conn(ctx)?;
        let mut kept: Vec<Pill> = Vec::new();
        let mut dropped: Vec<String> = Vec::new();
        for it in &items {
            let label: String = field(it, "label")
                .unwrap_or("")
                .chars()
                .take(MAX_LABEL_CHARS)
                .collect();
            let kind = field(it, "kind").unwrap_or("").to_ascii_lowercase();
            let why = if label.is_empty() {
                Some("no label".to_string())
            } else if !PILL_KINDS.contains(&kind.as_str()) {
                Some(format!("kind {kind:?} is not available"))
            } else if NEEDS_PATH.contains(&kind.as_str()) && field(it, "path").is_none() {
                Some(format!("{kind} needs a path"))
            } else {
                None
            };
            if let Some(why) = why {
                dropped.push(format!("{label:?} ({why})"));
                continue;
            }
            let path = match field(it, "path") {
                None => None,
                Some(raw) => match abs_path(ctx, raw) {
                    Err(_) => {
                        dropped.push(format!("{label:?} ({raw} is not an absolute path)"));
                        continue;
                    }
                    Ok(p) => match node_by_path(&conn, &p).map_err(failed)? {
                        Some(n) => Some(n.path),
                        None => {
                            dropped.push(format!("{label:?} ({p} is not in the index)"));
                            continue;
                        }
                    },
                },
            };
            let same = |k: &Pill| {
                k.kind == kind
                    && match (&k.path, &path) {
                        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                        (None, None) => k.label.eq_ignore_ascii_case(&label),
                        _ => false,
                    }
            };
            if kept.iter().any(same) {
                dropped.push(format!("{label:?} (a duplicate)"));
            } else if kept.len() >= MAX_PILLS {
                dropped.push(format!("{label:?} (only {MAX_PILLS} are shown)"));
            } else {
                kept.push(Pill { label, kind, path });
            }
        }
        let mut out = if kept.is_empty() {
            "No suggestion is shown.".to_string()
        } else {
            let names: Vec<String> = kept.iter().map(|p| format!("{:?}", p.label)).collect();
            format!(
                "Shown under your answer: {}. Your turn is complete; do not repeat the answer.",
                names.join(", ")
            )
        };
        if !dropped.is_empty() {
            let _ = write!(out, "\nDropped: {}.", dropped.join("; "));
        }
        let outcome = Outcome::text(out);
        if kept.is_empty() {
            return Ok(outcome);
        }
        let pills: Vec<Value> = kept
            .into_iter()
            .map(|p| match p.path {
                Some(path) => json!({"label": p.label, "kind": p.kind, "path": path}),
                None => json!({"label": p.label, "kind": p.kind}),
            })
            .collect();
        Ok(outcome.with_data(json!({"v": DATA_VERSION, "kind": "pills", "items": pills})))
    }
}

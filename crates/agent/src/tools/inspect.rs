use std::fmt::Write;
use std::path::Path;

use hoardini_core::evidence::{Evidence, PathKind, inspect};
use hoardini_core::paths::humanize_bytes;
use serde::Deserialize;
use serde_json::{Value, json};

use super::util::{abs_path, date, digits, parse, untrusted};
use super::{Outcome, Tool, ToolCtx, ToolError};

pub struct Inspect;

#[derive(Deserialize)]
struct Args {
    path: String,
}

fn enum_str<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

pub fn render(ev: &Evidence) -> String {
    let summary = if ev.summary.is_empty() {
        ev.compute_summary()
    } else {
        ev.summary.clone()
    };
    let mut d = String::new();
    let _ = writeln!(d, "Summary: {summary}");
    if let Some(a) = ev.alloc_bytes {
        let _ = write!(d, "Size: {}", humanize_bytes(a));
        if let Some(l) = ev.logical_bytes.filter(|l| *l != a) {
            let _ = write!(d, " (logical {})", humanize_bytes(l));
        }
        if let Some(n) = ev.file_count {
            let _ = write!(d, ", {} files", digits(n));
        }
        d.push('\n');
    }
    if let Some(c) = &ev.catalog {
        let _ = writeln!(
            d,
            "Catalog: {} ({}), {}; regenerable: {}; usual verdict: {}{}",
            c.name,
            c.app,
            enum_str(&c.category),
            if c.regenerable { "yes" } else { "no" },
            c.verdict_hint.as_str(),
            c.safe_unit
                .as_deref()
                .map(|u| format!("; delete only whole units: {u}"))
                .unwrap_or_default()
        );
        if !c.notes.is_empty() {
            let _ = writeln!(d, "Catalog notes: {}", c.notes);
        }
    }
    if let Some(i) = &ev.identity {
        let _ = write!(d, "Identity: {}", i.describe());
        if i.already_compressed {
            d.push_str(" (already compressed)");
        }
        if let Some(n) = &i.note {
            let _ = write!(d, "; {n}");
        }
        d.push('\n');
    }
    if let Some(o) = &ev.owner {
        let _ = writeln!(
            d,
            "Owner app: {}{} (from {})",
            o.name,
            o.publisher
                .as_deref()
                .map(|p| format!(", {p}"))
                .unwrap_or_default(),
            o.source
        );
    }
    if !ev.in_use.is_empty() {
        let names: Vec<String> = ev
            .in_use
            .iter()
            .take(5)
            .map(|p| format!("{} (pid {})", p.name, p.pid))
            .collect();
        let _ = writeln!(d, "In use by: {}", names.join(", "));
    }
    for s in ev.last_used.iter().take(6) {
        let _ = write!(
            d,
            "Last used ({}): {} [{} confidence]",
            s.source,
            date(s.value_ms),
            enum_str(&s.confidence)
        );
        if let Some(n) = &s.note {
            let _ = write!(d, " {n}");
        }
        d.push('\n');
    }
    if !ev.duplicates.is_empty() {
        let _ = writeln!(d, "Duplicates:");
        for x in ev.duplicates.iter().take(8) {
            let _ = writeln!(d, "- {} | {}", x.path, humanize_bytes(x.size));
        }
        if ev.duplicates.len() > 8 {
            let _ = writeln!(d, "+{} more", ev.duplicates.len() - 8);
        }
    }
    if !ev.memories.is_empty() {
        let _ = writeln!(d, "Saved notes:");
        for m in ev.memories.iter().take(8) {
            let _ = writeln!(
                d,
                "- #{} [{}] {}: {} (source {})",
                m.id,
                enum_str(&m.kind),
                m.key,
                m.text,
                m.source
            );
        }
        if ev.memories.len() > 8 {
            let _ = writeln!(d, "+{} more", ev.memories.len() - 8);
        }
    }
    let _ = writeln!(
        d,
        "Verdict cap: {}",
        ev.policy_cap
            .map(|c| c.as_str().to_string())
            .unwrap_or_else(|| "none".into())
    );
    d
}

impl Tool for Inspect {
    fn name(&self) -> &'static str {
        "inspect"
    }

    fn description(&self) -> &'static str {
        "Evidence about one file or folder: catalog match, file type and model metadata, owner app, open handles, last-used signals, duplicates, saved notes, and the verdict cap."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {
            "path": {"type": "string", "description": "File or folder, e.g. ~\\.lmstudio\\models\\x.gguf"}
        }, "required": ["path"]})
    }

    fn small(&self) -> bool {
        true
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: Args = parse(args)?;
        let path = abs_path(ctx, &a.path)?;
        let ev = inspect(&ctx.db, Path::new(&path))
            .map_err(|e| ToolError::Failed(format!("could not inspect {path}: {e}")))?;
        if ev.kind == PathKind::Missing {
            return Ok(Outcome::text(format!("{path} does not exist on disk.")));
        }
        Ok(Outcome::text(format!(
            "Evidence for {path}:\n{}",
            untrusted("evidence from file headers, the registry and saved notes", &render(&ev))
        )))
    }
}

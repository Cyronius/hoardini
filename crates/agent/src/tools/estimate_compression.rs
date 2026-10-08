use std::fmt::Write;
use std::path::Path;

use hoardini_core::compress::estimate;
use hoardini_core::paths::humanize_bytes;
use serde::Deserialize;
use serde_json::{Value, json};

use super::util::{abs_path, failed, parse};
use super::{Outcome, Tool, ToolCtx, ToolError};

pub struct EstimateCompression;

#[derive(Deserialize)]
struct Args {
    path: String,
}

fn secs(s: f64) -> String {
    if s < 90.0 {
        format!("~{:.0} s", s.max(1.0))
    } else {
        format!("~{:.0} min", s / 60.0)
    }
}

impl Tool for EstimateCompression {
    fn name(&self) -> &'static str {
        "estimate_compression"
    }

    fn description(&self) -> &'static str {
        "Estimate how much a file or folder would shrink per format (zip, 7z, tar.zst, lzx, lznt1) by sampling it. Estimates only; media, archives and quantized models report no gain."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {
            "path": {"type": "string", "description": "File or folder"}
        }, "required": ["path"]})
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: Args = parse(args)?;
        let path = abs_path(ctx, &a.path)?;
        let e = estimate(Path::new(&path)).map_err(failed)?;
        let mut out = format!(
            "Compression estimate for {path} ({}, {}; sampled {} from {} file{}):\n",
            if e.is_dir { "folder" } else { "file" },
            humanize_bytes(e.logical_bytes),
            humanize_bytes(e.sampled_bytes),
            e.files_sampled,
            if e.files_sampled == 1 { "" } else { "s" }
        );
        if let Some(n) = &e.note {
            let _ = writeln!(out, "{n}");
        }
        if e.already_compressed_bytes > 0 && !e.already_compressed {
            let _ = writeln!(
                out,
                "{} of it is already-compressed data and counted as no gain.",
                humanize_bytes(e.already_compressed_bytes)
            );
        }
        for f in &e.formats {
            let _ = writeln!(
                out,
                "- {} ({}) | ratio {:.2} | result ~{} | saves ~{} | time {}",
                f.format,
                if f.in_place { "in place" } else { "archive" },
                f.ratio,
                humanize_bytes(f.compressed_bytes),
                humanize_bytes(f.saved_bytes),
                secs(f.seconds)
            );
        }
        out.push_str("lzx is for data that is not written to again; lznt1 suits data that changes.\n");
        Ok(Outcome::text(out))
    }
}

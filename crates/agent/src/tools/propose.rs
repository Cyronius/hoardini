use hoardini_core::destinations;
use hoardini_core::policy::Verdict;
use hoardini_core::proposals::{self, DedupeParams, NewItem, NewProposal, ProposalKind};
use hoardini_core::syscleanup;
use serde::Deserialize;
use serde_json::{Value, json};

use super::util::{abs_path, conn, opt_string, opt_u64, parse};
use super::{Kind, Outcome, Tool, ToolCtx, ToolError};

pub struct ProposeDelete;
pub struct ProposeArchive;
pub struct ProposeCompress;
pub struct ProposeDedupe;
pub struct ProposeSystemCleanup;

#[derive(Deserialize)]
#[serde(untagged)]
enum Entry {
    Path(String),
    Item {
        path: String,
        #[serde(default, deserialize_with = "opt_string")]
        verdict: Option<String>,
        #[serde(default, deserialize_with = "opt_string")]
        reason: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Entries {
    Many(Vec<Entry>),
    One(Entry),
}

#[derive(Deserialize)]
struct PathsArgs {
    paths: Entries,
    #[serde(default, deserialize_with = "opt_string")]
    verdict: Option<String>,
    #[serde(default, deserialize_with = "opt_string")]
    reason: Option<String>,
    #[serde(default, deserialize_with = "opt_string")]
    destination: Option<String>,
}

fn verdict(s: Option<&str>) -> Option<Verdict> {
    s.and_then(|v| Verdict::parse(v.trim().to_lowercase().as_str()))
}

fn one_line(s: Option<&str>) -> Option<String> {
    s.map(|r| r.lines().next().unwrap_or("").trim().chars().take(200).collect::<String>())
        .filter(|r| !r.is_empty())
}

fn items(ctx: &ToolCtx, a: &PathsArgs) -> Result<Vec<NewItem>, ToolError> {
    entry_items(ctx, &a.paths, a.verdict.as_deref(), a.reason.as_deref())
}

fn entry_items(
    ctx: &ToolCtx,
    paths: &Entries,
    verdict_all: Option<&str>,
    reason_all: Option<&str>,
) -> Result<Vec<NewItem>, ToolError> {
    let entries: Vec<&Entry> = match paths {
        Entries::Many(v) => v.iter().collect(),
        Entries::One(e) => vec![e],
    };
    if entries.is_empty() {
        return Err(ToolError::BadArgs("paths is empty".into()));
    }
    entries
        .into_iter()
        .map(|e| {
            let (path, v, r) = match e {
                Entry::Path(p) => (p, None, None),
                Entry::Item {
                    path,
                    verdict,
                    reason,
                } => (path, verdict.as_deref(), reason.as_deref()),
            };
            Ok(NewItem {
                path: abs_path(ctx, path)?,
                verdict: verdict(v.or(verdict_all)),
                reason: one_line(r.or(reason_all)),
            })
        })
        .collect()
}

fn path_schema() -> Value {
    json!({"type": "array", "items": {"type": "string"},
           "description": "Absolute or ~ paths. An entry may also be {\"path\", \"verdict\", \"reason\"} for per-item notes."})
}

fn common_props() -> Value {
    json!({
        "verdict": {"type": "string", "enum": ["safe", "redownload", "review", "keep"],
                    "description": "Your verdict for the items; code caps it"},
        "reason": {"type": "string", "description": "One line: why this can go"}
    })
}

fn with(mut base: Value, extra: Value) -> Value {
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

fn submit(ctx: &ToolCtx, req: NewProposal) -> Result<Outcome, ToolError> {
    let p = proposals::create(&ctx.db, req)
        .map_err(|e| ToolError::Failed(format!("the proposal was not created: {e}")))?;
    Ok(Outcome::Proposal { proposal_id: p.id })
}

fn new_proposal(ctx: &ToolCtx, kind: ProposalKind, summary: String, items: Vec<NewItem>) -> NewProposal {
    NewProposal {
        conversation_id: ctx.conversation_id.clone(),
        tool_call_id: Some(ctx.tool_call_id.clone()),
        kind,
        destination_id: None,
        format: None,
        dedupe: None,
        summary,
        items,
    }
}

fn summary(verb: &str, reason: Option<&str>, n: usize) -> String {
    one_line(reason).unwrap_or_else(|| {
        format!("{verb} {n} item{}", if n == 1 { "" } else { "s" })
    })
}

impl Tool for ProposeDelete {
    fn name(&self) -> &'static str {
        "propose_delete"
    }

    fn description(&self) -> &'static str {
        "Propose permanently deleting files or folders (no Recycle Bin). Shows the user a card; nothing is deleted unless they approve. Stop after calling it."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": with(json!({"paths": path_schema()}), common_props()),
               "required": ["paths"]})
    }

    fn kind(&self) -> Kind {
        Kind::Propose
    }

    fn small(&self) -> bool {
        true
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: PathsArgs = parse(args)?;
        let items = items(ctx, &a)?;
        let s = summary("Delete", a.reason.as_deref(), items.len());
        submit(ctx, new_proposal(ctx, ProposalKind::Delete, s, items))
    }
}

fn resolve_destination(ctx: &ToolCtx, wanted: &str) -> Result<String, ToolError> {
    let w = wanted.trim();
    let Ok(conn) = conn(ctx) else {
        return Ok(w.to_string());
    };
    let Ok(all) = destinations::list(&conn) else {
        return Ok(w.to_string());
    };
    all.iter()
        .find(|d| d.id == w || d.name.eq_ignore_ascii_case(w))
        .map(|d| d.id.clone())
        .ok_or_else(|| {
            let names: Vec<&str> = all.iter().map(|d| d.name.as_str()).collect();
            ToolError::Failed(if names.is_empty() {
                "no destinations are registered; the user can add one in Settings, Destinations".into()
            } else {
                format!("unknown destination {w:?}; registered: {}", names.join(", "))
            })
        })
}

impl Tool for ProposeArchive {
    fn name(&self) -> &'static str {
        "propose_archive"
    }

    fn description(&self) -> &'static str {
        "Propose archiving: copy to a registered destination, verify, then delete the source. Reversible. Shows the user a card; stop after calling it."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": with(json!({
            "paths": path_schema(),
            "destination": {"type": "string", "description": "Destination name from destinations()"}
        }), common_props()), "required": ["paths", "destination"]})
    }

    fn kind(&self) -> Kind {
        Kind::Propose
    }

    fn small(&self) -> bool {
        true
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: PathsArgs = parse(args)?;
        let dest = a
            .destination
            .as_deref()
            .ok_or_else(|| ToolError::BadArgs("destination is required".into()))?;
        let dest = resolve_destination(ctx, dest)?;
        let items = items(ctx, &a)?;
        let s = summary("Archive", a.reason.as_deref(), items.len());
        let mut req = new_proposal(ctx, ProposalKind::Archive, s, items);
        req.destination_id = Some(dest);
        submit(ctx, req)
    }
}

const FORMATS: [&str; 5] = ["zip", "7z", "tar.zst", "lzx", "lznt1"];

#[derive(Deserialize)]
struct CompressArgs {
    #[serde(alias = "path")]
    paths: Entries,
    format: String,
    #[serde(default, deserialize_with = "opt_string")]
    verdict: Option<String>,
    #[serde(default, deserialize_with = "opt_string")]
    reason: Option<String>,
}

impl Tool for ProposeCompress {
    fn name(&self) -> &'static str {
        "propose_compress"
    }

    fn description(&self) -> &'static str {
        "Propose compressing files or folders: into archives (zip, 7z, tar.zst; originals deleted after a test) or in place (lzx for read-only data, lznt1 for data that changes). Several folders can share one card. Shows the user a card; stop after calling it."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": with(json!({
            "paths": path_schema(),
            "format": {"type": "string", "enum": FORMATS}
        }), common_props()), "required": ["paths", "format"]})
    }

    fn kind(&self) -> Kind {
        Kind::Propose
    }

    fn small(&self) -> bool {
        true
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: CompressArgs = parse(args)?;
        let format = a.format.trim().to_lowercase();
        if !FORMATS.contains(&format.as_str()) {
            return Err(ToolError::BadArgs(format!(
                "format must be one of {}, got {:?}",
                FORMATS.join(", "),
                a.format
            )));
        }
        let items = entry_items(ctx, &a.paths, a.verdict.as_deref(), a.reason.as_deref())?;
        let s = one_line(a.reason.as_deref()).unwrap_or_else(|| format!("Compress with {format}"));
        let mut req = new_proposal(ctx, ProposalKind::Compress, s, items);
        req.format = Some(format);
        submit(ctx, req)
    }
}

#[derive(Deserialize)]
struct DedupeArgs {
    #[serde(default, deserialize_with = "opt_u64")]
    group: Option<u64>,
    keep: String,
    mode: String,
    #[serde(default, deserialize_with = "opt_string")]
    reason: Option<String>,
}

impl Tool for ProposeDedupe {
    fn name(&self) -> &'static str {
        "propose_dedupe"
    }

    fn description(&self) -> &'static str {
        "Propose removing the extra copies in a duplicate group, keeping one: hardlink (same volume, keeps every path working) or delete. Shows the user a card; stop after calling it."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {
            "group": {"type": "integer", "description": "Duplicate group id"},
            "keep": {"type": "string", "description": "The copy to keep"},
            "mode": {"type": "string", "enum": ["hardlink", "delete"]},
            "reason": {"type": "string", "description": "One line: why"}
        }, "required": ["group", "keep", "mode"]})
    }

    fn kind(&self) -> Kind {
        Kind::Propose
    }

    fn small(&self) -> bool {
        true
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: DedupeArgs = parse(args)?;
        let group = a
            .group
            .ok_or_else(|| ToolError::BadArgs("group is required".into()))?;
        let mode = a.mode.trim().to_lowercase();
        if !matches!(mode.as_str(), "hardlink" | "delete") {
            return Err(ToolError::BadArgs(format!(
                "mode must be \"hardlink\" or \"delete\", got {:?}",
                a.mode
            )));
        }
        let keep = abs_path(ctx, &a.keep)?;
        let members = conn(ctx).ok().and_then(|c| {
            let mut st = c
                .prepare("SELECT path FROM dup_member WHERE group_id = ?1 ORDER BY path")
                .ok()?;
            let rows = st
                .query_map([group as i64], |r| r.get::<_, String>(0))
                .ok()?
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            Some(rows)
        });
        let members = members.unwrap_or_default();
        if members.is_empty() {
            return Err(ToolError::Failed(format!(
                "duplicate group {group} is unknown; call duplicates() and use a group id it lists"
            )));
        }
        if !members.iter().any(|p| p.eq_ignore_ascii_case(&keep)) {
            return Err(ToolError::Failed(format!(
                "{keep} is not a member of duplicate group {group}; keep must be one of its copies"
            )));
        }
        let items = members
            .into_iter()
            .filter(|p| !p.eq_ignore_ascii_case(&keep))
            .map(|path| NewItem {
                path,
                verdict: None,
                reason: one_line(a.reason.as_deref()),
            })
            .collect::<Vec<_>>();
        let s = one_line(a.reason.as_deref())
            .unwrap_or_else(|| format!("Remove duplicate copies ({mode}), keep {keep}"));
        let mut req = new_proposal(ctx, ProposalKind::Dedupe, s, items);
        req.dedupe = Some(DedupeParams {
            group_id: group as i64,
            keep,
            mode,
        });
        submit(ctx, req)
    }
}

#[derive(Deserialize)]
struct SystemArgs {
    #[serde(default)]
    operations: Option<Vec<String>>,
}

impl Tool for ProposeSystemCleanup {
    fn name(&self) -> &'static str {
        "propose_system_cleanup"
    }

    fn description(&self) -> &'static str {
        "Propose cleanups only Windows can do: component store cleanup (WinSxS), deleting older System Restore points, the reduced hibernation file, compacting WSL and Docker virtual disks, emptying the Recycle Bin. Only what Hoardini detects on this PC goes on the card; approving shows one Windows admin prompt. Stop after calling it."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {
            "operations": {"type": "array", "items": {"type": "string", "enum": syscleanup::OP_NAMES},
                           "description": "Which cleanups; omit for every one detected"}
        }})
    }

    fn kind(&self) -> Kind {
        Kind::Propose
    }

    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError> {
        let a: SystemArgs = parse(args)?;
        let wanted: Vec<String> = a
            .operations
            .unwrap_or_default()
            .iter()
            .map(|o| o.trim().to_lowercase())
            .collect();
        if let Some(bad) = wanted
            .iter()
            .find(|w| !syscleanup::OP_NAMES.contains(&w.as_str()))
        {
            return Err(ToolError::BadArgs(format!(
                "unknown operation {bad:?}; use {}",
                syscleanup::OP_NAMES.join(", ")
            )));
        }
        let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();
        let p = syscleanup::propose(
            &ctx.db,
            &syscleanup::SystemFacts::gather(),
            &wanted,
            ctx.conversation_id.clone(),
            Some(ctx.tool_call_id.clone()),
        )
        .map_err(|e| ToolError::Failed(format!("the proposal was not created: {e}")))?;
        Ok(Outcome::Proposal { proposal_id: p.id })
    }
}

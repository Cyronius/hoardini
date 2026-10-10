use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{COLS, Kind, Memory, Scope, compile_glob, from_row, get, normalize_key_with};
use crate::paths;
use crate::policy::{Action, Verdict};

pub const MODEL_WEIGHTS: &str = "model-weights";
// `.bin` counts only next to a config.json, which the index cannot see (REVIEW-UNITS checks the folder).
pub const WEIGHT_EXTS: &[&str] = &["gguf", "safetensors", "onnx", "pt", "ckpt"];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Detail {
    #[serde(default)]
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, rename = "use", skip_serializing_if = "Option::is_none")]
    pub use_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessed_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    // Entries belong to job learning (MEM-LEARN-JOB); this module only keeps them and checks emptiness.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cleaned: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept: Vec<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Detail {
    pub fn has_job_content(&self) -> bool {
        !self.cleaned.is_empty() || !self.kept.is_empty()
    }
}

pub fn is_web(source: &str) -> bool {
    source.starts_with("web:")
}

pub fn is_classifier(source: &str) -> bool {
    matches!(source, "deny" | "catalog" | "mix" | "index")
}

pub fn source_rank(source: &str) -> u8 {
    match source {
        "user" => 6,
        "job" => 5,
        "catalog" | "deny" => 4,
        "mix" => 3,
        "agent" | "model" => 1,
        s if s.starts_with("job:") => 5,
        s if s.starts_with("copy:") => 4,
        s if is_web(s) => 2,
        _ => 0,
    }
}

pub fn may_replace(new: &str, old: &str) -> bool {
    (is_classifier(new) && is_classifier(old)) || source_rank(new) >= source_rank(old)
}

#[derive(Clone, Debug)]
pub struct Knowledge {
    pub scope: Scope,
    pub key: String,
    pub kind: Kind,
    pub text: String,
    pub source: String,
    pub detail: Detail,
}

pub(crate) fn knowledge_at(
    conn: &Connection,
    scope: Scope,
    key: &str,
) -> anyhow::Result<Option<Memory>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLS} FROM memory WHERE scope = ?1 AND key = ?2 COLLATE NOCASE AND detail_json IS NOT NULL ORDER BY id LIMIT 1"
            ),
            params![scope.as_str(), key],
            from_row,
        )
        .optional()?)
}

pub fn upsert(conn: &Connection, k: Knowledge) -> anyhow::Result<(Memory, bool)> {
    upsert_with(
        conn,
        k,
        crate::store::now_ms(),
        &paths::profile_dir().to_string_lossy(),
    )
}

pub fn upsert_with(
    conn: &Connection,
    mut k: Knowledge,
    now: i64,
    profile: &str,
) -> anyhow::Result<(Memory, bool)> {
    let key = normalize_key_with(k.scope, &k.key, profile);
    if key.is_empty() || k.text.trim().is_empty() {
        anyhow::bail!("memory needs a key and text");
    }
    if k.scope == Scope::Glob {
        compile_glob(&key)?;
    }
    if is_web(&k.source) {
        k.detail.verdict = None;
        k.detail.actions.clear();
        k.kind = Kind::Fact;
    }
    let pinned = k.source == "user";
    let id = match knowledge_at(conn, k.scope, &key)? {
        Some(old) if !may_replace(&k.source, &old.source) => return Ok((old, false)),
        Some(old) => {
            let prev = old.detail.unwrap_or_default();
            if k.detail.cleaned.is_empty() {
                k.detail.cleaned = prev.cleaned;
            }
            if k.detail.kept.is_empty() {
                k.detail.kept = prev.kept;
            }
            conn.execute(
                "UPDATE memory SET key = ?1, kind = ?2, text = ?3, source = ?4, detail_json = ?5, updated_ms = ?6,
                   last_used_ms = ?6, forgotten_ms = NULL, review_after_ms = NULL, pinned = pinned OR ?7 WHERE id = ?8",
                params![
                    key,
                    k.kind.as_str(),
                    k.text,
                    k.source,
                    serde_json::to_string(&k.detail)?,
                    now,
                    pinned,
                    old.id
                ],
            )?;
            old.id
        }
        None => {
            conn.execute(
                "INSERT INTO memory(scope, key, kind, text, source, created_ms, updated_ms, pinned, last_used_ms, detail_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?6, ?8)",
                params![
                    k.scope.as_str(),
                    key,
                    k.kind.as_str(),
                    k.text,
                    k.source,
                    now,
                    pinned,
                    serde_json::to_string(&k.detail)?
                ],
            )?;
            conn.last_insert_rowid()
        }
    };
    let m = get(conn, id)?.ok_or_else(|| anyhow::anyhow!("memory vanished"))?;
    Ok((m, true))
}

// The user vouches for what a row says the folder or type is; a verdict still needs the catalog, a job or the user's answer.
pub fn confirm(
    conn: &Connection,
    id: i64,
    now: i64,
    profile: &str,
) -> anyhow::Result<Option<Memory>> {
    let Some(m) = get(conn, id)? else {
        return Ok(None);
    };
    let Some(detail) = m.detail else {
        anyhow::bail!("memory {id} is a note; only location and file-type rows can be confirmed");
    };
    let detail = Detail {
        verdict: None,
        actions: Vec::new(),
        ..detail
    };
    let k = Knowledge {
        scope: m.scope,
        key: m.key,
        kind: m.kind,
        text: m.text,
        source: "user".into(),
        detail,
    };
    upsert_with(conn, k, now, profile).map(|(m, _)| Some(m))
}

pub(crate) fn set_detail(conn: &Connection, id: i64, detail: &Detail) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE memory SET detail_json = ?1 WHERE id = ?2",
        params![serde_json::to_string(detail)?, id],
    )?;
    Ok(())
}

fn normalize_ext(ext: &str) -> Option<String> {
    let e = ext.trim();
    let e = e.strip_prefix("**\\").unwrap_or(e);
    let e = e.strip_prefix('*').unwrap_or(e);
    let e = e.strip_prefix('.').unwrap_or(e).to_ascii_lowercase();
    let ok = !e.is_empty()
        && e.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    ok.then_some(e)
}

pub fn filetype_key(ext: &str) -> Option<String> {
    normalize_ext(ext).map(|e| format!("**\\*.{e}"))
}

pub fn remember_filetype(
    conn: &Connection,
    ext: &str,
    text: &str,
    source: &str,
    detail: Detail,
    now: i64,
) -> anyhow::Result<(Memory, bool)> {
    let key =
        filetype_key(ext).ok_or_else(|| anyhow::anyhow!("{ext:?} is not a file extension"))?;
    upsert_with(
        conn,
        Knowledge {
            scope: Scope::Glob,
            key,
            kind: Kind::Fact,
            text: text.to_string(),
            source: source.to_string(),
            detail,
        },
        now,
        &paths::profile_dir().to_string_lossy(),
    )
}

pub fn filetype(conn: &Connection, ext: &str) -> anyhow::Result<Option<Memory>> {
    match filetype_key(ext) {
        Some(key) => knowledge_at(conn, Scope::Glob, &key),
        None => Ok(None),
    }
}

pub fn filetypes(conn: &Connection) -> anyhow::Result<Vec<Memory>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM memory WHERE scope = 'glob' AND detail_json IS NOT NULL AND key LIKE '**\\*.%' ESCAPE '!' ORDER BY key"
    ))?;
    let rows = st.query_map([], from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn weight_extensions(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let mut out: Vec<String> = WEIGHT_EXTS.iter().map(|e| e.to_string()).collect();
    for m in filetypes(conn)? {
        if m.detail
            .as_ref()
            .is_some_and(|d| d.category == MODEL_WEIGHTS)
            && let Some(e) = m.key.rsplit("*.").next()
            && !out.iter().any(|x| x.eq_ignore_ascii_case(e))
        {
            out.push(e.to_ascii_lowercase());
        }
    }
    Ok(out)
}

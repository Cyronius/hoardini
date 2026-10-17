use axum::Json;
use axum::extract::{Query, State};
use hoardini_core::index::{self, NodeKind, normalize_path};
use hoardini_core::paths::{expand, redact};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::{ApiError, ApiResult, AppState, blocking};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 1000;

pub(crate) fn absolute(raw: Option<&str>) -> Result<String, ApiError> {
    let raw = raw
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| ApiError::bad("path is required"))?;
    let p = normalize_path(&expand(raw));
    let b = p.as_bytes();
    // A ".." component would name a different folder than the index row it is looked up as.
    let dotted = p.split('\\').any(|c| c == "." || c == "..");
    if !(b.len() >= 3 && b[1] == b':' && b[2] == b'\\') || p.contains('"') || dotted {
        return Err(ApiError::bad(format!(
            "{} is not an absolute path",
            redact(raw)
        )));
    }
    Ok(p)
}

#[derive(Deserialize)]
pub struct ChildrenQuery {
    path: Option<String>,
    limit: Option<usize>,
}

pub async fn children(State(s): State<AppState>, Query(q): Query<ChildrenQuery>) -> ApiResult {
    let path = absolute(q.path.as_deref())?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    blocking(move || {
        let conn = s.db.conn()?;
        let node = index::node_by_path(&conn, &path)?
            .ok_or_else(|| ApiError::not_found(&format!("{} in the index", redact(&path))))?;
        if node.kind != NodeKind::Dir {
            return Err(ApiError::bad(format!("{} is not a folder", redact(&path))));
        }
        let page = index::child_rows(&conn, node.id, limit)?;
        let rows: Vec<Value> = page
            .items
            .into_iter()
            .map(|c| {
                json!({
                    "path": redact(&c.path),
                    "name": c.name,
                    "kind": c.kind,
                    "size": c.size,
                    "files": c.files,
                    "mtime_ms": c.mtime_ms,
                    "denied": c.denied,
                    "has_children": c.has_children,
                })
            })
            .collect();
        Ok(Json(json!({
            "path": redact(&node.path),
            "size": node.size,
            "files": node.files,
            "children": rows,
            "small": {"count": node.small_count, "bytes": node.small_bytes},
            "more": {"count": page.more_count, "bytes": page.more_bytes},
        })))
    })
    .await
}

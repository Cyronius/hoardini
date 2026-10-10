use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use hoardini_core::explore::{self, MAX_NODES, MapNode};
use hoardini_core::index::{self, NodeInfo, NodeKind};
use hoardini_core::paths::redact;
use hoardini_core::policy::PolicyEnv;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::{ApiError, ApiResult, AppState, blocking, parse};
use crate::tree::absolute;

const DEFAULT_W: u32 = 1200;
const DEFAULT_H: u32 = 800;
const MAX_SIDE: u32 = 8192;

fn folder(conn: &Connection, path: &str) -> Result<NodeInfo, ApiError> {
    let node = index::node_by_path(conn, path)?
        .ok_or_else(|| ApiError::not_found(&format!("{} in the index", redact(path))))?;
    if node.kind != NodeKind::Dir {
        return Err(ApiError::bad(format!("{} is not a folder", redact(path))));
    }
    Ok(node)
}

fn redact_tree(n: &mut MapNode) {
    n.path = redact(&n.path);
    n.tag.label = n.tag.label.as_deref().map(redact);
    for k in n.children.iter_mut().flatten() {
        redact_tree(k);
    }
}

#[derive(Deserialize)]
pub struct MapQuery {
    path: Option<String>,
    w: Option<u32>,
    h: Option<u32>,
}

pub async fn map(State(s): State<AppState>, Query(q): Query<MapQuery>) -> ApiResult {
    let path = absolute(q.path.as_deref())?;
    let w = q.w.unwrap_or(DEFAULT_W).clamp(16, MAX_SIDE);
    let h = q.h.unwrap_or(DEFAULT_H).clamp(16, MAX_SIDE);
    blocking(move || {
        let conn = s.db.conn()?;
        let node = folder(&conn, &path)?;
        let mut m = explore::map(&conn, &PolicyEnv::system(), &node, w, h, MAX_NODES)?;
        redact_tree(&mut m.tree);
        for c in &mut m.ancestors {
            c.path = redact(&c.path);
        }
        Ok(Json(json!(m)))
    })
    .await
}

#[derive(Deserialize)]
pub struct TypesQuery {
    path: Option<String>,
}

pub async fn types(State(s): State<AppState>, Query(q): Query<TypesQuery>) -> ApiResult {
    let path = absolute(q.path.as_deref())?;
    blocking(move || {
        let conn = s.db.conn()?;
        let node = folder(&conn, &path)?;
        Ok(Json(json!(explore::types(&conn, node.id)?)))
    })
    .await
}

#[derive(Deserialize)]
pub struct FocusBody {
    path: Option<String>,
}

pub async fn focus_put(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: FocusBody = parse(&body)?;
    let path = match b.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => Some(absolute(Some(p))?),
        None => None,
    };
    blocking(move || {
        explore::set_focus(&s.db.conn()?, path.as_deref())?;
        Ok(Json(
            json!({"path": path.as_deref().map(redact).map(Value::from).unwrap_or(Value::Null)}),
        ))
    })
    .await
}

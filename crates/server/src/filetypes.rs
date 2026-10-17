use axum::Json;
use axum::extract::{Query, State};
use hoardini_core::filetypes;
use hoardini_core::index::{self, NodeKind};
use hoardini_core::paths::redact;
use serde::Deserialize;
use serde_json::json;

use crate::state::{ApiError, ApiResult, AppState, blocking};
use crate::tree::absolute;

#[derive(Deserialize)]
pub struct TypesQuery {
    path: Option<String>,
}

pub async fn types(State(s): State<AppState>, Query(q): Query<TypesQuery>) -> ApiResult {
    let path = absolute(q.path.as_deref())?;
    blocking(move || {
        let conn = s.db.conn()?;
        let node = index::node_by_path(&conn, &path)?
            .ok_or_else(|| ApiError::not_found(&format!("{} in the index", redact(&path))))?;
        if node.kind != NodeKind::Dir {
            return Err(ApiError::bad(format!("{} is not a folder", redact(&path))));
        }
        Ok(Json(json!(filetypes::types(&conn, node.id)?)))
    })
    .await
}

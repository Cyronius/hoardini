use axum::Json;
use axum::extract::{Query, State};
use hoardini_core::copycheck::{self, CopyResult};
use hoardini_core::paths::{expand, redact};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::{ApiError, ApiResult, AppState, blocking};

fn redact_value(v: &mut Value) {
    match v {
        Value::String(s) => *s = redact(s),
        Value::Array(a) => a.iter_mut().for_each(redact_value),
        Value::Object(o) => o.values_mut().for_each(redact_value),
        _ => {}
    }
}

fn display(r: &CopyResult) -> Value {
    let mut v = json!({
        "check": r.check,
        "locator": r.locator,
        "confidence": r.confidence,
        "checked_ms": r.checked_ms,
        "detail": r.detail,
        "note": r.note(),
    });
    redact_value(&mut v);
    v
}

#[derive(Deserialize)]
pub struct PathQuery {
    path: Option<String>,
}

// Stored answers only; checks run in the background (copywork) and from inspect.
pub async fn get(State(s): State<AppState>, Query(q): Query<PathQuery>) -> ApiResult {
    let raw = q
        .path
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .ok_or_else(|| ApiError::bad("path is required"))?;
    let path = hoardini_core::index::normalize_path(&expand(&raw));
    blocking(move || {
        let conn = s.db.conn()?;
        let results = copycheck::results_for(&conn, &path);
        let only = copycheck::only_copy(&conn, &path);
        let note = copycheck::note_for(&conn, &path);
        Ok(Json(json!({
            "path": redact(&path),
            "results": results.iter().map(display).collect::<Vec<_>>(),
            "only_copy": only,
            "note": note.as_deref().map(redact),
        })))
    })
    .await
}

pub async fn waiting(State(s): State<AppState>) -> ApiResult {
    blocking(move || {
        let conn = s.db.conn()?;
        Ok(Json(
            json!({"online_waiting": copycheck::online_waiting(&conn)}),
        ))
    })
    .await
}

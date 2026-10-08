use std::path::Path;

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::state::{ApiResult, AppState};
use crate::supervisor::indexed_volumes;

fn build_sync(s: &AppState) -> Value {
    let volumes: Vec<Value> = indexed_volumes(&s.db)
        .iter()
        .filter_map(|root| {
            let v = hoardini_win::volume_info(Path::new(root)).ok()?;
            Some(json!({"root": root, "total_bytes": v.total_bytes, "free_bytes": v.free_bytes}))
        })
        .collect();
    let active =
        s.db.conn()
            .ok()
            .and_then(|c| crate::providers::active_summary(&c).ok().flatten());
    let running = s.runs.lock().map(|r| r.len()).unwrap_or(0);
    json!({
        "volumes": volumes,
        "index": s.supervisor.snapshot(),
        "jobs": s.exec.jobs(),
        "vanished_total_bytes": s.exec.vanished_total(),
        "active_provider": active,
        "running_runs": running,
    })
}

pub async fn build_status(s: &AppState) -> Value {
    let st = s.clone();
    tokio::task::spawn_blocking(move || build_sync(&st))
        .await
        .unwrap_or(Value::Null)
}

pub async fn get_status(State(s): State<AppState>) -> ApiResult {
    Ok(Json(build_status(&s).await))
}

use std::path::Path;

use axum::Json;
use axum::extract::State;
use hoardini_core::actions::Job;
use hoardini_core::paths::redact;
use serde_json::{Value, json};

use crate::state::{ApiResult, AppState};
use crate::supervisor::indexed_volumes;

// Paths shown to the UI use `~` for the profile, like proposals do.
pub fn display_job(mut job: Job) -> Job {
    job.progress.current = job.progress.current.as_deref().map(redact);
    job.error = job.error.as_deref().map(redact);
    job
}

fn build_sync(s: &AppState) -> Value {
    let mut volumes: Vec<Value> = Vec::new();
    for root in indexed_volumes(&s.db) {
        let Ok(v) = hoardini_win::volume_info(Path::new(&root)) else {
            continue;
        };
        if !volumes.iter().any(|x| x["root"] == v.root.as_str()) {
            volumes.push(
                json!({"root": v.root, "total_bytes": v.total_bytes, "free_bytes": v.free_bytes}),
            );
        }
    }
    let active =
        s.db.conn()
            .ok()
            .and_then(|c| crate::providers::active_summary(&c).ok().flatten());
    let running = s.runs.lock().map(|r| r.len()).unwrap_or(0);
    json!({
        "volumes": volumes,
        "index": s.supervisor.snapshot(),
        "jobs": s.exec.jobs().into_iter().map(display_job).collect::<Vec<_>>(),
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

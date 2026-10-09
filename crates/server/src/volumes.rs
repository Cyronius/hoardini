use std::path::Path;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use hoardini_core::index::{self, normalize_path};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::{ApiError, ApiResult, AppState, blocking, parse};

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

// An unplugged drive reports what its last refresh saw.
pub fn entry(conn: &Connection, root: &str, watched: bool, selected: bool) -> Value {
    let live = index::connected(root)
        .then(|| hoardini_win::volume_info(Path::new(root)).ok())
        .flatten();
    let (label, fs, total, free, connected) = match live {
        Some(v) => (v.label, v.fs_name, v.total_bytes, v.free_bytes, true),
        None => {
            let st = index::stored_volume(conn, root)
                .ok()
                .flatten()
                .unwrap_or_default();
            let (label, fs) = (st.label.unwrap_or_default(), st.fs_name.unwrap_or_default());
            (label, fs, st.total_bytes, st.free_bytes, false)
        }
    };
    let has_journal = fs.eq_ignore_ascii_case("NTFS") || fs.eq_ignore_ascii_case("ReFS");
    json!({
        "root": root,
        "label": label,
        "fs": fs,
        "total_bytes": total,
        "free_bytes": free,
        "watched": watched,
        "selected": selected,
        "connected": connected,
        "has_journal": has_journal,
    })
}

pub fn watched_entries(conn: &Connection) -> Vec<Value> {
    let selected = index::selected(conn);
    index::watched(conn)
        .iter()
        .map(|w| entry(conn, w, true, same(w, &selected)))
        .collect()
}

fn all_entries(conn: &Connection) -> Vec<Value> {
    let watched = index::watched(conn);
    let selected = index::selected(conn);
    let mut roots: Vec<String> = hoardini_win::fixed_volumes()
        .unwrap_or_default()
        .iter()
        .filter(|p| !hoardini_win::is_subst(p))
        .map(|p| normalize_path(&p.to_string_lossy()))
        .collect();
    for w in &watched {
        if !roots.iter().any(|r| same(r, w)) {
            roots.push(w.clone());
        }
    }
    roots
        .iter()
        .map(|r| {
            let on = watched.iter().any(|w| same(w, r));
            entry(conn, r, on, same(r, &selected))
        })
        .collect()
}

pub async fn list(State(s): State<AppState>) -> ApiResult {
    blocking(move || Ok(Json(json!(all_entries(&s.db.conn()?))))).await
}

#[derive(Deserialize)]
struct PutBody {
    watched: Vec<String>,
    selected: String,
}

pub async fn put(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: PutBody = parse(&body)?;
    let edit = s.supervisor.volume_edits.lock().await;
    let st = s.clone();
    let change = blocking(move || {
        let conn = st.db.conn()?;
        let current = index::watched(&conn);
        let c = index::plan_change(&current, &b.watched, &b.selected, &index::connected)
            .map_err(ApiError::bad)?;
        index::store_change(&conn, &c)?;
        Ok(c)
    })
    .await?;
    // A child that does not stop leaves the delete to finish_forgetting at the next start.
    if !change.removed.is_empty() && s.supervisor.release(&change.removed).await {
        let st = s.clone();
        let removed = change.removed.clone();
        blocking(move || {
            let conn = st.db.conn()?;
            for root in &removed {
                let nodes = index::forget(&conn, root)?;
                tracing::info!(
                    volume = root,
                    nodes,
                    "stopped watching; its index is deleted"
                );
            }
            Ok(())
        })
        .await?;
    }
    if let Err(e) = s.supervisor.volumes_added(&change.added) {
        tracing::warn!(error = %e, "first scan of an added drive did not start");
    }
    s.bump();
    drop(edit);
    list(State(s)).await
}

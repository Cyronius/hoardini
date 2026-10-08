use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::handler::Handler;
use axum::http::StatusCode;
use axum::routing::{MethodRouter, delete, get, post, put};
use hoardini_core::actions::Job;
use hoardini_core::destinations::{self, NewDestination};
use hoardini_core::history::{self, Since};
use hoardini_core::index;
use hoardini_core::memory::{self, Kind, MemoryPatch, NewMemory, Scope};
use hoardini_core::paths::redact;
use hoardini_core::proposals::{self, Proposal};
use hoardini_core::store::{get_meta, set_meta};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::state::{ApiError, ApiResult, AppState, blocking, parse};
use crate::{providers, status, stream};

pub struct Entry {
    pub method: &'static str,
    pub path: &'static str,
    route: MethodRouter<AppState>,
}

fn get_<H, T>(path: &'static str, h: H) -> Entry
where
    H: Handler<T, AppState>,
    T: 'static,
{
    Entry {
        method: "GET",
        path,
        route: get(h),
    }
}

fn post_<H, T>(path: &'static str, h: H) -> Entry
where
    H: Handler<T, AppState>,
    T: 'static,
{
    Entry {
        method: "POST",
        path,
        route: post(h),
    }
}

fn put_<H, T>(path: &'static str, h: H) -> Entry
where
    H: Handler<T, AppState>,
    T: 'static,
{
    Entry {
        method: "PUT",
        path,
        route: put(h),
    }
}

fn delete_<H, T>(path: &'static str, h: H) -> Entry
where
    H: Handler<T, AppState>,
    T: 'static,
{
    Entry {
        method: "DELETE",
        path,
        route: delete(h),
    }
}

// Single source for the router and for the auth test that walks every route.
fn entries() -> Vec<Entry> {
    vec![
        get_("/api/status", status::get_status),
        get_("/api/status/stream", stream::status_stream),
        post_("/api/index/refresh", index_refresh),
        get_("/api/index/summary", index_summary),
        post_("/api/agent/run", stream::agent_run),
        post_("/api/agent/runs/{run_id}/cancel", stream::cancel_run),
        get_("/api/threads", threads_list),
        get_("/api/threads/{id}", thread_get),
        delete_("/api/threads/{id}", thread_delete),
        get_("/api/proposals/{id}", proposal_get),
        post_("/api/proposals/{id}/approve", proposal_approve),
        post_("/api/proposals/{id}/decline", proposal_decline),
        get_("/api/jobs", jobs_list),
        post_("/api/jobs/{id}/cancel", job_cancel),
        get_("/api/memory", memory_list),
        post_("/api/memory", memory_create),
        put_("/api/memory/{id}", memory_update),
        delete_("/api/memory/{id}", memory_delete),
        get_("/api/providers", providers::list),
        post_("/api/providers", providers::create),
        get_("/api/providers/presets", providers::preset_list),
        put_("/api/providers/{id}", providers::update),
        delete_("/api/providers/{id}", providers::delete),
        post_("/api/providers/{id}/activate", providers::activate),
        post_("/api/providers/{id}/test", providers::test),
        put_("/api/secrets", providers::put_secret),
        get_("/api/destinations", destinations_list),
        post_("/api/destinations", destination_create),
        put_("/api/destinations/{id}", destination_update),
        delete_("/api/destinations/{id}", destination_delete),
        post_("/api/destinations/{id}/test", destination_test),
        get_("/api/search/providers", providers::search_list),
        put_("/api/search/providers/{kind}", providers::search_put),
        post_(
            "/api/search/providers/{kind}/activate",
            providers::search_activate,
        ),
        post_("/api/search/providers/{kind}/test", providers::search_test),
        get_("/api/history", history_points),
        get_("/api/settings", settings_get),
        put_("/api/settings", settings_put),
        get_("/api/archive", archive_list),
        post_("/api/archive/{id}/restore", archive_restore),
        post_("/api/dialog/pick-folder", pick_folder),
        get_("/api/builtin", builtin_unavailable),
        post_("/api/builtin/install", builtin_unavailable),
        delete_("/api/builtin", builtin_unavailable),
    ]
}

// (method, path template) for every registered route.
pub fn route_list() -> Vec<(&'static str, &'static str)> {
    entries().iter().map(|e| (e.method, e.path)).collect()
}

pub fn api_router() -> Router<AppState> {
    let mut by_path: Vec<(&'static str, MethodRouter<AppState>)> = Vec::new();
    for e in entries() {
        match by_path.iter_mut().find(|(p, _)| *p == e.path) {
            Some((_, mr)) => *mr = std::mem::take(mr).merge(e.route),
            None => by_path.push((e.path, e.route)),
        }
    }
    by_path
        .into_iter()
        .fold(Router::new(), |r, (p, mr)| r.route(p, mr))
}

#[derive(Deserialize)]
struct RefreshBody {
    mode: String,
}

async fn index_refresh(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: RefreshBody = parse(&body)?;
    let started = s
        .supervisor
        .start_refresh(&b.mode)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    Ok(Json(json!({"started": started})))
}

async fn index_summary(State(s): State<AppState>) -> ApiResult {
    blocking(move || {
        let conn = s.db.conn()?;
        let Some(st) = index::index_status(&conn, None)?.filter(|s| s.last_refresh_ms.is_some())
        else {
            return Err(ApiError::not_found("index"));
        };
        let ch = history::changes(&conn, Some(st.volume_id), Since::Previous, 10)?;
        let (free_delta, movers) = match ch {
            Some(c) => {
                let mut all: Vec<_> = c
                    .grew
                    .iter()
                    .chain(&c.shrank)
                    .chain(&c.new)
                    .chain(&c.removed)
                    .collect();
                all.sort_by_key(|m| std::cmp::Reverse(m.delta.unsigned_abs()));
                let movers: Vec<Value> = all
                    .iter()
                    .take(5)
                    .map(|m| json!({"path": redact(&m.path), "delta_bytes": m.delta}))
                    .collect();
                (c.free_delta.unwrap_or(0), movers)
            }
            None => (0, Vec::new()),
        };
        Ok(Json(json!({
            "age_ms": st.age_ms.unwrap_or(0),
            "free_delta_bytes": free_delta,
            "movers": movers,
            "duplicate_hint": Value::Null,
        })))
    })
    .await
}

async fn threads_list(State(s): State<AppState>) -> ApiResult {
    blocking(move || {
        let conn = s.db.conn()?;
        let mut q = conn.prepare(
            "SELECT c.id, c.title, c.updated_ms,
                    (SELECT json FROM message m WHERE m.conversation_id = c.id AND m.role = 'user' ORDER BY seq LIMIT 1)
             FROM conversation c ORDER BY c.updated_ms DESC",
        )?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let out: Vec<Value> = rows
            .into_iter()
            .map(|(id, title, updated_ms, first)| {
                let title = title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| {
                    first
                        .and_then(|j| serde_json::from_str::<Value>(&j).ok())
                        .and_then(|v| v.get("content").and_then(Value::as_str).map(str::to_string))
                        .map(|t| t.chars().take(60).collect::<String>())
                        .unwrap_or_else(|| "New chat".into())
                });
                json!({"id": id, "title": title, "updated_ms": updated_ms})
            })
            .collect();
        Ok(Json(json!(out)))
    })
    .await
}

fn text_of(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(""),
        ),
        _ => None,
    }
}

fn to_agui(id: &str, raw: &str) -> Option<Value> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let role = v.get("role")?.as_str()?;
    if role == "system" {
        return None;
    }
    let mut out = Map::new();
    out.insert("id".into(), json!(id));
    out.insert("role".into(), json!(role));
    if let Some(t) = v.get("content").and_then(text_of) {
        out.insert("content".into(), json!(t));
    }
    if let Some(calls) = v
        .get("tool_calls")
        .or_else(|| v.get("toolCalls"))
        .and_then(Value::as_array)
    {
        let calls: Vec<Value> = calls
            .iter()
            .map(|c| {
                let f = c.get("function").cloned().unwrap_or(Value::Null);
                let args = match f.get("arguments") {
                    Some(Value::String(a)) => a.clone(),
                    Some(other) => other.to_string(),
                    None => "{}".into(),
                };
                json!({
                    "id": c.get("id").cloned().unwrap_or(Value::Null),
                    "type": "function",
                    "function": {"name": f.get("name").cloned().unwrap_or(Value::Null), "arguments": args},
                })
            })
            .collect();
        if !calls.is_empty() {
            out.insert("toolCalls".into(), json!(calls));
        }
    }
    if let Some(t) = v
        .get("tool_call_id")
        .or_else(|| v.get("toolCallId"))
        .and_then(Value::as_str)
    {
        out.insert("toolCallId".into(), json!(t));
    }
    Some(Value::Object(out))
}

fn display_proposal(mut p: Proposal) -> Value {
    p.summary = redact(&p.summary);
    for it in &mut p.items {
        it.path = redact(&it.path);
        it.detail = it.detail.as_deref().map(redact);
        it.reason = it.reason.as_deref().map(redact);
        it.block_reason = it.block_reason.as_deref().map(redact);
    }
    let mut v = serde_json::to_value(&p).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.remove("conversation_id");
        o.remove("tool_call_id");
    }
    v
}

async fn thread_get(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult {
    blocking(move || {
        let conn = s.db.conn()?;
        let head: Option<(Option<String>, i64)> = {
            use rusqlite::OptionalExtension;
            conn.query_row(
                "SELECT title, updated_ms FROM conversation WHERE id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
        };
        let Some((title, _)) = head else {
            return Err(ApiError::not_found("thread"));
        };
        let mut q =
            conn.prepare("SELECT id, json FROM message WHERE conversation_id = ?1 ORDER BY seq")?;
        let msgs: Vec<Value> = q
            .query_map([&id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .filter_map(Result::ok)
            .filter_map(|(mid, raw)| to_agui(&mid, &raw))
            .collect();
        let props = match proposals::list_for_conversation(&s.db, &id) {
            Ok(list) => list,
            Err(e) => {
                tracing::warn!(error = %e, "listing proposals for thread failed");
                Vec::new()
            }
        };
        let mut map = Map::new();
        for p in props {
            map.insert(p.id.clone(), display_proposal(p));
        }
        Ok(Json(json!({
            "id": id,
            "title": title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "New chat".into()),
            "messages": msgs,
            "proposals": map,
        })))
    })
    .await
}

async fn thread_delete(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult {
    blocking(move || {
        let conn = s.db.conn()?;
        conn.execute("DELETE FROM message WHERE conversation_id = ?1", [&id])?;
        if conn.execute("DELETE FROM conversation WHERE id = ?1", [&id])? == 0 {
            return Err(ApiError::not_found("thread"));
        }
        Ok(Json(json!({"ok": true})))
    })
    .await
}

async fn proposal_get(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult {
    blocking(move || {
        let p = proposals::get(&s.db, &id)?.ok_or_else(|| ApiError::not_found("proposal"))?;
        Ok(Json(display_proposal(p)))
    })
    .await
}

#[derive(Deserialize)]
struct ApproveBody {
    #[serde(default)]
    item_ids: Vec<i64>,
}

fn classify(e: anyhow::Error) -> ApiError {
    let msg = format!("{e:#}");
    let code = if msg.to_ascii_lowercase().contains("not found") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::CONFLICT
    };
    ApiError::new(code, msg)
}

async fn proposal_approve(
    State(s): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult {
    let b: ApproveBody = parse(&body)?;
    if b.item_ids.is_empty() {
        return Err(ApiError::bad("nothing selected"));
    }
    let st = s.clone();
    let res = blocking(move || st.exec.approve(&id, &b.item_ids).map_err(classify)).await?;
    s.bump();
    Ok(Json(json!(res)))
}

async fn proposal_decline(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let st = s.clone();
    blocking(move || proposals::decline(&st.db, &id).map_err(classify)).await?;
    s.bump();
    Ok(Json(json!({"approved": false})))
}

async fn jobs_list(State(s): State<AppState>) -> ApiResult {
    let jobs: Vec<Job> = s.exec.jobs();
    Ok(Json(json!(jobs)))
}

async fn job_cancel(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult {
    if s.exec.cancel(&id) {
        Ok(Json(json!({"ok": true})))
    } else {
        Err(ApiError::not_found("running job"))
    }
}

#[derive(Deserialize)]
struct QueryQ {
    q: Option<String>,
}

async fn memory_list(State(s): State<AppState>, Query(q): Query<QueryQ>) -> ApiResult {
    blocking(move || {
        let conn = s.db.conn()?;
        let rows = match q.q.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => memory::recall_text(&conn, t)?,
            None => memory::list(&conn)?,
        };
        Ok(Json(json!(rows)))
    })
    .await
}

#[derive(Deserialize)]
struct MemoryNew {
    scope: Scope,
    key: String,
    kind: Kind,
    text: String,
}

async fn memory_create(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: MemoryNew = parse(&body)?;
    blocking(move || {
        let conn = s.db.conn()?;
        let m = memory::remember(
            &conn,
            NewMemory::new(b.scope, &b.key, b.kind, &b.text, "user"),
        )
        .map_err(|e| ApiError::bad(format!("{e:#}")))?;
        Ok(Json(json!(m)))
    })
    .await
}

#[derive(Deserialize)]
struct MemoryEdit {
    text: Option<String>,
    kind: Option<Kind>,
    key: Option<String>,
    scope: Option<Scope>,
}

async fn memory_update(State(s): State<AppState>, Path(id): Path<i64>, body: Bytes) -> ApiResult {
    let b: MemoryEdit = parse(&body)?;
    blocking(move || {
        let conn = s.db.conn()?;
        let patch = MemoryPatch {
            text: b.text,
            kind: b.kind,
            key: b.key,
            scope: b.scope,
            ..MemoryPatch::default()
        };
        let m = memory::update(&conn, id, patch)
            .map_err(|e| ApiError::bad(format!("{e:#}")))?
            .ok_or_else(|| ApiError::not_found("memory"))?;
        Ok(Json(json!(m)))
    })
    .await
}

async fn memory_delete(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    blocking(move || {
        if memory::delete(&s.db.conn()?, id)? {
            Ok(Json(json!({"ok": true})))
        } else {
            Err(ApiError::not_found("memory"))
        }
    })
    .await
}

async fn destinations_list(State(s): State<AppState>) -> ApiResult {
    blocking(move || Ok(Json(json!(destinations::list(&s.db.conn()?)?)))).await
}

async fn destination_create(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: NewDestination = parse(&body)?;
    blocking(move || Ok(Json(json!(destinations::create(&s.db.conn()?, b)?)))).await
}

async fn destination_update(
    State(s): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult {
    let b: NewDestination = parse(&body)?;
    blocking(move || Ok(Json(json!(destinations::update(&s.db.conn()?, &id, b)?)))).await
}

async fn destination_delete(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult {
    blocking(move || {
        if destinations::delete(&s.db.conn()?, &id)? {
            Ok(Json(json!({"ok": true})))
        } else {
            Err(ApiError::not_found("destination"))
        }
    })
    .await
}

async fn destination_test(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult {
    blocking(move || {
        let d = destinations::get(&s.db.conn()?, &id)?
            .ok_or_else(|| ApiError::not_found("destination"))?;
        let r = destinations::test(&d, s.secrets.as_ref());
        let mut o = Map::new();
        o.insert("ok".into(), json!(r.ok));
        if let Some(f) = r.free_bytes {
            o.insert("free_bytes".into(), json!(f));
        }
        if let Some(e) = r.error {
            o.insert("error".into(), json!(e));
        }
        Ok(Json(Value::Object(o)))
    })
    .await
}

async fn history_points(State(s): State<AppState>) -> ApiResult {
    blocking(move || {
        let conn = s.db.conn()?;
        let Some(st) = index::index_status(&conn, None)? else {
            return Ok(Json(json!([])));
        };
        let mut scans = history::scans(&conn, st.volume_id)?;
        scans.reverse();
        let pts: Vec<Value> = scans
            .iter()
            .map(|c| {
                json!({
                    "finished_ms": c.finished_ms,
                    "total_bytes": c.total_bytes.unwrap_or(0),
                    "free_bytes": c.free_bytes.unwrap_or(0),
                    "used_bytes": c.used_bytes.unwrap_or(0),
                    "indexed_bytes": c.indexed_bytes,
                })
            })
            .collect();
        Ok(Json(json!(pts)))
    })
    .await
}

fn meta_u64(conn: &rusqlite::Connection, key: &str, default: u64) -> u64 {
    get_meta(conn, key)
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn meta_bool(conn: &rusqlite::Connection, key: &str, default: bool) -> bool {
    match get_meta(conn, key).ok().flatten().as_deref().map(str::trim) {
        Some("true" | "1") => true,
        Some("false" | "0") => false,
        _ => default,
    }
}

fn settings_value(s: &AppState) -> anyhow::Result<Value> {
    let conn = s.db.conn()?;
    let size = |p: std::path::PathBuf| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let db = s.db.path().to_path_buf();
    let mut wal = db.clone().into_os_string();
    wal.push("-wal");
    Ok(json!({
        "threshold_bytes": meta_u64(&conn, "threshold_bytes", index::DEFAULT_THRESHOLD),
        "redact_paths": meta_bool(&conn, "redact_paths", true),
        "web_search_enabled": meta_bool(&conn, "web_search_enabled", false),
        "indexer_workers": meta_u64(&conn, "indexer_workers", 2),
        "indexer_memory_cap_mb": meta_u64(&conn, "indexer_memory_cap_mb", 256),
        "builtin_idle_minutes": meta_u64(&conn, "builtin_idle_minutes", 10),
        "index_size_bytes": size(db) + size(wal.into()),
    }))
}

async fn settings_get(State(s): State<AppState>) -> ApiResult {
    blocking(move || Ok(Json(settings_value(&s)?))).await
}

async fn settings_put(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let patch: Map<String, Value> = parse(&body)?;
    let res = blocking({
        let s = s.clone();
        move || {
            let conn = s.db.conn()?;
            for (k, v) in &patch {
                match k.as_str() {
                    "redact_paths" | "web_search_enabled" => {
                        let b = v
                            .as_bool()
                            .ok_or_else(|| ApiError::bad(format!("{k} must be a boolean")))?;
                        set_meta(&conn, k, &b.to_string())?;
                    }
                    "threshold_bytes"
                    | "indexer_workers"
                    | "indexer_memory_cap_mb"
                    | "builtin_idle_minutes" => {
                        let n = v.as_u64().ok_or_else(|| {
                            ApiError::bad(format!("{k} must be a non-negative integer"))
                        })?;
                        if n == 0 && k != "builtin_idle_minutes" {
                            return Err(ApiError::bad(format!("{k} must be at least 1")));
                        }
                        set_meta(&conn, k, &n.to_string())?;
                    }
                    _ => {}
                }
            }
            Ok(Json(settings_value(&s)?))
        }
    })
    .await?;
    s.bump();
    Ok(res)
}

async fn archive_list(State(s): State<AppState>, Query(q): Query<QueryQ>) -> ApiResult {
    blocking(move || {
        let conn = s.db.conn()?;
        let like =
            q.q.as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(|t| {
                    let esc = t
                        .replace('\\', "\\\\")
                        .replace('%', "\\%")
                        .replace('_', "\\_");
                    format!("%{esc}%")
                });
        let mut st = conn.prepare(
            "SELECT id, original_path, destination_id, dest_uri, size, archived_ms, restored_ms
             FROM archive_entry
             WHERE ?1 IS NULL OR original_path LIKE ?1 ESCAPE '\\' OR dest_uri LIKE ?1 ESCAPE '\\'
             ORDER BY archived_ms DESC, id DESC",
        )?;
        let rows: Vec<Value> = st
            .query_map([like], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "original_path": redact(&r.get::<_, String>(1)?),
                    "destination_id": r.get::<_, String>(2)?,
                    "dest_uri": r.get::<_, String>(3)?,
                    "size": r.get::<_, i64>(4)?,
                    "archived_ms": r.get::<_, i64>(5)?,
                    "restored_ms": r.get::<_, Option<i64>>(6)?,
                }))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Json(json!(rows)))
    })
    .await
}

async fn archive_restore(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let st = s.clone();
    let job_id = blocking(move || st.exec.restore(id).map_err(classify)).await?;
    s.bump();
    Ok(Json(json!({"job_id": job_id})))
}

async fn pick_folder(State(s): State<AppState>) -> ApiResult {
    let Some(d) = s.dialogs.clone() else {
        return Err(ApiError::unimplemented(
            "no native dialogs in headless mode",
        ));
    };
    let path = d.pick_folder().await;
    Ok(Json(
        json!({"path": path.map(|p| p.to_string_lossy().into_owned())}),
    ))
}

async fn builtin_unavailable() -> ApiResult {
    Err(ApiError::unimplemented(
        "the built-in model is not available yet",
    ))
}

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::handler::Handler;
use axum::http::StatusCode;
use axum::routing::{MethodRouter, delete, get, post, put};
use hoardini_agent::thread;
use hoardini_core::actions::Job;
use hoardini_core::apps;
use hoardini_core::copycheck::consent;
use hoardini_core::destinations::{self, NewDestination};
use hoardini_core::history::{self, Since};
use hoardini_core::index;
use hoardini_core::memory::{self, Kind, MemoryPatch, NewMemory, Scope};
use hoardini_core::paths::redact;
use hoardini_core::proposals::{self, Proposal};
use hoardini_core::store::{get_meta, now_ms, set_meta};
use hoardini_core::syscleanup::SystemOp;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::state::{ApiError, ApiResult, AppState, blocking, parse};
use crate::supervisor::index_speed;
use crate::{
    builtin, copychecks, explore, providers, rclone, review, status, stream, tree, volumes,
};

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
        post_("/api/presence", presence),
        get_("/api/volumes", volumes::list),
        put_("/api/volumes", volumes::put),
        get_("/api/index/summary", index_summary),
        get_("/api/index/children", tree::children),
        get_("/api/index/map", explore::map),
        get_("/api/index/types", explore::types),
        put_("/api/explore/focus", explore::focus_put),
        post_("/api/reveal", tree::reveal),
        post_("/api/agent/run", stream::agent_run),
        post_("/api/agent/runs/{run_id}/cancel", stream::cancel_run),
        get_("/api/thread", thread_get),
        post_("/api/thread/fresh", thread_fresh),
        delete_("/api/thread", thread_clear),
        get_("/api/proposals/{id}", proposal_get),
        post_("/api/proposals/{id}/approve", proposal_approve),
        post_("/api/proposals/{id}/decline", proposal_decline),
        delete_(
            "/api/proposals/{id}/learned/{memory_id}",
            proposal_learned_undo,
        ),
        put_("/api/proposals/{id}/selection", proposal_select),
        get_("/api/jobs", jobs_list),
        post_("/api/jobs/{id}/cancel", job_cancel),
        get_("/api/memory", memory_list),
        post_("/api/memory", memory_create),
        get_("/api/memory/limit", memory_limit_get),
        put_("/api/memory/limit", memory_limit_put),
        put_("/api/memory/{id}", memory_update),
        post_("/api/memory/{id}/restore", memory_restore),
        post_("/api/memory/{id}/confirm", memory_confirm),
        delete_("/api/memory/{id}", memory_delete),
        delete_("/api/learned/web/{memory_id}", learned_web_undo),
        get_("/api/providers", providers::list),
        post_("/api/providers", providers::create),
        get_("/api/providers/presets", providers::preset_list),
        get_("/api/providers/local", providers::local_servers),
        get_("/api/setup", providers::setup_get),
        post_("/api/setup/done", providers::setup_done),
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
        post_(
            "/api/destinations/rclone/authorize",
            rclone::authorize_remote,
        ),
        get_("/api/runtime/rclone", rclone::status),
        post_("/api/runtime/rclone/install", rclone::install),
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
        get_("/api/copy-checks", copychecks::get),
        get_("/api/copy-checks/waiting", copychecks::waiting),
        get_("/api/archive", archive_list),
        post_("/api/archive/{id}/restore", archive_restore),
        post_("/api/dialog/pick-folder", pick_folder),
        get_("/api/builtin", builtin_get),
        post_("/api/builtin/install", builtin_install),
        delete_("/api/builtin", builtin_delete),
        get_("/api/apps/rarely-used", apps_rarely_used),
        post_("/api/apps/{id}/uninstall", app_uninstall),
        post_("/api/review/open", review::open),
        post_("/api/review/run", review::run),
        get_("/api/review", review::get),
        post_("/api/review/look-closer", review::look_closer),
        post_("/api/locations/answer", review::answer),
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

async fn index_refresh(State(s): State<AppState>) -> ApiResult {
    let st = s.clone();
    let mode = blocking(move || Ok(st.supervisor.auto_mode())).await?;
    let started = s
        .supervisor
        .start_refresh(mode)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    Ok(Json(json!({"started": started})))
}

#[derive(Deserialize)]
struct PresenceBody {
    looking: bool,
}

async fn presence(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: PresenceBody = parse(&body)?;
    s.supervisor.set_looking(b.looking);
    Ok(Json(json!({"ok": true})))
}

async fn index_summary(State(s): State<AppState>) -> ApiResult {
    blocking(move || {
        let conn = s.db.conn()?;
        let selected = index::selected(&conn);
        let Some(st) =
            index::index_status(&conn, Some(&selected))?.filter(|s| s.last_refresh_ms.is_some())
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
            "duplicate_hint": hoardini_core::dupes::hint(&conn).map(|(bytes, count)| json!({"bytes": bytes, "count": count})),
        })))
    })
    .await
}

pub(crate) fn display_proposal(mut p: Proposal) -> Value {
    p.summary = redact(&p.summary);
    for it in &mut p.items {
        it.path = redact(&it.path);
        it.detail = it.detail.as_deref().map(redact);
        it.reason = it.reason.as_deref().map(redact);
        it.block_reason = it.block_reason.as_deref().map(redact);
        it.memory_note = it.memory_note.as_deref().map(redact);
        it.copy_note = it.copy_note.as_deref().map(redact);
        if let Some(op) = it.op.as_mut() {
            if let SystemOp::CompactVhdx { path } = &mut op.op {
                *path = redact(path);
            }
            for part in &mut op.parts {
                part.path = redact(&part.path);
            }
        }
    }
    for d in &mut p.dropped {
        d.path = redact(&d.path);
        d.reason = redact(&d.reason);
    }
    for l in &mut p.learned {
        l.text = redact(&l.text);
    }
    let mut v = serde_json::to_value(&p).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.remove("conversation_id");
        o.remove("tool_call_id");
    }
    v
}

#[derive(Deserialize)]
struct ThreadQuery {
    #[serde(default)]
    all: Option<u8>,
}

async fn thread_get(State(s): State<AppState>, Query(q): Query<ThreadQuery>) -> ApiResult {
    blocking(move || {
        let profile = hoardini_core::paths::profile_dir()
            .to_string_lossy()
            .into_owned();
        let conn = s.db.conn()?;
        let cleared = thread::view_start(&conn);
        let showing_all = q.all.is_some_and(|a| a != 0);
        let from = if showing_all { 0 } else { cleared };
        let msgs = thread::agui_messages_from(&s.db, thread::MAIN, Some(&profile), from)?;
        let view = thread::view_from(&conn, thread::MAIN, now_ms(), from)?;
        let earlier = thread::count_before(&conn, thread::MAIN, cleared)?;
        let props = match proposals::list_for_conversation(&s.db, thread::MAIN) {
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
            "id": thread::MAIN,
            "messages": msgs,
            "proposals": map,
            "window_start_id": view.window_start_id,
            "has_hidden": view.has_hidden,
            "fresh": view.fresh,
            "earlier": earlier,
            "showing_all": showing_all,
        })))
    })
    .await
}

async fn thread_fresh(State(s): State<AppState>) -> ApiResult {
    blocking(move || {
        thread::clear_screen(&s.db.conn()?, thread::MAIN)?;
        Ok(Json(json!({"ok": true})))
    })
    .await
}

async fn thread_clear(State(s): State<AppState>) -> ApiResult {
    let running = s.runs.lock().map(|r| !r.is_empty()).unwrap_or(true);
    if running {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "a chat run is in progress; stop it first",
        ));
    }
    blocking(move || {
        thread::clear(&mut s.db.conn()?, thread::MAIN)?;
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

async fn proposal_learned_undo(
    State(s): State<AppState>,
    Path((id, memory_id)): Path<(String, i64)>,
) -> ApiResult {
    blocking(move || {
        if memory::learn::undo(&s.db.conn()?, &id, memory_id)? {
            Ok(Json(json!({"ok": true})))
        } else {
            Err(ApiError::not_found("learned entry"))
        }
    })
    .await
}

#[derive(Deserialize)]
struct SelectBody {
    item_ids: Vec<i64>,
}

async fn proposal_select(
    State(s): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult {
    let b: SelectBody = parse(&body)?;
    let st = s.clone();
    // Only an unknown proposal is 404; an item blocked as "Not found" is still a conflict.
    let p = blocking(move || {
        if proposals::get(&st.db, &id)?.is_none() {
            return Err(ApiError::not_found("proposal"));
        }
        proposals::select(&st.db, &id, &b.item_ids)
            .map_err(|e| ApiError::new(StatusCode::CONFLICT, format!("{e:#}")))
    })
    .await?;
    s.bump();
    Ok(Json(display_proposal(p)))
}

async fn jobs_list(State(s): State<AppState>) -> ApiResult {
    let jobs: Vec<Job> = s.exec.jobs().into_iter().map(status::display_job).collect();
    Ok(Json(json!(jobs)))
}

async fn job_cancel(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult {
    if s.exec.cancel(&id) || s.builtin.install.cancel(&id) {
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
            Some(t) => memory::search(&conn, t)?,
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
    pinned: Option<bool>,
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
            pinned: b.pinned,
            ..MemoryPatch::default()
        };
        let m = memory::update(&conn, id, patch)
            .map_err(|e| ApiError::bad(format!("{e:#}")))?
            .ok_or_else(|| ApiError::not_found("memory"))?;
        Ok(Json(json!(m)))
    })
    .await
}

async fn memory_restore(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    blocking(move || {
        let m = memory::restore(&s.db.conn()?, id)?.ok_or_else(|| ApiError::not_found("memory"))?;
        Ok(Json(json!(m)))
    })
    .await
}

async fn memory_confirm(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    blocking(move || {
        let profile = hoardini_core::paths::profile_dir()
            .to_string_lossy()
            .into_owned();
        let conn = s.db.conn()?;
        let found = memory::get(&conn, id)?.ok_or_else(|| ApiError::not_found("memory"))?;
        if found.detail.is_none() {
            return Err(ApiError::bad(format!(
                "memory {id} is a note; only location and file-type rows can be confirmed"
            )));
        }
        let m = memory::knowledge::confirm(&conn, id, now_ms(), &profile)?
            .ok_or_else(|| ApiError::not_found("memory"))?;
        Ok(Json(json!(m)))
    })
    .await
}

#[derive(Deserialize)]
struct LearnedChip {
    key: Option<String>,
    text: String,
}

// A web search's Learned chip: only the row it names, so a reused id never deletes another memory.
async fn learned_web_undo(
    State(s): State<AppState>,
    Path(memory_id): Path<i64>,
    body: Bytes,
) -> ApiResult {
    let b: LearnedChip = parse(&body)?;
    blocking(move || {
        let profile = hoardini_core::paths::profile_dir()
            .to_string_lossy()
            .into_owned();
        let conn = s.db.conn()?;
        if memory::knowledge::forget_learned(&conn, memory_id, b.key.as_deref(), &b.text, &profile)?
        {
            Ok(Json(json!({"ok": true})))
        } else {
            Err(ApiError::not_found("learned entry"))
        }
    })
    .await
}

#[derive(Deserialize)]
struct MemoryLimit {
    limit: usize,
}

async fn memory_limit_get(State(s): State<AppState>) -> ApiResult {
    blocking(move || Ok(Json(json!({"limit": memory::limit(&s.db.conn()?)})))).await
}

async fn memory_limit_put(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: MemoryLimit = parse(&body)?;
    blocking(move || {
        let limit = memory::set_limit(&s.db.conn()?, b.limit)?;
        Ok(Json(json!({"limit": limit})))
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
    blocking(move || {
        let conn = s.db.conn()?;
        let old = destinations::get(&conn, &id)?;
        let d = destinations::update(&conn, &id, b)?;
        if d.kind == "rclone"
            && let Err(e) = destinations::rclone::sync_remote(
                s.secrets.as_ref(),
                old.as_ref().map(|o| &o.config),
                &d.config,
            )
        {
            tracing::warn!(error = %e, destination = %d.name, "updating the rclone remote failed");
        }
        Ok(Json(json!(d)))
    })
    .await
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

#[derive(Deserialize)]
struct HistoryQuery {
    since_ms: Option<i64>,
    until_ms: Option<i64>,
    volume: Option<String>,
}

async fn history_points(State(s): State<AppState>, Query(q): Query<HistoryQuery>) -> ApiResult {
    blocking(move || {
        let conn = s.db.conn()?;
        let volume = q.volume.clone().or_else(|| index::focus(&conn));
        let Some(st) = index::index_status(&conn, volume.as_deref())? else {
            return Ok(Json(json!([])));
        };
        let scans = history::scans_between(&conn, st.volume_id, q.since_ms, q.until_ms)?;
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

fn settings_value(s: &AppState) -> anyhow::Result<Value> {
    let conn = s.db.conn()?;
    let size = |p: std::path::PathBuf| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let db = s.db.path().to_path_buf();
    let mut wal = db.clone().into_os_string();
    wal.push("-wal");
    Ok(json!({
        "index_speed": index_speed(&conn),
        "builtin_idle_minutes": meta_u64(&conn, "builtin_idle_minutes", 10),
        "index_size_bytes": size(db) + size(wal.into()),
        "online_checks": consent::load(&conn),
    }))
}

async fn settings_get(State(s): State<AppState>) -> ApiResult {
    blocking(move || Ok(Json(settings_value(&s)?))).await
}

async fn settings_put(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let patch: Map<String, Value> = parse(&body)?;
    let mut writes: Vec<(&'static str, String)> = Vec::new();
    let online = match patch.get("online_checks") {
        Some(v) => {
            let st = s.clone();
            let current = blocking(move || Ok(consent::load(&st.db.conn()?))).await?;
            Some(consent::patch(current, v).map_err(ApiError::bad)?)
        }
        None => None,
    };
    for (k, v) in &patch {
        match k.as_str() {
            "index_speed" => {
                let sp = v
                    .as_str()
                    .filter(|x| matches!(*x, "lean" | "fast"))
                    .ok_or_else(|| ApiError::bad("index_speed must be \"lean\" or \"fast\""))?;
                writes.push(("index_speed", sp.to_string()));
            }
            "builtin_idle_minutes" => {
                let n = v.as_u64().ok_or_else(|| {
                    ApiError::bad("builtin_idle_minutes must be a non-negative integer")
                })?;
                writes.push(("builtin_idle_minutes", n.to_string()));
            }
            _ => {}
        }
    }
    let speed = writes
        .iter()
        .find(|(k, _)| *k == "index_speed")
        .map(|(_, v)| v.clone());
    let res = blocking({
        let s = s.clone();
        move || {
            let conn = s.db.conn()?;
            for (k, v) in &writes {
                set_meta(&conn, k, v)?;
            }
            if let Some(c) = &online {
                consent::save(&conn, c)?;
            }
            Ok(Json(settings_value(&s)?))
        }
    })
    .await?;
    if let Some(sp) = speed {
        s.supervisor.speed_changed(&sp);
    }
    if online.is_some() {
        s.copy.kick();
    }
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
                    "dest_uri": redact(&r.get::<_, String>(3)?),
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

async fn builtin_get(State(s): State<AppState>) -> ApiResult {
    blocking(move || Ok(Json(builtin::view(&s)?))).await
}

#[derive(Deserialize)]
struct InstallBody {
    model_id: String,
}

async fn builtin_install(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: InstallBody = parse(&body)?;
    let job = builtin::start_install(&s, b.model_id.trim()).await?;
    Ok(Json(json!(job)))
}

async fn builtin_delete(State(s): State<AppState>) -> ApiResult {
    Ok(Json(builtin::remove(&s).await?))
}

async fn apps_rarely_used(State(s): State<AppState>) -> ApiResult {
    blocking(move || {
        let mut list = apps::list_rarely_used(&s.db.conn()?, now_ms())?;
        for a in &mut list {
            a.path = redact(&a.path);
            a.last_run_from = a.last_run_from.as_deref().map(redact);
        }
        Ok(Json(json!(list)))
    })
    .await
}

// The body is never read: the command comes only from the app's own registration.
async fn app_uninstall(Path(id): Path<String>) -> ApiResult {
    let app = blocking(move || {
        apps::uninstall(&id).map_err(|e| ApiError::new(StatusCode::CONFLICT, format!("{e:#}")))
    })
    .await?
    .ok_or_else(|| ApiError::not_found("app"))?;
    Ok(Json(
        json!({"started": true, "id": app.id, "name": app.name}),
    ))
}

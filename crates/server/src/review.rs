use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use hoardini_agent::agui::{AgMessage, RunAgentInput};
use hoardini_agent::thread;
use hoardini_core::apps;
use hoardini_core::copycheck::{self, consent};
use hoardini_core::index::{self, WinLister};
use hoardini_core::memory::location::{self, Answer};
use hoardini_core::paths::redact;
use hoardini_core::policy::PolicyEnv;
use hoardini_core::proposals;
use hoardini_core::review::{self, Opened, Review, Scope};
use hoardini_core::store::now_ms;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::routes::display_proposal;
use crate::state::{ApiError, ApiResult, AppState, blocking, parse};

fn focus(conn: &Connection) -> Result<(i64, String), ApiError> {
    let selected = index::selected(conn);
    index::index_status(conn, Some(&selected))?
        .filter(|s| s.last_refresh_ms.is_some())
        .map(|s| (s.volume_id, s.volume))
        .ok_or_else(|| ApiError::not_found("index"))
}

pub fn env(s: &AppState) -> PolicyEnv {
    match &s.facts {
        Some(f) => PolicyEnv::system().with_facts(f.clone()),
        None => PolicyEnv::system(),
    }
}

fn same_or_under(a: &str, p: &str) -> bool {
    a.eq_ignore_ascii_case(p) || index::is_ancestor_path(a, p)
}

// The review's own scope only, so a folder review lists the apps inside it.
fn apps_under(conn: &Connection, root: &str) -> Vec<Value> {
    let list = match apps::list_rarely_used(conn, now_ms()) {
        Ok(l) => l,
        Err(e) => {
            tracing::debug!(error = %e, "rarely used apps not listed");
            Vec::new()
        }
    };
    list.into_iter()
        .filter(|a| same_or_under(root, &a.path))
        .map(|mut a| {
            a.path = redact(&a.path);
            a.last_run_from = a.last_run_from.as_deref().map(redact);
            json!(a)
        })
        .collect()
}

fn extras(s: &AppState, conn: &Connection, root: &str) -> Value {
    json!({
        "building": s.reviews.building(),
        "apps": apps_under(conn, root),
        "online": {
            "asked": consent::load(conn).asked,
            "waiting": copycheck::online_waiting(conn),
        },
    })
}

fn merged(mut base: Value, more: Value) -> Value {
    if let (Some(b), Value::Object(m)) = (base.as_object_mut(), more) {
        b.extend(m);
    }
    base
}

pub fn view(s: &AppState, r: &Review) -> anyhow::Result<Value> {
    let conn = s.db.conn()?;
    let st = index::index_status(&conn, Some(&r.volume))?;
    let mut props = Map::new();
    for id in review::proposal_ids(&conn, &r.id)? {
        if let Some(p) = proposals::get(&s.db, &id)? {
            props.insert(id, display_proposal(p));
        }
    }
    let areas: Vec<Value> = r
        .areas
        .iter()
        .map(|a| {
            let mut a = a.clone();
            a.path = redact(&a.path);
            a.why_not = redact(&a.why_not);
            serde_json::to_value(a).unwrap_or(Value::Null)
        })
        .collect();
    let base = json!({
        "review": {
            "id": r.id,
            "created_ms": r.created_ms,
            "volume": r.volume,
            "scope_path": r.scope_path.as_deref().map(redact),
        },
        "header": {
            "root": r.volume,
            "used_bytes": st.as_ref().and_then(|x| x.used_bytes),
            "free_bytes": st.as_ref().and_then(|x| x.free_bytes),
            "ready_bytes": review::ready_bytes(&conn, &r.id)?,
        },
        "cards": r.cards,
        "areas": areas,
        "proposals": props,
    });
    Ok(merged(base, extras(s, &conn, r.root())))
}

// Before the drive's first review is built there is only the drive to show.
fn first_view(s: &AppState, volume: &str) -> anyhow::Result<Value> {
    let conn = s.db.conn()?;
    let st = index::index_status(&conn, Some(volume))?;
    let base = json!({
        "review": Value::Null,
        "header": {
            "root": volume,
            "used_bytes": st.as_ref().and_then(|x| x.used_bytes),
            "free_bytes": st.as_ref().and_then(|x| x.free_bytes),
            "ready_bytes": 0,
        },
        "cards": [],
        "areas": [],
        "proposals": {},
    });
    Ok(merged(base, extras(s, &conn, volume)))
}

// Estimates fill in after a build; a folder they make worth compressing rebuilds an untouched review.
fn after_build(s: &AppState, r: &Review) {
    let (st, r) = (s.clone(), r.clone());
    let spawned = std::thread::Builder::new()
        .name("hoardini-compress-estimates".into())
        .spawn(move || {
            let _ = hoardini_win::enter_thread_background_mode();
            let env = env(&st);
            match review::compress_estimates(&st.db, &env, &r, now_ms()) {
                Ok(true) => {
                    let rebuilt = st.reviews.exclusive(|now| {
                        review::rebuild_if_current(&st.db, &env, &WinLister, &r.id, now)
                    });
                    match rebuilt {
                        Ok(Some(n)) => {
                            tracing::info!(review = %n.id, "review rebuilt with compression estimates")
                        }
                        Ok(None) => {}
                        Err(e) => tracing::warn!(error = %e, "review not rebuilt after estimates"),
                    }
                }
                Ok(false) => {}
                Err(e) => tracing::debug!(error = %e, "compression estimates failed"),
            }
        });
    if let Err(e) = spawned {
        tracing::debug!(error = %e, "compression estimates did not start");
    }
}

fn build_volume(s: &AppState, volume_id: i64, now: i64) -> anyhow::Result<()> {
    if let Some(r) = review::build_if_stale(&s.db, &env(s), &WinLister, volume_id, now)? {
        after_build(s, &r);
    }
    Ok(())
}

pub async fn open(State(s): State<AppState>) -> ApiResult {
    let st = s.clone();
    let out = blocking(move || {
        let conn = st.db.conn()?;
        let (vid, volume) = focus(&conn)?;
        let job = st.clone();
        let opened = st
            .reviews
            .open(&conn, vid, move |now| build_volume(&job, vid, now))?;
        Ok(match opened {
            Opened::Ready(r) | Opened::Building(Some(r)) => view(&st, &r)?,
            Opened::Building(None) => first_view(&st, &volume)?,
        })
    })
    .await?;
    s.bump();
    Ok(Json(out))
}

#[derive(Deserialize)]
struct RunBody {
    path: Option<String>,
}

pub async fn run(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: RunBody = parse(&body)?;
    let st = s.clone();
    let out = blocking(move || {
        let env = env(&st);
        let scope = match b.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => {
                let p = index::normalize_path(&env.expand(p));
                if index::node_id_by_path(&st.db.conn()?, &p)?.is_none() {
                    return Err(ApiError::not_found("folder in the index"));
                }
                Scope::Folder(p)
            }
            None => Scope::Volume(focus(&st.db.conn()?)?.0),
        };
        let r = st
            .reviews
            .exclusive(|now| review::build(&st.db, &env, &WinLister, scope, now))?;
        after_build(&st, &r);
        Ok(view(&st, &r)?)
    })
    .await?;
    s.bump();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct GetQuery {
    id: Option<String>,
}

pub async fn get(State(s): State<AppState>, Query(q): Query<GetQuery>) -> ApiResult {
    blocking(move || {
        let conn = s.db.conn()?;
        let r = match q.id {
            Some(id) => review::get(&conn, &id)?,
            None => review::latest(&conn, focus(&conn)?.0, None)?,
        }
        .ok_or_else(|| ApiError::not_found("review"))?;
        Ok(Json(view(&s, &r)?))
    })
    .await
}

// Status carries the selected drive's latest review, so the UI refetches when a rebuild replaces it.
pub fn status_ref(s: &AppState) -> Value {
    let found = s.db.conn().ok().and_then(|c| {
        let (vid, _) = focus(&c).ok()?;
        review::latest(&c, vid, None).ok().flatten()
    });
    match found {
        Some(r) => json!({"id": r.id, "created_ms": r.created_ms}),
        None => Value::Null,
    }
}

pub fn rebuild_after_launch(s: &AppState) -> anyhow::Result<bool> {
    let conn = s.db.conn()?;
    let Ok((vid, _)) = focus(&conn) else {
        return Ok(false);
    };
    let env = env(s);
    let r = s
        .reviews
        .exclusive(|now| review::rebuild_if_untouched(&s.db, &env, &WinLister, vid, now))?;
    if let Some(r) = &r {
        after_build(s, r);
    }
    Ok(r.is_some())
}

#[derive(Deserialize)]
struct AnswerBody {
    path: String,
    answer: Answer,
}

pub async fn answer(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: AnswerBody = parse(&body)?;
    let st = s.clone();
    let out = blocking(move || {
        let env = env(&st);
        let path = index::normalize_path(&env.expand(b.path.trim()));
        if path.is_empty() {
            return Err(ApiError::bad("path is required"));
        }
        let conn = st.db.conn()?;
        let m = location::answer(&conn, &env, &path, b.answer, now_ms())
            .map_err(|e| ApiError::bad(format!("{e:#}")))?;
        review::apply_answer(&conn, &path, &m)?;
        Ok(Json(json!(m)))
    })
    .await?;
    s.bump();
    Ok(out)
}

#[derive(Deserialize)]
struct LookBody {
    path: String,
}

pub async fn look_closer(State(s): State<AppState>, body: Bytes) -> ApiResult<Response> {
    let b: LookBody = parse(&body)?;
    let path = redact(&index::normalize_path(
        &PolicyEnv::system().expand(b.path.trim()),
    ));
    if path.is_empty() {
        return Err(ApiError::bad("path is required"));
    }
    if s.runs.lock().map(|r| !r.is_empty()).unwrap_or(true) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "a chat run is in progress; stop it first",
        ));
    }
    let input = RunAgentInput {
        thread_id: thread::MAIN.into(),
        run_id: uuid::Uuid::now_v7().to_string(),
        messages: vec![AgMessage {
            id: uuid::Uuid::now_v7().to_string(),
            role: "user".into(),
            content: Some(format!(
                "What is {path}? Look inside and tell me whether to keep it, delete it, or move it elsewhere."
            )),
            tool_calls: None,
            tool_call_id: None,
            data: None,
        }],
        ..RunAgentInput::default()
    };
    crate::stream::start_run(s, input).await
}

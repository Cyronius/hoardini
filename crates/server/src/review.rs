use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use hoardini_core::apps;
use hoardini_core::index::{self, WinLister};
use hoardini_core::memory::location::{self, Answer};
use hoardini_core::paths::redact;
use hoardini_core::policy::PolicyEnv;
use hoardini_core::proposals::{self, NewItem, NewProposal, ProposalKind};
use hoardini_core::review::{self, Opened, Review};
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

// Each drive's review lists only the apps installed on it.
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
    Ok(merged(base, extras(s, &conn, &r.volume)))
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

pub async fn run(State(s): State<AppState>) -> ApiResult {
    let st = s.clone();
    let out = blocking(move || {
        let env = env(&st);
        let vid = focus(&st.db.conn()?)?.0;
        let r = st
            .reviews
            .exclusive(|now| review::build(&st.db, &env, &WinLister, vid, now))?;
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
            None => review::latest(&conn, focus(&conn)?.0)?,
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
        review::latest(&c, vid).ok().flatten()
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

// Proposals made by an action on Home, kept apart from the review's.
pub const HOME: &str = "home";

#[derive(Deserialize)]
struct ActBody {
    kind: String,
    paths: Vec<String>,
    #[serde(default)]
    destination_id: Option<String>,
}

// The answer is the action: the paths the user chose become a proposal that is approved at once (UI-HOME).
pub async fn act(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: ActBody = parse(&body)?;
    let kind = ProposalKind::parse(&b.kind)
        .filter(|k| {
            matches!(
                k,
                ProposalKind::Delete
                    | ProposalKind::Compress
                    | ProposalKind::OnlineOnly
                    | ProposalKind::Archive
            )
        })
        .ok_or_else(|| ApiError::bad("kind must be delete, compress, online_only or archive"))?;
    if kind == ProposalKind::Archive && b.destination_id.is_none() {
        return Err(ApiError::bad("moving needs a destination_id"));
    }
    let st = s.clone();
    let out = blocking(move || {
        let env = env(&st);
        let items: Vec<NewItem> = b
            .paths
            .iter()
            .map(|p| index::normalize_path(&env.expand(p.trim())))
            .filter(|p| !p.is_empty())
            .map(|path| NewItem {
                path,
                verdict: None,
                reason: None,
            })
            .collect();
        if items.is_empty() {
            return Err(ApiError::bad("paths is required"));
        }
        let req = NewProposal {
            conversation_id: Some(HOME.into()),
            tool_call_id: None,
            kind,
            destination_id: b.destination_id,
            format: (kind == ProposalKind::Compress).then(|| "lzx".to_string()),
            dedupe: None,
            summary: "Chosen on Home".into(),
            items,
        };
        let refused = |d: &proposals::DroppedItem| json!({"path": redact(&d.path), "reason": redact(&d.reason)});
        let p = match proposals::create_or_dropped(&st.db, &env, req)? {
            Ok(p) => p,
            Err(dropped) => {
                let blocked: Vec<Value> = dropped.iter().map(refused).collect();
                return Ok(Json(
                    json!({"proposal_id": Value::Null, "job_id": Value::Null, "blocked": blocked}),
                ));
            }
        };
        let ids: Vec<i64> = p
            .items
            .iter()
            .filter(|i| i.status == "pending" && i.block_reason.is_none())
            .map(|i| i.id)
            .collect();
        let blocked: Vec<Value> = p
            .items
            .iter()
            .filter(|i| !ids.contains(&i.id))
            .map(|i| {
                json!({
                    "path": redact(&i.path),
                    "reason": redact(i.block_reason.as_deref().unwrap_or(&i.status)),
                })
            })
            .chain(p.dropped.iter().map(refused))
            .collect();
        if ids.is_empty() {
            proposals::decline(&st.db, &p.id)?;
            return Ok(Json(
                json!({"proposal_id": p.id, "job_id": Value::Null, "blocked": blocked}),
            ));
        }
        let r = st
            .exec
            .approve_deferred(&p.id, &ids)
            .map_err(|e| ApiError::bad(redact(&format!("{e:#}"))))?;
        Ok(Json(
            json!({"proposal_id": p.id, "job_id": r.job_id, "blocked": blocked}),
        ))
    })
    .await?;
    s.bump();
    Ok(out)
}

#[derive(Deserialize)]
struct ForgetBody {
    path: String,
}

// Bring back: drops the user's own answer for a folder, so the next review judges it again.
pub async fn forget_answer(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: ForgetBody = parse(&body)?;
    let st = s.clone();
    let out = blocking(move || {
        let env = env(&st);
        let path = index::normalize_path(&env.expand(b.path.trim()));
        if path.is_empty() {
            return Err(ApiError::bad("path is required"));
        }
        let conn = st.db.conn()?;
        let mut forgot = 0;
        while let Some(m) =
            location::location_at(&conn, &env.profile, &path)?.filter(|m| m.source == "user")
        {
            if !hoardini_core::memory::delete(&conn, m.id)? {
                break;
            }
            forgot += 1;
        }
        let protects = hoardini_core::memory::list(&conn)?.into_iter().filter(|m| {
            m.kind == hoardini_core::memory::Kind::Protect
                && m.source == "user"
                && location::expanded(&env, &m.key).eq_ignore_ascii_case(&path)
        });
        for m in protects {
            if hoardini_core::memory::delete(&conn, m.id)? {
                forgot += 1;
            }
        }
        Ok(Json(json!({"forgot": forgot})))
    })
    .await?;
    s.bump();
    Ok(out)
}

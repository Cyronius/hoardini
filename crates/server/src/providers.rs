use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use hoardini_agent::provider::{Caps, Client, ProviderConfig, ProviderView, presets};
use hoardini_agent::search;
use hoardini_core::store::{get_meta, now_ms, set_meta};
use hoardini_core::vault::{SecretStore, is_ref};
use rusqlite::{Connection, OptionalExtension, params};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::{ApiError, ApiResult, AppState, blocking, parse};

struct Row {
    id: String,
    config: ProviderConfig,
    caps: Option<Caps>,
    is_active: bool,
}

fn row_from(r: &rusqlite::Row) -> rusqlite::Result<(String, String, Option<String>, bool)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? != 0))
}

fn decode(raw: (String, String, Option<String>, bool)) -> Option<Row> {
    let (id, cfg, caps, is_active) = raw;
    let mut config: ProviderConfig = serde_json::from_str(&cfg).ok()?;
    config.id = None;
    let caps = caps.and_then(|c| serde_json::from_str(&c).ok());
    Some(Row {
        id,
        config,
        caps,
        is_active,
    })
}

fn load_all(conn: &Connection) -> anyhow::Result<Vec<Row>> {
    let mut st = conn.prepare(
        "SELECT id, config_json, caps_json, is_active FROM provider ORDER BY created_ms, id",
    )?;
    let rows = st
        .query_map([], row_from)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().filter_map(decode).collect())
}

fn load_one(conn: &Connection, id: &str) -> anyhow::Result<Option<Row>> {
    let raw = conn
        .query_row(
            "SELECT id, config_json, caps_json, is_active FROM provider WHERE id = ?1",
            [id],
            row_from,
        )
        .optional()?;
    Ok(raw.and_then(decode))
}

fn has_key(secrets: &dyn SecretStore, cfg: &ProviderConfig) -> bool {
    cfg.api_key
        .as_deref()
        .is_some_and(|r| matches!(secrets.get(r), Ok(Some(_))))
}

fn view(secrets: &dyn SecretStore, row: Row) -> ProviderView {
    ProviderView {
        has_key: has_key(secrets, &row.config),
        config: row.config,
        id: row.id,
        is_active: row.is_active,
        caps: row.caps,
    }
}

fn validate(cfg: &mut ProviderConfig) -> Result<(), ApiError> {
    cfg.id = None;
    cfg.name = cfg.name.trim().to_string();
    cfg.base_url = cfg.base_url.trim().to_string();
    if cfg.name.is_empty() {
        return Err(ApiError::bad("name is required"));
    }
    if !(cfg.base_url.starts_with("http://") || cfg.base_url.starts_with("https://")) {
        return Err(ApiError::bad(
            "base_url must start with http:// or https://",
        ));
    }
    cfg.model = cfg.model.take().filter(|m| !m.trim().is_empty());
    for (name, value) in &cfg.headers {
        let n = name.to_ascii_lowercase();
        let secretish = ["auth", "key", "token", "secret", "password", "cookie"]
            .iter()
            .any(|w| n.contains(w));
        if secretish && !is_ref(value.trim()) {
            return Err(ApiError::bad(format!(
                "header {name} must be a secret:// reference; store its value with PUT /api/secrets"
            )));
        }
    }
    match cfg.api_key.as_deref().map(str::trim) {
        None | Some("") => cfg.api_key = None,
        Some(r) if is_ref(r) => cfg.api_key = Some(r.to_string()),
        Some(_) => {
            return Err(ApiError::bad(
                "api_key must be a secret:// reference; store the key with PUT /api/secrets",
            ));
        }
    }
    Ok(())
}

pub fn active_summary(conn: &Connection) -> anyhow::Result<Option<Value>> {
    let preferred = get_meta(conn, "active_provider")?.filter(|s| !s.is_empty());
    let rows = load_all(conn)?;
    let row = rows
        .iter()
        .find(|r| Some(&r.id) == preferred.as_ref())
        .or_else(|| rows.iter().find(|r| r.is_active));
    Ok(row.map(|r| json!({"id": r.id, "name": r.config.name, "model": r.config.model})))
}

fn insert(conn: &Connection, cfg: &ProviderConfig, active: bool) -> anyhow::Result<String> {
    let id = format!("prov-{}", &uuid::Uuid::now_v7().simple().to_string()[12..]);
    conn.execute(
        "INSERT INTO provider(id, name, config_json, caps_json, is_active, created_ms) VALUES (?1, ?2, ?3, NULL, ?4, ?5)",
        params![id, cfg.name, serde_json::to_string(cfg)?, active as i64, now_ms()],
    )?;
    if active {
        set_meta(conn, "active_provider", &id)?;
    }
    Ok(id)
}

// The first run on a fresh install gets an active LM Studio profile so chat works with zero setup.
pub fn active_for_run(s: &AppState) -> anyhow::Result<Option<ProviderConfig>> {
    let conn = s.db.conn()?;
    let mut rows = load_all(&conn)?;
    if rows.is_empty()
        && let Some(p) = presets().into_iter().find(|p| p.id == "lmstudio")
    {
        let cfg = ProviderConfig::new(p.name, p.base_url);
        insert(&conn, &cfg, true)?;
        rows = load_all(&conn)?;
        s.bump();
    }
    let preferred = get_meta(&conn, "active_provider")?.filter(|v| !v.is_empty());
    let pos = rows
        .iter()
        .position(|r| Some(&r.id) == preferred.as_ref())
        .or_else(|| rows.iter().position(|r| r.is_active));
    Ok(pos.map(|i| {
        let r = rows.swap_remove(i);
        let mut cfg = r.config;
        cfg.id = Some(r.id);
        cfg
    }))
}

pub async fn list(State(s): State<AppState>) -> ApiResult {
    let st = s.clone();
    let views = blocking(move || {
        let rows = load_all(&st.db.conn()?)?;
        Ok(rows
            .into_iter()
            .map(|r| view(st.secrets.as_ref(), r))
            .collect::<Vec<_>>())
    })
    .await?;
    Ok(Json(json!(views)))
}

pub async fn create(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let mut cfg: ProviderConfig = parse(&body)?;
    validate(&mut cfg)?;
    let st = s.clone();
    let v = blocking(move || {
        let conn = st.db.conn()?;
        let first = load_all(&conn)?.is_empty();
        let id = insert(&conn, &cfg, first)?;
        let row = load_one(&conn, &id)?.ok_or_else(|| anyhow::anyhow!("provider vanished"))?;
        Ok(view(st.secrets.as_ref(), row))
    })
    .await?;
    s.bump();
    Ok(Json(json!(v)))
}

pub async fn update(State(s): State<AppState>, Path(id): Path<String>, body: Bytes) -> ApiResult {
    let mut cfg: ProviderConfig = parse(&body)?;
    validate(&mut cfg)?;
    let st = s.clone();
    let v = blocking(move || {
        let conn = st.db.conn()?;
        let old = load_one(&conn, &id)?.ok_or_else(|| ApiError::not_found("provider"))?;
        let changed = old.config.base_url != cfg.base_url || old.config.model != cfg.model;
        conn.execute(
            "UPDATE provider SET name = ?2, config_json = ?3, caps_json = CASE WHEN ?4 THEN NULL ELSE caps_json END WHERE id = ?1",
            params![id, cfg.name, serde_json::to_string(&cfg)?, changed],
        )?;
        let row = load_one(&conn, &id)?.ok_or_else(|| ApiError::not_found("provider"))?;
        Ok(view(st.secrets.as_ref(), row))
    })
    .await?;
    s.bump();
    Ok(Json(json!(v)))
}

pub async fn delete(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let st = s.clone();
    blocking(move || {
        let conn = st.db.conn()?;
        if conn.execute("DELETE FROM provider WHERE id = ?1", [&id])? == 0 {
            return Err(ApiError::not_found("provider"));
        }
        if get_meta(&conn, "active_provider")?.as_deref() == Some(id.as_str()) {
            set_meta(&conn, "active_provider", "")?;
        }
        Ok(())
    })
    .await?;
    s.bump();
    Ok(Json(json!({"ok": true})))
}

pub async fn activate(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let st = s.clone();
    blocking(move || {
        let mut conn = st.db.conn()?;
        let tx = conn.transaction()?;
        tx.execute("UPDATE provider SET is_active = 0", [])?;
        if tx.execute("UPDATE provider SET is_active = 1 WHERE id = ?1", [&id])? == 0 {
            return Err(ApiError::not_found("provider"));
        }
        set_meta(&tx, "active_provider", &id)?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    s.bump();
    Ok(Json(json!({"ok": true})))
}

pub async fn test(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let st = s.clone();
    let (row, key) = blocking(move || {
        let row = load_one(&st.db.conn()?, &id)?.ok_or_else(|| ApiError::not_found("provider"))?;
        let key = match row.config.api_key.as_deref() {
            Some(r) => st
                .secrets
                .get(r)?
                .map(|k: SecretString| k.expose_secret().to_string()),
            None => None,
        };
        Ok((row, key))
    })
    .await?;
    let mut cfg = row.config;
    cfg.id = Some(row.id.clone());
    let report = Client::new().test_connection(&cfg, key.as_deref()).await;
    if let Some(caps) = report.caps {
        let st = s.clone();
        let id = row.id;
        blocking(move || {
            st.db.conn()?.execute(
                "UPDATE provider SET caps_json = ?2 WHERE id = ?1",
                params![id, serde_json::to_string(&caps)?],
            )?;
            Ok(())
        })
        .await?;
        s.bump();
    }
    Ok(Json(json!(report)))
}

pub async fn preset_list() -> ApiResult {
    let v: Vec<Value> = presets()
        .into_iter()
        .map(|p| json!({"name": p.name, "base_url": p.base_url, "needs_key": p.needs_key}))
        .collect();
    Ok(Json(json!(v)))
}

#[derive(Deserialize)]
struct SecretBody {
    #[serde(rename = "ref")]
    reference: String,
    value: String,
}

// Errors here must never echo the body: serde's messages can quote the offending value.
pub async fn put_secret(State(s): State<AppState>, body: Bytes) -> ApiResult {
    let b: SecretBody = serde_json::from_slice(&body)
        .map_err(|_| ApiError::bad("expected {\"ref\": \"secret://...\", \"value\": \"...\"}"))?;
    if !is_ref(&b.reference) {
        return Err(ApiError::bad("ref must start with secret://"));
    }
    if b.value.is_empty() {
        return Err(ApiError::bad("value must not be empty"));
    }
    let st = s.clone();
    blocking(move || {
        st.secrets
            .set(&b.reference, SecretString::from(b.value))
            .map_err(ApiError::from)
    })
    .await?;
    s.bump();
    Ok(Json(json!({"ok": true})))
}

const SEARCH_KINDS: [&str; 5] = ["tavily", "brave", "exa", "serper", "searxng"];

fn check_kind(kind: &str) -> Result<(), ApiError> {
    if SEARCH_KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(ApiError::not_found("search provider"))
    }
}

pub async fn search_list(State(s): State<AppState>) -> ApiResult {
    let st = s.clone();
    let out = blocking(move || {
        let conn = st.db.conn()?;
        let mut st_q = conn.prepare("SELECT kind, config_json, is_active FROM search_provider")?;
        let rows = st_q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)? != 0,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let views: Vec<Value> = SEARCH_KINDS
            .iter()
            .map(|kind| {
                let found = rows.iter().find(|(k, _, _)| k == kind);
                let cfg: Value = found
                    .and_then(|(_, c, _)| serde_json::from_str(c).ok())
                    .unwrap_or(Value::Null);
                let key_ref = cfg.get("api_key").and_then(Value::as_str);
                let has_key = key_ref.is_some_and(|r| matches!(st.secrets.get(r), Ok(Some(_))));
                json!({
                    "kind": kind,
                    "is_active": found.is_some_and(|f| f.2),
                    "has_key": has_key,
                    "base_url": cfg.get("base_url").cloned().unwrap_or(Value::Null),
                    "api_key": key_ref,
                })
            })
            .collect();
        Ok(views)
    })
    .await?;
    Ok(Json(json!(out)))
}

pub async fn search_put(
    State(s): State<AppState>,
    Path(kind): Path<String>,
    body: Bytes,
) -> ApiResult {
    check_kind(&kind)?;
    let patch: Value = parse(&body)?;
    let Some(patch) = patch.as_object() else {
        return Err(ApiError::bad("expected an object"));
    };
    if let Some(r) = patch.get("api_key").and_then(Value::as_str)
        && !r.is_empty()
        && !is_ref(r)
    {
        return Err(ApiError::bad("api_key must be a secret:// reference"));
    }
    let patch = patch.clone();
    let st = s.clone();
    blocking(move || {
        let conn = st.db.conn()?;
        let old: Option<String> = conn
            .query_row(
                "SELECT config_json FROM search_provider WHERE id = ?1",
                [&kind],
                |r| r.get(0),
            )
            .optional()?;
        let mut cfg = old
            .and_then(|c| serde_json::from_str::<Value>(&c).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        for k in ["api_key", "base_url"] {
            if let Some(v) = patch.get(k) {
                cfg[k] = if v.as_str() == Some("") { Value::Null } else { v.clone() };
            }
        }
        conn.execute(
            "INSERT INTO search_provider(id, kind, config_json, is_active, created_ms) VALUES (?1, ?1, ?2, 0, ?3)
             ON CONFLICT(id) DO UPDATE SET config_json = excluded.config_json",
            params![kind, cfg.to_string(), now_ms()],
        )?;
        Ok(())
    })
    .await?;
    s.bump();
    Ok(Json(json!({"ok": true})))
}

pub async fn search_activate(State(s): State<AppState>, Path(kind): Path<String>) -> ApiResult {
    check_kind(&kind)?;
    let st = s.clone();
    blocking(move || {
        let mut conn = st.db.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO search_provider(id, kind, config_json, is_active, created_ms) VALUES (?1, ?1, '{}', 0, ?2)
             ON CONFLICT(id) DO NOTHING",
            params![kind, now_ms()],
        )?;
        tx.execute("UPDATE search_provider SET is_active = (id = ?1)", [&kind])?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    s.bump();
    Ok(Json(json!({"ok": true})))
}

pub async fn search_test(State(s): State<AppState>, Path(kind): Path<String>) -> ApiResult {
    check_kind(&kind)?;
    let kind: search::Kind = serde_json::from_value(Value::String(kind))?;
    let st = s.clone();
    let found = blocking(move || {
        Ok(search::configured(
            &st.db.conn()?,
            st.secrets.as_ref(),
            Some(kind),
        )?)
    })
    .await?;
    let Some((cfg, key)) = found else {
        return Err(ApiError::not_found("search provider"));
    };
    let mut report = search::test(&cfg, key.as_deref()).await;
    // The provider's error text could quote the key back.
    if let (Some(k), Some(e)) = (
        key.as_deref().filter(|k| k.len() >= 4),
        report.error.as_mut(),
    ) {
        *e = e.replace(k, "[key]");
    }
    Ok(Json(json!(report)))
}

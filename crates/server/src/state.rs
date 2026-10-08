use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use hoardini_agent::run::AgentEnv;
use hoardini_core::actions::Executor;
use hoardini_core::store::Db;
use hoardini_core::vault::SecretStore;
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio::sync::watch;

use crate::DialogProvider;
use crate::supervisor::Supervisor;

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn new(code: StatusCode, msg: impl Into<String>) -> Self {
        ApiError(code, msg.into())
    }
    pub fn bad(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }
    pub fn not_found(what: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, format!("{what} not found"))
    }
    pub fn unimplemented(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_IMPLEMENTED, msg)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

pub type ApiResult<T = Json<serde_json::Value>> = Result<T, ApiError>;

pub struct RunSlot {
    pub abort: tokio::task::AbortHandle,
    pub cancelled: bool,
}

pub struct Inner {
    pub db: Db,
    pub secrets: Arc<dyn SecretStore>,
    pub exec: Arc<Executor>,
    pub supervisor: Arc<Supervisor>,
    pub env: Arc<AgentEnv>,
    pub dialogs: Option<Arc<dyn DialogProvider>>,
    pub ui_dir: Option<std::path::PathBuf>,
    pub runs: Mutex<HashMap<String, RunSlot>>,
    pub change: Arc<watch::Sender<u64>>,
    pub shutdown: watch::Sender<bool>,
    pub token: String,
    pub port: u16,
}

pub type AppState = Arc<Inner>;

impl Inner {
    pub fn bump(&self) {
        self.change.send_modify(|n| *n += 1);
    }
}

pub async fn blocking<T, F>(f: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r,
        Err(e) => Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("task failed: {e}"),
        )),
    }
}

// Empty body counts as `{}` so handlers with all-optional fields accept a bare POST.
pub fn parse<T: DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    let src: &[u8] = if body.iter().all(u8::is_ascii_whitespace) {
        b"{}"
    } else {
        body
    };
    serde_json::from_slice(src).map_err(|e| ApiError::bad(format!("invalid request body: {e}")))
}

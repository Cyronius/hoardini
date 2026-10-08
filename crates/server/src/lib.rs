// SEAM (wave 2): signatures are the contract between the server and the Tauri shell / CLI.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use futures::future::BoxFuture;
use hoardini_agent::provider::Client;
use hoardini_agent::run::{AgentEnv, IndexControl};
use hoardini_core::actions::Executor;
use hoardini_core::store::Db;
use hoardini_core::vault::{DpapiVault, SecretStore};
use serde_json::json;
use tokio::sync::watch;

mod providers;
mod routes;
mod state;
mod status;
mod stream;
mod supervisor;
mod ui;

pub use routes::route_list;
pub use state::AppState;

const CSP: &str = "default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:; script-src 'self'";
const BODY_LIMIT: usize = 2 * 1024 * 1024;

// The Tauri shell implements this with native dialogs; headless serve has none (routes return 501).
pub trait DialogProvider: Send + Sync {
    fn pick_folder(&self) -> BoxFuture<'static, Option<PathBuf>>;
}

pub struct ServerOptions {
    pub port: u16,
    pub token: Option<String>,
    pub db_path: PathBuf,
    pub ui_dir: Option<PathBuf>,
    pub dialogs: Option<Arc<dyn DialogProvider>>,
    pub refresh_on_start: bool,
    // None = current_exe(); tests and the Tauri shell may point elsewhere.
    pub indexer_exe: Option<PathBuf>,
}

pub struct ServerHandle {
    pub port: u16,
    pub token: String,
    pub shutdown: tokio::sync::oneshot::Sender<()>,
}

impl ServerHandle {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/#token={}", self.port, self.token)
    }
}

pub fn default_log_dir() -> PathBuf {
    hoardini_core::paths::data_dir().join("logs")
}

pub struct LogGuard {
    _guard: tracing_appender::non_blocking::WorkerGuard,
}

// Request bodies and query strings are never logged: /api/secrets carries key material.
pub fn init_logging(dir: &Path, to_stderr: bool) -> anyhow::Result<LogGuard> {
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::prelude::*;

    std::fs::create_dir_all(dir)?;
    let appender = tracing_appender::rolling::daily(dir, "hoardini.log");
    let (file, guard) = tracing_appender::non_blocking(appender);
    let stderr = to_stderr.then(|| {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(false)
    });
    let _ = tracing_subscriber::registry()
        .with(LevelFilter::INFO)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(file)
                .with_ansi(false),
        )
        .with(stderr)
        .try_init();
    Ok(LogGuard { _guard: guard })
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() != b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(if b[i] == b'+' { b' ' } else { b[i] });
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn presented_token(req: &Request) -> Option<String> {
    if let Some(v) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        && let Some(t) = v.strip_prefix("Bearer ")
    {
        return Some(t.trim().to_string());
    }
    req.uri().query().and_then(|q| {
        q.split('&')
            .find_map(|kv| kv.strip_prefix("token=").map(percent_decode))
    })
}

fn host_ok(req: &Request, port: u16) -> bool {
    let Some(h) = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
    else {
        return true;
    };
    let h = h.to_ascii_lowercase();
    [format!("127.0.0.1:{port}"), format!("localhost:{port}")].contains(&h)
        || h == "127.0.0.1"
        || h == "localhost"
}

fn json_error(code: StatusCode, msg: &str) -> Response {
    (code, axum::Json(json!({"error": msg}))).into_response()
}

async fn guard(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let is_api = req.uri().path() == "/api" || req.uri().path().starts_with("/api/");
    let mut resp = if !host_ok(&req, s.port) {
        json_error(StatusCode::FORBIDDEN, "forbidden host")
    } else if is_api
        && !presented_token(&req).is_some_and(|t| ct_eq(t.as_bytes(), s.token.as_bytes()))
    {
        json_error(StatusCode::UNAUTHORIZED, "unauthorized")
    } else {
        next.run(req).await
    };
    if is_api && (resp.status().is_client_error() || resp.status().is_server_error()) {
        let ct = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !ct.starts_with("application/json") {
            let reason = resp.status().canonical_reason().unwrap_or("error");
            resp = json_error(resp.status(), &reason.to_ascii_lowercase());
        }
    }
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    resp
}

async fn fallback(State(s): State<AppState>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    if path == "/api" || path.starts_with("/api/") {
        return json_error(StatusCode::NOT_FOUND, "not found");
    }
    ui::serve(s.ui_dir.as_ref(), req.method(), &path).await
}

pub async fn start(opts: ServerOptions) -> anyhow::Result<ServerHandle> {
    start_with(opts, None).await
}

// `secrets` overrides the DPAPI vault; tests inject an in-memory store.
pub async fn start_with(
    opts: ServerOptions,
    secrets: Option<Arc<dyn SecretStore>>,
) -> anyhow::Result<ServerHandle> {
    let db = Db::open(&opts.db_path)?;
    let secrets: Arc<dyn SecretStore> = match secrets {
        Some(s) => s,
        None => {
            let dir = opts
                .db_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            Arc::new(DpapiVault::open(&dir.join("secrets.vault"))?)
        }
    };
    let token = match opts.token.filter(|t| !t.is_empty()) {
        Some(t) => t,
        None => rand::random::<[u8; 32]>()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], opts.port))).await?;
    let port = listener.local_addr()?.port();

    let (change, _) = watch::channel(0u64);
    let change = Arc::new(change);
    let exe = match opts.indexer_exe {
        Some(e) => e,
        None => std::env::current_exe()?,
    };
    let supervisor = supervisor::Supervisor::new(
        db.clone(),
        exe,
        change.clone(),
        tokio::runtime::Handle::current(),
    );
    let index: Arc<dyn IndexControl> = supervisor.clone();
    let env = Arc::new(AgentEnv {
        db: db.clone(),
        client: Client::new(),
        secrets: secrets.clone(),
        index,
    });
    let (shutdown_tx, _) = watch::channel(false);
    let state: AppState = Arc::new(state::Inner {
        db: db.clone(),
        secrets: secrets.clone(),
        exec: Arc::new(Executor::new(db, secrets)),
        supervisor: supervisor.clone(),
        env,
        dialogs: opts.dialogs,
        ui_dir: opts.ui_dir,
        runs: Mutex::new(HashMap::new()),
        change,
        shutdown: shutdown_tx,
        token: token.clone(),
        port,
    });

    let app: Router = routes::api_router()
        .fallback(fallback)
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state.clone());

    if opts.refresh_on_start
        && let Err(e) = supervisor.start_refresh("background")
    {
        tracing::warn!(error = %e, "refresh on start failed");
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let mut serve = tokio::spawn(async move {
        let r = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = stop_rx.wait_for(|v| *v).await;
            })
            .await;
        if let Err(e) = r {
            tracing::error!(error = %e, "server stopped");
        }
    });
    tokio::spawn(async move {
        let _ = rx.await;
        let _ = state.shutdown.send(true);
        if let Ok(mut runs) = state.runs.lock() {
            for (_, slot) in runs.drain() {
                slot.abort.abort();
            }
        }
        supervisor.shutdown();
        let _ = stop_tx.send(true);
        if tokio::time::timeout(Duration::from_secs(2), &mut serve)
            .await
            .is_err()
        {
            serve.abort();
        }
    });
    Ok(ServerHandle {
        port,
        token,
        shutdown: tx,
    })
}

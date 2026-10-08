use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use futures::future::BoxFuture;
use hoardini_agent::provider::ProviderConfig;
use hoardini_core::actions::{Job, JobProgress};
use hoardini_core::runtime::{self, LLAMA_SERVER, MODELS, Pinned};
use hoardini_core::store::{get_meta, now_ms, set_meta};
use rusqlite::params;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::state::{ApiError, AppState, blocking};

pub const PROVIDER_ID: &str = "builtin";
// Never dialled: the run resolves the live child's address at request time.
pub const BASE_PLACEHOLDER: &str = "builtin://llama-server/v1";
// The longest eval prompt was ~2.7k tokens plus 1024 output; 8k keeps 3x headroom and halves KV RAM.
pub const CONTEXT: u32 = 8_192;
// Decided by the M6 eval (.claude/plans/m6-eval-results.md); None keeps the built-in option hidden.
pub const RECOMMENDED: Option<&str> = None;
const MODEL_META: &str = "builtin_model";
const HEALTH_TIMEOUT: Duration = Duration::from_secs(180);
const WATCHDOG_TICK: Duration = Duration::from_secs(15);

pub struct ModelInfo {
    pub name: &'static str,
    pub license: &'static str,
}

pub fn model_info(id: &str) -> ModelInfo {
    let (name, license) = match id {
        "lfm2.5-1.2b-instruct" => ("LFM2.5 1.2B Instruct", "LFM Open License v1.0"),
        "lfm2.5-2.6b" => ("LFM2.5 2.6B", "LFM Open License v1.0"),
        "qwen3-4b-instruct-2507" => ("Qwen3 4B Instruct 2507", "Apache-2.0"),
        "granite-4.0-micro" => ("Granite 4.0 Micro", "Apache-2.0"),
        "minicpm5-2b" => ("MiniCPM5 2B", "Apache-2.0"),
        _ => ("", ""),
    };
    ModelInfo {
        name: if name.is_empty() {
            "Unknown model"
        } else {
            name
        },
        license: if license.is_empty() {
            "see link"
        } else {
            license
        },
    }
}

pub fn model_pin(id: &str) -> Option<&'static Pinned> {
    MODELS.iter().find(|p| p.id == id)
}

pub fn profile(model_id: &str) -> ProviderConfig {
    let mut p = ProviderConfig::new("Built-in", BASE_PLACEHOLDER);
    p.id = Some(PROVIDER_ID.into());
    p.model = Some(model_id.into());
    p.small = true;
    p.redact_paths = true;
    p.context_window = CONTEXT;
    p.timeout_s = 300;
    p.max_tool_steps = 10;
    p.params.temperature = Some(0.2);
    p.params.max_tokens = Some(1024);
    p
}

// You are waiting for the answer, so generation gets every core but two (design 5.12).
pub fn threads() -> usize {
    hoardini_win::physical_cores().saturating_sub(2).max(1)
}

pub struct LaunchSpec {
    pub exe: PathBuf,
    pub model: PathBuf,
    pub model_id: String,
    pub port: u16,
    pub threads: usize,
    pub ctx: u32,
}

pub fn args(spec: &LaunchSpec) -> Vec<String> {
    [
        "-m",
        &spec.model.to_string_lossy(),
        "--host",
        "127.0.0.1",
        "--port",
        &spec.port.to_string(),
        "--jinja",
        "-t",
        &spec.threads.to_string(),
        "-c",
        &spec.ctx.to_string(),
        "--no-webui",
        "-np",
        "1",
        "--cache-ram",
        "0",
        "--cache-type-k",
        "q8_0",
        "--cache-type-v",
        "q8_0",
        "-ngl",
        "0",
        "-a",
        &spec.model_id,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

pub trait ChildProc: Send {
    fn pid(&self) -> Option<u32>;
    // Some(reason) once the process has exited.
    fn exited(&mut self) -> Option<String>;
    fn kill(&mut self);
}

pub struct Launched {
    pub child: Box<dyn ChildProc>,
    pub root_url: String,
}

pub trait Launcher: Send + Sync {
    fn launch(&self, model_id: &str) -> BoxFuture<'static, Result<Launched>>;
}

pub struct LlamaLauncher {
    pub root: PathBuf,
    pub ctx: u32,
}

struct RealChild {
    child: std::process::Child,
    _job: Option<hoardini_win::JobGuard>,
}

impl ChildProc for RealChild {
    fn pid(&self) -> Option<u32> {
        Some(self.child.id())
    }
    fn exited(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(st)) => Some(format!("llama-server exited ({st})")),
            Ok(None) => None,
            Err(e) => Some(format!("llama-server state unknown: {e}")),
        }
    }
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for RealChild {
    fn drop(&mut self) {
        if self.exited().is_none() {
            self.kill();
        }
    }
}

fn free_port() -> Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

pub fn log_path(root: &Path) -> PathBuf {
    root.join("logs").join("llama-server.log")
}

fn spawn_llama(root: &Path, model_id: &str, ctx: u32) -> Result<Launched> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const NORMAL_PRIORITY_CLASS: u32 = 0x0000_0020;
    let pin = model_pin(model_id).with_context(|| format!("{model_id} is not a built-in model"))?;
    let exe = runtime::entry_path(&LLAMA_SERVER, root);
    let model = runtime::entry_path(pin, root);
    // Every launch re-checks both files against the pins (RUNTIME-PINNED-HASH).
    runtime::verify_before_run(&exe, &LLAMA_SERVER)?;
    runtime::verify_before_run(&model, pin)?;
    let spec = LaunchSpec {
        exe,
        model,
        model_id: model_id.to_string(),
        port: free_port()?,
        threads: threads(),
        ctx,
    };
    let log = log_path(root);
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let out = std::fs::File::create(&log)?;
    let child = std::process::Command::new(&spec.exe)
        .args(args(&spec))
        .current_dir(spec.exe.parent().unwrap_or(root))
        .stdin(std::process::Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out)
        .creation_flags(CREATE_NO_WINDOW | NORMAL_PRIORITY_CLASS)
        .spawn()
        .with_context(|| format!("starting {}", spec.exe.display()))?;
    let job = match hoardini_win::kill_with_job(child.id()) {
        Ok(j) => Some(j),
        Err(e) => {
            tracing::warn!(error = %e, "llama-server not tied to a job object");
            None
        }
    };
    tracing::info!(
        model = model_id,
        port = spec.port,
        threads = spec.threads,
        "llama-server started"
    );
    Ok(Launched {
        child: Box::new(RealChild { child, _job: job }),
        root_url: format!("http://127.0.0.1:{}", spec.port),
    })
}

impl Launcher for LlamaLauncher {
    fn launch(&self, model_id: &str) -> BoxFuture<'static, Result<Launched>> {
        let root = self.root.clone();
        let id = model_id.to_string();
        let ctx = self.ctx;
        Box::pin(async move {
            tokio::task::spawn_blocking(move || spawn_llama(&root, &id, ctx))
                .await
                .context("launch task failed")?
        })
    }
}

async fn health(root_url: &str) -> Option<u16> {
    let addr = root_url.strip_prefix("http://")?;
    let mut s = tokio::net::TcpStream::connect(addr).await.ok()?;
    let req = format!("GET /health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.ok()?;
    let mut buf = [0u8; 64];
    let n = s.read(&mut buf).await.ok()?;
    std::str::from_utf8(&buf[..n])
        .ok()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn log_tail(root: Option<&Path>) -> String {
    let Some(text) = root.and_then(|r| std::fs::read_to_string(log_path(r)).ok()) else {
        return String::new();
    };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(3)..].join(" / ");
    if tail.is_empty() {
        tail
    } else {
        format!(": {tail}")
    }
}

struct Running {
    child: Box<dyn ChildProc>,
    base_url: String,
    model_id: String,
}

pub struct Builtin {
    launcher: Arc<dyn Launcher>,
    idle_minutes: Arc<dyn Fn() -> u64 + Send + Sync>,
    root: Option<PathBuf>,
    state: tokio::sync::Mutex<Option<Running>>,
    leases: AtomicUsize,
    last_active: Mutex<Instant>,
    launches: AtomicU64,
    running: AtomicBool,
    pid: Mutex<Option<u32>>,
    pub install: Installer,
}

pub struct Lease {
    sup: Arc<Builtin>,
    pub base_url: String,
}

impl Drop for Lease {
    fn drop(&mut self) {
        *self
            .sup
            .last_active
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Instant::now();
        self.sup.leases.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Builtin {
    pub fn new(
        launcher: Arc<dyn Launcher>,
        idle_minutes: Arc<dyn Fn() -> u64 + Send + Sync>,
        root: Option<PathBuf>,
    ) -> Arc<Builtin> {
        Arc::new(Builtin {
            launcher,
            idle_minutes,
            root,
            state: tokio::sync::Mutex::new(None),
            leases: AtomicUsize::new(0),
            last_active: Mutex::new(Instant::now()),
            launches: AtomicU64::new(0),
            running: AtomicBool::new(false),
            pid: Mutex::new(None),
            install: Installer::default(),
        })
    }

    pub fn start_watchdog(self: &Arc<Self>) {
        let weak: Weak<Builtin> = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(WATCHDOG_TICK).await;
                let Some(b) = weak.upgrade() else { return };
                if b.check_idle(Instant::now()).await {
                    tracing::info!("llama-server stopped after the idle timeout");
                }
            }
        });
    }

    pub fn launches(&self) -> u64 {
        self.launches.load(Ordering::SeqCst)
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn pid(&self) -> Option<u32> {
        *self.pid.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_running(&self, running: bool, pid: Option<u32>) {
        self.running.store(running, Ordering::SeqCst);
        *self.pid.lock().unwrap_or_else(|e| e.into_inner()) = pid;
    }

    // Starts the child on first use (PROV-BUILTIN-LAZY) and restarts it after a crash.
    pub async fn acquire(self: &Arc<Self>, model_id: &str) -> Result<Lease> {
        self.leases.fetch_add(1, Ordering::SeqCst);
        let mut lease = Lease {
            sup: self.clone(),
            base_url: String::new(),
        };
        let mut g = self.state.lock().await;
        if let Some(r) = g.as_mut() {
            if let Some(why) = r.child.exited() {
                tracing::warn!(reason = %why, "llama-server died; restarting it");
                *g = None;
            } else if r.model_id != model_id {
                r.child.kill();
                *g = None;
            } else {
                lease.base_url = r.base_url.clone();
                return Ok(lease);
            }
        }
        self.set_running(false, None);
        let mut launched = self.launcher.launch(model_id).await?;
        self.launches.fetch_add(1, Ordering::SeqCst);
        let deadline = Instant::now() + HEALTH_TIMEOUT;
        loop {
            if health(&launched.root_url).await == Some(200) {
                break;
            }
            if let Some(why) = launched.child.exited() {
                bail!(
                    "the built-in model did not start ({why}){}",
                    log_tail(self.root.as_deref())
                );
            }
            if Instant::now() > deadline {
                launched.child.kill();
                bail!(
                    "the built-in model did not become ready within {} s",
                    HEALTH_TIMEOUT.as_secs()
                );
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let base = format!("{}/v1", launched.root_url);
        self.set_running(true, launched.child.pid());
        *g = Some(Running {
            child: launched.child,
            base_url: base.clone(),
            model_id: model_id.to_string(),
        });
        lease.base_url = base;
        Ok(lease)
    }

    // Stops the child once nothing used it for `builtin_idle_minutes` (PROV-BUILTIN-IDLE).
    pub async fn check_idle(&self, now: Instant) -> bool {
        if self.leases.load(Ordering::SeqCst) > 0 {
            return false;
        }
        let idle = Duration::from_secs((self.idle_minutes)().saturating_mul(60));
        let last = *self.last_active.lock().unwrap_or_else(|e| e.into_inner());
        if now.saturating_duration_since(last) < idle {
            return false;
        }
        let mut g = self.state.lock().await;
        if self.leases.load(Ordering::SeqCst) > 0 {
            return false;
        }
        match g.take() {
            Some(mut r) => {
                r.child.kill();
                self.set_running(false, None);
                true
            }
            None => false,
        }
    }

    pub async fn stop(&self) {
        if let Some(mut r) = self.state.lock().await.take() {
            r.child.kill();
        }
        self.set_running(false, None);
    }

    // App exit: never block; the job object kills a child that a busy lock keeps us from reaching.
    pub fn shutdown(&self) {
        if let Ok(mut g) = self.state.try_lock()
            && let Some(mut r) = g.take()
        {
            r.child.kill();
        }
        self.set_running(false, None);
    }
}

#[derive(Default)]
pub struct Installer {
    job: Mutex<Option<Job>>,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
}

impl Installer {
    pub fn job(&self) -> Option<Job> {
        self.job.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn cancel(&self, id: &str) -> bool {
        let running = self
            .job()
            .is_some_and(|j| j.id == id && matches!(j.status.as_str(), "queued" | "running"));
        if running
            && let Some(c) = self
                .cancel
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
        {
            c.store(true, Ordering::Relaxed);
        }
        running
    }

    fn busy(&self) -> bool {
        self.job()
            .is_some_and(|j| matches!(j.status.as_str(), "queued" | "running"))
    }
}

pub fn data_root(s: &AppState) -> PathBuf {
    s.db.path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(hoardini_core::paths::data_dir)
}

pub fn free_bytes(path: &Path) -> u64 {
    let mut p = path.to_path_buf();
    while !p.exists() {
        match p.parent() {
            Some(q) => p = q.to_path_buf(),
            None => return 0,
        }
    }
    hoardini_win::volume_root_of(&p)
        .and_then(|r| hoardini_win::volume_info(&r))
        .map(|v| v.free_bytes)
        .unwrap_or(0)
}

fn chosen_model(conn: &rusqlite::Connection, root: &Path) -> Option<&'static Pinned> {
    let wanted = get_meta(conn, MODEL_META).ok().flatten();
    let installed = |p: &&'static Pinned| runtime::is_installed(p, root);
    wanted
        .and_then(|w| model_pin(&w))
        .filter(installed)
        .or_else(|| MODELS.iter().find(installed))
}

fn pin_json(p: &Pinned, root: &Path) -> Value {
    let info = model_info(p.id);
    json!({
        "id": p.id,
        "name": info.name,
        "size": p.size,
        "license": info.license,
        "license_url": p.license_url,
        "installed": runtime::is_installed(p, root),
        "recommended": RECOMMENDED == Some(p.id),
    })
}

pub fn view(s: &AppState) -> Result<Value> {
    let root = data_root(s);
    let conn = s.db.conn()?;
    let model = chosen_model(&conn, &root);
    let runtime_ok = runtime::is_installed(&LLAMA_SERVER, &root);
    let has_row: bool = conn.query_row(
        "SELECT COUNT(*) FROM provider WHERE id = ?1",
        [PROVIDER_ID],
        |r| r.get::<_, i64>(0),
    )? > 0;
    Ok(json!({
        "installed": runtime_ok && model.is_some(),
        "running": s.builtin.is_running(),
        "runtime": {
            "version": LLAMA_SERVER.version,
            "size": LLAMA_SERVER.size,
            "installed": runtime_ok,
            "license": "MIT",
            "license_url": LLAMA_SERVER.license_url,
        },
        "model": model.map(|p| pin_json(p, &root)),
        "candidates": MODELS.iter().map(|p| pin_json(p, &root)).collect::<Vec<_>>(),
        "free_bytes": free_bytes(&root),
        "recommended": RECOMMENDED,
        "install_job": s.builtin.install.job(),
        "provider_id": has_row.then_some(PROVIDER_ID),
        "threads": threads(),
        "context": CONTEXT,
    }))
}

fn publish(s: &AppState, job: &Job) {
    *s.builtin
        .install
        .job
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(job.clone());
    let _ = s.exec.events.send(job.clone());
}

// The Built-in row exists once a model is installed; the first provider on a fresh install becomes active.
fn upsert_row(conn: &rusqlite::Connection, model_id: &str) -> Result<()> {
    let cfg = profile(model_id);
    let others: i64 = conn.query_row(
        "SELECT COUNT(*) FROM provider WHERE id <> ?1",
        [PROVIDER_ID],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO provider(id, name, config_json, caps_json, is_active, created_ms) VALUES (?1, ?2, ?3, NULL, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET config_json = excluded.config_json, caps_json = NULL",
        params![PROVIDER_ID, cfg.name, serde_json::to_string(&cfg)?, (others == 0) as i64, now_ms()],
    )?;
    if others == 0 {
        set_meta(conn, "active_provider", PROVIDER_ID)?;
    }
    set_meta(conn, MODEL_META, model_id)?;
    Ok(())
}

pub async fn start_install(s: &AppState, model_id: &str) -> Result<Job, ApiError> {
    let pin = model_pin(model_id)
        .ok_or_else(|| ApiError::bad(format!("{model_id} is not a built-in model")))?;
    if s.builtin.install.busy() {
        return Err(ApiError::new(
            axum::http::StatusCode::CONFLICT,
            "a built-in model download is already running",
        ));
    }
    let root = data_root(s);
    let pins: Vec<&'static Pinned> = [&LLAMA_SERVER, pin]
        .into_iter()
        .filter(|p| !runtime::is_installed(p, &root))
        .collect();
    let total: u64 = pins.iter().map(|p| p.size).sum();
    let free = free_bytes(&root);
    let margin = 512 << 20;
    if total + margin > free {
        return Err(ApiError::bad(format!(
            "not enough free space: the download needs {} plus {} headroom, {} is free",
            hoardini_core::paths::humanize_bytes(total),
            hoardini_core::paths::humanize_bytes(margin),
            hoardini_core::paths::humanize_bytes(free)
        )));
    }
    let job = Job {
        id: uuid::Uuid::now_v7().to_string(),
        kind: "builtin_install".into(),
        proposal_id: None,
        status: "running".into(),
        progress: JobProgress {
            done_bytes: 0,
            total_bytes: total as i64,
            done_items: 0,
            total_items: pins.len() as i64,
            current: pins.first().map(|p| label(p)),
        },
        freed_bytes: 0,
        error: None,
        created_ms: now_ms(),
        finished_ms: None,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    *s.builtin
        .install
        .cancel
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(cancel.clone());
    publish(s, &job);
    let st = s.clone();
    let id = model_id.to_string();
    let out = job.clone();
    tokio::spawn(async move {
        let mut job = job;
        let result = download_all(&st, &root, &pins, &mut job, &cancel).await;
        let result = match result {
            Ok(()) => {
                let st2 = st.clone();
                blocking(move || Ok(upsert_row(&st2.db.conn()?, &id)?))
                    .await
                    .map_err(|e| anyhow::anyhow!(e.1))
            }
            Err(e) => Err(e),
        };
        job.finished_ms = Some(now_ms());
        match result {
            Ok(()) => {
                job.status = "done".into();
                job.progress.current = None;
            }
            Err(e) => {
                job.status = if cancel.load(Ordering::Relaxed) {
                    "cancelled".into()
                } else {
                    "failed".into()
                };
                job.error = Some(format!("{e:#}"));
            }
        }
        publish(&st, &job);
        st.bump();
    });
    Ok(out)
}

fn label(p: &Pinned) -> String {
    if p.id == LLAMA_SERVER.id {
        format!("llama.cpp {}", p.version)
    } else {
        model_info(p.id).name.to_string()
    }
}

async fn download_all(
    s: &AppState,
    root: &Path,
    pins: &[&'static Pinned],
    job: &mut Job,
    cancel: &AtomicBool,
) -> Result<()> {
    let mut base = 0u64;
    for (i, pin) in pins.iter().enumerate() {
        job.progress.current = Some(label(pin));
        job.progress.done_items = i as i64;
        publish(s, job);
        let snapshot = Mutex::new((job.clone(), Instant::now()));
        let progress = |done: u64, _total: u64| {
            let mut g = snapshot.lock().unwrap_or_else(|e| e.into_inner());
            g.0.progress.done_bytes = (base + done) as i64;
            if g.1.elapsed() >= Duration::from_millis(250) {
                g.1 = Instant::now();
                publish(s, &g.0);
            }
        };
        runtime::ensure(pin, root, &progress, cancel).await?;
        base += pin.size;
        job.progress.done_bytes = base as i64;
    }
    job.progress.done_items = pins.len() as i64;
    Ok(())
}

pub async fn remove(s: &AppState) -> Result<Value, ApiError> {
    if s.builtin.install.busy() {
        return Err(ApiError::new(
            axum::http::StatusCode::CONFLICT,
            "wait for the download to finish or cancel it first",
        ));
    }
    s.builtin.stop().await;
    let st = s.clone();
    let freed = blocking(move || {
        let root = data_root(&st);
        let mut freed = 0u64;
        for pin in std::iter::once(&LLAMA_SERVER).chain(MODELS.iter()) {
            let entry = runtime::entry_path(pin, &root);
            if entry.exists() {
                freed += match pin.kind {
                    runtime::Kind::File => std::fs::metadata(&entry).map(|m| m.len()).unwrap_or(0),
                    _ => dir_size(&runtime::install_dir(pin, &root)),
                };
                runtime::remove(pin, &root)?;
            }
        }
        let conn = st.db.conn()?;
        conn.execute("DELETE FROM provider WHERE id = ?1", [PROVIDER_ID])?;
        if get_meta(&conn, "active_provider")?.as_deref() == Some(PROVIDER_ID) {
            set_meta(&conn, "active_provider", "")?;
        }
        set_meta(&conn, MODEL_META, "")?;
        Ok(freed)
    })
    .await?;
    s.bump();
    Ok(json!({"ok": true, "freed_bytes": freed}))
}

fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

// The Built-in row carries no address; a run or a connection test gets the live child's here.
pub async fn resolve(
    s: &AppState,
    mut cfg: ProviderConfig,
) -> Result<(ProviderConfig, Option<Lease>), String> {
    if cfg.id.as_deref() != Some(PROVIDER_ID) {
        return Ok((cfg, None));
    }
    let root = data_root(s);
    let model = cfg
        .model
        .clone()
        .filter(|m| model_pin(m).is_some_and(|p| runtime::is_installed(p, &root)))
        .or_else(|| {
            s.db.conn()
                .ok()
                .and_then(|c| chosen_model(&c, &root))
                .map(|p| p.id.to_string())
        });
    let Some(model) = model.filter(|_| runtime::is_installed(&LLAMA_SERVER, &root)) else {
        return Err(
            "The built-in model is not installed. Download it in Settings, Providers, or pick another provider."
                .into(),
        );
    };
    match s.builtin.acquire(&model).await {
        Ok(lease) => {
            cfg.base_url = lease.base_url.clone();
            cfg.model = Some(model);
            Ok((cfg, Some(lease)))
        }
        Err(e) => Err(format!("The built-in model could not start: {e:#}")),
    }
}

pub fn idle_minutes_from(db: hoardini_core::store::Db) -> Arc<dyn Fn() -> u64 + Send + Sync> {
    Arc::new(move || {
        db.conn()
            .ok()
            .and_then(|c| get_meta(&c, "builtin_idle_minutes").ok().flatten())
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(10)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_is_cpu_only_loopback_and_lazy_sized() {
        let a = args(&LaunchSpec {
            exe: PathBuf::from(r"C:\rt\llama-server.exe"),
            model: PathBuf::from(r"C:\m\x.gguf"),
            model_id: "x".into(),
            port: 4321,
            threads: 10,
            ctx: 8192,
        });
        let s = a.join(" ");
        assert!(s.contains(
            r"-m C:\m\x.gguf --host 127.0.0.1 --port 4321 --jinja -t 10 -c 8192 --no-webui"
        ));
        assert!(s.contains("-ngl 0"));
        assert!(s.contains("--cache-type-k q8_0 --cache-type-v q8_0"));
        assert!(s.ends_with("-a x"));
    }

    #[test]
    fn profile_is_small_and_redacted() {
        let p = profile("granite-4.0-micro");
        assert!(p.small && p.redact_paths);
        assert_eq!(p.id.as_deref(), Some(PROVIDER_ID));
        assert_eq!(p.context_window, CONTEXT);
        assert!(
            MODELS
                .iter()
                .all(|m| model_info(m.id).name != "Unknown model")
        );
    }
}

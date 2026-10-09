use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Destination, DestinationTest};
use crate::runtime::{self, RCLONE};
use crate::vault::{SecretStore, SecretString, is_ref};

pub const CONFIG_PASS_REF: &str = "secret://rclone/config-pass";
pub const URI_PREFIX: &str = "rclone:";
const CONFIG_FILE: &str = "rclone.conf";
const ENCRYPTED_HEADER: &str = "# Encrypted rclone configuration File";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const IDLE_STOP: Duration = Duration::from_secs(600);
const START_TIMEOUT: Duration = Duration::from_secs(15);

fn random_hex(bytes: usize) -> String {
    (0..bytes)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect()
}

#[derive(Clone)]
pub struct Endpoint {
    pub addr: String,
    pub user: String,
    pub pass: String,
}

impl Endpoint {
    pub fn random(port: u16) -> Endpoint {
        Endpoint {
            addr: format!("127.0.0.1:{port}"),
            user: format!("hoardini-{}", random_hex(8)),
            pass: random_hex(24),
        }
    }
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Usage {
    active: AtomicUsize,
    last_used: Mutex<Option<Instant>>,
}

// Keeps the idle reaper away while a job still talks to the daemon.
pub struct Lease(Arc<Usage>);

impl Lease {
    fn new(u: &Arc<Usage>) -> Lease {
        u.active.fetch_add(1, Ordering::SeqCst);
        Lease(u.clone())
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        *self.0.last_used.lock() = Some(Instant::now());
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct RcClient {
    ep: Endpoint,
    http: reqwest::blocking::Client,
    _lease: Option<Lease>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct About {
    pub total: Option<i64>,
    pub used: Option<i64>,
    pub free: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListItem {
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Size", default)]
    pub size: i64,
    #[serde(rename = "IsDir", default)]
    pub is_dir: bool,
    #[serde(rename = "Hashes", default)]
    pub hashes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobStatus {
    #[serde(default)]
    pub finished: bool,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub output: Value,
}

impl RcClient {
    pub fn new(ep: Endpoint) -> RcClient {
        Self::with_lease(ep, None)
    }

    fn with_lease(ep: Endpoint, lease: Option<Lease>) -> RcClient {
        let http = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(900))
            .build()
            .expect("reqwest blocking client");
        RcClient {
            ep,
            http,
            _lease: lease,
        }
    }

    pub fn call(&self, method: &str, params: &Value) -> Result<Value> {
        let resp = self
            .http
            .post(format!("http://{}/{method}", self.ep.addr))
            .basic_auth(&self.ep.user, Some(&self.ep.pass))
            .header("Content-Type", "application/json")
            .body(params.to_string())
            .send()
            .with_context(|| format!("rclone {method}: the rclone daemon is not reachable"))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            let msg = parsed
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or(text.as_str());
            bail!(
                "rclone {method}: {}",
                msg.chars().take(500).collect::<String>()
            );
        }
        Ok(parsed)
    }

    pub fn version(&self) -> Result<String> {
        let v = self.call("core/version", &json!({}))?;
        Ok(v["version"].as_str().unwrap_or_default().to_string())
    }

    pub fn list_remotes(&self) -> Result<Vec<String>> {
        let v = self.call("config/listremotes", &json!({}))?;
        Ok(v["remotes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|r| r.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn config_create(
        &self,
        name: &str,
        kind: &str,
        parameters: &Map<String, Value>,
    ) -> Result<()> {
        self.call(
            "config/create",
            &json!({
                "name": name,
                "type": kind,
                "parameters": parameters,
                "opt": {"obscure": true, "nonInteractive": true, "noOutput": true},
            }),
        )
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("could not create the rclone remote {name}: {e}"))
    }

    pub fn config_update(&self, name: &str, parameters: &Map<String, Value>) -> Result<()> {
        self.call(
            "config/update",
            &json!({
                "name": name,
                "parameters": parameters,
                "opt": {"obscure": true, "nonInteractive": true, "noOutput": true},
            }),
        )
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("could not update the rclone remote {name}: {e}"))
    }

    pub fn config_delete(&self, name: &str) -> Result<()> {
        self.call("config/delete", &json!({"name": name}))
            .map(|_| ())
    }

    pub fn about(&self, fs: &str) -> Result<About> {
        let v = self.call("operations/about", &json!({"fs": fs}))?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    pub fn hash_types(&self, fs: &str) -> Result<Vec<String>> {
        let v = self.call("operations/fsinfo", &json!({"fs": fs}))?;
        Ok(v["Hashes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|h| h.as_str().map(str::to_lowercase))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn list(&self, fs: &str, recurse: bool, hash_type: Option<&str>) -> Result<Vec<ListItem>> {
        let mut opt =
            json!({"recurse": recurse, "noModTime": true, "noMimeType": true, "filesOnly": true});
        if let Some(h) = hash_type {
            opt["showHash"] = json!(true);
            opt["hashTypes"] = json!([h]);
        }
        let v = self.call(
            "operations/list",
            &json!({"fs": fs, "remote": "", "opt": opt}),
        )?;
        Ok(serde_json::from_value(v["list"].clone()).unwrap_or_default())
    }

    pub fn start(&self, method: &str, mut params: Value, group: &str) -> Result<i64> {
        params["_async"] = json!(true);
        params["_group"] = json!(group);
        let v = self.call(method, &params)?;
        v["jobid"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("rclone {method} returned no job id"))
    }

    pub fn job_status(&self, id: i64) -> Result<JobStatus> {
        let v = self.call("job/status", &json!({"jobid": id}))?;
        Ok(serde_json::from_value(v)?)
    }

    pub fn group_bytes(&self, group: &str) -> Result<i64> {
        let v = self.call("core/stats", &json!({"group": group}))?;
        Ok(v["bytes"].as_i64().unwrap_or(0))
    }

    pub fn job_stop(&self, id: i64) -> Result<()> {
        self.call("job/stop", &json!({"jobid": id})).map(|_| ())
    }

    pub fn stats_reset(&self, group: &str) {
        let _ = self.call("core/stats-delete", &json!({"group": group}));
    }

    pub fn delete_file(&self, fs: &str, remote: &str) -> Result<()> {
        self.call(
            "operations/deletefile",
            &json!({"fs": fs, "remote": remote}),
        )
        .map(|_| ())
    }

    pub fn purge(&self, fs: &str) -> Result<()> {
        self.call("operations/purge", &json!({"fs": fs, "remote": ""}))
            .map(|_| ())
    }
}

pub fn daemon_args(ep: &Endpoint, config: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec!["rcd".into()];
    for (k, v) in [
        ("--rc-addr", ep.addr.as_str()),
        ("--rc-user", ep.user.as_str()),
        ("--rc-pass", ep.pass.as_str()),
    ] {
        a.push(k.into());
        a.push(v.into());
    }
    a.push("--config".into());
    a.push(config.as_os_str().to_owned());
    a
}

fn free_port() -> Result<u16> {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(l.local_addr()?.port())
}

fn rclone_command(exe: &Path) -> Command {
    let mut cmd = Command::new(exe);
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("RCLONE_")
        {
            cmd.env_remove(&k);
        }
    }
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

fn config_pass(secrets: &dyn SecretStore) -> Result<SecretString> {
    if let Some(p) = secrets.get(CONFIG_PASS_REF)? {
        return Ok(p);
    }
    let pass = random_hex(32);
    secrets.set(CONFIG_PASS_REF, SecretString::from(pass.clone()))?;
    Ok(SecretString::from(pass))
}

fn config_is_encrypted(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|b| b.starts_with(ENCRYPTED_HEADER.as_bytes()))
}

fn ensure_encrypted_config(exe: &Path, config: &Path, pass: &SecretString) -> Result<()> {
    if config_is_encrypted(config) {
        return Ok(());
    }
    if let Some(dir) = config.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut child = rclone_command(exe)
        .arg("--config")
        .arg(config)
        .args(["config", "encryption", "set"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("could not start rclone to encrypt its config")?;
    if let Some(mut stdin) = child.stdin.take() {
        let p = pass.expose_secret();
        let _ = write!(stdin, "{p}\n{p}\n");
    }
    let status = child.wait()?;
    if !status.success() || !config_is_encrypted(config) {
        bail!(
            "rclone would not encrypt its config file; refusing to store cloud credentials in plain text"
        );
    }
    Ok(())
}

struct Running {
    child: Child,
    ep: Endpoint,
}

pub struct RcloneDaemon {
    root: PathBuf,
    auto_install: bool,
    usage: Arc<Usage>,
    running: Mutex<Option<Running>>,
}

impl RcloneDaemon {
    pub fn new(root: &Path) -> RcloneDaemon {
        RcloneDaemon {
            root: root.to_path_buf(),
            auto_install: true,
            usage: Arc::new(Usage::default()),
            running: Mutex::new(None),
        }
    }

    pub fn without_auto_install(mut self) -> RcloneDaemon {
        self.auto_install = false;
        self
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join(CONFIG_FILE)
    }

    pub fn is_running(&self) -> bool {
        self.running.lock().is_some()
    }

    fn exe(&self) -> Result<PathBuf> {
        let entry = runtime::entry_path(&RCLONE, &self.root);
        if !entry.exists() && self.auto_install {
            ensure_blocking(&self.root, &|_, _| {})?;
        }
        if !entry.exists() {
            bail!("rclone is not installed; install it from the destinations settings");
        }
        runtime::verify_before_run(&entry, &RCLONE)?;
        Ok(entry)
    }

    fn kill(&self, run: &mut Option<Running>) {
        if let Some(mut r) = run.take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
    }

    pub fn stop(&self) {
        let mut run = self.running.lock();
        self.kill(&mut run);
    }

    // True while a child is still running afterwards.
    fn reap_idle(&self) -> bool {
        let mut run = self.running.lock();
        let dead = run
            .as_mut()
            .is_some_and(|r| !matches!(r.child.try_wait(), Ok(None)));
        let idle = self
            .usage
            .last_used
            .lock()
            .is_none_or(|t| t.elapsed() >= IDLE_STOP);
        if dead || (idle && self.usage.active.load(Ordering::SeqCst) == 0) {
            self.kill(&mut run);
        }
        run.is_some()
    }

    pub fn client(&self, secrets: &dyn SecretStore) -> Result<RcClient> {
        let mut run = self.running.lock();
        if let Some(r) = run.as_mut() {
            let alive = matches!(r.child.try_wait(), Ok(None));
            if alive {
                let c = RcClient::with_lease(r.ep.clone(), Some(Lease::new(&self.usage)));
                if c.version().is_ok() {
                    return Ok(c);
                }
            }
            self.kill(&mut run);
        }
        let (child, ep) = self.launch(secrets)?;
        *run = Some(Running {
            child,
            ep: ep.clone(),
        });
        let lease = Lease::new(&self.usage);
        drop(run);
        ensure_reaper();
        Ok(RcClient::with_lease(ep, Some(lease)))
    }

    fn launch(&self, secrets: &dyn SecretStore) -> Result<(Child, Endpoint)> {
        let exe = self.exe()?;
        let pass = config_pass(secrets)?;
        let config = self.config_path();
        if config.exists() && !config_is_encrypted(&config) {
            tracing::warn!("rclone.conf was not encrypted; encrypting it");
        }
        ensure_encrypted_config(&exe, &config, &pass)?;
        let mut last = anyhow::anyhow!("rclone did not start");
        for _ in 0..3 {
            let ep = Endpoint::random(free_port()?);
            let mut cmd = rclone_command(&exe);
            cmd.args(daemon_args(&ep, &config))
                .env("RCLONE_CONFIG_PASS", pass.expose_secret())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let mut child = cmd.spawn().context("could not start rclone")?;
            if let Err(e) = hoardini_win::kill_child_on_exit(&child) {
                tracing::debug!(error = %e, "rclone is not tied to this process's lifetime");
            }
            let client = RcClient::new(ep.clone());
            let deadline = Instant::now() + START_TIMEOUT;
            loop {
                if client.version().is_ok() {
                    return Ok((child, ep));
                }
                if !matches!(child.try_wait(), Ok(None)) {
                    last = anyhow::anyhow!("rclone exited while starting");
                    break;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    last = anyhow::anyhow!("rclone did not answer within {START_TIMEOUT:?}");
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        Err(last)
    }

    // The exe path for one-shot commands such as `rclone authorize`.
    pub fn verified_exe(&self) -> Result<PathBuf> {
        self.exe()
    }
}

type Registry = Vec<(PathBuf, Arc<RcloneDaemon>)>;
static DAEMONS: LazyLock<Mutex<Registry>> = LazyLock::new(|| Mutex::new(Vec::new()));
// True while the reaper thread runs; it lives only as long as some daemon child does.
static REAPER: Mutex<bool> = Mutex::new(false);

pub fn daemon() -> Arc<RcloneDaemon> {
    let root = crate::paths::data_dir();
    let mut all = DAEMONS.lock();
    if let Some((_, d)) = all.iter().find(|(r, _)| *r == root) {
        return d.clone();
    }
    let d = Arc::new(RcloneDaemon::new(&root));
    all.push((root, d.clone()));
    d
}

// Called after a launch with no daemon lock held; the reaper checks and exits under REAPER, so none is missed.
fn ensure_reaper() {
    let mut on = REAPER.lock();
    if *on {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("hoardini-rclone-reaper".into())
        .spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(30));
                let mut on = REAPER.lock();
                let daemons: Vec<_> = DAEMONS.lock().iter().map(|(_, d)| d.clone()).collect();
                let mut alive = false;
                for d in daemons {
                    alive |= d.reap_idle();
                }
                if !alive {
                    *on = false;
                    return;
                }
            }
        });
    match spawned {
        Ok(_) => *on = true,
        Err(e) => tracing::warn!(error = %e, "rclone idle reaper did not start"),
    }
}

pub fn stop_all() {
    let daemons: Vec<_> = DAEMONS.lock().iter().map(|(_, d)| d.clone()).collect();
    for d in daemons {
        d.stop();
    }
}

// Sync entry to `runtime::ensure` for threads that are not on an async runtime.
pub fn ensure_blocking(
    root: &Path,
    progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<PathBuf> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    rt.block_on(runtime::ensure(&RCLONE, root, progress, &cancel))
}

pub fn open_client(secrets: &dyn SecretStore, ep: Option<&Endpoint>) -> Result<RcClient> {
    match ep {
        Some(ep) => Ok(RcClient::new(ep.clone())),
        None => daemon().client(secrets),
    }
}

#[derive(Clone, Debug)]
pub struct RcloneConfig {
    pub remote: String,
    pub kind: String,
    pub path: String,
    pub params: BTreeMap<String, String>,
}

fn clean_path(p: &str) -> String {
    p.replace('\\', "/")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

pub fn parse_config(config: &Value) -> Result<RcloneConfig> {
    let remote = config
        .get("remote")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if !valid_name(remote) {
        bail!("config.remote must be a name made of letters, digits, - and _");
    }
    let kind = config
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if !valid_name(kind) {
        bail!("config.type must be an rclone backend such as drive, s3, or sftp");
    }
    let mut params = BTreeMap::new();
    match config.get("params") {
        None | Some(Value::Null) => {}
        Some(Value::Object(o)) => {
            for (k, v) in o {
                let s = scalar(v).ok_or_else(|| {
                    anyhow::anyhow!("config.params.{k} must be a string, number, or boolean")
                })?;
                params.insert(k.clone(), s);
            }
        }
        Some(_) => bail!("config.params must be an object"),
    }
    let path = clean_path(config.get("path").and_then(Value::as_str).unwrap_or(""));
    Ok(RcloneConfig {
        remote: remote.to_string(),
        kind: kind.to_string(),
        path,
        params,
    })
}

pub fn validate(config: &mut Value) -> Result<()> {
    let mut cfg = parse_config(config)?;
    if cfg.kind == "drive" {
        let has = |k: &str| cfg.params.get(k).is_some_and(|v| !v.trim().is_empty());
        if !has("client_id") || !has("client_secret") {
            bail!(
                "Google Drive needs your own OAuth client ID and secret; rclone's shared one is being retired"
            );
        }
        match cfg.params.get("scope").map(String::as_str) {
            None | Some("") => {
                cfg.params.insert("scope".into(), "drive.file".into());
            }
            Some("drive.file") => {}
            Some(_) => bail!("Google Drive access is limited to the drive.file scope"),
        }
    }
    config["remote"] = Value::String(cfg.remote);
    config["type"] = Value::String(cfg.kind);
    config["path"] = Value::String(cfg.path);
    config["params"] = json!(cfg.params);
    Ok(())
}

impl RcloneConfig {
    pub fn root_fs(&self) -> String {
        format!("{}:", self.remote)
    }

    pub fn fs(&self, path: &str) -> String {
        format!("{}:{}", self.remote, path)
    }

    pub fn join(&self, rel: &str) -> String {
        [self.path.as_str(), rel.trim_matches('/')]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("/")
    }

    pub fn uri(&self, path: &str) -> String {
        format!("{URI_PREFIX}{}:{path}", self.remote)
    }

    fn materialize(&self, secrets: &dyn SecretStore) -> Result<Map<String, Value>> {
        let mut out = Map::new();
        for (k, v) in &self.params {
            let value = if is_ref(v) {
                secrets
                    .get(v)?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "the secret for {k} is missing from the vault; enter it again"
                        )
                    })?
                    .expose_secret()
                    .to_string()
            } else {
                v.clone()
            };
            out.insert(k.clone(), Value::String(value));
        }
        Ok(out)
    }
}

pub fn parse_uri(uri: &str) -> Option<(&str, &str)> {
    uri.strip_prefix(URI_PREFIX)?.split_once(':')
}

// An existing remote is kept: rclone writes refreshed OAuth tokens back into it.
pub fn prepare_remote(
    client: &RcClient,
    secrets: &dyn SecretStore,
    cfg: &RcloneConfig,
) -> Result<()> {
    if client.list_remotes()?.contains(&cfg.remote) {
        return Ok(());
    }
    client.config_create(&cfg.remote, &cfg.kind, &cfg.materialize(secrets)?)
}

// prepare_remote keeps an existing remote as-is, so edits to a destination must be pushed here.
pub fn sync_remote(secrets: &dyn SecretStore, old: Option<&Value>, new: &Value) -> Result<()> {
    let cfg = parse_config(new)?;
    let old = old.and_then(|o| parse_config(o).ok());
    let client = open_client(secrets, None)?;
    let remotes = client.list_remotes()?;
    if let Some(o) = &old
        && o.remote != cfg.remote
        && remotes.contains(&o.remote)
    {
        client.config_delete(&o.remote)?;
    }
    let params = cfg.materialize(secrets)?;
    if !remotes.contains(&cfg.remote) {
        return client.config_create(&cfg.remote, &cfg.kind, &params);
    }
    if old.as_ref().is_some_and(|o| o.kind != cfg.kind) {
        client.config_delete(&cfg.remote)?;
        return client.config_create(&cfg.remote, &cfg.kind, &params);
    }
    client.config_update(&cfg.remote, &params)
}

pub fn remove_remote(secrets: &dyn SecretStore, remote: &str) -> Result<()> {
    let client = open_client(secrets, None)?;
    if client.list_remotes()?.iter().any(|r| r == remote) {
        client.config_delete(remote)?;
    }
    Ok(())
}

fn probe_write(client: &RcClient, cfg: &RcloneConfig) -> Result<()> {
    let name = format!(".hoardini-write-test-{}", uuid::Uuid::now_v7().simple());
    let dir = tempfile_dir()?;
    std::fs::write(dir.join(&name), b"hoardini")?;
    let fs = cfg.fs(&cfg.path);
    let put = client.call(
        "operations/copyfile",
        &json!({
            "srcFs": dir.to_string_lossy(), "srcRemote": name,
            "dstFs": fs, "dstRemote": name,
        }),
    );
    let _ = std::fs::remove_dir_all(&dir);
    put.map_err(|e| anyhow::anyhow!("not writable: {e}"))?;
    client.delete_file(&fs, &name)
}

fn tempfile_dir() -> Result<PathBuf> {
    let dir =
        std::env::temp_dir().join(format!("hoardini-probe-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn test(dest: &Destination, secrets: &dyn SecretStore) -> DestinationTest {
    test_at(dest, secrets, None)
}

pub fn test_at(
    dest: &Destination,
    secrets: &dyn SecretStore,
    ep: Option<&Endpoint>,
) -> DestinationTest {
    let run = || -> Result<Option<i64>> {
        let cfg = parse_config(&dest.config)?;
        let client = open_client(secrets, ep)?;
        prepare_remote(&client, secrets, &cfg)?;
        let free = client.about(&cfg.root_fs()).ok().and_then(|a| a.free);
        probe_write(&client, &cfg)?;
        Ok(free)
    };
    match run() {
        Ok(free) => DestinationTest {
            ok: true,
            free_bytes: free,
            error: None,
        },
        Err(e) => DestinationTest {
            ok: false,
            free_bytes: None,
            error: Some(format!("{e:#}")),
        },
    }
}

// Runs `rclone authorize` (it opens the browser) and returns the token JSON it prints.
pub fn authorize(
    exe: &Path,
    kind: &str,
    client_id: Option<&str>,
    client_secret: Option<&str>,
    timeout: Duration,
) -> Result<SecretString> {
    if !valid_name(kind) {
        bail!("not an rclone backend type: {kind}");
    }
    let mut cmd = rclone_command(exe);
    cmd.args(["authorize", kind]);
    if let (Some(id), Some(secret)) = (client_id, client_secret) {
        cmd.arg(id).arg(secret);
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("could not start rclone authorize")?;
    // It waits on the browser's OAuth callback; closing the app mid sign-in must not leave it behind.
    if let Err(e) = hoardini_win::kill_child_on_exit(&child) {
        tracing::debug!(error = %e, "rclone authorize is not tied to this process's lifetime");
    }
    let stdout = child
        .stdout
        .take()
        .context("no stdout from rclone authorize")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::BufReader::new(stdout), &mut text);
        let _ = tx.send(text);
    });
    let out = match rx.recv_timeout(timeout) {
        Ok(t) => t,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "authorization timed out; finish the sign-in in the browser window and try again"
            );
        }
    };
    let _ = child.wait();
    token_from_output(&out)
        .map(SecretString::from)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "rclone authorize did not return a token; the sign-in may have been cancelled"
            )
        })
}

pub fn token_from_output(out: &str) -> Option<String> {
    out.lines()
        .map(str::trim)
        .find(|l| l.starts_with('{') && l.ends_with('}') && l.contains("access_token"))
        .map(str::to_string)
}

pub fn is_installed() -> bool {
    let root = crate::paths::data_dir();
    runtime::verify_before_run(&runtime::entry_path(&RCLONE, &root), &RCLONE).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rules() {
        let mut ok = json!({"remote": "my-s3", "type": "s3", "path": "\\backups//hoardini/",
            "params": {"provider": "AWS", "secret_access_key": "secret://destination/x/sk"}});
        validate(&mut ok).unwrap();
        assert_eq!(ok["path"], "backups/hoardini");
        let mut no_client = json!({"remote": "gd", "type": "drive"});
        assert!(validate(&mut no_client).is_err());
        let mut broad = json!({"remote": "gd", "type": "drive",
            "params": {"client_id": "id", "client_secret": "secret://rclone/cs", "scope": "drive"}});
        assert!(validate(&mut broad).is_err());
        let mut drive = json!({"remote": "gd", "type": "drive",
            "params": {"client_id": "id", "client_secret": "secret://rclone/cs"}});
        validate(&mut drive).unwrap();
        assert_eq!(drive["params"]["scope"], "drive.file");
        assert!(validate(&mut json!({"remote": "bad name", "type": "s3"})).is_err());
        assert_eq!(parse_uri("rclone:gd:a/b:c"), Some(("gd", "a/b:c")));
    }

    #[test]
    fn token_is_picked_out_of_authorize_output() {
        let out = "Paste the following into your remote machine --->\r\n{\"access_token\":\"a\",\"expiry\":\"x\"}\r\n<---End paste\r\n";
        assert_eq!(
            token_from_output(out).as_deref(),
            Some("{\"access_token\":\"a\",\"expiry\":\"x\"}")
        );
        assert!(token_from_output("nothing here").is_none());
    }
}

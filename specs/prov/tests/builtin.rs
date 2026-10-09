use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hoardini_core::runtime::{self, LLAMA_SERVER, MODELS};
use hoardini_core::vault::SecretStore;
use hoardini_server::builtin::{self, Builtin, ChildProc, Launched, Launcher};
use hoardini_server::{ServerHandle, ServerOptions, start_with_launcher};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const TOKEN: &str = "builtin-test-token";
const REPLY: &str = "Hello from the built-in model.";

struct FakeChild {
    alive: Arc<AtomicBool>,
    task: tokio::task::AbortHandle,
}

impl ChildProc for FakeChild {
    fn pid(&self) -> Option<u32> {
        None
    }
    fn exited(&mut self) -> Option<String> {
        (!self.alive.load(Ordering::SeqCst)).then(|| "exited".to_string())
    }
    fn kill(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        self.task.abort();
    }
}

#[derive(Default)]
struct FakeLauncher {
    launches: AtomicUsize,
    children: Mutex<Vec<Arc<AtomicBool>>>,
}

impl FakeLauncher {
    fn launches(&self) -> usize {
        self.launches.load(Ordering::SeqCst)
    }
    fn alive(&self, i: usize) -> bool {
        self.children.lock().unwrap()[i].load(Ordering::SeqCst)
    }
    fn crash(&self, i: usize) {
        self.children.lock().unwrap()[i].store(false, Ordering::SeqCst);
    }
}

fn sse_reply() -> String {
    let chunk = |delta: Value, finish: Value| {
        format!(
            "data: {}\n\n",
            json!({"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
        )
    };
    format!(
        "{}{}data: [DONE]\n\n",
        chunk(json!({"content": REPLY}), Value::Null),
        chunk(json!({}), json!("stop"))
    )
}

// A stand-in llama-server: /health is ready at once, chat answers one fixed text turn.
async fn serve(l: TcpListener) {
    loop {
        let Ok((mut s, _)) = l.accept().await else {
            return;
        };
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                let n = s.read(&mut tmp).await.unwrap_or(0);
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&tmp[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(he) = text.find("\r\n\r\n") {
                    let cl = text[..he]
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if buf.len() >= he + 4 + cl {
                        break;
                    }
                }
            }
            let head = String::from_utf8_lossy(&buf).to_string();
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
            let (ctype, body) = if path.starts_with("/health") {
                ("application/json", r#"{"status":"ok"}"#.to_string())
            } else if path.ends_with("/chat/completions") {
                ("text/event-stream", sse_reply())
            } else {
                ("application/json", r#"{"data":[]}"#.to_string())
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes()).await;
            let _ = s.shutdown().await;
        });
    }
}

impl Launcher for FakeLauncher {
    fn launch(
        &self,
        _model_id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Launched>> + Send + 'static>> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        let alive = Arc::new(AtomicBool::new(true));
        self.children.lock().unwrap().push(alive.clone());
        Box::pin(async move {
            let l = TcpListener::bind("127.0.0.1:0").await?;
            let addr = l.local_addr()?;
            let task = tokio::spawn(serve(l)).abort_handle();
            let child: Box<dyn ChildProc> = Box::new(FakeChild { alive, task });
            Ok(Launched {
                child,
                root_url: format!("http://{addr}"),
                api_key: "test-key".into(),
            })
        })
    }
}

fn supervisor(idle_minutes: u64) -> (Arc<FakeLauncher>, Arc<Builtin>) {
    let fake = Arc::new(FakeLauncher::default());
    let b = Builtin::new(fake.clone(), Arc::new(move || idle_minutes), None);
    (fake, b)
}

// Traces: PROV-BUILTIN-LAZY
#[tokio::test]
async fn child_starts_on_first_use_and_is_reused() {
    let (fake, b) = supervisor(10);
    assert_eq!(fake.launches(), 0);
    assert!(!b.is_running());
    let lease = b.acquire("lfm2.5-1.2b-instruct").await.unwrap();
    assert_eq!(fake.launches(), 1);
    assert!(b.is_running());
    assert!(lease.base_url.starts_with("http://127.0.0.1:") && lease.base_url.ends_with("/v1"));
    let again = b.acquire("lfm2.5-1.2b-instruct").await.unwrap();
    assert_eq!(again.base_url, lease.base_url);
    assert_eq!(fake.launches(), 1);
}

// Traces: PROV-BUILTIN-IDLE
#[tokio::test]
async fn idle_child_stops_after_the_setting_and_never_while_in_use() {
    let (fake, b) = supervisor(10);
    let lease = b.acquire("m").await.unwrap();
    assert!(
        !b.check_idle(Instant::now() + Duration::from_secs(3600))
            .await,
        "a run holds it"
    );
    drop(lease);
    let t = Instant::now();
    assert!(!b.check_idle(t + Duration::from_secs(9 * 60)).await);
    assert!(fake.alive(0));
    assert!(b.check_idle(t + Duration::from_secs(10 * 60 + 1)).await);
    assert!(!fake.alive(0));
    assert!(!b.is_running());

    let (fake, b) = supervisor(0);
    drop(b.acquire("m").await.unwrap());
    assert!(
        b.check_idle(Instant::now()).await,
        "0 minutes stops it as soon as it is idle"
    );
    assert!(!fake.alive(0));
}

// Traces: PROV-BUILTIN-RESTART
#[tokio::test]
async fn crashed_child_is_restarted_on_next_use() {
    let (fake, b) = supervisor(10);
    let first = b.acquire("m").await.unwrap().base_url.clone();
    fake.crash(0);
    let second = b.acquire("m").await.unwrap().base_url.clone();
    assert_eq!(fake.launches(), 2);
    assert_ne!(first, second);
    assert!(fake.alive(1));
    let (fake, b) = supervisor(10);
    drop(b.acquire("a").await.unwrap());
    drop(b.acquire("b").await.unwrap());
    assert_eq!(fake.launches(), 2, "switching models restarts the child");
    assert!(!fake.alive(0));
}

// Traces: PROV-BUILTIN-CHILD
#[test]
fn child_command_line_is_loopback_cpu_only_with_cores_minus_two() {
    let spec = builtin::LaunchSpec {
        exe: r"C:\d\runtime\llama.cpp-b11503\llama-server.exe".into(),
        model: r"C:\d\models\m.gguf".into(),
        model_id: "m".into(),
        port: 51000,
        threads: builtin::threads(),
        ctx: builtin::CONTEXT,
    };
    let a = builtin::args(&spec);
    let pair = |flag: &str| a.iter().position(|x| x == flag).map(|i| a[i + 1].clone());
    assert_eq!(pair("--host").as_deref(), Some("127.0.0.1"));
    assert_eq!(pair("--port").as_deref(), Some("51000"));
    assert_eq!(pair("-m").as_deref(), Some(r"C:\d\models\m.gguf"));
    assert_eq!(pair("-ngl").as_deref(), Some("0"));
    assert_eq!(pair("-c"), Some(builtin::CONTEXT.to_string()));
    let cores = hoardini_win::physical_cores();
    assert_eq!(pair("-t"), Some(cores.saturating_sub(2).max(1).to_string()));
    assert!(a.contains(&"--jinja".to_string()) && a.contains(&"--no-webui".to_string()));
    let p = builtin::profile("m");
    assert!(p.small && p.redact_paths);
}

#[derive(Default)]
struct MemStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemStore {
    fn get(&self, r: &str) -> anyhow::Result<Option<SecretString>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(r)
            .cloned()
            .map(SecretString::from))
    }
    fn set(&self, r: &str, v: SecretString) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(r.into(), v.expose_secret().into());
        Ok(())
    }
    fn delete(&self, r: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().remove(r);
        Ok(())
    }
    fn refs(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.0.lock().unwrap().keys().cloned().collect())
    }
}

// Stand-ins for the downloaded runtime and model: present with an install record, as after `ensure`.
fn fake_install(root: &Path) {
    let exe = runtime::entry_path(&LLAMA_SERVER, root);
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, vec![0u8; 2048]).unwrap();
    std::fs::write(exe.parent().unwrap().join(".verified"), "{}").unwrap();
    let model = runtime::entry_path(&MODELS[0], root);
    std::fs::create_dir_all(model.parent().unwrap()).unwrap();
    std::fs::write(&model, vec![0u8; 4096]).unwrap();
    std::fs::write(format!("{}.verified", model.display()), "{}").unwrap();
}

struct Server {
    h: ServerHandle,
    dir: tempfile::TempDir,
    fake: Arc<FakeLauncher>,
    http: reqwest::Client,
}

impl Server {
    async fn start(installed: bool) -> Server {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("h.db");
        if installed {
            fake_install(dir.path());
            let conn = hoardini_core::store::Db::open(&db).unwrap().conn().unwrap();
            let cfg = builtin::profile(MODELS[0].id);
            conn.execute(
                "INSERT INTO provider(id, name, config_json, is_active, created_ms) VALUES ('builtin', 'Built-in', ?1, 1, 0)",
                [serde_json::to_string(&cfg).unwrap()],
            )
            .unwrap();
            hoardini_core::store::set_meta(&conn, "active_provider", "builtin").unwrap();
        }
        let fake = Arc::new(FakeLauncher::default());
        let h = start_with_launcher(
            ServerOptions {
                port: 0,
                token: Some(TOKEN.into()),
                db_path: db,
                ui_dir: None,
                dialogs: None,
                refresh_on_start: false,
                indexer_exe: Some(dir.path().join("no-indexer.exe")),
            },
            Some(Arc::new(MemStore::default())),
            Some(fake.clone()),
        )
        .await
        .unwrap();
        Server {
            h,
            dir,
            fake,
            http: reqwest::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.h.port)
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        let r = self
            .http
            .get(self.url(path))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
    }
}

// Traces: PROV-BUILTIN-LAZY
#[tokio::test]
async fn a_run_on_the_builtin_profile_starts_the_child_and_streams_its_answer() {
    let s = Server::start(true).await;
    let (_, v) = s.get("/api/builtin").await;
    assert_eq!(v["installed"], true);
    assert_eq!(v["running"], false);
    assert_eq!(
        s.fake.launches(),
        0,
        "nothing starts before the first chat turn"
    );
    let body = json!({"threadId": "t1", "runId": "r1", "messages": [{"id": "m1", "role": "user", "content": "hi"}]});
    let text = s
        .http
        .post(s.url("/api/agent/run"))
        .bearer_auth(TOKEN)
        .json(&body)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(text.contains(REPLY), "{text}");
    assert!(text.contains("RUN_FINISHED"), "{text}");
    assert_eq!(s.fake.launches(), 1);
    let (_, v) = s.get("/api/builtin").await;
    assert_eq!(v["running"], true);
}

// Traces: PROV-BUILTIN-LAZY
#[tokio::test]
async fn a_run_without_the_files_explains_instead_of_starting() {
    let s = Server::start(false).await;
    let conn = hoardini_core::store::Db::open(s.dir.path().join("h.db"))
        .unwrap()
        .conn()
        .unwrap();
    let cfg = builtin::profile(MODELS[0].id);
    conn.execute(
        "INSERT INTO provider(id, name, config_json, is_active, created_ms) VALUES ('builtin', 'Built-in', ?1, 1, 0)",
        [serde_json::to_string(&cfg).unwrap()],
    )
    .unwrap();
    hoardini_core::store::set_meta(&conn, "active_provider", "builtin").unwrap();
    let body = json!({"threadId": "t2", "runId": "r2", "messages": [{"id": "m2", "role": "user", "content": "hi"}]});
    let text = s
        .http
        .post(s.url("/api/agent/run"))
        .bearer_auth(TOKEN)
        .json(&body)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        text.contains("RUN_ERROR") && text.contains("not installed"),
        "{text}"
    );
    assert_eq!(s.fake.launches(), 0);
}

// Traces: PROV-BUILTIN-STATUS
#[tokio::test]
async fn status_lists_sizes_licenses_and_free_space() {
    let s = Server::start(false).await;
    let (code, v) = s.get("/api/builtin").await;
    assert_eq!(code, 200);
    assert_eq!(v["installed"], false);
    assert_eq!(v["model"], Value::Null);
    assert_eq!(v["runtime"]["size"], LLAMA_SERVER.size);
    assert_eq!(v["runtime"]["version"], LLAMA_SERVER.version);
    let c = v["candidates"].as_array().unwrap();
    assert_eq!(c.len(), MODELS.len());
    for (cand, pin) in c.iter().zip(MODELS) {
        assert_eq!(cand["id"], pin.id);
        assert_eq!(cand["size"], pin.size);
        assert!(
            cand["license_url"]
                .as_str()
                .unwrap()
                .starts_with("https://")
        );
        assert_ne!(cand["license"], "see link");
    }
    assert!(v["free_bytes"].as_u64().unwrap() > 0);
    assert_eq!(v["recommended"], json!(builtin::RECOMMENDED));
    let r = s
        .http
        .post(s.url("/api/builtin/install"))
        .bearer_auth(TOKEN)
        .json(&json!({"model_id": "not-a-model"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 400);
}

// Traces: PROV-BUILTIN-REMOVE
#[tokio::test]
async fn remove_stops_the_child_deletes_the_files_and_the_profile() {
    let s = Server::start(true).await;
    let body = json!({"threadId": "t3", "runId": "r3", "messages": [{"id": "m3", "role": "user", "content": "hi"}]});
    s.http
        .post(s.url("/api/agent/run"))
        .bearer_auth(TOKEN)
        .json(&body)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(s.fake.alive(0));
    let r: Value = s
        .http
        .delete(s.url("/api/builtin"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["ok"], true);
    assert!(r["freed_bytes"].as_u64().unwrap() >= 2048 + 4096);
    assert!(!s.fake.alive(0));
    assert!(!runtime::entry_path(&LLAMA_SERVER, s.dir.path()).exists());
    assert!(!runtime::entry_path(&MODELS[0], s.dir.path()).exists());
    let (_, providers) = s.get("/api/providers").await;
    assert!(
        providers
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["id"] != "builtin")
    );
    let (_, v) = s.get("/api/builtin").await;
    assert_eq!(v["installed"], false);
    assert_eq!(v["provider_id"], Value::Null);
}

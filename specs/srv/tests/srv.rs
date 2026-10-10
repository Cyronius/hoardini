use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hoardini_core::vault::SecretStore;
use hoardini_server::{ServerHandle, ServerOptions, journal_wait, route_list, start_with};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

const TOKEN: &str = "test-token-123";

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

struct Fixture {
    h: ServerHandle,
    base: String,
    dir: tempfile::TempDir,
    http: reqwest::Client,
}

impl Fixture {
    async fn new() -> Fixture {
        Self::with(None, None).await
    }

    async fn with(ui_dir: Option<PathBuf>, indexer: Option<PathBuf>) -> Fixture {
        Self::in_dir(tempfile::tempdir().unwrap(), ui_dir, indexer, false).await
    }

    async fn in_dir(
        dir: tempfile::TempDir,
        ui_dir: Option<PathBuf>,
        indexer: Option<PathBuf>,
        refresh_on_start: bool,
    ) -> Fixture {
        let h = start_with(
            ServerOptions {
                port: 0,
                token: Some(TOKEN.into()),
                db_path: dir.path().join("h.db"),
                ui_dir,
                dialogs: None,
                refresh_on_start,
                indexer_exe: Some(
                    indexer.unwrap_or_else(|| dir.path().join("no-such-indexer.exe")),
                ),
            },
            Some(Arc::new(MemStore::default())),
        )
        .await
        .unwrap();
        Fixture {
            base: format!("http://127.0.0.1:{}", h.port),
            h,
            dir,
            http: reqwest::Client::new(),
        }
    }

    async fn stream(&self) -> reqwest::Response {
        let r = self
            .authed("GET", "/api/status/stream")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        r
    }

    async fn presence(&self, looking: bool) {
        let (st, _) = self
            .call("POST", "/api/presence", Some(json!({"looking": looking})))
            .await;
        assert_eq!(st, 200);
    }
}

// A completed index of a fake C:\, last refreshed `age` ago.
fn seed_index(db_path: &Path, age: Duration) {
    let db = hoardini_core::store::Db::open(db_path).unwrap();
    let conn = db.conn().unwrap();
    let fs = hoardini_core::index::FakeFs::new(r"C:\");
    fs.file(r"C:\data\big.bin", 3 << 20);
    let out = hoardini_core::index::refresh(
        &conn,
        &fs,
        r"C:\",
        &hoardini_core::index::RefreshOptions::default(),
    )
    .unwrap();
    assert!(out.completed);
    set_index_age(&conn, age);
}

fn set_index_age(conn: &rusqlite::Connection, age: Duration) {
    let at = hoardini_core::store::now_ms() - age.as_millis() as i64;
    conn.execute("UPDATE volume SET last_refresh_ms = ?1", [at])
        .unwrap();
}

fn lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .collect()
}

async fn wait_for(path: &Path, pred: impl Fn(&[String]) -> bool, secs: u64) -> Vec<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let l = lines(path);
        if pred(&l) || std::time::Instant::now() > deadline {
            return l;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

impl Fixture {
    fn req(&self, method: &str, path: &str) -> reqwest::RequestBuilder {
        let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        self.http.request(m, format!("{}{path}", self.base))
    }

    fn authed(&self, method: &str, path: &str) -> reqwest::RequestBuilder {
        self.req(method, path).bearer_auth(TOKEN)
    }

    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut r = self.authed(method, path);
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await.unwrap();
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }
}

fn concrete(path: &str) -> String {
    let mut out = String::new();
    let mut in_brace = false;
    for c in path.chars() {
        match c {
            '{' => {
                in_brace = true;
                out.push_str("x1");
            }
            '}' => in_brace = false,
            _ if !in_brace => out.push(c),
            _ => {}
        }
    }
    out
}

// Traces: SRV-TOKEN
#[tokio::test(flavor = "multi_thread")]
async fn every_api_route_requires_the_token() {
    let f = Fixture::new().await;
    let routes = route_list();
    assert!(routes.len() > 40, "route table looks truncated");
    for (method, template) in routes {
        let path = concrete(template);
        let none = f.req(method, &path).send().await.unwrap();
        assert_eq!(none.status(), 401, "{method} {path} without a token");
        let wrong = f
            .req(method, &path)
            .bearer_auth("nope")
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), 401, "{method} {path} with a wrong token");
        let wrong_q = f
            .req(method, &format!("{path}?token=nope"))
            .send()
            .await
            .unwrap();
        assert_eq!(wrong_q.status(), 401, "{method} {path} with a wrong ?token");
        let right = f.authed(method, &path).send().await.unwrap();
        assert_ne!(right.status(), 401, "{method} {path} with the token");
        let right_q = f
            .req(method, &format!("{path}?token={TOKEN}"))
            .send()
            .await
            .unwrap();
        assert_ne!(right_q.status(), 401, "{method} {path} with ?token");
    }
    for p in ["/api", "/api/", "/api/no-such-route"] {
        let r = f.req("GET", p).send().await.unwrap();
        assert_eq!(r.status(), 401, "unknown {p} must also be gated");
    }
    let r = f.authed("GET", "/api/no-such-route").send().await.unwrap();
    assert_eq!(r.status(), 404);
    assert!(r.text().await.unwrap().contains("\"error\""));
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-STATIC-OPEN
#[tokio::test(flavor = "multi_thread")]
async fn static_ui_is_served_without_a_token_with_csp() {
    let ui = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(ui.path().join("assets")).unwrap();
    std::fs::write(ui.path().join("index.html"), "<html>HOARDINI-UI</html>").unwrap();
    std::fs::write(ui.path().join("assets").join("app.js"), "console.log(1)").unwrap();
    let f = Fixture::with(Some(ui.path().to_path_buf()), None).await;

    let r = f.req("GET", "/").send().await.unwrap();
    assert_eq!(r.status(), 200);
    let csp = r.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(csp.contains("default-src 'self'") && csp.contains("script-src 'self'"));
    assert!(r.headers().get("access-control-allow-origin").is_none());
    assert!(r.text().await.unwrap().contains("HOARDINI-UI"));

    let js = f.req("GET", "/assets/app.js").send().await.unwrap();
    assert_eq!(js.status(), 200);
    assert!(
        js.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("javascript")
    );

    let spa = f.req("GET", "/threads/abc").send().await.unwrap();
    assert_eq!(spa.status(), 200);
    assert!(spa.text().await.unwrap().contains("HOARDINI-UI"));

    assert_eq!(
        f.req("GET", "/assets/missing.js")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );

    let evil = f
        .req("GET", "/")
        .header("Host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(evil.status(), 403);
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-SECRETS-WRITE-ONLY
#[tokio::test(flavor = "multi_thread")]
async fn secret_values_are_never_returned_or_stored_in_config() {
    let f = Fixture::new().await;
    let secret = "sk-very-secret-value";
    let (st, _) = f
        .call(
            "PUT",
            "/api/secrets",
            Some(json!({"ref": "secret://llm/main", "value": secret})),
        )
        .await;
    assert_eq!(st, 200);

    let cfg = json!({"name": "Cloud", "base_url": "https://api.example.com/v1", "api_key": "secret://llm/main"});
    let (st, created) = f.call("POST", "/api/providers", Some(cfg)).await;
    assert_eq!(st, 200, "{created}");
    assert_eq!(created["has_key"], true);
    assert_eq!(created["api_key"], "secret://llm/main");

    let (_, list) = f.call("GET", "/api/providers", None).await;
    assert!(!list.to_string().contains(secret));
    assert_eq!(list[0]["has_key"], true);

    let raw = json!({"name": "Bad", "base_url": "https://api.example.com/v1", "api_key": secret});
    let (st, err) = f.call("POST", "/api/providers", Some(raw)).await;
    assert_eq!(st, 400);
    assert!(!err.to_string().contains(secret));

    let bad = f
        .authed("PUT", "/api/secrets")
        .body(format!("{{\"ref\": \"{secret}\", \"value\": 5}}"))
        .send()
        .await
        .unwrap();
    assert!(bad.status().is_client_error());
    assert!(!bad.text().await.unwrap().contains(secret));

    let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
    let stored: String = db
        .conn()
        .unwrap()
        .query_row("SELECT group_concat(config_json) FROM provider", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(!stored.contains(secret));
    assert!(stored.contains("secret://llm/main"));
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-SECRETS-WRITE-ONLY
#[tokio::test(flavor = "multi_thread")]
async fn a_key_in_a_provider_header_must_be_a_vault_reference() {
    let f = Fixture::new().await;
    let secret = "azure-key-0123456789abcdef";
    for headers in [
        json!({"api-key": secret}),
        json!({"Authorization": format!("Bearer {secret}")}),
        json!({"X-Goog-Api-Key": secret}),
    ] {
        let cfg = json!({"name": "Azure", "base_url": "https://x.openai.azure.com/v1", "headers": headers});
        let (st, err) = f.call("POST", "/api/providers", Some(cfg)).await;
        assert_eq!(st, 400, "{headers} {err}");
        assert!(err.to_string().contains("secret://"), "{err}");
        assert!(!err.to_string().contains(secret));
    }
    let ok = json!({"name": "Azure", "base_url": "https://x.openai.azure.com/v1",
                    "headers": {"api-key": "secret://provider/azure", "HTTP-Referer": "https://hoardini.app"}});
    let (st, created) = f.call("POST", "/api/providers", Some(ok)).await;
    assert_eq!(st, 200, "{created}");
    let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
    let stored: String = db
        .conn()
        .unwrap()
        .query_row("SELECT group_concat(config_json) FROM provider", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(!stored.contains(secret), "{stored}");
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-SETTINGS
#[tokio::test(flavor = "multi_thread")]
async fn settings_round_trip() {
    let f = Fixture::new().await;
    let (_, s) = f.call("GET", "/api/settings", None).await;
    let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        ["builtin_idle_minutes", "index_size_bytes", "index_speed"]
    );
    assert_eq!(s["index_speed"], "lean");
    assert_eq!(s["builtin_idle_minutes"], 10);
    assert!(s["index_size_bytes"].as_u64().unwrap() > 0);

    let (st, s) = f
        .call(
            "PUT",
            "/api/settings",
            Some(json!({"index_speed": "fast", "builtin_idle_minutes": 0, "index_size_bytes": 1, "threshold_bytes": 5})),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(s["index_speed"], "fast");
    assert_eq!(s["builtin_idle_minutes"], 0);
    assert_ne!(s["index_size_bytes"], 1);
    assert!(s.get("threshold_bytes").is_none());

    let (_, again) = f.call("GET", "/api/settings", None).await;
    assert_eq!(again["index_speed"], "fast");
    for bad in [
        json!({"index_speed": "turbo"}),
        json!({"index_speed": true}),
        json!({"builtin_idle_minutes": "x"}),
    ] {
        let (st, e) = f.call("PUT", "/api/settings", Some(bad.clone())).await;
        assert_eq!(st, 400, "{bad}");
        assert!(e["error"].is_string());
    }
    let (_, again) = f.call("GET", "/api/settings", None).await;
    assert_eq!(
        again["index_speed"], "fast",
        "a rejected PUT changes nothing"
    );
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-MEMORY-CRUD
#[tokio::test(flavor = "multi_thread")]
async fn memory_crud_and_search() {
    let f = Fixture::new().await;
    let (st, m) = f
        .call(
            "POST",
            "/api/memory",
            Some(json!({"scope": "global", "key": "note", "kind": "fact", "text": "Keep the game saves"})),
        )
        .await;
    assert_eq!(st, 200, "{m}");
    let id = m["id"].as_i64().unwrap();
    assert_eq!(m["source"], "user");

    let (_, all) = f.call("GET", "/api/memory", None).await;
    assert_eq!(all.as_array().unwrap().len(), 1);
    let (_, hits) = f.call("GET", "/api/memory?q=saves", None).await;
    assert_eq!(hits[0]["id"], id);
    let (_, none) = f.call("GET", "/api/memory?q=zebra", None).await;
    assert!(none.as_array().unwrap().is_empty());

    let (st, upd) = f
        .call(
            "PUT",
            &format!("/api/memory/{id}"),
            Some(json!({"text": "Keep the game saves forever", "kind": "protect"})),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(upd["kind"], "protect");
    assert_eq!(
        f.call("PUT", "/api/memory/9999", Some(json!({"text": "x"})))
            .await
            .0,
        404
    );

    assert_eq!(
        f.call("DELETE", &format!("/api/memory/{id}"), None).await.0,
        200
    );
    assert_eq!(
        f.call("DELETE", &format!("/api/memory/{id}"), None).await.0,
        404
    );
    let (_, all) = f.call("GET", "/api/memory", None).await;
    assert!(all.as_array().unwrap().is_empty());

    assert!(m["detail"].is_null(), "{m}");
    let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
    hoardini_core::memory::knowledge::remember_filetype(
        &db.conn().unwrap(),
        "q4nx",
        ".q4nx: FastFlowLM NPU model weights.",
        "user",
        hoardini_core::memory::Detail {
            category: "model-weights".into(),
            app: Some("FastFlowLM".into()),
            ..Default::default()
        },
        1,
    )
    .unwrap();
    let (_, all) = f.call("GET", "/api/memory", None).await;
    assert_eq!(all[0]["detail"]["category"], "model-weights", "{all}");
    assert_eq!(all[0]["detail"]["app"], "FastFlowLM");
    let _ = f.h.shutdown.send(());
}

// Traces: PROV-PRESETS
#[tokio::test(flavor = "multi_thread")]
async fn a_run_with_no_provider_creates_none_and_setup_state_round_trips() {
    let f = Fixture::new().await;
    let (_, setup) = f.call("GET", "/api/setup", None).await;
    assert_eq!(setup["done"], false);
    let resp = f
        .authed("POST", "/api/agent/run")
        .json(&json!({"threadId": "t1", "runId": "r1", "messages": [{"id": "u1", "role": "user", "content": "hi"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(text.contains("RUN_ERROR"), "{text}");
    let (_, after) = f.call("GET", "/api/providers", None).await;
    assert!(after.as_array().unwrap().is_empty());
    let (_, status) = f.call("GET", "/api/status", None).await;
    assert!(status["active_provider"].is_null());
    f.call("POST", "/api/setup/done", None).await;
    let (_, setup) = f.call("GET", "/api/setup", None).await;
    assert_eq!(setup["done"], true);
    let _ = f.h.shutdown.send(());
}

// Traces: UI-FIRST-RUN
#[tokio::test(flavor = "multi_thread")]
async fn an_install_that_already_has_a_provider_counts_as_set_up() {
    let f = Fixture::new().await;
    let cfg = json!({"name": "Old", "base_url": "http://127.0.0.1:9/v1"});
    let (st, _) = f.call("POST", "/api/providers", Some(cfg)).await;
    assert_eq!(st, 200);
    let (_, setup) = f.call("GET", "/api/setup", None).await;
    assert_eq!(setup["done"], true);
    let _ = f.h.shutdown.send(());
}

fn fake_indexer(dir: &Path, seconds: u32) -> PathBuf {
    let log = dir.join("spawns.txt");
    let script = dir.join("fake-indexer.cmd");
    let body = format!(
        "@echo off\r\necho %7>>\"{log}\"\r\necho {{\"type\":\"start\",\"volume\":\"C:\\\\\",\"mode\":\"%7\",\"gen\":1,\"kind\":\"full\"}}\r\necho {{\"type\":\"progress\",\"phase\":\"walk\",\"dirs\":1,\"files\":2,\"bytes\":3,\"fraction\":0.5}}\r\nping -n {n} 127.0.0.1 >nul\r\n",
        log = log.display(),
        n = seconds + 1
    );
    std::fs::write(&script, body).unwrap();
    script
}

async fn index_state(f: &Fixture) -> (String, Value) {
    let (_, s) = f.call("GET", "/api/status", None).await;
    (
        s["index"]["state"].as_str().unwrap().to_string(),
        s["index"].clone(),
    )
}

// Traces: SRV-ONE-REFRESH
#[tokio::test(flavor = "multi_thread")]
async fn one_refresh_at_a_time_and_fast_takes_over_lean() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_indexer(dir.path(), 10);
    let fdir = tempfile::tempdir().unwrap();
    seed_index(&fdir.path().join("h.db"), Duration::ZERO);
    let f = Fixture::in_dir(fdir, None, Some(exe), false).await;

    let (st, r) = f.call("POST", "/api/index/refresh", None).await;
    assert_eq!((st, &r["started"]), (200, &json!(true)));
    let (_, r) = f.call("POST", "/api/index/refresh", None).await;
    assert_eq!(r["started"], false, "a second refresh must not start");

    let mut progress = Value::Null;
    for _ in 0..50 {
        let (state, idx) = index_state(&f).await;
        assert_eq!(state, "refreshing");
        if !idx["progress"].is_null() {
            progress = idx["progress"].clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(progress["phase"], "walk");
    assert_eq!(progress["files"], 2);

    let (st, _) = f
        .call("PUT", "/api/settings", Some(json!({"index_speed": "fast"})))
        .await;
    assert_eq!(st, 200);
    let (_, r) = f.call("POST", "/api/index/refresh", None).await;
    assert_eq!(r["started"], false);

    let spawns = dir.path().join("spawns.txt");
    let modes = wait_for(&spawns, |l| l.len() >= 2, 5).await;
    assert_eq!(
        modes,
        ["background", "fast"],
        "fast takes over a lean refresh"
    );
    let (_, idx) = index_state(&f).await;
    assert_eq!(idx["mode"], "fast");

    f.call("PUT", "/api/settings", Some(json!({"index_speed": "lean"})))
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(lines(&spawns).len(), 2, "lean lets a fast refresh finish");
    let (state, idx) = index_state(&f).await;
    assert_eq!(
        (state.as_str(), &idx["mode"]),
        ("refreshing", &json!("fast"))
    );
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-ONE-REFRESH
#[tokio::test(flavor = "multi_thread")]
async fn the_first_full_scan_runs_fast_even_when_lean() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_indexer(dir.path(), 0);
    let f = Fixture::with(None, Some(exe)).await;
    let (_, s) = f.call("GET", "/api/settings", None).await;
    assert_eq!(s["index_speed"], "lean");
    let (_, r) = f.call("POST", "/api/index/refresh", None).await;
    assert_eq!(r["started"], true);
    let modes = wait_for(&dir.path().join("spawns.txt"), |l| !l.is_empty(), 5).await;
    assert_eq!(modes, ["fast"]);
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-STATUS-STREAM
#[tokio::test(flavor = "multi_thread")]
async fn status_stream_sends_an_initial_status_frame() {
    let f = Fixture::new().await;
    let mut resp = f.authed("GET", "/api/status/stream").send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let chunk = tokio::time::timeout(Duration::from_secs(5), resp.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(&chunk).to_string();
    let data = text
        .strip_prefix("data: ")
        .expect("first frame is data")
        .trim();
    let v: Value = serde_json::from_str(data).unwrap();
    assert_eq!(v["type"], "status");
    assert!(v["index"]["state"].is_string());
    assert_eq!(v["running_runs"], 0);
    drop(resp);
    let _ = f.h.shutdown.send(());
}

// An OpenAI-compatible stream that sends one text chunk and then hangs.
async fn stalled_llm() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 65536];
                let _ = s.read(&mut buf).await;
                let chunk =
                    json!({"choices": [{"index": 0, "delta": {"content": "Thinking about"}}]});
                let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n";
                let _ = s
                    .write_all(format!("{head}data: {chunk}\n\n").as_bytes())
                    .await;
                tokio::time::sleep(Duration::from_secs(60)).await;
            });
        }
    });
    format!("http://{addr}/v1")
}

// Traces: SRV-RUN-CANCEL
#[tokio::test(flavor = "multi_thread")]
async fn cancel_closes_the_open_message_and_ends_with_run_finished() {
    let f = Fixture::new().await;
    let base = stalled_llm().await;
    let (_, p) = f
        .call(
            "POST",
            "/api/providers",
            Some(json!({"name": "stall", "base_url": base, "model": "m"})),
        )
        .await;
    let id = p["id"].as_str().unwrap();
    f.call("POST", &format!("/api/providers/{id}/activate"), None)
        .await;
    let mut resp = f
        .authed("POST", "/api/agent/run")
        .json(&json!({"threadId": "t-cancel", "runId": "r-cancel", "messages": [{"id": "u1", "role": "user", "content": "hi"}]}))
        .send()
        .await
        .unwrap();
    let mut text = String::new();
    let mut cancelled = false;
    while let Ok(Some(chunk)) = tokio::time::timeout(Duration::from_secs(10), resp.chunk())
        .await
        .unwrap()
    {
        text.push_str(&String::from_utf8_lossy(&chunk));
        if !cancelled && text.contains("TEXT_MESSAGE_CONTENT") {
            let (_, st) = f.call("GET", "/api/status", None).await;
            assert_eq!(st["running_runs"], 1);
            f.call("POST", "/api/agent/runs/r-cancel/cancel", None)
                .await;
            cancelled = true;
        }
    }
    let types: Vec<String> = text
        .split("\n\n")
        .filter_map(|f| f.strip_prefix("data: "))
        .map(|d| {
            serde_json::from_str::<Value>(d).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert!(cancelled);
    assert_eq!(
        types[types.len() - 2..],
        ["TEXT_MESSAGE_END", "RUN_FINISHED"],
        "{types:?}"
    );
    let (_, st) = f.call("GET", "/api/status", None).await;
    assert_eq!(st["running_runs"], 0);
    let _ = f.h.shutdown.send(());
}

fn journal_fake_indexer(dir: &Path) -> PathBuf {
    let log = dir.join("kinds.txt");
    let script = dir.join("journal-indexer.cmd");
    let body = format!(
        "@echo off\r\necho %7 %9>>\"{log}\"\r\nif \"%9\"==\"journal\" goto needs\r\necho {{\"type\":\"start\",\"volume\":\"C:\\\\\",\"mode\":\"%7\",\"gen\":1,\"kind\":\"full\"}}\r\necho {{\"type\":\"done\",\"kind\":\"full\",\"scan_id\":1,\"duration_ms\":5}}\r\nexit /b 0\r\n:needs\r\necho {{\"type\":\"needs_full\",\"reason\":\"no_position\"}}\r\nexit /b 3\r\n",
        log = log.display(),
    );
    std::fs::write(&script, body).unwrap();
    script
}

// Traces: SRV-JOURNAL-REFRESH
#[tokio::test(flavor = "multi_thread")]
async fn launch_runs_auto_then_quiet_journal_refreshes_fall_back_to_full() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("h.db");
    seed_index(&db_path, Duration::ZERO);
    let db = hoardini_core::store::Db::open(&db_path).unwrap();
    hoardini_core::store::set_meta(&db.conn().unwrap(), "journal_every_s", "1").unwrap();
    let h = start_with(
        ServerOptions {
            port: 0,
            token: Some(TOKEN.into()),
            db_path,
            ui_dir: None,
            dialogs: None,
            refresh_on_start: true,
            indexer_exe: Some(journal_fake_indexer(dir.path())),
        },
        Some(Arc::new(MemStore::default())),
    )
    .await
    .unwrap();
    let http = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/api/status", h.port);
    let _stream = http
        .get(format!("http://127.0.0.1:{}/api/status/stream", h.port))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    let r = http
        .post(format!("http://127.0.0.1:{}/api/presence", h.port))
        .bearer_auth(TOKEN)
        .json(&json!({"looking": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let log = dir.path().join("kinds.txt");
    let mut kinds: Vec<String> = Vec::new();
    let mut loud_after_launch = false;
    for _ in 0..200 {
        kinds = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .collect();
        if kinds.len() >= 3 {
            break;
        }
        if kinds.len() >= 2 {
            let s: Value = http
                .get(&url)
                .bearer_auth(TOKEN)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            loud_after_launch |= s["index"]["state"] == "refreshing";
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        kinds[..3],
        ["background auto", "background journal", "background full"]
    );
    assert!(
        !loud_after_launch,
        "a short timer-driven refresh must not show as refreshing"
    );
    let _ = h.shutdown.send(());
}

// Logs "<mode> <kind>" per spawn and "end <kind>" when a child finishes uncancelled; `auto` runs `auto_s` seconds.
fn presence_fake_indexer(dir: &Path, auto_s: u32) -> PathBuf {
    let log = dir.join("presence.txt");
    let script = dir.join("presence-indexer.cmd");
    let body = format!(
        "@echo off\r\necho %7 %9>>\"{log}\"\r\nif not \"%9\"==\"auto\" goto quick\r\necho {{\"type\":\"start\",\"volume\":\"C:\\\\\",\"mode\":\"%7\",\"gen\":1,\"kind\":\"full\"}}\r\nping -n {n} 127.0.0.1 >nul\r\n:quick\r\necho {{\"type\":\"done\",\"kind\":\"%9\",\"scan_id\":1,\"duration_ms\":5}}\r\necho end %9>>\"{log}\"\r\nexit /b 0\r\n",
        log = log.display(),
        n = auto_s + 1
    );
    std::fs::write(&script, body).unwrap();
    script
}

fn journal_spawns(l: &[String]) -> usize {
    l.iter()
        .filter(|x| x.ends_with(" journal") && !x.starts_with("end "))
        .count()
}

// Traces: SRV-PRESENCE
#[test]
fn the_next_journal_refresh_is_due_when_the_index_reaches_the_interval() {
    let min = |m: u64| Duration::from_secs(m * 60);
    assert_eq!(
        journal_wait(None, min(3)),
        Duration::ZERO,
        "never completed"
    );
    assert_eq!(journal_wait(Some(min(4)), min(3)), Duration::ZERO);
    assert_eq!(journal_wait(Some(min(3)), min(3)), Duration::ZERO);
    assert_eq!(journal_wait(Some(min(1)), min(3)), min(2));
    assert_eq!(journal_wait(Some(Duration::ZERO), min(3)), min(3));
}

// Traces: SRV-PRESENCE
#[tokio::test(flavor = "multi_thread")]
async fn the_journal_timer_runs_only_while_someone_is_looking() {
    let dir = tempfile::tempdir().unwrap();
    let exe = presence_fake_indexer(dir.path(), 3);
    let fdir = tempfile::tempdir().unwrap();
    let db = hoardini_core::store::Db::open(fdir.path().join("h.db")).unwrap();
    hoardini_core::store::set_meta(&db.conn().unwrap(), "journal_every_s", "1").unwrap();
    let f = Fixture::in_dir(fdir, None, Some(exe), true).await;
    let log = dir.path().join("presence.txt");

    wait_for(&log, |l| !l.is_empty(), 5).await;
    let s = f.stream().await;
    f.presence(true).await;
    drop(s);
    f.presence(false).await;
    let (state, _) = index_state(&f).await;
    assert_eq!(
        state, "refreshing",
        "presence never cancels a running refresh"
    );
    let l = wait_for(&log, |l| l.iter().any(|x| x == "end auto"), 10).await;
    assert_eq!(
        l[1..],
        ["end auto"],
        "the launch refresh ran to its end: {l:?}"
    );

    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(journal_spawns(&lines(&log)), 0, "nobody looking: no timer");
    f.presence(true).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        journal_spawns(&lines(&log)),
        0,
        "no open stream counts as not looking"
    );

    let s = f.stream().await;
    let l = wait_for(&log, |l| journal_spawns(l) >= 1, 3).await;
    assert!(
        journal_spawns(&l) >= 1,
        "looking with a stale index refreshes: {l:?}"
    );
    drop(s);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let settled = lines(&log).len();
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(
        lines(&log).len(),
        settled,
        "the stream closing stops the timer"
    );
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-PRESENCE
#[tokio::test(flavor = "multi_thread")]
async fn arriving_with_a_stale_index_starts_one_journal_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let exe = presence_fake_indexer(dir.path(), 0);
    let fdir = tempfile::tempdir().unwrap();
    let db_path = fdir.path().join("h.db");
    seed_index(&db_path, Duration::from_secs(10));
    let db = hoardini_core::store::Db::open(&db_path).unwrap();
    hoardini_core::store::set_meta(&db.conn().unwrap(), "journal_every_s", "60").unwrap();
    let f = Fixture::in_dir(fdir, None, Some(exe), true).await;
    let log = dir.path().join("presence.txt");
    let l = wait_for(&log, |l| l.iter().any(|x| x == "end auto"), 5).await;
    assert_eq!(l[0], "background auto");

    let _s = f.stream().await;
    f.presence(true).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        journal_spawns(&lines(&log)),
        0,
        "an index 10 s old is not due"
    );

    set_index_age(&db.conn().unwrap(), Duration::from_secs(300));
    f.presence(false).await;
    f.presence(true).await;
    let l = wait_for(&log, |l| journal_spawns(l) >= 1, 3).await;
    assert_eq!(journal_spawns(&l), 1, "{l:?}");
    assert!(l.contains(&"background journal".to_string()), "{l:?}");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        journal_spawns(&lines(&log)),
        1,
        "one refresh, then the interval"
    );
    let _ = f.h.shutdown.send(());
}

fn seed(f: &Fixture, id: &str, role: &str, text: &str) {
    use hoardini_agent::{provider::ChatMessage, thread};
    let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
    let conn = db.conn().unwrap();
    thread::ensure_conversation(&conn, thread::MAIN, None).unwrap();
    thread::append(&conn, thread::MAIN, id, &ChatMessage::text(role, text)).unwrap();
}

fn sql_count(f: &Fixture, sql: &str) -> i64 {
    let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
    db.conn().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
}

// Traces: SRV-THREAD
#[tokio::test(flavor = "multi_thread")]
async fn the_conversation_routes_report_the_window_start_fresh_and_clear_only_messages() {
    let f = Fixture::new().await;
    assert!(
        route_list()
            .iter()
            .all(|(_, p)| !p.starts_with("/api/threads")),
        "the per-thread routes are gone"
    );
    let (st, t) = f.call("GET", "/api/thread", None).await;
    assert_eq!(st, 200, "{t}");
    assert_eq!(t["id"], "main");
    assert_eq!(t["messages"], json!([]));
    assert_eq!(t["window_start_id"], Value::Null);
    assert_eq!(t["has_hidden"], false);
    assert_eq!(t["fresh"], true);

    seed(&f, "u1", "user", "hello");
    seed(&f, "a1", "assistant", "hi there");
    let (_, t) = f.call("GET", "/api/thread", None).await;
    assert_eq!(t["messages"].as_array().unwrap().len(), 2);
    assert_eq!(t["window_start_id"], "u1");
    assert_eq!(t["has_hidden"], false);
    assert_eq!(t["fresh"], false);

    let (st, r) = f.call("POST", "/api/thread/fresh", None).await;
    assert_eq!((st, r), (200, json!({"ok": true})));
    let (_, t) = f.call("GET", "/api/thread", None).await;
    assert_eq!(t["messages"], json!([]), "clearing empties the screen");
    assert_eq!(t["earlier"], 2);
    assert_eq!(t["showing_all"], false);
    assert_eq!(t["window_start_id"], Value::Null);
    assert_eq!(t["has_hidden"], false);
    assert_eq!(t["fresh"], true);
    let (_, all) = f.call("GET", "/api/thread?all=1", None).await;
    assert_eq!(
        all["messages"].as_array().unwrap().len(),
        2,
        "earlier messages stay stored"
    );
    assert_eq!(all["showing_all"], true);
    assert_eq!(all["has_hidden"], true);

    seed(&f, "u2", "user", "a new topic");
    let (_, t) = f.call("GET", "/api/thread", None).await;
    assert_eq!(t["messages"].as_array().unwrap().len(), 1);
    assert_eq!(t["window_start_id"], "u2");
    assert_eq!(t["has_hidden"], false);
    assert_eq!(t["fresh"], false);
    let (_, all) = f.call("GET", "/api/thread?all=1", None).await;
    assert_eq!(all["messages"].as_array().unwrap().len(), 3);
    assert_eq!(all["has_hidden"], true);

    let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
    let conn = db.conn().unwrap();
    conn.execute(
        "UPDATE message SET created_ms = created_ms - 7 * 3600 * 1000",
        [],
    )
    .unwrap();
    let (_, t) = f.call("GET", "/api/thread", None).await;
    assert_eq!(t["fresh"], true, "the last message is 7 h old");

    conn.execute(
        "INSERT INTO proposal(id, conversation_id, kind, params_json, status, created_ms)
         VALUES ('p-keep', 'main', 'delete', '{}', 'pending', 0)",
        [],
    )
    .unwrap();
    f.call(
        "POST",
        "/api/memory",
        Some(json!({"scope": "global", "key": "k", "kind": "fact", "text": "stays"})),
    )
    .await;
    let memories = sql_count(&f, "SELECT COUNT(*) FROM memory");
    let (st, r) = f.call("DELETE", "/api/thread", None).await;
    assert_eq!((st, r), (200, json!({"ok": true})));
    let (_, t) = f.call("GET", "/api/thread", None).await;
    assert_eq!(t["messages"], json!([]));
    assert_eq!(t["has_hidden"], false);
    assert_eq!(t["fresh"], true);
    assert_eq!(
        t["earlier"], 0,
        "deleting the history also resets the cleared mark"
    );
    assert_eq!(sql_count(&f, "SELECT COUNT(*) FROM message"), 0);
    assert_eq!(
        sql_count(&f, "SELECT COUNT(*) FROM proposal WHERE id = 'p-keep'"),
        1
    );
    assert_eq!(sql_count(&f, "SELECT COUNT(*) FROM memory"), memories);

    seed(&f, "u3", "user", "after clearing");
    let (_, t) = f.call("GET", "/api/thread", None).await;
    assert_eq!(t["window_start_id"], "u3", "the window start was reset");
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-THREAD
#[tokio::test(flavor = "multi_thread")]
async fn a_run_always_uses_main_and_clearing_is_refused_while_it_runs() {
    let f = Fixture::new().await;
    let base = stalled_llm().await;
    let (_, p) = f
        .call(
            "POST",
            "/api/providers",
            Some(json!({"name": "stall", "base_url": base, "model": "m"})),
        )
        .await;
    let id = p["id"].as_str().unwrap();
    f.call("POST", &format!("/api/providers/{id}/activate"), None)
        .await;
    let mut resp = f
        .authed("POST", "/api/agent/run")
        .json(&json!({"threadId": "t-other", "runId": "r-main", "messages": [{"id": "u-main", "role": "user", "content": "hi"}]}))
        .send()
        .await
        .unwrap();
    let mut text = String::new();
    while !text.contains("TEXT_MESSAGE_CONTENT") {
        let chunk = tokio::time::timeout(Duration::from_secs(10), resp.chunk())
            .await
            .unwrap()
            .unwrap()
            .expect("stream open");
        text.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(text.contains(r#""threadId":"main""#), "{text}");
    let (st, _) = f.call("DELETE", "/api/thread", None).await;
    assert_eq!(st, 409);
    let (_, t) = f.call("GET", "/api/thread", None).await;
    assert_eq!(t["messages"][0]["id"], "u-main");
    f.call("POST", "/api/agent/runs/r-main/cancel", None).await;
    drop(resp);
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-HISTORY-RANGE
#[tokio::test(flavor = "multi_thread")]
async fn history_filters_by_range() {
    use hoardini_core::history::{ScanRecord, record_scan_rows};
    use hoardini_core::index::{FakeFs, RefreshOptions, refresh};
    use hoardini_core::store::Db;

    let f = Fixture::new().await;
    let (_, none) = f.call("GET", "/api/history", None).await;
    assert_eq!(none, json!([]));

    let db = Db::open(f.dir.path().join("h.db")).unwrap();
    let conn = db.conn().unwrap();
    let fs = FakeFs::new(r"C:\");
    fs.file(r"C:\seed.bin", 1 << 20);
    let vol = refresh(&conn, &fs, fs.root(), &RefreshOptions::default())
        .unwrap()
        .volume_id;
    for (ms, free) in [(1_000, 50u64), (2_000, 40), (3_000, 30), (4_000, 20)] {
        let rec = ScanRecord {
            volume_id: vol,
            kind: "full".into(),
            mode: "background".into(),
            started_ms: ms - 1,
            finished_ms: ms,
            total_bytes: Some(100),
            free_bytes: Some(free),
            ..ScanRecord::default()
        };
        record_scan_rows(&conn, &rec, &[], &[]).unwrap();
    }
    let finished = |v: &Value| -> Vec<i64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|p| p["finished_ms"].as_i64().unwrap())
            .collect()
    };
    let (_, all) = f.call("GET", "/api/history", None).await;
    let all = finished(&all);
    assert_eq!(all.len(), 5);
    assert!(all.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(&all[..4], &[1_000, 2_000, 3_000, 4_000]);

    let (_, mid) = f
        .call("GET", "/api/history?since_ms=2000&until_ms=3000", None)
        .await;
    assert_eq!(finished(&mid), vec![2_000, 3_000]);
    let (_, since) = f.call("GET", "/api/history?since_ms=3000", None).await;
    assert_eq!(&finished(&since)[..2], &[3_000, 4_000]);
    assert_eq!(finished(&since).len(), 3);
    let (_, until) = f.call("GET", "/api/history?until_ms=1000", None).await;
    assert_eq!(finished(&until).last(), Some(&1_000));
    let (_, mid) = f
        .call("GET", "/api/history?since_ms=2000&until_ms=3000", None)
        .await;
    assert_eq!(mid[0]["free_bytes"], 40);
    let _ = f.h.shutdown.send(());
}

// Logs "<volume> <mode> <kind>" per spawn and "end <volume>" when a child runs `seconds` to its end.
fn volume_fake_indexer(dir: &Path, seconds: u32) -> PathBuf {
    let log = dir.join("volumes.txt");
    let script = dir.join("volume-indexer.cmd");
    let body = format!(
        "@echo off\r\necho %5 %7 %9>>\"{log}\"\r\nping -n {n} 127.0.0.1 >nul\r\necho end %5>>\"{log}\"\r\nexit /b 0\r\n",
        log = log.display(),
        n = seconds + 1
    );
    std::fs::write(&script, body).unwrap();
    script
}

// cmd sees a root that ends in a backslash quoted, with the backslash doubled.
fn spawned(log: &Path) -> Vec<String> {
    lines(log)
        .iter()
        .map(|l| l.replace('"', "").replace(r"\\", r"\"))
        .collect()
}

// A scratch folder stands in for a drive; its index is a fake one under the same root.
fn fake_drive(db_path: &Path, folder: &Path) -> String {
    let root = hoardini_core::index::normalize_path(&folder.to_string_lossy());
    let db = hoardini_core::store::Db::open(db_path).unwrap();
    let fs = hoardini_core::index::FakeFs::new(&root);
    fs.set_volume(500 << 30, 120 << 30);
    fs.file(&format!(r"{root}\video\big.mkv"), 2 << 30);
    let out = hoardini_core::index::refresh(
        &db.conn().unwrap(),
        &fs,
        &root,
        &hoardini_core::index::RefreshOptions::default(),
    )
    .unwrap();
    assert!(out.completed);
    root
}

fn rows_of(db_path: &Path, root: &str) -> (i64, i64, i64) {
    let db = hoardini_core::store::Db::open(db_path).unwrap();
    let conn = db.conn().unwrap();
    let one = |sql: &str| -> i64 { conn.query_row(sql, [root], |r| r.get(0)).unwrap() };
    (
        one("SELECT COUNT(*) FROM volume WHERE root = ?1"),
        one("SELECT COUNT(*) FROM node WHERE volume_id IN (SELECT id FROM volume WHERE root = ?1)"),
        one("SELECT COUNT(*) FROM scan WHERE volume_id IN (SELECT id FROM volume WHERE root = ?1)"),
    )
}

fn by_root<'a>(list: &'a Value, root: &str) -> Option<&'a Value> {
    list.as_array().unwrap().iter().find(|v| {
        v["root"]
            .as_str()
            .is_some_and(|r| r.eq_ignore_ascii_case(root))
    })
}

async fn wait_idle(f: &Fixture, secs: u64) {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    while index_state(f).await.0 != "idle" && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// Traces: INDEX-VOLUMES
#[tokio::test(flavor = "multi_thread")]
async fn adding_a_drive_selects_it_and_starts_its_first_scan_fast() {
    let dir = tempfile::tempdir().unwrap();
    let exe = volume_fake_indexer(dir.path(), 0);
    let drive = tempfile::tempdir().unwrap();
    let a = hoardini_core::index::normalize_path(&drive.path().to_string_lossy());
    let sys = hoardini_core::index::system_root();
    let fdir = tempfile::tempdir().unwrap();
    seed_index(&fdir.path().join("h.db"), Duration::ZERO);
    let f = Fixture::in_dir(fdir, None, Some(exe), false).await;

    let (st, list) = f.call("GET", "/api/volumes", None).await;
    assert_eq!(st, 200);
    for fixed in hoardini_win::fixed_volumes().unwrap() {
        let root = hoardini_core::index::normalize_path(&fixed.to_string_lossy());
        if hoardini_win::is_subst(&fixed) {
            assert!(by_root(&list, &root).is_none(), "a subst drive is a folder");
            continue;
        }
        let v = by_root(&list, &root).unwrap_or_else(|| panic!("{root} missing: {list}"));
        assert_eq!(v["connected"], true);
        assert!(v["total_bytes"].as_u64().unwrap() > 0);
        assert_eq!(v["watched"], root == sys);
        assert_eq!(v["selected"], root == sys);
        assert_eq!(v["has_journal"], v["fs"] == "NTFS" || v["fs"] == "ReFS");
    }

    let body = json!({"watched": [sys, a.replace('\\', "/")], "selected": a});
    let (st, list) = f.call("PUT", "/api/volumes", Some(body)).await;
    assert_eq!(st, 200, "{list}");
    let added = by_root(&list, &a).unwrap();
    assert_eq!(
        (&added["watched"], &added["selected"]),
        (&json!(true), &json!(true))
    );
    assert_eq!(by_root(&list, &sys).unwrap()["selected"], false);

    let log = dir.path().join("volumes.txt");
    wait_for(&log, |l| l.iter().any(|x| x.starts_with("end ")), 5).await;
    wait_idle(&f, 5).await;
    assert_eq!(
        spawned(&log),
        [format!("{a} fast auto"), format!("end {a}")],
        "only the added drive is scanned, fast"
    );
    let (_, status) = f.call("GET", "/api/status", None).await;
    let vols = &status["volumes"];
    assert_eq!(vols.as_array().unwrap().len(), 2);
    assert_eq!(by_root(vols, &a).unwrap()["selected"], true);
    assert_eq!(by_root(vols, &sys).unwrap()["selected"], false);

    let missing = drive.path().join("not-here").to_string_lossy().into_owned();
    for bad in [
        json!({"watched": [], "selected": sys}),
        json!({"watched": [a], "selected": sys}),
        json!({"watched": [sys, a, missing], "selected": a}),
        json!({"watched": ["relative"], "selected": "relative"}),
    ] {
        let (st, _) = f.call("PUT", "/api/volumes", Some(bad.clone())).await;
        assert_eq!(st, 400, "{bad}");
    }
    let (_, list) = f.call("GET", "/api/volumes", None).await;
    assert_eq!(
        by_root(&list, &a).unwrap()["selected"],
        true,
        "a refused PUT stores nothing"
    );
    assert_eq!(by_root(&list, &sys).unwrap()["watched"], true);
    let _ = f.h.shutdown.send(());
}

// Traces: INDEX-VOLUMES
#[tokio::test(flavor = "multi_thread")]
async fn removing_a_drive_stops_its_refresh_and_deletes_its_index() {
    let dir = tempfile::tempdir().unwrap();
    let exe = volume_fake_indexer(dir.path(), 10);
    let drive = tempfile::tempdir().unwrap();
    let sys = hoardini_core::index::system_root();
    let fdir = tempfile::tempdir().unwrap();
    let db_path = fdir.path().join("h.db");
    seed_index(&db_path, Duration::ZERO);
    let a = fake_drive(&db_path, drive.path());
    let db = hoardini_core::store::Db::open(&db_path).unwrap();
    hoardini_core::store::set_meta(
        &db.conn().unwrap(),
        "indexed_volumes",
        &serde_json::to_string(&[&a, &sys]).unwrap(),
    )
    .unwrap();
    assert!(rows_of(&db_path, &a).1 > 0);
    let c_before = rows_of(&db_path, r"C:\");
    let f = Fixture::in_dir(fdir, None, Some(exe), false).await;

    let (_, r) = f.call("POST", "/api/index/refresh", None).await;
    assert_eq!(r["started"], true);
    let log = dir.path().join("volumes.txt");
    wait_for(&log, |l| !l.is_empty(), 5).await;
    let l = spawned(&log);
    assert_eq!(l, [format!("{a} background auto")]);

    let t0 = std::time::Instant::now();
    let body = json!({"watched": [sys], "selected": sys});
    let (st, list) = f.call("PUT", "/api/volumes", Some(body)).await;
    assert_eq!(st, 200, "{list}");
    assert!(
        t0.elapsed() < Duration::from_secs(8),
        "the running child was stopped"
    );
    assert!(
        by_root(&list, &a).is_none(),
        "an unwatched folder is not listed"
    );
    assert_eq!(rows_of(&db_path, &a), (0, 0, 0));
    assert_eq!(rows_of(&db_path, r"C:\"), c_before);

    wait_for(&log, |l| l.len() >= 2, 5).await;
    let l = spawned(&log);
    assert_eq!(
        l[1],
        format!("{sys} background auto"),
        "the pass moves on: {l:?}"
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !spawned(&log).contains(&format!("end {a}")),
        "the removed drive's child never finished"
    );
    let ok: String = db
        .conn()
        .unwrap()
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");

    let none = json!({"watched": [], "selected": sys});
    let (st, _) = f.call("PUT", "/api/volumes", Some(none)).await;
    assert_eq!(st, 400, "the last watched drive stays");
    let _ = f.h.shutdown.send(());
}

// Traces: INDEX-VOLUMES
#[tokio::test(flavor = "multi_thread")]
async fn an_unplugged_drive_is_skipped_and_keeps_its_index() {
    let dir = tempfile::tempdir().unwrap();
    let exe = volume_fake_indexer(dir.path(), 0);
    let drive = tempfile::tempdir().unwrap();
    let sys = hoardini_core::index::system_root();
    let fdir = tempfile::tempdir().unwrap();
    let db_path = fdir.path().join("h.db");
    seed_index(&db_path, Duration::ZERO);
    let a = fake_drive(&db_path, drive.path());
    let db = hoardini_core::store::Db::open(&db_path).unwrap();
    let conn = db.conn().unwrap();
    hoardini_core::store::set_meta(
        &conn,
        "indexed_volumes",
        &serde_json::to_string(&[&sys, &a]).unwrap(),
    )
    .unwrap();
    hoardini_core::store::set_meta(&conn, "selected_volume", &a).unwrap();
    drop(drive);
    let before = rows_of(&db_path, &a);
    let f = Fixture::in_dir(fdir, None, Some(exe), false).await;

    let (_, r) = f.call("POST", "/api/index/refresh", None).await;
    assert_eq!(r["started"], true);
    let log = dir.path().join("volumes.txt");
    wait_for(&log, |l| l.iter().any(|x| x.starts_with("end ")), 5).await;
    wait_idle(&f, 5).await;
    assert_eq!(
        spawned(&log),
        [format!("{sys} background auto"), format!("end {sys}")]
    );
    assert_eq!(rows_of(&db_path, &a), before, "its index is kept");

    let (_, list) = f.call("GET", "/api/volumes", None).await;
    let v = by_root(&list, &a).unwrap();
    assert_eq!(
        (&v["watched"], &v["selected"], &v["connected"]),
        (&json!(true), &json!(true), &json!(false))
    );
    assert_eq!(v["total_bytes"], 500u64 << 30, "last known size");
    let (_, status) = f.call("GET", "/api/status", None).await;
    let s = by_root(&status["volumes"], &a).unwrap();
    assert_eq!(
        (&s["selected"], &s["connected"]),
        (&json!(true), &json!(false))
    );
    let _ = f.h.shutdown.send(());
}

async fn next_frame(resp: &mut reqwest::Response, buf: &mut String) -> Value {
    loop {
        if let Some(end) = buf.find("\n\n") {
            let frame: String = buf.drain(..end + 2).collect();
            if let Some(data) = frame.trim().strip_prefix("data: ") {
                return serde_json::from_str(data).unwrap();
            }
            continue;
        }
        let chunk = tokio::time::timeout(Duration::from_secs(30), resp.chunk())
            .await
            .expect("a frame within 30 s")
            .unwrap()
            .expect("the stream stays open");
        buf.push_str(&String::from_utf8_lossy(&chunk));
    }
}

// Traces: ACT-FOLLOW-UP, POLICY-DEFAULT-TICK, POLICY-ITEM-MEMORY
#[tokio::test(flavor = "multi_thread")]
async fn a_finished_job_links_its_follow_up_before_the_stream_reports_it() {
    use hoardini_core::proposals::{self, NewItem, NewProposal, ProposalKind};
    let f = Fixture::new().await;
    let work = f.dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let go = work.join("go.bin");
    let stay = work.join("stay.bin");
    std::fs::write(&go, vec![1u8; 8192]).unwrap();
    std::fs::write(&stay, vec![2u8; 8192]).unwrap();
    let work_s = work.to_string_lossy().into_owned();
    let (st, _) = f
        .call(
            "POST",
            "/api/memory",
            Some(json!({"scope": "path", "key": work_s, "kind": "fact", "text": "scratch is disposable"})),
        )
        .await;
    assert_eq!(st, 200);

    let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
    let item = |p: &Path| NewItem {
        path: p.to_string_lossy().into_owned(),
        verdict: None,
        reason: Some("test".into()),
    };
    let p = proposals::create(
        &db,
        NewProposal {
            conversation_id: Some("main".into()),
            tool_call_id: None,
            kind: ProposalKind::Delete,
            destination_id: None,
            format: None,
            dedupe: None,
            summary: "two files".into(),
            items: vec![item(&go), item(&stay)],
        },
    )
    .unwrap();

    let (_, v) = f
        .call("GET", &format!("/api/proposals/{}", p.id), None)
        .await;
    assert_eq!(v["parent_id"], Value::Null);
    assert_eq!(v["follow_up_id"], Value::Null);
    for it in v["items"].as_array().unwrap() {
        assert_eq!(it["small"], false, "{it}");
        assert_eq!(it["selected"], false, "no verdict starts unticked: {it}");
        assert_eq!(it["memory_note"], "scratch is disposable", "{it}");
    }

    let mut stream = f.stream().await;
    let mut buf = String::new();
    let go_id = p
        .items
        .iter()
        .find(|i| i.path.ends_with("go.bin"))
        .unwrap()
        .id;
    let (st, r) = f
        .call(
            "POST",
            &format!("/api/proposals/{}/approve", p.id),
            Some(json!({"item_ids": [go_id]})),
        )
        .await;
    assert_eq!(st, 200, "{r}");
    let job_id = r["job_id"].as_str().unwrap().to_string();
    loop {
        let fr = next_frame(&mut stream, &mut buf).await;
        if fr["type"] == "job"
            && fr["id"] == job_id.as_str()
            && matches!(fr["status"].as_str(), Some("done" | "failed" | "cancelled"))
        {
            break;
        }
    }
    let (_, parent) = f
        .call("GET", &format!("/api/proposals/{}", p.id), None)
        .await;
    let fid = parent["follow_up_id"]
        .as_str()
        .expect("the follow-up exists when the job's end is streamed")
        .to_string();
    let (_, child) = f.call("GET", &format!("/api/proposals/{fid}"), None).await;
    assert_eq!(child["parent_id"], p.id.as_str());
    assert_eq!(child["follow_up_id"], Value::Null);
    let items = child["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["path"].as_str().unwrap().ends_with("stay.bin"));
    assert_eq!(items[0]["selected"], false);
    assert_eq!(items[0]["memory_note"], "scratch is disposable");

    let (_, t) = f.call("GET", "/api/thread", None).await;
    assert_eq!(t["proposals"][&p.id]["follow_up_id"], fid.as_str());
    assert_eq!(t["proposals"][&fid]["parent_id"], p.id.as_str());
    drop(stream);
    let _ = f.h.shutdown.send(());
}

// Traces: PROV-ZERO-DEFAULTS
#[tokio::test]
async fn blank_timeout_steps_and_context_mean_the_defaults() {
    let f = Fixture::new().await;
    let cfg = json!({"name": "LM Studio", "base_url": "http://127.0.0.1:1234/v1",
                     "context_window": 0, "timeout_s": 0, "max_tool_steps": 0});
    let (st, created) = f.call("POST", "/api/providers", Some(cfg)).await;
    assert_eq!(st, 200, "{created}");
    assert_eq!(created["timeout_s"], 120);
    assert_eq!(created["max_tool_steps"], 20);
    assert_eq!(created["context_window"], 32768);

    let conn = rusqlite::Connection::open(f.dir.path().join("h.db")).unwrap();
    conn.execute(
        "UPDATE provider SET config_json = json_set(config_json, '$.timeout_s', 0, '$.max_tool_steps', 0, '$.context_window', 0)",
        [],
    )
    .unwrap();
    let (_, list) = f.call("GET", "/api/providers", None).await;
    assert_eq!(
        list[0]["timeout_s"], 120,
        "a row saved with zeros loads with the defaults"
    );
    assert_eq!(list[0]["max_tool_steps"], 20);
    assert_eq!(list[0]["context_window"], 32768);
}

// Traces: SRV-INDEX-CHILDREN
#[tokio::test(flavor = "multi_thread")]
async fn index_children_lists_a_folder_biggest_first_with_its_small_files_and_the_rest() {
    use hoardini_core::index::{FakeFs, RefreshOptions, refresh};
    use hoardini_core::store::Db;

    const MIB: u64 = 1 << 20;
    let f = Fixture::new().await;
    let db = Db::open(f.dir.path().join("h.db")).unwrap();
    let fs = FakeFs::new(r"C:\");
    fs.file_alloc(r"C:\data\models\a.gguf", 300 * MIB, 300 * MIB);
    fs.file_alloc(r"C:\data\models\b.gguf", 100 * MIB, 100 * MIB);
    fs.file_alloc(r"C:\data\big.iso", 200 * MIB, 200 * MIB);
    fs.file_alloc(r"C:\data\cache\c.bin", 50 * MIB, 50 * MIB);
    fs.dir(r"C:\data\empty");
    fs.file_alloc(r"C:\data\note.txt", 4096, 1000);
    fs.file_alloc(r"C:\data\todo.txt", 8192, 5000);
    let out = refresh(
        &db.conn().unwrap(),
        &fs,
        fs.root(),
        &RefreshOptions::default(),
    )
    .unwrap();
    assert!(out.completed);

    let (st, v) = f
        .call("GET", "/api/index/children?path=C%3A%5Cdata&limit=2", None)
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["path"], r"C:\data");
    assert_eq!(v["size"], 650 * MIB + 12288);
    assert_eq!(v["files"], 6);
    let kids = v["children"].as_array().unwrap();
    assert_eq!(kids.len(), 2);
    let models = &kids[0];
    assert_eq!(models["path"], r"C:\data\models");
    assert_eq!(models["name"], "models");
    assert_eq!(models["kind"], "dir");
    assert_eq!(models["size"], 400 * MIB);
    assert_eq!(models["files"], 2);
    assert_eq!(models["denied"], false);
    assert_eq!(models["has_children"], true);
    assert!(models.get("mtime_ms").is_some());
    assert_eq!(kids[1]["path"], r"C:\data\big.iso");
    assert_eq!(kids[1]["kind"], "file");
    assert_eq!(kids[1]["files"], 1);
    assert_eq!(kids[1]["has_children"], false);
    assert_eq!(v["small"], json!({"count": 2, "bytes": 12288}));
    assert_eq!(v["more"], json!({"count": 2, "bytes": 50 * MIB}));

    let (_, all) = f
        .call("GET", "/api/index/children?path=c%3A%5CDATA", None)
        .await;
    let names: Vec<&str> = all["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["models", "big.iso", "cache", "empty"]);
    assert_eq!(all["children"][3]["has_children"], false);
    assert_eq!(all["more"], json!({"count": 0, "bytes": 0}));

    let (st, _) = f
        .call(
            "GET",
            "/api/index/children?path=C%3A%5Cdata%5Cbig.iso",
            None,
        )
        .await;
    assert_eq!(st, 400, "a file has no children");
    let (st, _) = f
        .call("GET", "/api/index/children?path=C%3A%5Cnope", None)
        .await;
    assert_eq!(st, 404);
    let (st, _) = f.call("GET", "/api/index/children", None).await;
    assert_eq!(st, 400);
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-REVEAL
#[tokio::test(flavor = "multi_thread")]
async fn reveal_refuses_paths_off_the_watched_drives_and_paths_that_do_not_exist() {
    let f = Fixture::new().await;
    let watched = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("x.txt"), b"x").unwrap();
    let root = hoardini_core::index::normalize_path(&watched.path().to_string_lossy());
    let other = hoardini_core::index::normalize_path(&outside.path().to_string_lossy());
    {
        let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
        hoardini_core::store::set_meta(
            &db.conn().unwrap(),
            "indexed_volumes",
            &serde_json::to_string(&[&root]).unwrap(),
        )
        .unwrap();
    }
    let other_name = other.rsplit('\\').next().unwrap();
    let cases = [
        (json!({}), 400),
        (json!({"path": ""}), 400),
        (json!({"path": r"relative\x.txt"}), 400),
        (json!({"path": format!(r"{other}\x.txt")}), 403),
        (
            json!({"path": format!(r"{root}\..\{other_name}\x.txt")}),
            400,
        ),
        (json!({"path": format!(r"{root}\missing.txt")}), 404),
    ];
    for (body, want) in cases {
        let (st, v) = f.call("POST", "/api/reveal", Some(body.clone())).await;
        assert_eq!(st, want, "{body}: {v}");
        assert!(v["error"].is_string(), "{v}");
    }
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-LEARNED, MEM-LEARN-JOB
#[tokio::test(flavor = "multi_thread")]
async fn a_finished_job_lists_what_it_learned_and_the_chip_routes_undo_and_confirm() {
    use hoardini_core::memory::knowledge::{Detail, remember_filetype};
    use hoardini_core::proposals::{self, NewItem, NewProposal, ProposalKind};
    let f = Fixture::new().await;
    let work = f.dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let go = work.join("go.bin");
    let stay = work.join("stay.bin");
    std::fs::write(&go, vec![1u8; 8192]).unwrap();
    std::fs::write(&stay, vec![2u8; 8192]).unwrap();
    let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
    let item = |p: &Path| NewItem {
        path: p.to_string_lossy().into_owned(),
        verdict: None,
        reason: None,
    };
    let p = proposals::create(
        &db,
        NewProposal {
            conversation_id: Some("main".into()),
            tool_call_id: None,
            kind: ProposalKind::Delete,
            destination_id: None,
            format: None,
            dedupe: None,
            summary: "two files".into(),
            items: vec![item(&go), item(&stay)],
        },
    )
    .unwrap();
    let (_, pending) = f
        .call("GET", &format!("/api/proposals/{}", p.id), None)
        .await;
    assert_eq!(pending["learned"], json!([]));

    let mut stream = f.stream().await;
    let mut buf = String::new();
    let go_id = p
        .items
        .iter()
        .find(|i| i.path.ends_with("go.bin"))
        .unwrap()
        .id;
    let (st, r) = f
        .call(
            "POST",
            &format!("/api/proposals/{}/approve", p.id),
            Some(json!({"item_ids": [go_id]})),
        )
        .await;
    assert_eq!(st, 200, "{r}");
    let job_id = r["job_id"].as_str().unwrap().to_string();
    loop {
        let fr = next_frame(&mut stream, &mut buf).await;
        if fr["type"] == "job"
            && fr["id"] == job_id.as_str()
            && matches!(fr["status"].as_str(), Some("done" | "failed" | "cancelled"))
        {
            break;
        }
    }
    let (_, done) = f
        .call("GET", &format!("/api/proposals/{}", p.id), None)
        .await;
    let learned = done["learned"]
        .as_array()
        .expect("learned is listed by the job's end");
    assert_eq!(learned.len(), 1, "{done}");
    let text = learned[0]["text"].as_str().unwrap();
    assert!(text.starts_with(r"go.bin in ~\"), "{text}");
    assert!(
        text.contains(" — cleaned 8.0 KB on ")
            && text.ends_with("; 1 item you left unticked is kept"),
        "{text}"
    );
    let memory_id = learned[0]["memory_id"].as_i64().unwrap();
    let (_, t) = f.call("GET", "/api/thread", None).await;
    assert_eq!(t["proposals"][&p.id]["learned"], done["learned"]);

    let undo = format!("/api/proposals/{}/learned/{memory_id}", p.id);
    let (st, v) = f.call("DELETE", &undo, None).await;
    assert_eq!((st, v), (200, json!({"ok": true})));
    let (_, after) = f
        .call("GET", &format!("/api/proposals/{}", p.id), None)
        .await;
    assert_eq!(after["learned"], json!([]));
    assert_eq!(f.call("DELETE", &undo, None).await.0, 404);

    let (ty, _) = remember_filetype(
        &db.conn().unwrap(),
        "q4nx",
        ".q4nx: NPU model weights (FastFlowLM).",
        "web:https://example.test/flm",
        Detail {
            category: "model-weights".into(),
            app: Some("FastFlowLM".into()),
            ..Detail::default()
        },
        1,
    )
    .unwrap();
    let (st, m) = f
        .call("POST", &format!("/api/memory/{}/confirm", ty.id), None)
        .await;
    assert_eq!(st, 200, "{m}");
    assert_eq!(
        (m["id"].as_i64(), &m["source"], &m["pinned"]),
        (Some(ty.id), &json!("user"), &json!(true))
    );
    assert_eq!(m["text"], ".q4nx: NPU model weights (FastFlowLM).");
    assert_eq!(m["detail"]["app"], "FastFlowLM");
    let (st, note) = f
        .call(
            "POST",
            "/api/memory",
            Some(json!({"scope": "global", "key": "n", "kind": "fact", "text": "a note"})),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(
        f.call("POST", &format!("/api/memory/{}/confirm", note["id"]), None)
            .await
            .0,
        400
    );
    assert_eq!(
        f.call("POST", "/api/memory/999999/confirm", None).await.0,
        404
    );
    drop(stream);
    let _ = f.h.shutdown.send(());
}

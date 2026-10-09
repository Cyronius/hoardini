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
        let r = self.authed("GET", "/api/status/stream").send().await.unwrap();
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
    assert_eq!(again["index_speed"], "fast", "a rejected PUT changes nothing");
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
    let _ = f.h.shutdown.send(());
}

#[tokio::test(flavor = "multi_thread")]
async fn first_agent_run_seeds_an_active_lm_studio_profile() {
    let f = Fixture::new().await;
    let (_, before) = f.call("GET", "/api/providers", None).await;
    assert!(before.as_array().unwrap().is_empty());
    let resp = f
        .authed("POST", "/api/agent/run")
        .json(&json!({"threadId": "t1", "runId": "r1", "messages": [{"id": "u1", "role": "user", "content": "hi"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    // Seeding happens before streaming; dropping the stream cancels the run so no real model is called.
    drop(resp);
    let (_, after) = f.call("GET", "/api/providers", None).await;
    assert_eq!(after[0]["name"], "LM Studio");
    assert_eq!(after[0]["is_active"], true);
    let (_, status) = f.call("GET", "/api/status", None).await;
    assert_eq!(status["active_provider"]["name"], "LM Studio");
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
    assert_eq!(modes, ["background", "fast"], "fast takes over a lean refresh");
    let (_, idx) = index_state(&f).await;
    assert_eq!(idx["mode"], "fast");

    f.call("PUT", "/api/settings", Some(json!({"index_speed": "lean"})))
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(lines(&spawns).len(), 2, "lean lets a fast refresh finish");
    let (state, idx) = index_state(&f).await;
    assert_eq!((state.as_str(), &idx["mode"]), ("refreshing", &json!("fast")));
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
    assert_eq!(journal_wait(None, min(3)), Duration::ZERO, "never completed");
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

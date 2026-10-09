use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hoardini_core::vault::SecretStore;
use hoardini_server::{ServerHandle, ServerOptions, route_list, start_with};
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
        let dir = tempfile::tempdir().unwrap();
        let h = start_with(
            ServerOptions {
                port: 0,
                token: Some(TOKEN.into()),
                db_path: dir.path().join("h.db"),
                ui_dir,
                dialogs: None,
                refresh_on_start: false,
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
    assert_eq!(s["threshold_bytes"], 1_048_576);
    assert_eq!(s["redact_paths"], true);
    assert_eq!(s["web_search_enabled"], false);
    assert_eq!(s["indexer_workers"], 2);
    assert_eq!(s["indexer_memory_cap_mb"], 256);
    assert_eq!(s["builtin_idle_minutes"], 10);
    assert!(s["index_size_bytes"].as_u64().unwrap() > 0);

    let (st, s) = f
        .call(
            "PUT",
            "/api/settings",
            Some(json!({"threshold_bytes": 2_097_152, "redact_paths": false, "indexer_workers": 4, "index_size_bytes": 1})),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(s["threshold_bytes"], 2_097_152);
    assert_eq!(s["redact_paths"], false);
    assert_eq!(s["indexer_workers"], 4);
    assert_ne!(s["index_size_bytes"], 1);

    let (_, again) = f.call("GET", "/api/settings", None).await;
    assert_eq!(again["threshold_bytes"], 2_097_152);
    let (st, e) = f
        .call("PUT", "/api/settings", Some(json!({"redact_paths": "yes"})))
        .await;
    assert_eq!(st, 400);
    assert!(e["error"].is_string());
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
async fn one_refresh_at_a_time_and_fast_preempts_background() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_indexer(dir.path(), 10);
    let f = Fixture::with(None, Some(exe)).await;

    let (st, r) = f
        .call(
            "POST",
            "/api/index/refresh",
            Some(json!({"mode": "background"})),
        )
        .await;
    assert_eq!((st, &r["started"]), (200, &json!(true)));
    let (_, r) = f
        .call(
            "POST",
            "/api/index/refresh",
            Some(json!({"mode": "background"})),
        )
        .await;
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

    let (_, r) = f
        .call("POST", "/api/index/refresh", Some(json!({"mode": "fast"})))
        .await;
    assert_eq!(
        r["started"], true,
        "fast must take over a background refresh"
    );
    let (_, r) = f
        .call("POST", "/api/index/refresh", Some(json!({"mode": "fast"})))
        .await;
    assert_eq!(r["started"], false);

    let spawns = dir.path().join("spawns.txt");
    let mut modes = Vec::new();
    for _ in 0..50 {
        modes = std::fs::read_to_string(&spawns)
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .collect();
        if modes.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(modes, ["background", "fast"]);
    let (_, idx) = index_state(&f).await;
    assert_eq!(idx["mode"], "fast");
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

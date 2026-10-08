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
    let _ = resp.text().await;
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

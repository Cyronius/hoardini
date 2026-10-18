use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hoardini_core::syscleanup::SystemFacts;
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
                system_facts: Some(SystemFacts::default()),
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

// Traces: SRV-SETTINGS, EVID-COPY-CONSENT
#[tokio::test(flavor = "multi_thread")]
async fn settings_round_trip() {
    let f = Fixture::new().await;
    let (_, s) = f.call("GET", "/api/settings", None).await;
    let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        ["builtin_idle_minutes", "index_size_bytes", "online_checks"]
    );
    assert_eq!(s["builtin_idle_minutes"], 10);
    assert!(s["index_size_bytes"].as_u64().unwrap() > 0);
    assert_eq!(
        s["online_checks"],
        json!({"asked": true, "services": {"huggingface": true, "winget": true}})
    );

    let (st, s) = f
        .call(
            "PUT",
            "/api/settings",
            Some(json!({"index_speed": "fast", "builtin_idle_minutes": 0, "index_size_bytes": 1, "threshold_bytes": 5})),
        )
        .await;
    assert_eq!(st, 200);
    assert!(s.get("index_speed").is_none(), "the speed switch is gone");
    assert_eq!(s["builtin_idle_minutes"], 0);
    assert_ne!(s["index_size_bytes"], 1);
    assert!(s.get("threshold_bytes").is_none());

    let (st, s) = f
        .call(
            "PUT",
            "/api/settings",
            Some(json!({"online_checks": {"asked": true, "services": {"winget": false}}})),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(
        s["online_checks"],
        json!({"asked": true, "services": {"huggingface": true, "winget": false}})
    );
    {
        use hoardini_core::copycheck::consent::{self, Service};
        let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
        let c = consent::load(&db.conn().unwrap());
        assert!(c.asked, "the copy checks read what was saved");
        assert!(c.allows(Service::Huggingface) && !c.allows(Service::Winget));
    }

    let (_, again) = f.call("GET", "/api/settings", None).await;
    assert_eq!(again["online_checks"]["services"]["huggingface"], true);
    for bad in [
        json!({"builtin_idle_minutes": "x"}),
        json!({"online_checks": 3}),
        json!({"online_checks": {"services": {"huggingface": "yes"}}}),
        json!({"online_checks": {"services": {"bing": true}}}),
        json!({"builtin_idle_minutes": 5, "online_checks": {"asked": "no"}}),
    ] {
        let (st, e) = f.call("PUT", "/api/settings", Some(bad.clone())).await;
        assert_eq!(st, 400, "{bad}");
        assert!(e["error"].is_string());
    }
    let (_, again) = f.call("GET", "/api/settings", None).await;
    assert_eq!(
        again["builtin_idle_minutes"], 0,
        "a rejected PUT changes nothing"
    );
    assert_eq!(
        again["online_checks"],
        json!({"asked": true, "services": {"huggingface": true, "winget": false}})
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
        .json(&json!({"threadId": "item:t1", "runId": "r1", "messages": [{"id": "u1", "role": "user", "content": "hi"}]}))
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
async fn one_refresh_at_a_time_and_rescan_runs_fast() {
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
    let (_, idx) = index_state(&f).await;
    assert_eq!(idx["mode"], "fast");

    // The old switch is ignored.
    f.call("PUT", "/api/settings", Some(json!({"index_speed": "lean"})))
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(lines(&dir.path().join("spawns.txt")), ["fast"]);
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

// An OpenAI-compatible stream that sends a text chunk every 200 ms for a minute.
async fn trickling_llm() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 65536];
                let _ = s.read(&mut buf).await;
                let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n";
                if s.write_all(head.as_bytes()).await.is_err() {
                    return;
                }
                let chunk = json!({"choices": [{"index": 0, "delta": {"content": "more "}}]});
                for _ in 0..300 {
                    if s.write_all(format!("data: {chunk}\n\n").as_bytes())
                        .await
                        .is_err()
                    {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            });
        }
    });
    format!("http://{addr}/v1")
}

// Traces: SRV-RUN-TEARDOWN, SRV-THREAD
#[tokio::test(flavor = "multi_thread")]
async fn closing_the_run_stream_stops_the_run() {
    let f = Fixture::new().await;
    let base = trickling_llm().await;
    let (_, p) = f
        .call(
            "POST",
            "/api/providers",
            Some(json!({"name": "trickle", "base_url": base, "model": "m"})),
        )
        .await;
    let id = p["id"].as_str().unwrap();
    f.call("POST", &format!("/api/providers/{id}/activate"), None)
        .await;
    let mut resp = f
        .authed("POST", "/api/agent/run")
        .json(&json!({"threadId": "item:teardown", "runId": "r-drop", "messages": [{"id": "u1", "role": "user", "content": "hi"}]}))
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
    assert!(text.contains(r#""threadId":"item:teardown""#), "{text}");
    let (_, st) = f.call("GET", "/api/status", None).await;
    assert_eq!(st["running_runs"], 1);
    drop(resp);
    let mut runs = 1;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (_, st) = f.call("GET", "/api/status", None).await;
        runs = st["running_runs"].as_u64().unwrap();
        if runs == 0 {
            break;
        }
    }
    assert_eq!(runs, 0, "the run ends once its stream is closed");
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
            system_facts: Some(SystemFacts::default()),
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
    assert_eq!(kinds[..3], ["fast auto", "background journal", "fast full"]);
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
    assert_eq!(l[0], "fast auto");

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

fn seed_item(f: &Fixture, conv: &str, id: &str, role: &str, text: &str) {
    use hoardini_agent::{provider::ChatMessage, thread};
    let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
    let conn = db.conn().unwrap();
    thread::ensure_conversation(&conn, conv, None).unwrap();
    thread::append(&conn, conv, id, &ChatMessage::text(role, text)).unwrap();
}

// Traces: SRV-THREAD
#[tokio::test(flavor = "multi_thread")]
async fn only_item_threads_load_and_run() {
    let f = Fixture::new().await;
    let routes = route_list();
    assert!(routes.iter().all(|(_, p)| !p.starts_with("/api/threads")));
    for gone in [("POST", "/api/thread/fresh"), ("DELETE", "/api/thread")] {
        assert!(!routes.contains(&gone), "{gone:?}");
    }
    let profile = hoardini_core::paths::profile_dir()
        .to_string_lossy()
        .into_owned();
    seed_item(
        &f,
        "item:abc",
        "u1",
        "user",
        &format!(r"About {profile}\big (big): what is it?"),
    );
    seed_item(&f, "item:abc", "a1", "assistant", "A cache.");
    let (st, t) = f.call("GET", "/api/thread?id=item:abc", None).await;
    assert_eq!(st, 200, "{t}");
    assert_eq!(t["id"], "item:abc");
    let msgs = t["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["content"], r"About ~\big (big): what is it?");
    for q in [
        "/api/thread",
        "/api/thread?id=main",
        "/api/thread?id=elsewhere",
    ] {
        let (st, _) = f.call("GET", q, None).await;
        assert_eq!(st, 400, "{q}");
    }
    let (st, r) = f
        .call(
            "POST",
            "/api/agent/run",
            Some(json!({"threadId": "t-other", "runId": "r1", "messages": [{"id": "u9", "role": "user", "content": "hi"}]})),
        )
        .await;
    assert_eq!(st, 400, "{r}");
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
    assert_eq!(l, [format!("{a} fast auto")]);

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
    assert_eq!(l[1], format!("{sys} fast auto"), "the pass moves on: {l:?}");
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
        [format!("{sys} fast auto"), format!("end {sys}")]
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
            conversation_id: Some("home".into()),
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
    drop(stream);
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-LOCATION-FORGET, SRV-THREAD
#[tokio::test(flavor = "multi_thread")]
async fn bring_back_drops_the_users_answer_and_item_threads_load_by_id() {
    let f = Fixture::new().await;
    let work = f.dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let path = work.to_string_lossy().into_owned();
    let users = |v: &Value| {
        v.as_array()
            .unwrap()
            .iter()
            .filter(|m| m["source"] == "user" && m["scope"] == "path")
            .count()
    };

    let (st, r) = f
        .call(
            "POST",
            "/api/locations/answer",
            Some(json!({"path": path, "answer": "never-touch"})),
        )
        .await;
    assert_eq!(st, 200, "{r}");
    let (_, mems) = f.call("GET", "/api/memory", None).await;
    assert_eq!(users(&mems), 2, "a verdict row and a protect note: {mems}");
    let (st, r) = f
        .call("POST", "/api/locations/forget", Some(json!({"path": path})))
        .await;
    assert_eq!((st, r["forgot"].as_u64()), (200, Some(2)), "{r}");
    let (_, mems) = f.call("GET", "/api/memory", None).await;
    assert_eq!(users(&mems), 0, "{mems}");
    let (st, r) = f
        .call("POST", "/api/locations/forget", Some(json!({"path": path})))
        .await;
    assert_eq!((st, r["forgot"].as_u64()), (200, Some(0)), "{r}");
    let (st, _) = f
        .call("POST", "/api/locations/forget", Some(json!({"path": " "})))
        .await;
    assert_eq!(st, 400);

    let (st, t) = f.call("GET", "/api/thread?id=item:abc", None).await;
    assert_eq!(st, 200, "{t}");
    assert_eq!(
        (t["id"].as_str(), t["messages"].as_array().map(Vec::len)),
        (Some("item:abc"), Some(0))
    );
    let (st, _) = f.call("GET", "/api/thread?id=elsewhere", None).await;
    assert_eq!(st, 400);
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-ACT
#[tokio::test(flavor = "multi_thread")]
async fn an_action_on_home_runs_at_once_and_reports_what_policy_held_back() {
    let f = Fixture::new().await;
    let work = f.dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let go = work.join("go.bin");
    let kept = work.join("kept.bin");
    std::fs::write(&go, vec![1u8; 8192]).unwrap();
    std::fs::write(&kept, vec![2u8; 8192]).unwrap();
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let (st, _) = f
        .call(
            "POST",
            "/api/memory",
            Some(json!({"scope": "path", "key": s(&kept), "kind": "protect", "text": "mine"})),
        )
        .await;
    assert_eq!(st, 200);

    let mut stream = f.stream().await;
    let mut buf = String::new();
    let (st, r) = f
        .call(
            "POST",
            "/api/act",
            Some(json!({"kind": "delete", "paths": [s(&go), s(&kept)]})),
        )
        .await;
    assert_eq!(st, 200, "{r}");
    let job_id = r["job_id"]
        .as_str()
        .expect("the allowed path runs")
        .to_string();
    let blocked = r["blocked"].as_array().unwrap();
    assert_eq!(blocked.len(), 1, "{r}");
    assert!(
        blocked[0]["path"].as_str().unwrap().ends_with("kept.bin"),
        "{r}"
    );
    assert!(!blocked[0]["reason"].as_str().unwrap().is_empty(), "{r}");
    loop {
        let fr = next_frame(&mut stream, &mut buf).await;
        if fr["type"] == "job" && fr["id"] == job_id.as_str() && fr["status"] == "done" {
            break;
        }
    }
    assert!(!go.exists(), "approved without a second step");
    assert!(kept.exists());

    let (st, r) = f
        .call(
            "POST",
            "/api/act",
            Some(json!({"kind": "delete", "paths": [s(&kept)]})),
        )
        .await;
    assert_eq!(st, 200, "{r}");
    assert_eq!(r["job_id"], Value::Null, "{r}");
    assert_eq!(r["blocked"].as_array().unwrap().len(), 1, "{r}");
    if let Some(id) = r["proposal_id"].as_str() {
        let (_, p) = f.call("GET", &format!("/api/proposals/{id}"), None).await;
        assert_eq!(p["status"], "declined", "nothing is left pending: {p}");
    }

    for (body, why) in [
        (json!({"kind": "dedupe", "paths": [s(&kept)]}), "kind"),
        (
            json!({"kind": "archive", "paths": [s(&kept)]}),
            "destination",
        ),
        (json!({"kind": "delete", "paths": ["  "]}), "paths"),
    ] {
        let (st, r) = f.call("POST", "/api/act", Some(body)).await;
        assert_eq!(st, 400, "{why}: {r}");
    }
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

struct FakeApps {
    inputs: hoardini_core::apps::AppInputs,
    launched: Mutex<Vec<hoardini_core::apps::Uninstaller>>,
}

impl hoardini_core::apps::AppSource for FakeApps {
    fn inputs(&self) -> hoardini_core::apps::AppInputs {
        self.inputs.clone()
    }
    fn launch(&self, how: &hoardini_core::apps::Uninstaller) -> anyhow::Result<()> {
        self.launched.lock().unwrap().push(how.clone());
        Ok(())
    }
}

// Traces: SRV-APPS
#[tokio::test(flavor = "multi_thread")]
async fn the_review_lists_rarely_used_apps_and_uninstall_runs_only_the_registered_command() {
    const GIB: u64 = 1 << 30;
    const DAY: i64 = 86_400_000;
    let f = Fixture::new().await;
    let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
    let select = |root: &str| {
        hoardini_core::store::set_meta(&db.conn().unwrap(), "selected_volume", root).unwrap()
    };
    {
        let conn = db.conn().unwrap();
        let drives = [
            (
                r"D:\",
                vec![r"D:\Apps\Big\big.dat", r"D:\Apps\Busy\busy.dat"],
            ),
            (r"C:\", vec![r"C:\data\x.dat"]),
        ];
        for (root, files) in drives {
            let fs = hoardini_core::index::FakeFs::new(root);
            for p in files {
                fs.file(p, 3 * GIB);
            }
            let out = hoardini_core::index::refresh(
                &conn,
                &fs,
                root,
                &hoardini_core::index::RefreshOptions::default(),
            )
            .unwrap();
            assert!(out.completed);
        }
        hoardini_core::store::set_meta(&conn, "indexed_volumes", r#"["C:\\","D:\\"]"#).unwrap();
    }
    select(r"D:\");
    let ran = hoardini_core::store::now_ms() - 200 * DAY;
    let program = |name: &str, loc: &str, cmd: &str| hoardini_win::InstalledProgram {
        name: name.into(),
        publisher: Some("Contoso".into()),
        install_location: Some(loc.into()),
        uninstall_key: format!(r"HKLM\Uninstall\{name}"),
        uninstall_string: Some(cmd.into()),
        ..Default::default()
    };
    let fake = Arc::new(FakeApps {
        inputs: hoardini_core::apps::AppInputs {
            programs: vec![
                program(
                    "Big Tool",
                    r"D:\Apps\Big",
                    r#""D:\Apps\Big\unins000.exe" /S"#,
                ),
                program("Busy", r"D:\Apps\Busy", r"D:\Apps\Busy\u.exe"),
            ],
            assist: vec![
                hoardini_win::UserAssistEntry {
                    name: r"D:\Apps\Big\big.exe".into(),
                    run_count: 1,
                    last_run_ms: Some(ran),
                },
                hoardini_win::UserAssistEntry {
                    name: r"D:\Apps\Busy\busy.exe".into(),
                    run_count: 9,
                    last_run_ms: Some(hoardini_core::store::now_ms() - DAY),
                },
            ],
            steam: Vec::new(),
        },
        launched: Mutex::new(Vec::new()),
    });
    hoardini_core::apps::set_source(Some(fake.clone()));

    let (st, review) = f.call("POST", "/api/review/run", None).await;
    assert_eq!(st, 200, "{review}");
    let list = &review["apps"];
    let id = list[0]["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("reg-"), "{id}");
    assert_eq!(
        list,
        &json!([{
            "id": id,
            "name": "Big Tool",
            "publisher": "Contoso",
            "source": "registry",
            "path": r"D:\Apps\Big",
            "size": 3 * GIB,
            "last_run_ms": ran,
            "last_run_from": r"last run of d:\apps\big\big.exe",
        }])
    );
    select(r"C:\");
    let (st, other) = f.call("POST", "/api/review/run", None).await;
    assert_eq!(st, 200, "{other}");
    assert_eq!(other["apps"], json!([]), "only apps on the review's drive");

    let (st, r) = f
        .call(
            "POST",
            &format!("/api/apps/{id}/uninstall"),
            Some(json!({"command": "calc.exe", "file": r"C:\Windows\System32\calc.exe"})),
        )
        .await;
    assert_eq!(st, 200, "{r}");
    assert_eq!(r, json!({"started": true, "id": id, "name": "Big Tool"}));
    assert_eq!(
        *fake.launched.lock().unwrap(),
        vec![hoardini_core::apps::Uninstaller::Command {
            file: r"D:\Apps\Big\unins000.exe".into(),
            params: Some("/S".into()),
        }]
    );

    let (st, _) = f
        .call("POST", "/api/apps/reg-0000000000000000/uninstall", None)
        .await;
    assert_eq!(st, 404);
    assert_eq!(fake.launched.lock().unwrap().len(), 1);
    hoardini_core::apps::set_source(None);
    let _ = f.h.shutdown.send(());
}

fn seed_drive(db_path: &Path) {
    let db = hoardini_core::store::Db::open(db_path).unwrap();
    let fs = hoardini_core::index::FakeFs::new(r"C:\");
    fs.set_volume(2000 << 30, 800 << 30);
    fs.file(r"C:\code\covenant-agent\models\m.dat", 100 << 30);
    fs.file(r"C:\code\covenant-agent\data\x.dat", 61 << 30);
    let out = hoardini_core::index::refresh(
        &db.conn().unwrap(),
        &fs,
        r"C:\",
        &hoardini_core::index::RefreshOptions::default(),
    )
    .unwrap();
    assert!(out.completed);
}

// The build open starts runs in the background; the status says when it is done.
async fn settled_review(f: &Fixture) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let (_, status) = f.call("GET", "/api/status", None).await;
        if status["review_building"] == json!(false) {
            let (st, v) = f.call("GET", "/api/review", None).await;
            assert_eq!(st, 200, "{v}");
            return v;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the review build did not end: {status}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// Traces: REVIEW-TRIGGER
#[tokio::test(flavor = "multi_thread")]
async fn the_review_is_built_on_open_when_stale() {
    let f = Fixture::new().await;
    let (st, _) = f.call("POST", "/api/review/open", None).await;
    assert_eq!(st, 404, "no index yet");
    let dbp = f.dir.path().join("h.db");
    seed_drive(&dbp);

    let (st, first) = f.call("POST", "/api/review/open", None).await;
    assert_eq!(st, 200, "{first}");
    assert_eq!(first["review"], Value::Null, "nothing built yet: {first}");
    assert_eq!(first["building"], true);
    assert_eq!(first["header"]["root"], r"C:\");
    assert_eq!(first["cards"], json!([]));
    let a = settled_review(&f).await;
    assert_eq!(a["building"], false);
    let id = a["review"]["id"].as_str().unwrap().to_string();
    assert_eq!(a["review"]["volume"], r"C:\");
    assert!(a["review"]["created_ms"].as_i64().unwrap() > 0);
    assert_eq!(
        a["header"],
        json!({"root": r"C:\", "used_bytes": 1200u64 << 30, "free_bytes": 800u64 << 30, "ready_bytes": 0})
    );
    assert_eq!(a["cards"], json!([]));
    assert_eq!(a["proposals"], json!({}));
    let area = &a["areas"][0];
    assert_eq!(area["path"], r"C:\code\covenant-agent");
    assert_eq!(area["size"], 161u64 << 30);
    assert_eq!(
        (
            &area["unknown"],
            &area["label"],
            &area["category"],
            &area["verdict"]
        ),
        (
            &json!(true),
            &json!("unknown"),
            &json!("unknown"),
            &Value::Null
        )
    );
    assert!(
        area["why_not"]
            .as_str()
            .unwrap()
            .starts_with("Not identified")
    );
    assert_eq!(area["hint"], Value::Null);
    assert_eq!(area["actions"], json!([]));

    let (_, again) = f.call("POST", "/api/review/open", None).await;
    assert_eq!(again["review"]["id"], id.as_str());
    assert_eq!(again["building"], false, "fresh, so nothing is rebuilt");
    let (_, got) = f.call("GET", "/api/review", None).await;
    assert_eq!(got["review"]["id"], id.as_str());
    let (_, status) = f.call("GET", "/api/status", None).await;
    assert_eq!(status["review"]["id"], id.as_str());
    assert_eq!(status["review_building"], false);
    assert_eq!(a["apps"], json!([]));

    seed_drive(&dbp);
    let (_, stale) = f.call("POST", "/api/review/open", None).await;
    assert_eq!(
        stale["review"]["id"],
        id.as_str(),
        "the last review shows while the new one builds"
    );
    assert_eq!(stale["building"], true);
    let b = settled_review(&f).await;
    let bid = b["review"]["id"].as_str().unwrap().to_string();
    assert_ne!(bid, id, "a refresh finished since");
    hoardini_core::store::Db::open(&dbp)
        .unwrap()
        .conn()
        .unwrap()
        .execute(
            "UPDATE review SET created_ms = created_ms - 7 * 3600000 WHERE id = ?1",
            [&bid],
        )
        .unwrap();
    f.call("POST", "/api/review/open", None).await;
    let c = settled_review(&f).await;
    assert_ne!(c["review"]["id"], bid.as_str(), "older than 6 hours");
    let (_, d) = f.call("POST", "/api/review/run", None).await;
    assert_ne!(d["review"]["id"], c["review"]["id"]);
    assert_eq!(d["review"]["volume"], r"C:\");
    let _ = f.h.shutdown.send(());
}

// Traces: REVIEW-SUPERSEDE
#[tokio::test(flavor = "multi_thread")]
async fn the_launch_refresh_rebuilds_an_untouched_review() {
    use hoardini_core::review;
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_indexer(dir.path(), 0);
    let fdir = tempfile::tempdir().unwrap();
    let dbp = fdir.path().join("h.db");
    seed_drive(&dbp);
    let first = {
        let db = hoardini_core::store::Db::open(&dbp).unwrap();
        let vid: i64 = db
            .conn()
            .unwrap()
            .query_row("SELECT id FROM volume", [], |r| r.get(0))
            .unwrap();
        review::build(
            &db,
            &hoardini_core::policy::PolicyEnv::system(),
            &hoardini_core::index::WinLister,
            vid,
            hoardini_core::store::now_ms(),
        )
        .unwrap()
    };
    seed_drive(&dbp);
    let f = Fixture::in_dir(fdir, None, Some(exe), true).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let (_, got) = f.call("GET", "/api/review", None).await;
        if got["review"]["id"]
            .as_str()
            .is_some_and(|id| id != first.id)
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "not rebuilt after the launch refresh: {got}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-LOCATION-ANSWER, MEM-LOCATION
#[tokio::test(flavor = "multi_thread")]
async fn a_what_is_this_answer_is_a_user_row_and_shows_on_the_review() {
    let f = Fixture::new().await;
    seed_drive(&f.dir.path().join("h.db"));
    let (st, _) = f.call("POST", "/api/review/open", None).await;
    assert_eq!(st, 200);
    settled_review(&f).await;

    let (st, m) = f
        .call(
            "POST",
            "/api/locations/answer",
            Some(json!({"path": r"C:\code\covenant-agent", "answer": "keep"})),
        )
        .await;
    assert_eq!(st, 200, "{m}");
    assert_eq!(m["source"], "user");
    assert_eq!(m["pinned"], true);
    assert_eq!(m["key"], r"C:\code\covenant-agent");
    assert_eq!(
        (
            &m["detail"]["label"],
            &m["detail"]["verdict"],
            &m["detail"]["actions"]
        ),
        (
            &json!("yours"),
            &json!("keep"),
            &json!(["archive", "compress"])
        )
    );
    let (_, r) = f.call("GET", "/api/review", None).await;
    let area = &r["areas"][0];
    assert_eq!(area["path"], r"C:\code\covenant-agent");
    assert_eq!(
        (&area["label"], &area["unknown"], &area["verdict"]),
        (&json!("yours"), &json!(false), &json!("keep"))
    );
    assert_eq!(
        area["why_not"],
        "You keep this folder; it may still be moved or compressed."
    );

    let (st, _) = f
        .call(
            "POST",
            "/api/locations/answer",
            Some(json!({"path": r"C:\code\x", "answer": "maybe"})),
        )
        .await;
    assert_eq!(st, 400);
    let (st, _) = f
        .call(
            "POST",
            "/api/locations/answer",
            Some(json!({"answer": "keep"})),
        )
        .await;
    assert_eq!(st, 400);
    let (st, _) = f
        .call(
            "POST",
            "/api/locations/answer",
            Some(json!({"path": r"C:\code\covenant-agent\data", "answer": "never-touch"})),
        )
        .await;
    assert_eq!(st, 200);
    let (_, rows) = f.call("GET", "/api/memory", None).await;
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "protect"
                && r["key"] == r"C:\code\covenant-agent\data"
                && r["source"] == "user")
    );

    for (path, answer) in [
        (r"code\covenant-agent", "disposable"),
        (r"C:\code\..\Windows", "disposable"),
        (r"C:\", "keep"),
        (r"C:\", "disposable"),
        ("~", "disposable"),
        (r"C:\Users", "keep"),
        (r"~\AppData\Local", "disposable"),
        (r"~\.lmstudio", "disposable"),
        (r"C:\Windows\Temp", "disposable"),
        (r"C:\Program Files\Contoso", "never-touch"),
    ] {
        let (st, e) = f
            .call(
                "POST",
                "/api/locations/answer",
                Some(json!({"path": path, "answer": answer})),
            )
            .await;
        assert_eq!(st, 400, "{path} {answer}: {e}");
    }
    let (_, after) = f.call("GET", "/api/memory", None).await;
    assert_eq!(
        after.as_array().unwrap().len(),
        rows.as_array().unwrap().len(),
        "a refused answer writes nothing"
    );
    let _ = f.h.shutdown.send(());
}

// Traces: SRV-PROPOSAL-SELECTION, REVIEW-SUPERSEDE
#[tokio::test(flavor = "multi_thread")]
async fn the_server_keeps_a_cards_ticks_and_any_change_or_decision_touches_it() {
    use hoardini_core::proposals::{self, NewItem, NewProposal, ProposalKind};
    let f = Fixture::new().await;
    let work = f.dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let a = work.join("a.bin");
    let b = work.join("b.bin");
    let c = work.join("c.bin");
    for p in [&a, &b, &c] {
        std::fs::write(p, vec![1u8; 8192]).unwrap();
    }
    let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
    let make = |paths: &[&Path]| {
        proposals::create(
            &db,
            NewProposal {
                conversation_id: Some("home".into()),
                tool_call_id: None,
                kind: ProposalKind::Delete,
                destination_id: None,
                format: None,
                dedupe: None,
                summary: "files".into(),
                items: paths
                    .iter()
                    .map(|p| NewItem {
                        path: p.to_string_lossy().into_owned(),
                        verdict: None,
                        reason: None,
                    })
                    .collect(),
            },
        )
        .unwrap()
    };
    let p = make(&[&a, &b]);
    assert!(!p.touched);
    let ida = p
        .items
        .iter()
        .find(|i| i.path.ends_with("a.bin"))
        .unwrap()
        .id;
    let url = format!("/api/proposals/{}/selection", p.id);
    let (st, v) = f.call("PUT", &url, Some(json!({"item_ids": [ida]}))).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["touched"], true);
    let ticks: Vec<(bool, bool)> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["path"].as_str().unwrap().ends_with("a.bin"),
                i["selected"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(ticks, [(true, true), (false, false)]);
    let (_, again) = f
        .call("GET", &format!("/api/proposals/{}", p.id), None)
        .await;
    assert_eq!(again["items"], v["items"]);

    let (st, _) = f
        .call("PUT", &url, Some(json!({"item_ids": [999_999]})))
        .await;
    assert_eq!(st, 409);
    let idb = p.items.iter().find(|i| i.id != ida).unwrap().id;
    db.conn()
        .unwrap()
        .execute(
            "UPDATE proposal_item SET status = 'blocked', block_reason = 'Not found' WHERE id = ?1",
            [idb],
        )
        .unwrap();
    let (st, v) = f
        .call("PUT", &url, Some(json!({"item_ids": [ida, idb]})))
        .await;
    assert_eq!(st, 409, "a blocked item, whatever its reason says: {v}");
    let (st, _) = f
        .call(
            "PUT",
            "/api/proposals/no-such/selection",
            Some(json!({"item_ids": []})),
        )
        .await;
    assert_eq!(st, 404);
    proposals::decline(&db, &p.id).unwrap();
    let (st, _) = f.call("PUT", &url, Some(json!({"item_ids": []}))).await;
    assert_eq!(st, 409, "only a pending card takes ticks");

    let q = make(&[&c]);
    let idc = q.items[0].id;
    let (st, r) = f
        .call(
            "POST",
            &format!("/api/proposals/{}/approve", q.id),
            Some(json!({"item_ids": [idc]})),
        )
        .await;
    assert_eq!(st, 200, "{r}");
    let (_, got) = f
        .call("GET", &format!("/api/proposals/{}", q.id), None)
        .await;
    assert_eq!(got["touched"], true);
    let _ = f.h.shutdown.send(());
}

const MIB: u64 = 1 << 20;

fn explore_index(db_path: &Path) -> hoardini_core::store::Db {
    use hoardini_core::index::{FakeFs, RefreshOptions, refresh};
    let db = hoardini_core::store::Db::open(db_path).unwrap();
    let fs = FakeFs::new(r"C:\");
    fs.file_alloc(r"C:\data\models\a.gguf", 300 * MIB, 300 * MIB);
    fs.file_alloc(r"C:\data\models\b.gguf", 100 * MIB, 100 * MIB);
    fs.file_alloc(r"C:\data\big.iso", 200 * MIB, 200 * MIB);
    fs.file_alloc(r"C:\data\proj\node_modules\dep.bin", 60 * MIB, 60 * MIB);
    fs.file_alloc(r"C:\data\cache\c.bin", 20 * MIB, 20 * MIB);
    fs.file_alloc(r"C:\data\tiny\t1.bin", 2 * MIB, 2 * MIB);
    fs.file_alloc(r"C:\data\tiny\t2.jpg", 2 * MIB, 2 * MIB);
    fs.file_alloc(r"C:\data\note.txt", 4096, 1000);
    fs.file_alloc(r"C:\data\todo.txt", 8192, 5000);
    fs.file_alloc(r"C:\Windows\x.dll", 50 * MIB, 50 * MIB);
    let out = refresh(
        &db.conn().unwrap(),
        &fs,
        fs.root(),
        &RefreshOptions::default(),
    )
    .unwrap();
    assert!(out.completed);
    db
}

// Traces: SRV-INDEX-TYPES
#[tokio::test(flavor = "multi_thread")]
async fn index_types_lists_extensions_with_what_each_one_is() {
    use hoardini_core::memory::knowledge::{self, Detail};
    let f = Fixture::new().await;
    let db = explore_index(&f.dir.path().join("h.db"));
    knowledge::remember_filetype(
        &db.conn().unwrap(),
        "gguf",
        "GGUF: llama.cpp model weights.",
        "user",
        Detail {
            category: "model-weights".into(),
            app: Some("llama.cpp".into()),
            use_: Some("model weights".into()),
            ..Detail::default()
        },
        1,
    )
    .unwrap();
    let (st, v) = f
        .call("GET", "/api/index/types?path=C%3A%5Cdata", None)
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(
        v,
        json!([
            {"ext": "gguf", "bytes": 400 * MIB, "files": 2, "label": "GGUF: llama.cpp model weights.",
             "app": "llama.cpp", "use": "model weights", "category": "model-weights"},
            {"ext": "iso", "bytes": 200 * MIB, "files": 1},
            {"ext": "bin", "bytes": 82 * MIB, "files": 3},
            {"ext": "jpg", "bytes": 2 * MIB, "files": 1, "category": "photos"},
            {"ext": null, "bytes": 12288, "files": 2, "small": true},
        ])
    );
    let (_, drive) = f.call("GET", "/api/index/types?path=C%3A%5C", None).await;
    let summary: Vec<(Value, Value)> = drive
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["ext"].clone(), t["bytes"].clone()))
        .collect();
    assert_eq!(
        summary,
        [
            (json!("gguf"), json!(400 * MIB)),
            (json!("iso"), json!(200 * MIB)),
            (json!("bin"), json!(82 * MIB)),
            (json!("dll"), json!(50 * MIB)),
            (json!("jpg"), json!(2 * MIB)),
            (Value::Null, json!(12288)),
        ]
    );
    let (st, _) = f
        .call("GET", "/api/index/types?path=C%3A%5Cdata%5Cbig.iso", None)
        .await;
    assert_eq!(st, 400);
    let (st, _) = f
        .call("GET", "/api/index/types?path=C%3A%5Cnope", None)
        .await;
    assert_eq!(st, 404);
    let _ = f.h.shutdown.send(());
}

// Traces: AGENT-REDACT, EVID-COPY-CHECK
#[tokio::test(flavor = "multi_thread")]
async fn a_cards_copy_note_and_dropped_paths_show_the_profile_as_tilde() {
    use hoardini_core::copycheck::{self, Confidence, CopyResult, Subject};
    use hoardini_core::proposals::{self, NewItem, NewProposal, ProposalKind};
    let f = Fixture::new().await;
    let work = f.dir.path().join("work");
    std::fs::create_dir_all(work.join("sub")).unwrap();
    let a = work.join("a.bin");
    let copy = f.dir.path().join(r"archive\a.bin");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    for p in [&a, &copy, &work.join(r"sub\c.bin")] {
        std::fs::write(p, vec![1u8; 8192]).unwrap();
    }
    let s = |p: &Path| hoardini_core::index::normalize_path(&p.to_string_lossy());
    let profile = hoardini_core::paths::profile_dir()
        .to_string_lossy()
        .into_owned();
    assert!(
        s(&a).to_lowercase().starts_with(&profile.to_lowercase()),
        "the test needs its files under the profile"
    );
    let db = hoardini_core::store::Db::open(f.dir.path().join("h.db")).unwrap();
    {
        let conn = db.conn().unwrap();
        let sub = Subject::load(&conn, &s(&a)).unwrap();
        let r = CopyResult::new(
            "archive",
            Some(s(&copy)),
            Confidence::Identical,
            json!({ "note": format!("archived copy at {}", s(&copy)) }),
        );
        copycheck::store(&conn, &sub, &r).unwrap();
    }
    let p = proposals::create(
        &db,
        NewProposal {
            conversation_id: Some("home".into()),
            tool_call_id: None,
            kind: ProposalKind::Delete,
            destination_id: None,
            format: None,
            dedupe: None,
            summary: "files".into(),
            items: [a.clone(), work.join("sub"), work.join(r"sub\c.bin")]
                .iter()
                .map(|p| NewItem {
                    path: s(p),
                    verdict: None,
                    reason: None,
                })
                .collect(),
        },
    )
    .unwrap();
    assert_eq!(p.dropped.len(), 1, "{:?}", p.dropped);

    let (st, v) = f
        .call("GET", &format!("/api/proposals/{}", p.id), None)
        .await;
    assert_eq!(st, 200, "{v}");
    let item = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["path"].as_str().is_some_and(|p| p.ends_with("a.bin")))
        .unwrap();
    assert_eq!(
        item["copy_note"].as_str(),
        Some(
            format!(
                "archived copy at {}",
                hoardini_core::paths::redact(&s(&copy))
            )
            .as_str()
        )
    );
    assert!(item["copy_note"].as_str().unwrap().contains(r"at ~\"));
    let dropped = &v["dropped"][0];
    assert!(
        dropped["path"].as_str().unwrap().starts_with(r"~\"),
        "{dropped}"
    );
    assert!(
        dropped["reason"].as_str().unwrap().contains(r"~\"),
        "{dropped}"
    );
    let escaped = profile.to_lowercase().replace('\\', "\\\\");
    assert!(!v.to_string().to_lowercase().contains(&escaped), "{v}");
    let _ = f.h.shutdown.send(());
}


// Traces: SRV-SHOW-IN-EXPLORER
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

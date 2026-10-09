#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hoardini_agent::agui::{AgMessage, RunAgentInput};
use hoardini_agent::provider::{Client, ProviderConfig};
use hoardini_agent::run::{AgentEnv, IndexControl, run_agent};
use hoardini_core::index::{FakeFs, RefreshOptions, refresh};
use hoardini_core::store::Db;
use hoardini_core::vault::SecretStore;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub args: String,
}

pub fn call(id: &str, name: &str, args: Value) -> Call {
    Call {
        id: id.into(),
        name: name.into(),
        args: args.to_string(),
    }
}

pub fn raw_call(id: &str, name: &str, args: &str) -> Call {
    Call {
        id: id.into(),
        name: name.into(),
        args: args.into(),
    }
}

#[derive(Clone, Debug)]
pub enum Turn {
    Text(String),
    Calls(Option<String>, Vec<Call>),
    Reasoning(String, String),
    Http(u16, String),
    Error200(String),
}

#[derive(Clone, Debug)]
pub struct Captured {
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Captured {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("request body is JSON")
    }

    pub fn messages(&self) -> Vec<Value> {
        self.json()["messages"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.json()["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t["function"]["name"].as_str().map(str::to_string))
            .collect()
    }
}

pub struct MockLlm {
    pub base_url: String,
    captured: Arc<Mutex<Vec<Captured>>>,
}

fn chunk(delta: Value, finish: Option<&str>) -> String {
    let c = json!({"id": "cmpl-mock", "object": "chat.completion.chunk", "created": 0, "model": "mock",
                   "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]});
    format!("data: {c}\n\n")
}

fn text_chunks(text: &str) -> String {
    let mid = text
        .char_indices()
        .nth(text.chars().count() / 2)
        .map_or(0, |(i, _)| i);
    let (a, b) = text.split_at(mid);
    [a, b]
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| chunk(json!({"content": s}), None))
        .collect()
}

fn sse(turn: &Turn) -> (u16, &'static str, String) {
    let done = "data: [DONE]\n\n";
    match turn {
        Turn::Text(t) => (
            200,
            "text/event-stream",
            format!(
                "{}{}{}{done}",
                chunk(json!({"role": "assistant"}), None),
                text_chunks(t),
                chunk(json!({}), Some("stop"))
            ),
        ),
        Turn::Reasoning(r, t) => (
            200,
            "text/event-stream",
            format!(
                "{}{}{}{done}",
                chunk(json!({"reasoning_content": r}), None),
                text_chunks(t),
                chunk(json!({}), Some("stop"))
            ),
        ),
        Turn::Calls(text, calls) => {
            let mut s = text.as_deref().map(text_chunks).unwrap_or_default();
            for (i, c) in calls.iter().enumerate() {
                s.push_str(&chunk(
                    json!({"tool_calls": [{"index": i, "id": c.id, "type": "function",
                                           "function": {"name": c.name, "arguments": ""}}]}),
                    None,
                ));
                s.push_str(&chunk(
                    json!({"tool_calls": [{"index": i, "function": {"arguments": c.args}}]}),
                    None,
                ));
            }
            s.push_str(&chunk(json!({}), Some("tool_calls")));
            s.push_str(done);
            (200, "text/event-stream", s)
        }
        Turn::Http(status, body) => (*status, "application/json", body.clone()),
        Turn::Error200(m) => (
            200,
            "application/json",
            json!({"error": {"message": m, "type": "server_error"}}).to_string(),
        ),
    }
}

impl MockLlm {
    pub async fn start(turns: Vec<Turn>) -> MockLlm {
        let l = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = l.local_addr().expect("addr");
        let captured: Arc<Mutex<Vec<Captured>>> = Arc::default();
        let script = Arc::new(Mutex::new(
            turns.into_iter().collect::<std::collections::VecDeque<_>>(),
        ));
        let cap = captured.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = l.accept().await else {
                    return;
                };
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                let (head_end, cl) = loop {
                    let n = s.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break (usize::MAX, 0);
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
                            break (he, cl);
                        }
                    }
                };
                if head_end == usize::MAX {
                    continue;
                }
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let body =
                    String::from_utf8_lossy(&buf[head_end + 4..head_end + 4 + cl]).to_string();
                let mut lines = head.lines();
                let path = lines
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("")
                    .to_string();
                let headers = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    .collect();
                cap.lock().unwrap().push(Captured {
                    path,
                    headers,
                    body,
                });
                let turn = script
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| Turn::Text("(script exhausted)".into()));
                let (status, ctype, out) = sse(&turn);
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                    out.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
                let _ = s.shutdown().await;
            }
        });
        MockLlm {
            base_url: format!("http://{addr}/v1"),
            captured,
        }
    }

    pub fn requests(&self) -> Vec<Captured> {
        self.captured.lock().unwrap().clone()
    }
}

#[derive(Default)]
pub struct MemSecrets(Mutex<HashMap<String, String>>);

impl MemSecrets {
    pub fn put(&self, r: &str, v: &str) {
        self.0.lock().unwrap().insert(r.into(), v.into());
    }
}

impl SecretStore for MemSecrets {
    fn get(&self, reference: &str) -> anyhow::Result<Option<SecretString>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(reference)
            .map(|v| SecretString::from(v.clone())))
    }
    fn set(&self, reference: &str, value: SecretString) -> anyhow::Result<()> {
        use secrecy::ExposeSecret;
        self.put(reference, value.expose_secret());
        Ok(())
    }
    fn delete(&self, reference: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().remove(reference);
        Ok(())
    }
    fn refs(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.0.lock().unwrap().keys().cloned().collect())
    }
}

pub struct FakeIndex {
    pub refreshes: Mutex<Vec<(Option<String>, String)>>,
    pub status: Mutex<Value>,
}

impl IndexControl for FakeIndex {
    fn status(&self) -> Value {
        self.status.lock().unwrap().clone()
    }
    fn refresh(&self, mode: &str) -> anyhow::Result<bool> {
        self.refresh_scope(None, mode)
    }
    fn refresh_scope(&self, scope: Option<&str>, mode: &str) -> anyhow::Result<bool> {
        self.refreshes
            .lock()
            .unwrap()
            .push((scope.map(str::to_string), mode.to_string()));
        Ok(true)
    }
}

pub struct Harness {
    pub dir: tempfile::TempDir,
    pub db: Db,
    pub env: Arc<AgentEnv>,
    pub index: Arc<FakeIndex>,
    pub secrets: Arc<MemSecrets>,
}

impl Harness {
    pub fn new(profile: Option<&str>) -> Harness {
        Self::build(profile, None)
    }

    pub fn with_clock(clock: Arc<AtomicI64>) -> Harness {
        Self::build(None, Some(clock))
    }

    fn build(profile: Option<&str>, clock: Option<Arc<AtomicI64>>) -> Harness {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open(dir.path().join("h.db")).expect("db");
        let index = Arc::new(FakeIndex {
            refreshes: Mutex::default(),
            status: Mutex::new(json!({"state": "idle"})),
        });
        let secrets = Arc::new(MemSecrets::default());
        let mut env = AgentEnv::new(db.clone(), Client::new(), secrets.clone(), index.clone());
        env.redact_profile = profile.map(str::to_string);
        if let Some(c) = clock {
            env.clock = Arc::new(move || c.load(Ordering::SeqCst));
        }
        Harness {
            dir,
            db,
            env: Arc::new(env),
            index,
            secrets,
        }
    }

    pub fn provider(&self, base_url: &str) -> ProviderConfig {
        let mut p = ProviderConfig::new("mock", base_url);
        p.id = Some("mock".into());
        p.model = Some("mock-model".into());
        p.timeout_s = 10;
        p
    }

    pub fn index_fake(&self, root: &str, files: &[(&str, u64)]) {
        let fs = FakeFs::new(root);
        fs.set_volume(2 << 40, 64 << 30);
        for (p, size) in files {
            fs.file(p, *size);
        }
        let out = refresh(
            &self.db.conn().expect("conn"),
            &fs,
            root,
            &RefreshOptions::default(),
        )
        .expect("refresh");
        assert!(out.completed);
    }

    pub async fn run(&self, provider: ProviderConfig, input: RunAgentInput) -> Vec<Value> {
        let (tx, mut rx) = mpsc::channel(1024);
        let task = tokio::spawn(run_agent(self.env.clone(), provider, input, tx));
        let mut out = Vec::new();
        let collect = async {
            while let Some(ev) = rx.recv().await {
                out.push(serde_json::to_value(&ev).expect("event json"));
            }
        };
        tokio::time::timeout(Duration::from_secs(60), collect)
            .await
            .expect("run finished in time");
        task.await.expect("join").expect("run_agent ok");
        out
    }
}

pub fn msg(id: &str, role: &str, content: &str) -> AgMessage {
    AgMessage {
        id: id.into(),
        role: role.into(),
        content: Some(content.into()),
        tool_calls: None,
        tool_call_id: None,
        data: None,
    }
}

pub fn input(thread: &str, messages: Vec<AgMessage>) -> RunAgentInput {
    RunAgentInput {
        thread_id: thread.into(),
        run_id: format!("run-{}", messages.first().map_or("x", |m| m.id.as_str())),
        messages,
        ..Default::default()
    }
}

pub fn user(thread: &str, id: &str, text: &str) -> RunAgentInput {
    input(thread, vec![msg(id, "user", text)])
}

pub fn types(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .map(|e| e["type"].as_str().unwrap_or("").to_string())
        .collect()
}

pub fn of_type<'a>(events: &'a [Value], t: &str) -> Vec<&'a Value> {
    events.iter().filter(|e| e["type"] == t).collect()
}

pub fn custom<'a>(events: &'a [Value], name: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|e| e["type"] == "CUSTOM" && e["name"] == name)
        .collect()
}

// The ordering rules @ag-ui/client enforces; a violation makes the UI drop the run.
pub fn assert_valid_sequence(events: &[Value]) {
    let t = types(events);
    assert_eq!(t.first().map(String::as_str), Some("RUN_STARTED"), "{t:?}");
    let last = t.last().map(String::as_str).unwrap_or("");
    assert!(last == "RUN_FINISHED" || last == "RUN_ERROR", "{t:?}");
    assert_eq!(
        t.iter()
            .filter(|x| *x == "RUN_FINISHED" || *x == "RUN_ERROR")
            .count(),
        1,
        "{t:?}"
    );
    let mut open_text: Option<String> = None;
    let mut open_calls: Vec<String> = Vec::new();
    let mut thinking = false;
    for e in events {
        match e["type"].as_str().unwrap_or("") {
            "TEXT_MESSAGE_START" => {
                assert!(open_text.is_none(), "nested text message");
                open_text = Some(e["messageId"].as_str().unwrap().to_string());
            }
            "TEXT_MESSAGE_CONTENT" => {
                assert_eq!(
                    open_text.as_deref(),
                    e["messageId"].as_str(),
                    "content outside its message"
                )
            }
            "TEXT_MESSAGE_END" => {
                assert_eq!(open_text.as_deref(), e["messageId"].as_str());
                open_text = None;
            }
            "TOOL_CALL_START" => open_calls.push(e["toolCallId"].as_str().unwrap().to_string()),
            "TOOL_CALL_ARGS" => assert!(open_calls.iter().any(|c| e["toolCallId"] == c.as_str())),
            "TOOL_CALL_END" => {
                let id = e["toolCallId"].as_str().unwrap();
                let pos = open_calls
                    .iter()
                    .position(|c| c == id)
                    .expect("end of an open call");
                open_calls.remove(pos);
            }
            "THINKING_START" => {
                assert!(!thinking);
                thinking = true;
            }
            "THINKING_END" => {
                assert!(thinking);
                thinking = false;
            }
            "RUN_FINISHED" => {
                assert!(
                    open_text.is_none() && open_calls.is_empty() && !thinking,
                    "open at finish"
                );
            }
            _ => {}
        }
    }
}

pub fn is_uuid_v7(s: &str) -> bool {
    let b = s.as_bytes();
    s.len() == 36
        && [8, 13, 18, 23].iter().all(|&i| b[i] == b'-')
        && b[14] == b'7'
        && s.chars()
            .enumerate()
            .all(|(i, c)| [8, 13, 18, 23].contains(&i) || c.is_ascii_hexdigit())
}

// Recursive listing with sizes, mtimes and content: what "the filesystem changed" means here.
pub fn fs_snapshot(
    root: &std::path::Path,
) -> Vec<(String, u64, Option<std::time::SystemTime>, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .expect("read_dir")
            .flatten()
            .collect();
        entries.sort_by_key(|e| e.path());
        for e in entries {
            let p = e.path();
            let md = std::fs::symlink_metadata(&p).expect("metadata");
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().to_string();
            if md.is_dir() {
                out.push((format!("{rel}\\"), 0, md.modified().ok(), Vec::new()));
                stack.push(p);
            } else {
                let content = std::fs::read(&p).unwrap_or_default();
                out.push((rel, md.len(), md.modified().ok(), content));
            }
        }
    }
    out.sort();
    out
}

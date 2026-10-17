mod builtin;

use hoardini_agent::provider::{
    AssembledTurn, ChatMessage, ChatRequest, Client, ProviderConfig, ProviderError, StreamEvent,
    StreamExt, ToolCallMsg, assemble_sse, build_body, presets, suggest_models,
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn fixture(name: &str) -> String {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/prov/fixtures/");
    std::fs::read_to_string(format!("{p}{name}")).expect("fixture")
}

fn assemble(name: &str) -> AssembledTurn {
    let raw = fixture(name);
    let whole = assemble_sse(raw.as_bytes(), raw.len()).expect("whole");
    for n in [1, 7] {
        assert_eq!(
            assemble_sse(raw.as_bytes(), n).expect("chunked").text,
            whole.text,
            "{name} text differs at chunk size {n}"
        );
    }
    let one = assemble_sse(raw.as_bytes(), 1).expect("chunked");
    assert_eq!(one.tool_calls.len(), whole.tool_calls.len(), "{name}");
    assert_eq!(one.reasoning, whole.reasoning, "{name}");
    assert_eq!(one.usage, whole.usage, "{name}");
    for (a, b) in one.tool_calls.iter().zip(&whole.tool_calls) {
        assert_eq!((&a.name, &a.arguments), (&b.name, &b.arguments), "{name}");
    }
    whole
}

type Handler = Arc<dyn Fn(&str, &str, &str) -> (u16, &'static str, String) + Send + Sync>;

async fn serve(h: Handler) -> String {
    let l = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = l.local_addr().expect("addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else {
                return;
            };
            let h = h.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let (head_end, cl) = loop {
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
                            break (he, cl);
                        }
                    }
                };
                let text = String::from_utf8_lossy(&buf).to_string();
                let first = text.lines().next().unwrap_or("").to_string();
                let mut parts = first.split_whitespace();
                let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                let body = &text[head_end + 4..head_end + 4 + cl];
                let (status, ctype, out) = h(method, path, body);
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                    out.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
                let _ = s.shutdown().await;
            });
        }
    });
    format!("http://{addr}/v1")
}

fn user_req(text: &str) -> ChatRequest {
    ChatRequest {
        messages: vec![ChatMessage::text("user", text)],
        ..Default::default()
    }
}

// Traces: PROV-COMPAT
#[test]
fn openai_indexed_tool_calls_and_usage() {
    let t = assemble("openai-tools.sse");
    assert_eq!(t.tool_calls.len(), 2);
    assert_eq!(t.tool_calls[0].id, "call_aaa");
    assert_eq!(t.tool_calls[0].arguments, r#"{"drive":"C"}"#);
    assert_eq!(t.tool_calls[1].id, "call_bbb");
    assert_eq!(t.tool_calls[1].arguments, r#"{"drive":"D"}"#);
    assert_eq!(t.finish_reason.as_deref(), Some("tool_calls"));
    let u = t.usage.expect("usage");
    assert_eq!(
        (u.prompt_tokens, u.completion_tokens, u.total_tokens),
        (Some(71), Some(38), Some(109))
    );
}

// Traces: PROV-COMPAT
#[test]
fn openrouter_comments_and_reasoning_field() {
    let t = assemble("openrouter-reasoning.sse");
    assert_eq!(t.text, "Your disk is fine.");
    assert_eq!(t.reasoning, "Let me think about it.");
    assert_eq!(t.finish_reason.as_deref(), Some("stop"));
    assert_eq!(t.usage.expect("usage").total_tokens, Some(21));
}

// Traces: PROV-COMPAT
#[test]
fn deepseek_reasoning_content() {
    let t = assemble("deepseek-reasoning.sse");
    assert_eq!(t.reasoning, "Check sizes first.");
    assert_eq!(t.text, "Delete the temp folder.");
    assert_eq!(t.usage.expect("usage").prompt_tokens, Some(20));
}

// Traces: PROV-COMPAT
#[test]
fn ollama_whole_call_in_one_chunk() {
    let t = assemble("ollama-tool.sse");
    assert_eq!(t.tool_calls.len(), 1);
    assert_eq!(t.tool_calls[0].id, "call_k1x9m2ab");
    assert_eq!(t.tool_calls[0].name, "get_free_space");
    assert_eq!(t.tool_calls[0].arguments, r#"{"drive":"C"}"#);
}

// Traces: PROV-COMPAT
#[test]
fn llamacpp_missing_index_and_ids() {
    let t = assemble("llamacpp-noindex.sse");
    assert_eq!(t.tool_calls.len(), 2);
    assert!(t.tool_calls[0].id.starts_with("call_"));
    assert_ne!(t.tool_calls[0].id, "call_second");
    assert_eq!(t.tool_calls[0].arguments, r#"{"drive":"C"}"#);
    assert_eq!(t.tool_calls[1].id, "call_second");
    assert_eq!(t.tool_calls[1].arguments, r#"{"drive":"D"}"#);
    assert_eq!(t.usage.expect("usage").total_tokens, Some(120));
}

// Traces: PROV-COMPAT
#[test]
fn arguments_sent_as_object_are_reserialized() {
    let t = assemble("llamacpp-object-args.sse");
    assert_eq!(t.tool_calls.len(), 1);
    let args: Value = serde_json::from_str(&t.tool_calls[0].arguments).expect("json");
    assert_eq!(args, json!({"drive": "C"}));
}

// Traces: PROV-COMPAT
#[test]
fn lmstudio_recordings() {
    let text = assemble("lmstudio-text.sse");
    assert!(!text.text.trim().is_empty());
    assert!(text.usage.is_some());

    let tool = assemble("lmstudio-tool.sse");
    assert_eq!(tool.tool_calls.len(), 1);
    assert_eq!(tool.tool_calls[0].name, "get_free_space");
    let args: Value = serde_json::from_str(&tool.tool_calls[0].arguments).expect("json");
    assert!(args["drive"].as_str().is_some_and(|d| d.starts_with('C')));

    let par = assemble("lmstudio-parallel.sse");
    assert!(par.tool_calls.len() >= 2);
    assert_ne!(par.tool_calls[0].id, par.tool_calls[1].id);
    for c in &par.tool_calls {
        serde_json::from_str::<Value>(&c.arguments).expect("valid json args");
    }

    let unknown = assemble("lmstudio-unknown-model.sse");
    assert!(!unknown.text.trim().is_empty() || !unknown.reasoning.trim().is_empty());
}

// Traces: PROV-COMPAT
#[test]
fn sse_error_data_in_200_is_an_error() {
    let raw = fixture("error-200.sse");
    match assemble_sse(raw.as_bytes(), 5) {
        Err(ProviderError::Api(m)) => assert!(m.contains("gpt-9") && m.contains("does not exist")),
        other => panic!("expected Api error, got {other:?}"),
    }
}

// Traces: PROV-COMPAT
#[tokio::test]
async fn client_streams_fixture_and_surfaces_200_errors() {
    let sse = fixture("openai-tools.sse");
    let err_sse = fixture("error-200.sse");
    let base = serve(Arc::new(move |_, path, body| match path {
        "/v1/chat/completions" if body.contains("trigger-json-error") => (
            200,
            "application/json",
            r#"{"error":{"message":"Context size has been exceeded.","type":"server_error"}}"#
                .into(),
        ),
        "/v1/chat/completions" if body.contains("trigger-sse-error") => {
            (200, "text/event-stream", err_sse.clone())
        }
        _ => (200, "text/event-stream", sse.clone()),
    }))
    .await;
    let cfg = ProviderConfig::new("mock", &base);
    let c = Client::new();

    let mut s = c
        .chat_stream(&cfg, None, &user_req("hi"))
        .await
        .expect("stream");
    let mut starts = 0;
    let mut done = None;
    while let Some(ev) = s.next().await {
        match ev.expect("event") {
            StreamEvent::ToolCallStart { .. } => starts += 1,
            StreamEvent::Done(t) => done = Some(t),
            _ => {}
        }
    }
    assert_eq!(starts, 2);
    assert_eq!(done.expect("done").tool_calls.len(), 2);

    let mut s = c
        .chat_stream(&cfg, None, &user_req("trigger-json-error"))
        .await
        .expect("stream");
    let first = s.next().await.expect("item");
    assert!(matches!(&first, Err(ProviderError::Api(m)) if m.contains("Context size")));
    assert!(hoardini_agent::provider::is_context_overflow(
        &first.unwrap_err()
    ));

    let mut s = c
        .chat_stream(&cfg, None, &user_req("trigger-sse-error"))
        .await
        .expect("stream");
    assert!(matches!(s.next().await, Some(Err(ProviderError::Api(_)))));

    let err = c
        .chat(&cfg, None, &user_req("trigger-json-error"))
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Api(_)));
}

// Traces: PROV-COMPAT
#[test]
fn presets_cover_required_providers() {
    let names: Vec<_> = presets().iter().map(|p| p.id).collect();
    for want in [
        "openai",
        "openrouter",
        "anthropic",
        "groq",
        "together",
        "ollama",
        "lmstudio",
        "llamacpp",
        "vllm",
    ] {
        assert!(names.contains(&want), "{want}");
    }
    let ollama = presets()
        .into_iter()
        .find(|p| p.id == "ollama")
        .expect("ollama");
    assert_eq!(ollama.base_url, "http://127.0.0.1:11434/v1");
    assert!(!ollama.needs_key);
}

fn body_cfg() -> ProviderConfig {
    let mut c = ProviderConfig::new("t", "http://x/v1");
    c.model = Some("m".into());
    c
}

// Traces: PROV-EXTRA-BODY
#[test]
fn extra_body_merges_last() {
    let mut cfg = body_cfg();
    cfg.params.temperature = Some(0.2);
    cfg.extra_body = serde_json::from_value(json!({
        "reasoning_effort": "low",
        "provider": {"order": ["A"]},
        "temperature": 0.9,
        "stop": ["x"],
    }))
    .expect("map");
    let mut req = user_req("hi");
    req.tools = vec![];
    let mut body = build_body(&cfg, &req, true);
    assert_eq!(body["reasoning_effort"], "low");
    assert_eq!(body["temperature"], 0.9);
    assert_eq!(body["stop"], json!(["x"]));

    // pre-existing nested object and array are merged / replaced respectively
    let mut cfg2 = body_cfg();
    cfg2.extra_body =
        serde_json::from_value(json!({"provider": {"order": ["A"]}, "stop": ["new"]}))
            .expect("map");
    body = build_body(&cfg2, &user_req("hi"), false);
    let mut base = json!({"provider": {"sort": "price"}, "stop": ["old", "older"]});
    hoardini_agent::provider::deep_merge(&mut base, &Value::Object(cfg2.extra_body.clone()));
    assert_eq!(base["provider"], json!({"sort": "price", "order": ["A"]}));
    assert_eq!(base["stop"], json!(["new"]));
    assert_eq!(body["provider"], json!({"order": ["A"]}));
}

// Traces: PROV-SANITIZE
#[test]
fn blank_model_is_omitted() {
    for m in [None, Some(""), Some("  ")] {
        let mut cfg = body_cfg();
        cfg.model = m.map(str::to_string);
        let body = build_body(&cfg, &user_req("hi"), false);
        assert!(body.get("model").is_none(), "{m:?}");
    }
    assert_eq!(
        build_body(&body_cfg(), &user_req("hi"), false)["model"],
        "m"
    );
}

// Traces: PROV-SANITIZE
#[test]
fn reasoning_stripped_from_assistant_history_only() {
    let mut asst = ChatMessage::text("assistant", "answer");
    asst.reasoning_content = Some("secret thoughts".into());
    asst.tool_calls = Some(vec![ToolCallMsg::new("call_1", "get_free_space", "{}")]);
    let mut user = ChatMessage::text("user", "q");
    user.reasoning_content = Some("keep".into());
    let req = ChatRequest {
        messages: vec![user, asst],
        ..Default::default()
    };
    let body = build_body(&body_cfg(), &req, false);
    let msgs = body["messages"].as_array().expect("messages");
    assert_eq!(msgs[0]["reasoning_content"], "keep");
    assert!(msgs[1].get("reasoning_content").is_none());
    assert!(msgs[1].get("reasoning").is_none());
    assert_eq!(msgs[1]["content"], "answer");
    assert_eq!(
        msgs[1]["tool_calls"][0]["function"]["name"],
        "get_free_space"
    );
}

// Traces: PROV-SANITIZE
#[test]
fn stream_options_only_when_opted_in() {
    let mut req = user_req("hi");
    assert!(
        build_body(&body_cfg(), &req, true)
            .get("stream_options")
            .is_none()
    );
    req.include_usage = true;
    assert_eq!(
        build_body(&body_cfg(), &req, true)["stream_options"],
        json!({"include_usage": true})
    );
    assert!(
        build_body(&body_cfg(), &req, false)
            .get("stream_options")
            .is_none()
    );
}

fn model_mock(models: &'static [&'static str], chats: Arc<AtomicUsize>) -> Handler {
    let tool_stream = fixture("ollama-tool.sse");
    Arc::new(move |method, path, body| match (method, path) {
        ("GET", "/v1/models") => {
            let data: Vec<_> = models
                .iter()
                .map(|m| json!({"id": m, "object": "model"}))
                .collect();
            (200, "application/json", json!({"data": data}).to_string())
        }
        ("POST", "/v1/chat/completions") => {
            chats.fetch_add(1, Ordering::SeqCst);
            if body.contains("\"stream\":true") {
                (200, "text/event-stream", tool_stream.clone())
            } else {
                let msg = json!({"role": "assistant", "content": null, "tool_calls": [
                    {"id": "call_z", "type": "function", "function": {"name": "ping", "arguments": "{\"message\":\"hi\"}"}}]});
                (200, "application/json", json!({"choices": [{"index": 0, "message": msg, "finish_reason": "tool_calls"}]}).to_string())
            }
        }
        _ => (404, "application/json", "{}".into()),
    })
}

const MODELS: &[&str] = &["qwen3-8b-instruct", "llama3.2:3b", "qwen3-4b"];

// Traces: PROV-MODEL-CHECK
#[tokio::test]
async fn unlisted_model_is_flagged_with_suggestions() {
    let chats = Arc::new(AtomicUsize::new(0));
    let base = serve(model_mock(MODELS, chats.clone())).await;
    let mut cfg = ProviderConfig::new("mock", &base);
    cfg.model = Some("qwen3-8b-instrcut".into());
    let rep = Client::new().test_connection(&cfg, None).await;
    assert!(!rep.ok);
    assert_eq!(rep.model_found, Some(false));
    assert_eq!(rep.suggestions[0], "qwen3-8b-instruct");
    assert!(rep.suggestions.len() <= 3);
    assert_eq!(rep.models.len(), 3);
    assert!(rep.caps.is_none());
    assert!(
        rep.error
            .as_deref()
            .unwrap_or("")
            .contains("qwen3-8b-instrcut")
    );
    assert!(rep.latency_ms.is_some());
    assert_eq!(chats.load(Ordering::SeqCst), 0);
}

// Traces: PROV-MODEL-CHECK
#[tokio::test]
async fn listed_model_passes_and_probes_caps() {
    let chats = Arc::new(AtomicUsize::new(0));
    let base = serve(model_mock(MODELS, chats.clone())).await;
    let mut cfg = ProviderConfig::new("mock", &base);
    cfg.model = Some("llama3.2:3b".into());
    let rep = Client::new().test_connection(&cfg, None).await;
    assert!(rep.ok, "{:?}", rep.error);
    assert_eq!(rep.model_found, Some(true));
    let caps = rep.caps.expect("caps");
    assert!(caps.tools && caps.streaming && caps.streamed_tool_calls && caps.tool_choice);
}

// Traces: PROV-MODEL-CHECK
#[tokio::test]
async fn missing_models_endpoint_is_tolerated() {
    let base = serve(Arc::new(|_, path, _| match path {
        "/v1/chat/completions" => (200, "application/json", json!({"choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}]}).to_string()),
        _ => (404, "text/plain", "nope".into()),
    }))
    .await;
    let mut cfg = ProviderConfig::new("mock", &base);
    cfg.model = Some("anything".into());
    let rep = Client::new().test_connection(&cfg, None).await;
    assert_eq!(rep.model_found, None);
    assert!(rep.ok, "{:?}", rep.error);
    assert!(!rep.caps.expect("caps").tools);
}

// Traces: PROV-MODEL-CHECK
#[test]
fn suggestions_rank_near_matches_first() {
    let avail: Vec<String> = [
        "gpt-4o-mini",
        "QWEN3-8B-Instruct",
        "mistral-7b",
        "qwen3-8b",
        "llama-3-8b-instruct",
        "deepseek-r1",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let s = suggest_models("qwen3-8b-instruct", &avail);
    assert_eq!(s[0], "QWEN3-8B-Instruct");
    assert!(s.contains(&"qwen3-8b".to_string()));
    assert!(s.len() <= 3);
    assert!(!s.contains(&"deepseek-r1".to_string()));
    assert!(suggest_models("zzzzzz", &avail).is_empty());
}

const OK_CHAT: &str = r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]}"#;

type Waits = Arc<std::sync::Mutex<Vec<std::time::Duration>>>;
type Notices = Arc<std::sync::Mutex<Vec<u64>>>;

fn recording_client() -> (Client, Waits, Notices) {
    let waits: Waits = Arc::default();
    let noticed: Notices = Arc::default();
    let (w, n) = (waits.clone(), noticed.clone());
    let client = Client::new()
        .with_sleeper(Arc::new(move |d| {
            w.lock().unwrap().push(d);
            Box::pin(async {})
        }))
        .with_wait_notice(Arc::new(move |s| n.lock().unwrap().push(s)));
    (client, waits, noticed)
}

// The ctype slot is written raw into the response head, so it can carry an extra header.
fn limited(first: &'static str, hits: Arc<AtomicUsize>, then_ok: bool) -> Handler {
    Arc::new(move |_, _, _| {
        let n = hits.fetch_add(1, Ordering::SeqCst);
        if then_ok && n >= 1 {
            (200, "application/json", OK_CHAT.into())
        } else {
            (429, first, r#"{"error":{"message":"slow down"}}"#.into())
        }
    })
}

fn cfg_for(base: &str) -> ProviderConfig {
    ProviderConfig::new("mock", base)
}

// Traces: PROV-RATE-LIMIT
#[tokio::test]
async fn retry_after_is_honored_then_the_request_succeeds() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = serve(limited(
        "application/json\r\nRetry-After: 7",
        hits.clone(),
        true,
    ))
    .await;
    let (client, waits, noticed) = recording_client();
    let turn = client
        .chat(&cfg_for(&base), None, &user_req("hi"))
        .await
        .expect("retried");
    assert_eq!(turn.text, "hello");
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(
        *waits.lock().unwrap(),
        vec![std::time::Duration::from_secs(7)]
    );
    assert_eq!(*noticed.lock().unwrap(), vec![7]);
}

// Traces: PROV-RATE-LIMIT
#[tokio::test]
async fn retry_after_http_date_and_streaming_path() {
    let at = std::time::SystemTime::now() + std::time::Duration::from_secs(30);
    let ctype: &'static str = Box::leak(
        format!(
            "application/json\r\nRetry-After: {}",
            httpdate::fmt_http_date(at)
        )
        .into_boxed_str(),
    );
    let hits = Arc::new(AtomicUsize::new(0));
    let base = serve(limited(ctype, hits.clone(), true)).await;
    let (client, waits, _) = recording_client();
    let mut s = client
        .chat_stream(&cfg_for(&base), None, &user_req("hi"))
        .await
        .expect("retried");
    while s.next().await.is_some() {}
    let w = waits.lock().unwrap()[0].as_secs();
    assert!((27..=30).contains(&w), "{w}");
}

// Traces: PROV-RATE-LIMIT
#[tokio::test]
async fn waits_are_capped_at_sixty_seconds() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = serve(limited("application/json\r\nRetry-After: 3600", hits, true)).await;
    let (client, waits, _) = recording_client();
    client
        .chat(&cfg_for(&base), None, &user_req("hi"))
        .await
        .expect("ok");
    assert_eq!(
        *waits.lock().unwrap(),
        vec![std::time::Duration::from_secs(60)]
    );
}

// Traces: PROV-RATE-LIMIT
#[tokio::test]
async fn three_429s_in_a_row_end_with_a_rate_limit_error() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = serve(limited("application/json", hits.clone(), false)).await;
    let (client, waits, _) = recording_client();
    let err = client
        .chat(&cfg_for(&base), None, &user_req("hi"))
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::RateLimited(_)), "{err:?}");
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    let secs: Vec<u64> = waits.lock().unwrap().iter().map(|d| d.as_secs()).collect();
    assert_eq!(secs, vec![2, 4]);
}

// Traces: PROV-RATE-LIMIT
#[tokio::test]
async fn daily_quota_429s_end_at_once() {
    let bodies = [
        r#"{"error":{"message":"Rate limit exceeded: free-models-per-day"}}"#,
        r#"{"error":{"status":"RESOURCE_EXHAUSTED","message":"Quota exceeded for metric GenerateRequestsPerDayPerProjectPerModel-FreeTier"}}"#,
        r#"{"message":"Tokens per day limit exceeded - too many tokens processed."}"#,
    ];
    for body in bodies {
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let base = serve(Arc::new(move |_, _, _| {
            h.fetch_add(1, Ordering::SeqCst);
            (429, "application/json", body.to_string())
        }))
        .await;
        let (client, waits, _) = recording_client();
        let err = client
            .chat_stream(&cfg_for(&base), None, &user_req("hi"))
            .await
            .err()
            .expect("error");
        assert!(
            matches!(err, ProviderError::DailyQuota(_)),
            "{body}: {err:?}"
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(waits.lock().unwrap().is_empty());
    }
}

// Traces: PROV-PRESETS
#[test]
fn cerebras_and_gemini_presets_have_base_urls_and_models() {
    let all = presets();
    let get = |id: &str| all.iter().find(|p| p.id == id).expect(id);
    let c = get("cerebras");
    assert_eq!(c.base_url, "https://api.cerebras.ai/v1");
    assert_eq!(c.default_model, Some("gpt-oss-120b"));
    assert!(c.needs_key);
    let g = get("gemini");
    assert_eq!(
        g.base_url,
        "https://generativelanguage.googleapis.com/v1beta/openai/"
    );
    assert!(g.default_model.is_some_and(|m| m.contains("flash")));
    assert!(
        get("openrouter")
            .default_model
            .is_some_and(|m| m.ends_with(":free"))
    );
    assert!(get("ollama").default_model.is_none());
    for id in [
        "gemini",
        "openrouter",
        "mistral",
        "zai",
        "ollama-cloud",
        "cloudflare",
    ] {
        let p = get(id);
        assert!(
            p.needs_key && p.default_model.is_some() && p.key_url.is_some(),
            "{id}"
        );
    }
    assert!(get("cloudflare").base_url.contains("{account_id}"));
}

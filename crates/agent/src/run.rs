use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use hoardini_core::history::fmt_date;
use hoardini_core::index::prompt_summary;
use hoardini_core::memory::globals_for;
use hoardini_core::paths::profile_dir;
use hoardini_core::store::{Db, now_ms};
use hoardini_core::vault::{SecretStore, is_ref};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::agui::{Event, RunAgentInput};
use crate::prompts::{PromptInput, system_prompt};
use crate::provider::{
    AssembledTurn, ChatMessage, ChatRequest, Client, ProviderConfig, ProviderError, StreamEvent,
    ToolCallMsg, is_context_overflow,
};
use crate::search::{SearchResult, format_for_llm};
use crate::thread::{self, Stored};
use crate::tools::util::{redact_all, redact_value};
use crate::tools::{
    About, DATA_VERSION, Kind, Outcome, Registry, ToolCtx, ToolError, learn_prompt, parse_identity,
    refresh_note, remember_identity,
};

// Implemented by the server's indexer supervisor; the agent only asks.
pub trait IndexControl: Send + Sync {
    fn status(&self) -> Value;
    fn refresh(&self, mode: &str) -> anyhow::Result<bool>;
    // Supervisors without scoped refreshes refresh everything.
    fn refresh_scope(&self, _scope: Option<&str>, mode: &str) -> anyhow::Result<bool> {
        self.refresh(mode)
    }
}

pub struct AgentEnv {
    pub db: Db,
    pub client: Client,
    pub secrets: Arc<dyn SecretStore>,
    pub index: Arc<dyn IndexControl>,
    // Profile used for `~` redaction and expansion; None = the real one (tests inject a fake).
    pub redact_profile: Option<String>,
    // Milliseconds since the epoch; tests move it to cross the session gap.
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl AgentEnv {
    pub fn new(
        db: Db,
        client: Client,
        secrets: Arc<dyn SecretStore>,
        index: Arc<dyn IndexControl>,
    ) -> AgentEnv {
        AgentEnv {
            db,
            client,
            secrets,
            index,
            redact_profile: None,
            clock: Arc::new(now_ms),
        }
    }

    pub fn now(&self) -> i64 {
        (self.clock)()
    }

    pub fn profile(&self) -> String {
        self.redact_profile
            .clone()
            .unwrap_or_else(|| profile_dir().to_string_lossy().into_owned())
    }
}

const GLOBAL_MEMORIES: usize = 30;
const SMALL_GLOBAL_MEMORIES: usize = 10;
const SMALL_SUMMARY_CHARS: usize = 1500;
const MAX_SHEDS: usize = 3;

pub const OVERFLOW_MESSAGE: &str = "This conversation no longer fits the model's context window, even after dropping old tool results and earlier turns. Press Start fresh, or switch to a provider profile with a larger context window.";

pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

enum Stop {
    Closed,
    Fail(String),
}

impl From<anyhow::Error> for Stop {
    fn from(e: anyhow::Error) -> Stop {
        Stop::Fail(format!("Internal error: {e:#}"))
    }
}

struct RoundEnd {
    proposal_pending: bool,
    bad_args: bool,
    // The answer is written and its follow-ups are shown, so another model round adds nothing.
    answered: bool,
}

enum TurnErr {
    Provider(ProviderError),
    Stop(Stop),
}

struct Emitter {
    tx: mpsc::Sender<Event>,
    text: Option<String>,
    turn_text: Option<String>,
    thinking: bool,
}

impl Emitter {
    async fn send(&mut self, ev: Event) -> Result<(), Stop> {
        self.tx.send(ev).await.map_err(|_| Stop::Closed)
    }

    async fn custom(&mut self, name: &str, value: Value) -> Result<(), Stop> {
        self.send(Event::Custom {
            name: name.into(),
            value,
        })
        .await
    }

    async fn text_delta(&mut self, delta: String) -> Result<(), Stop> {
        self.end_thinking().await?;
        let id = match &self.text {
            Some(id) => id.clone(),
            None => {
                let id = new_id();
                self.send(Event::TextMessageStart {
                    message_id: id.clone(),
                    role: "assistant".into(),
                })
                .await?;
                self.text = Some(id.clone());
                self.turn_text.get_or_insert_with(|| id.clone());
                id
            }
        };
        self.send(Event::TextMessageContent {
            message_id: id,
            delta,
        })
        .await
    }

    async fn reasoning_delta(&mut self, delta: String) -> Result<(), Stop> {
        if !self.thinking {
            self.send(Event::ThinkingStart {}).await?;
            self.send(Event::ThinkingTextMessageStart {}).await?;
            self.thinking = true;
        }
        self.send(Event::ThinkingTextMessageContent { delta }).await
    }

    async fn end_thinking(&mut self) -> Result<(), Stop> {
        if self.thinking {
            self.thinking = false;
            self.send(Event::ThinkingTextMessageEnd {}).await?;
            self.send(Event::ThinkingEnd {}).await?;
        }
        Ok(())
    }

    async fn end_text(&mut self) -> Result<(), Stop> {
        if let Some(id) = self.text.take() {
            self.send(Event::TextMessageEnd { message_id: id }).await?;
        }
        Ok(())
    }

    async fn close_open(&mut self) {
        let _ = self.end_thinking().await;
        let _ = self.end_text().await;
    }
}

struct Pending {
    call_id: String,
    result_id: String,
    about: About,
    results: Vec<SearchResult>,
}

struct Run {
    env: Arc<AgentEnv>,
    provider: ProviderConfig,
    registry: Registry,
    profile: String,
    thread_id: String,
    secrets_used: Vec<String>,
    steps: u32,
    limit_reached: bool,
    parse_retry_used: bool,
    resolved: Option<(ProviderConfig, Option<String>)>,
    learn: Vec<Pending>,
}

// Streams AG-UI events into `tx`; the server aborts the task to cancel a run.
pub async fn run_agent(
    env: Arc<AgentEnv>,
    provider: ProviderConfig,
    input: RunAgentInput,
    tx: mpsc::Sender<Event>,
) -> anyhow::Result<()> {
    let started = Instant::now();
    let nonempty = |s: &str| (!s.trim().is_empty()).then(|| s.to_string());
    let thread_id = nonempty(&input.thread_id).unwrap_or_else(new_id);
    let run_id = nonempty(&input.run_id).unwrap_or_else(new_id);
    let mut em = Emitter {
        tx,
        text: None,
        turn_text: None,
        thinking: false,
    };
    if em
        .send(Event::RunStarted {
            thread_id: thread_id.clone(),
            run_id: run_id.clone(),
        })
        .await
        .is_err()
    {
        return Ok(());
    }
    let mut run = Run {
        registry: Registry::for_profile(provider.small).available(&env.db),
        profile: env.profile(),
        env,
        provider,
        thread_id: thread_id.clone(),
        secrets_used: Vec::new(),
        steps: 0,
        limit_reached: false,
        parse_retry_used: false,
        resolved: None,
        learn: Vec::new(),
    };
    let result = run.go(&mut em, input).await;
    em.close_open().await;
    if result.is_ok() {
        run.learn_from_searches(&mut em).await;
    }
    let stats = json!({
        "elapsed_ms": started.elapsed().as_millis() as u64,
        "steps": run.steps,
        "limit_reached": run.limit_reached,
    });
    match result {
        Ok(()) => {
            let _ = em.custom("run_stats", stats).await;
            let _ = em.send(Event::RunFinished { thread_id, run_id }).await;
        }
        Err(Stop::Fail(message)) => {
            let _ = em.custom("run_stats", stats).await;
            let message = run.scrub(&message);
            let _ = em
                .send(Event::RunError {
                    message,
                    code: None,
                })
                .await;
        }
        Err(Stop::Closed) => {}
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, Stop> {
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r.map_err(Stop::from),
        Err(e) => Err(Stop::Fail(format!("Internal error: {e}"))),
    }
}

impl Run {
    fn scrub(&self, s: &str) -> String {
        let mut out = s.to_string();
        for secret in self.secrets_used.iter().filter(|v| v.len() >= 4) {
            out = out.replace(secret.as_str(), "[secret]");
        }
        out
    }

    fn secret(&mut self, reference: &str) -> Result<String, Stop> {
        match self.env.secrets.get(reference) {
            Ok(Some(v)) => {
                let v = v.expose_secret().to_string();
                self.secrets_used.push(v.clone());
                Ok(v)
            }
            Ok(None) => Err(Stop::Fail(format!(
                "{reference} is not in the vault. Set it in Settings, AI."
            ))),
            Err(e) => Err(Stop::Fail(format!(
                "Could not read {reference} from the vault: {e}"
            ))),
        }
    }

    // The key travels only as the Authorization header; secret:// header values are resolved here.
    fn resolve_provider(&mut self) -> Result<(ProviderConfig, Option<String>), Stop> {
        let mut cfg = self.provider.clone();
        let key = match cfg.api_key.take().filter(|k| !k.trim().is_empty()) {
            None => None,
            Some(k) if is_ref(k.trim()) => Some(self.secret(k.trim())?),
            Some(k) => Some(k),
        };
        let refs: Vec<(String, String)> = cfg
            .headers
            .iter()
            .filter(|(_, v)| is_ref(v.trim()))
            .map(|(k, v)| (k.clone(), v.trim().to_string()))
            .collect();
        for (k, r) in refs {
            let v = self.secret(&r)?;
            cfg.headers.insert(k, v);
        }
        Ok((cfg, key))
    }

    async fn go(&mut self, em: &mut Emitter, input: RunAgentInput) -> Result<(), Stop> {
        let (cfg, key) = self.resolve_provider()?;
        self.resolved = Some((cfg.clone(), key.clone()));
        let db = self.env.db.clone();
        let conv = self.thread_id.clone();
        let provider_id = self.provider.id.clone();
        let messages = input.messages;
        let now = self.env.now();
        blocking(move || thread::ingest(&db, &conv, provider_id.as_deref(), &messages, now))
            .await?;

        let max_steps = self.provider.max_tool_steps.max(1);
        let context_tokens = thread::context_tokens(self.provider.context_window);
        let mut sheds = 0;
        let mut first_system: Option<String> = None;
        loop {
            let (system, mut stored) = self.load_context().await?;
            // The summary's "as of N minutes ago" ticks; a fixed prefix keeps a local server's prompt cache warm.
            let system = first_system.get_or_insert(system).clone();
            let tools = self.registry.defs();
            let overhead = system.len() + serde_json::to_string(&tools).map_or(0, |t| t.len());
            if let Some(i) = thread::trim_point(&stored, overhead, context_tokens) {
                self.move_window(&stored[i].id).await?;
                stored.drain(..i);
            }
            let db = self.env.db.clone();
            let conv = self.thread_id.clone();
            let snapshot = stored.clone();
            let mut msgs = blocking(move || Ok(thread::context(&db, &conv, &snapshot))).await?;
            msgs.insert(0, ChatMessage::text("system", &system));
            redact_messages(&mut msgs, &self.profile);
            let req = ChatRequest {
                messages: msgs,
                tools,
                tool_choice: None,
                include_usage: false,
            };
            let turn = match self.stream_turn(em, &cfg, key.as_deref(), &req).await {
                Ok(t) => t,
                Err(TurnErr::Stop(s)) => return Err(s),
                Err(TurnErr::Provider(e)) if is_context_overflow(&e) => {
                    if sheds < MAX_SHEDS && self.shed(&mut stored).await? {
                        sheds += 1;
                        continue;
                    }
                    let Some(i) = thread::next_turn(&stored) else {
                        return Err(Stop::Fail(OVERFLOW_MESSAGE.into()));
                    };
                    self.move_window(&stored[i].id).await?;
                    continue;
                }
                Err(TurnErr::Provider(e)) => return Err(Stop::Fail(describe(&e, &cfg))),
            };
            let msg_id = em.turn_text.take().unwrap_or_else(new_id);
            if turn.tool_calls.is_empty() {
                if !turn.text.trim().is_empty() {
                    self.store(
                        vec![Stored {
                            id: msg_id,
                            msg: turn.to_message(),
                        }],
                        Vec::new(),
                    )
                    .await?;
                }
                return Ok(());
            }
            let end = self.handle_calls(em, turn, msg_id, &stored).await?;
            if end.proposal_pending || end.answered {
                return Ok(());
            }
            self.steps += 1;
            if end.bad_args {
                if self.parse_retry_used {
                    return Err(Stop::Fail(
                        "The model sent tool calls Hoardini could not parse twice in a row. Try again, rephrase the request, or switch to a stronger provider profile.".into(),
                    ));
                }
                self.parse_retry_used = true;
            } else {
                self.parse_retry_used = false;
            }
            if self.steps >= max_steps {
                // The UI offers "keep going" when a run ends on answered tool calls.
                self.limit_reached = true;
                return Ok(());
            }
        }
    }

    async fn load_context(&self) -> Result<(String, Vec<Stored>), Stop> {
        let db = self.env.db.clone();
        let conv = self.thread_id.clone();
        let small = self.provider.small;
        let refresh = refresh_note(&self.env.index.status());
        blocking(move || {
            let conn = db.conn()?;
            // A broken summary should cost the model context, not the whole chat.
            let mut summary = prompt_summary(&conn, small)
                .unwrap_or_else(|e| format!("The index summary is unavailable ({e})."));
            if small && summary.len() > SMALL_SUMMARY_CHARS {
                let mut cut = SMALL_SUMMARY_CHARS;
                while !summary.is_char_boundary(cut) {
                    cut -= 1;
                }
                summary.truncate(cut);
            }
            let cap = if small {
                SMALL_GLOBAL_MEMORIES
            } else {
                GLOBAL_MEMORIES
            };
            let stored = thread::load_window(&conn, &conv)?;
            let said = stored.iter().rev().find(|s| s.msg.role == "user");
            let notes = globals_for(
                &conn,
                cap,
                said.and_then(|s| s.msg.content.as_deref()).unwrap_or(""),
            )
            .unwrap_or_default()
            .into_iter()
            .map(|m| {
                let kind = serde_json::to_value(m.kind).unwrap_or_default();
                format!(
                    "[{}, {}] {}",
                    kind.as_str().unwrap_or("note"),
                    m.source,
                    m.text
                )
            })
            .collect();
            let open_proposals = thread::open_proposals(&db, &conv).unwrap_or_default();
            let system = system_prompt(&PromptInput {
                small,
                today: fmt_date(now_ms()),
                index_summary: summary,
                refresh,
                globals: notes,
                open_proposals,
            });
            Ok((system, stored))
        })
        .await
    }

    async fn move_window(&self, message_id: &str) -> Result<(), Stop> {
        let db = self.env.db.clone();
        let conv = self.thread_id.clone();
        let id = message_id.to_string();
        blocking(move || thread::set_window_start_at(&db.conn()?, &conv, &id)).await
    }

    async fn shed(&self, stored: &mut [Stored]) -> Result<bool, Stop> {
        let db = self.env.db.clone();
        let mut owned = stored.to_vec();
        let (shed, owned) = blocking(move || {
            let r = thread::shed_largest(&db.conn()?, &mut owned)?;
            Ok((r.is_some(), owned))
        })
        .await?;
        stored.clone_from_slice(&owned);
        Ok(shed)
    }

    async fn store(&self, msgs: Vec<Stored>, ui: Vec<(String, Value)>) -> Result<(), Stop> {
        let db = self.env.db.clone();
        let conv = self.thread_id.clone();
        let now = self.env.now();
        blocking(move || {
            let mut conn = db.conn()?;
            thread::append_all_at(&mut conn, &conv, &msgs, now)?;
            for (id, data) in &ui {
                thread::set_ui(&conn, id, data)?;
            }
            Ok(())
        })
        .await
    }

    async fn stream_turn(
        &self,
        em: &mut Emitter,
        cfg: &ProviderConfig,
        key: Option<&str>,
        req: &ChatRequest,
    ) -> Result<AssembledTurn, TurnErr> {
        em.turn_text = None;
        let (wtx, mut wrx) = mpsc::unbounded_channel::<u64>();
        let client = self.env.client.clone().with_wait_notice(Arc::new(move |s| {
            let _ = wtx.send(s);
        }));
        let open = client.chat_stream(cfg, key, req);
        tokio::pin!(open);
        let mut stream = loop {
            tokio::select! {
                r = &mut open => break r.map_err(TurnErr::Provider)?,
                Some(seconds) = wrx.recv() => {
                    em.custom("rate_limit_wait", json!({ "seconds": seconds }))
                        .await
                        .map_err(TurnErr::Stop)?;
                }
            }
        };
        while let Some(item) = stream.next().await {
            match item {
                Err(e) => {
                    em.close_open().await;
                    return Err(TurnErr::Provider(e));
                }
                Ok(StreamEvent::Text(t)) => em.text_delta(t).await.map_err(TurnErr::Stop)?,
                Ok(StreamEvent::Reasoning(r)) => {
                    em.reasoning_delta(r).await.map_err(TurnErr::Stop)?
                }
                Ok(StreamEvent::Done(turn)) => {
                    em.end_thinking().await.map_err(TurnErr::Stop)?;
                    em.end_text().await.map_err(TurnErr::Stop)?;
                    return Ok(turn);
                }
                Ok(_) => {}
            }
        }
        em.close_open().await;
        Err(TurnErr::Provider(ProviderError::Protocol(
            "the stream ended without a final chunk".into(),
        )))
    }

    fn ctx(&self, call_id: &str) -> ToolCtx {
        ToolCtx {
            db: self.env.db.clone(),
            index: self.env.index.clone(),
            secrets: self.env.secrets.clone(),
            profile: self.profile.clone(),
            conversation_id: Some(self.thread_id.clone()),
            tool_call_id: call_id.to_string(),
        }
    }

    async fn handle_calls(
        &mut self,
        em: &mut Emitter,
        turn: AssembledTurn,
        msg_id: String,
        stored: &[Stored],
    ) -> Result<RoundEnd, Stop> {
        let has_text = !turn.text.trim().is_empty();
        let mut assistant = turn.to_message();
        let mut seen = thread::call_ids(stored);
        // Some servers reuse ids like "call_0" every turn; results are matched by id, so keep them unique.
        if let Some(calls) = assistant.tool_calls.as_mut() {
            for c in calls.iter_mut() {
                if c.id.trim().is_empty() || !seen.insert(c.id.clone()) {
                    c.id = format!("call_{}", uuid::Uuid::now_v7().simple());
                    seen.insert(c.id.clone());
                }
            }
        }
        let calls: Vec<ToolCallMsg> = assistant.tool_calls.clone().unwrap_or_default();
        let mut results: Vec<Stored> = Vec::new();
        let mut ui: Vec<(String, Value)> = Vec::new();
        let mut pending: Vec<(String, String, String)> = Vec::new();
        let mut bad_args = false;
        let mut only_suggestions = !calls.is_empty();
        for (i, call) in calls.iter().enumerate() {
            let kind = self.registry.get(&call.function.name).map(|t| t.kind());
            if kind != Some(Kind::Propose) {
                self.emit_call(em, call, &call.function.arguments, &msg_id)
                    .await?;
            }
            let registry = self.registry.clone();
            let ctx = self.ctx(&call.id);
            let name = call.function.name.clone();
            let raw = call.function.arguments.clone();
            let outcome =
                match tokio::task::spawn_blocking(move || registry.call(&ctx, &name, &raw)).await {
                    Ok(o) => o,
                    Err(e) => Err(ToolError::Failed(format!("the tool crashed: {e}"))),
                };
            let content = match outcome {
                Ok(Outcome::Proposal { proposal_id }) => {
                    let args = args_with_proposal(&call.function.arguments, &proposal_id);
                    if let Some(c) = assistant.tool_calls.as_mut().and_then(|c| c.get_mut(i)) {
                        c.function.arguments = args.clone();
                    }
                    pending.push((call.id.clone(), proposal_id, args));
                    continue;
                }
                Ok(Outcome::Text {
                    content,
                    custom,
                    data,
                    learn,
                }) => {
                    let result_id = new_id();
                    self.emit_result(em, &result_id, &call.id, &content).await?;
                    if let Some(l) = learn {
                        self.learn.push(Pending {
                            call_id: call.id.clone(),
                            result_id: result_id.clone(),
                            about: l.about,
                            results: l.results,
                        });
                    }
                    if call.function.name == "remember"
                        && let Some(a) = remembered(&call.function.arguments, &self.profile)
                    {
                        self.learn.retain(|p| !p.about.same(&a));
                    }
                    only_suggestions &= call.function.name == "suggest_next" && data.is_some();
                    if let Some(data) = data {
                        em.custom(
                            "tool_data",
                            json!({"tool_call_id": call.id, "data": redact_value(&data, &self.profile)}),
                        )
                        .await?;
                        ui.push((result_id.clone(), data));
                    }
                    for (name, value) in custom {
                        em.custom(&name, value).await?;
                    }
                    results.push(Stored {
                        id: result_id,
                        msg: thread::tool_msg(&call.id, &content),
                    });
                    continue;
                }
                Err(e) => {
                    bad_args |= matches!(e, ToolError::BadArgs(_));
                    only_suggestions = false;
                    e.message()
                }
            };
            let result_id = new_id();
            if kind != Some(Kind::Propose) {
                self.emit_result(em, &result_id, &call.id, &content).await?;
            }
            results.push(Stored {
                id: result_id,
                msg: thread::tool_msg(&call.id, &content),
            });
        }
        let mut batch = vec![Stored {
            id: msg_id.clone(),
            msg: assistant,
        }];
        batch.extend(results);
        self.store(batch, ui).await?;
        for (call_id, proposal_id, args) in &pending {
            let name = calls
                .iter()
                .find(|c| &c.id == call_id)
                .map(|c| c.function.name.clone())
                .unwrap_or_default();
            let call = ToolCallMsg::new(call_id, &name, args);
            self.emit_call(em, &call, args, &msg_id).await?;
            em.custom(
                "proposal",
                json!({"proposal_id": proposal_id, "tool_call_id": call_id}),
            )
            .await?;
        }
        Ok(RoundEnd {
            proposal_pending: !pending.is_empty(),
            bad_args,
            answered: only_suggestions && has_text,
        })
    }

    // Each `about` gets one small request after the run, so the answer the model gave is never held up.
    async fn learn_from_searches(&mut self, em: &mut Emitter) {
        let Some((cfg, key)) = self.resolved.clone() else {
            return;
        };
        let mut todo: Vec<Pending> = Vec::new();
        for p in std::mem::take(&mut self.learn).into_iter().rev() {
            if !todo.iter().any(|t| t.about.same(&p.about)) {
                todo.push(p);
            }
        }
        for p in todo.into_iter().rev() {
            let item = match self.learn_one(&cfg, key.as_deref(), &p).await {
                Ok(Some(item)) => item,
                Ok(None) => continue,
                Err(e) => {
                    tracing::debug!(error = %e, "learning from a web search failed");
                    continue;
                }
            };
            let data = json!({"v": DATA_VERSION, "kind": "learned", "items": [item]});
            let (db, id, stored) = (self.env.db.clone(), p.result_id.clone(), data.clone());
            if blocking(move || thread::set_ui(&db.conn()?, &id, &stored))
                .await
                .is_err()
            {
                continue;
            }
            let shown =
                json!({"tool_call_id": p.call_id, "data": redact_value(&data, &self.profile)});
            if em.custom("tool_data", shown).await.is_err() {
                return;
            }
        }
    }

    async fn learn_one(
        &self,
        cfg: &ProviderConfig,
        key: Option<&str>,
        p: &Pending,
    ) -> anyhow::Result<Option<Value>> {
        let Some(url) = p.results.first().map(|r| r.url.clone()) else {
            return Ok(None);
        };
        let ask = format!(
            "What is {}?\n{}",
            p.about.subject(&self.profile),
            format_for_llm(&p.results)
        );
        let mut messages = vec![
            ChatMessage::text("system", &learn_prompt()),
            ChatMessage::text("user", &ask),
        ];
        redact_messages(&mut messages, &self.profile);
        let req = ChatRequest {
            messages,
            ..ChatRequest::default()
        };
        let turn = self
            .env
            .client
            .chat(cfg, key, &req)
            .await
            .map_err(|e| anyhow::anyhow!(self.scrub(&describe(&e, cfg))))?;
        let Some(id) = parse_identity(&turn.text) else {
            return Ok(None);
        };
        let (db, about, profile, now) = (
            self.env.db.clone(),
            p.about.clone(),
            self.profile.clone(),
            self.env.now(),
        );
        let link = url.clone();
        let m = tokio::task::spawn_blocking(move || {
            remember_identity(&db.conn()?, &about, id, &link, now, &profile)
        })
        .await??;
        Ok(m.map(|m| json!({"memory_id": m.id, "text": m.text, "url": url})))
    }

    async fn emit_call(
        &self,
        em: &mut Emitter,
        call: &ToolCallMsg,
        args: &str,
        parent: &str,
    ) -> Result<(), Stop> {
        em.send(Event::ToolCallStart {
            tool_call_id: call.id.clone(),
            tool_call_name: call.function.name.clone(),
            parent_message_id: Some(parent.to_string()),
        })
        .await?;
        em.send(Event::ToolCallArgs {
            tool_call_id: call.id.clone(),
            delta: redact_all(args, &self.profile),
        })
        .await?;
        em.send(Event::ToolCallEnd {
            tool_call_id: call.id.clone(),
        })
        .await
    }

    async fn emit_result(
        &self,
        em: &mut Emitter,
        message_id: &str,
        call_id: &str,
        content: &str,
    ) -> Result<(), Stop> {
        em.send(Event::ToolCallResult {
            message_id: message_id.to_string(),
            tool_call_id: call_id.to_string(),
            content: redact_all(content, &self.profile),
            role: "tool".into(),
        })
        .await
    }
}

fn remembered(raw: &str, profile: &str) -> Option<About> {
    let args = crate::tools::parse_args(raw).ok()?;
    About::from_scope(args.get("scope")?.as_str()?, profile)
}

fn args_with_proposal(raw: &str, proposal_id: &str) -> String {
    let mut obj = crate::tools::parse_args(raw)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    obj.insert("proposal_id".into(), Value::String(proposal_id.into()));
    Value::Object(obj).to_string()
}

pub fn redact_messages(msgs: &mut [ChatMessage], profile: &str) {
    for m in msgs {
        if let Some(c) = m.content.as_mut() {
            *c = redact_all(c, profile);
        }
        for tc in m.tool_calls.iter_mut().flatten() {
            tc.function.arguments = redact_all(&tc.function.arguments, profile);
        }
    }
}

pub fn describe(e: &ProviderError, cfg: &ProviderConfig) -> String {
    let base = cfg.base_url.trim();
    match e {
        ProviderError::Http { status, body } if matches!(status, 401 | 403) => format!(
            "The provider at {base} rejected the request (HTTP {status}): {body}. Check the API key in Settings, AI."
        ),
        ProviderError::Http { status, body } => {
            format!("The provider at {base} returned HTTP {status}: {body}")
        }
        ProviderError::Api(m) => format!("The model provider reported an error: {m}"),
        ProviderError::Transport(m) => format!("Could not reach the model server at {base}: {m}"),
        ProviderError::Timeout(m) => format!(
            "The model server at {base} did not respond in time ({m}). Try again, or raise the timeout in Settings, AI."
        ),
        ProviderError::Protocol(m) => {
            format!("The model server sent a response Hoardini could not read: {m}")
        }
        ProviderError::RateLimited(m) => format!(
            "The provider at {base} kept saying to slow down ({m}). Wait a minute and try again, or switch provider in the AI tab."
        ),
        ProviderError::DailyQuota(_) => format!(
            "The free tier at {base} is used up for today. It resets on its own, or you can switch to another provider in the AI tab."
        ),
        ProviderError::Config(m) => format!("The provider settings are invalid: {m}"),
    }
}

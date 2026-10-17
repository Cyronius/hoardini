use std::collections::HashMap;
use std::convert::Infallible;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use hoardini_agent::agui::{Event, RunAgentInput};
use hoardini_agent::run::run_agent;
use hoardini_agent::thread;
use hoardini_core::actions::Job;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{Instant, interval_at, sleep_until};
use tokio_stream::wrappers::ReceiverStream;

use crate::state::{ApiError, ApiResult, AppState, RunSlot, blocking, parse};
use crate::status::{build_status, display_job};
use crate::{builtin, providers};

const STATUS_MIN_GAP: Duration = Duration::from_millis(500);
const JOB_FLUSH: Duration = Duration::from_millis(200);

fn sse_headers(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    r
}

fn frame(v: &Value) -> Bytes {
    Bytes::from(format!("data: {v}\n\n"))
}

fn status_frame(mut v: Value) -> Bytes {
    if let Some(o) = v.as_object_mut() {
        o.insert("type".into(), json!("status"));
    }
    frame(&v)
}

fn job_frame(job: &Job) -> Bytes {
    let mut v = serde_json::to_value(display_job(job.clone())).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.insert("type".into(), json!("job"));
    }
    frame(&v)
}

pub async fn status_stream(State(s): State<AppState>) -> Response {
    let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(16);
    tokio::spawn(status_pump(s, tx));
    sse_headers(Body::from_stream(ReceiverStream::new(rx)).into_response())
}

async fn status_pump(s: AppState, tx: mpsc::Sender<Result<Bytes, Infallible>>) {
    let _watcher = s.supervisor.watch();
    let mut changes = s.change.subscribe();
    let mut jobs = s.exec.subscribe();
    let mut shutdown = s.shutdown.subscribe();
    let start = Instant::now();
    let mut ping = interval_at(start + Duration::from_secs(15), Duration::from_secs(15));
    let mut poll = interval_at(start + Duration::from_secs(5), Duration::from_secs(5));
    let mut last_json = String::new();
    let mut last_sent = Instant::now() - STATUS_MIN_GAP;
    let mut dirty = true;
    let mut pending: HashMap<String, Job> = HashMap::new();
    let mut flush_at: Option<Instant> = None;

    loop {
        tokio::select! {
            _ = tx.closed() => break,
            _ = shutdown.changed() => break,
            _ = ping.tick() => {
                if tx.send(Ok(Bytes::from_static(b": ping\n\n"))).await.is_err() { break; }
            }
            _ = poll.tick() => dirty = true,
            r = changes.changed() => {
                if r.is_err() { break; }
                dirty = true;
            }
            j = jobs.recv() => match j {
                Ok(job) => {
                    pending.insert(job.id.clone(), job);
                    flush_at.get_or_insert(Instant::now() + JOB_FLUSH);
                    dirty = true;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => dirty = true,
                Err(broadcast::error::RecvError::Closed) => {
                    jobs = s.exec.subscribe();
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            },
            _ = sleep_until(flush_at.unwrap_or_else(Instant::now)), if flush_at.is_some() => {
                flush_at = None;
                for (_, job) in pending.drain() {
                    if tx.send(Ok(job_frame(&job))).await.is_err() { return; }
                }
            }
            _ = sleep_until(last_sent + STATUS_MIN_GAP), if dirty => {
                dirty = false;
                let snap = build_status(&s).await;
                let text = snap.to_string();
                if text != last_json {
                    last_json = text;
                    last_sent = Instant::now();
                    if tx.send(Ok(status_frame(snap))).await.is_err() { break; }
                }
            }
        }
    }
}

struct RunGuard {
    state: AppState,
    run_id: String,
    abort: tokio::task::AbortHandle,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.abort.abort();
        if let Ok(mut runs) = self.state.runs.lock() {
            runs.remove(&self.run_id);
        }
        self.state.bump();
    }
}

pub async fn agent_run(State(s): State<AppState>, body: Bytes) -> ApiResult<Response> {
    let input: RunAgentInput = parse(&body)?;
    if input.run_id.is_empty() {
        return Err(ApiError::bad("runId is required"));
    }
    // Every question is about one item on Home; there is no app-wide conversation (AGENT-ASK).
    if !thread::is_item_thread(&input.thread_id) {
        return Err(ApiError::bad("threadId must be an item thread"));
    }
    let provider = {
        let st = s.clone();
        blocking(move || Ok(providers::active_for_run(&st)?)).await?
    };
    let (tx, rx) = mpsc::channel::<Event>(64);
    let thread_id = input.thread_id.clone();
    let run_id = input.run_id.clone();

    let Some(provider) = provider else {
        let _ = tx.send(Event::RunStarted { thread_id, run_id }).await;
        let _ = tx
            .send(Event::RunError {
                message: "No model provider is configured. Add one in Settings.".into(),
                code: None,
            })
            .await;
        drop(tx);
        return Ok(event_response(ReceiverStream::new(rx), None));
    };

    let env = s.env.clone();
    let st = s.clone();
    let tx_run = tx.clone();
    let (tid, rid) = (thread_id, run_id.clone());
    let inner = tokio::spawn(async move {
        // The lease keeps the built-in child alive until this run ends.
        let (provider, _lease) = match builtin::resolve(&st, provider).await {
            Ok(x) => x,
            Err(message) => {
                let _ = tx_run
                    .send(Event::RunStarted {
                        thread_id: tid,
                        run_id: rid,
                    })
                    .await;
                let _ = tx_run
                    .send(Event::RunError {
                        message,
                        code: None,
                    })
                    .await;
                return Ok(());
            }
        };
        run_agent(env, provider, input, tx_run).await
    });
    let abort = inner.abort_handle();
    {
        let mut runs = s
            .runs
            .lock()
            .map_err(|_| ApiError::from(anyhow::anyhow!("run table poisoned")))?;
        runs.insert(
            run_id.clone(),
            RunSlot {
                abort: abort.clone(),
            },
        );
    }
    s.bump();
    tokio::spawn(async move {
        let end = match inner.await {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(Event::RunError {
                message: format!("{e:#}"),
                code: None,
            }),
            Err(e) if e.is_panic() => {
                tracing::error!("agent run panicked");
                Some(Event::RunError {
                    message: "internal error in the agent loop".into(),
                    code: None,
                })
            }
            // Aborted: the client closed the stream, so nobody is left to tell.
            Err(_) => None,
        };
        if let Some(ev) = end {
            let _ = tx.send(ev).await;
        }
    });

    let guard = RunGuard {
        state: s.clone(),
        run_id,
        abort,
    };
    Ok(event_response(ReceiverStream::new(rx), Some(guard)))
}

// A run that fails can stop mid-message; the terminal event must not find anything still open.
#[derive(Default)]
struct OpenParts {
    text: Option<String>,
    thinking_text: bool,
    thinking: bool,
    calls: Vec<String>,
}

impl OpenParts {
    fn closers_before(&mut self, ev: &Event) -> Vec<Event> {
        match ev {
            Event::TextMessageStart { message_id, .. } => self.text = Some(message_id.clone()),
            Event::TextMessageEnd { .. } => self.text = None,
            Event::ThinkingTextMessageStart {} => self.thinking_text = true,
            Event::ThinkingTextMessageEnd {} => self.thinking_text = false,
            Event::ThinkingStart {} => self.thinking = true,
            Event::ThinkingEnd {} => self.thinking = false,
            Event::ToolCallStart { tool_call_id, .. } => self.calls.push(tool_call_id.clone()),
            Event::ToolCallEnd { tool_call_id } => self.calls.retain(|c| c != tool_call_id),
            Event::RunFinished { .. } | Event::RunError { .. } => {
                let mut out = Vec::new();
                if let Some(message_id) = self.text.take() {
                    out.push(Event::TextMessageEnd { message_id });
                }
                if std::mem::take(&mut self.thinking_text) {
                    out.push(Event::ThinkingTextMessageEnd {});
                }
                if std::mem::take(&mut self.thinking) {
                    out.push(Event::ThinkingEnd {});
                }
                for tool_call_id in self.calls.drain(..) {
                    out.push(Event::ToolCallEnd { tool_call_id });
                }
                return out;
            }
            _ => {}
        }
        Vec::new()
    }
}

fn event_response(rx: ReceiverStream<Event>, guard: Option<RunGuard>) -> Response {
    let mut open = OpenParts::default();
    let stream = rx.flat_map(move |ev| {
        let _keep = &guard;
        let mut frames: Vec<Result<Bytes, Infallible>> = open
            .closers_before(&ev)
            .iter()
            .map(|e| Ok(Bytes::from(e.to_sse())))
            .collect();
        frames.push(Ok(Bytes::from(ev.to_sse())));
        futures::stream::iter(frames)
    });
    sse_headers(Body::from_stream(stream).into_response())
}

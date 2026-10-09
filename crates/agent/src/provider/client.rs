use bytes::Bytes;
use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::{self, BoxStream};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use super::accumulate::{TurnAccumulator, assemble_sse};
use super::config::ProviderConfig;
use super::error::ProviderError;
use super::sanitize::build_body;
use super::sse::SseParser;
use super::types::{AssembledTurn, ChatRequest, StreamEvent};

// Sees every raw stream payload of a chat request; the eval harness times tokens with it.
pub trait Meter: Send + Sync {
    fn request(&self);
    fn data(&self, data: &str);
    fn end(&self);
}

pub type Sleeper = Arc<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>;
pub type WaitNotice = Arc<dyn Fn(u64) + Send + Sync>;

pub const MAX_ATTEMPTS: u32 = 3;
pub const MAX_WAIT: Duration = Duration::from_secs(60);

#[derive(Clone, Default)]
pub struct Client {
    meter: Option<Arc<dyn Meter>>,
    sleeper: Option<Sleeper>,
    on_wait: Option<WaitNotice>,
}

fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let v = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let at = httpdate::parse_http_date(v).ok()?;
    Some(at.duration_since(std::time::SystemTime::now()).unwrap_or_default())
}

pub(super) fn map_reqwest(e: reqwest::Error) -> ProviderError {
    let mut msg = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(s) = src {
        msg.push_str(": ");
        msg.push_str(&s.to_string());
        src = s.source();
    }
    if e.is_timeout() {
        ProviderError::Timeout(msg)
    } else {
        ProviderError::Transport(msg)
    }
}

pub(super) fn http_client(
    cfg: &ProviderConfig,
    total: bool,
) -> Result<reqwest::Client, ProviderError> {
    let t = Duration::from_secs(cfg.timeout_s.max(1));
    let mut b = reqwest::Client::builder().connect_timeout(t);
    if total {
        b = b.timeout(t);
    }
    b.build().map_err(|e| ProviderError::Config(e.to_string()))
}

pub(super) fn decorate(
    mut rb: reqwest::RequestBuilder,
    cfg: &ProviderConfig,
    key: Option<&str>,
) -> reqwest::RequestBuilder {
    for (k, v) in &cfg.headers {
        rb = rb.header(k.as_str(), v.as_str());
    }
    match key.filter(|k| !k.is_empty()) {
        Some(k) => rb.bearer_auth(k),
        None => rb,
    }
}

pub(super) async fn ensure_ok(resp: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    Err(ProviderError::http(status, &body))
}

fn assemble_body(text: &str) -> Result<AssembledTurn, ProviderError> {
    if text.trim_start().starts_with("data:") || text.trim_start().starts_with(':') {
        return assemble_sse(text.as_bytes(), text.len().max(1));
    }
    let v: Value = serde_json::from_str(text.trim()).map_err(|_| {
        ProviderError::Protocol(format!(
            "response is not JSON: {}",
            super::error::excerpt(text)
        ))
    })?;
    let mut acc = TurnAccumulator::new();
    acc.push_value(&v)?;
    if acc.chunks_seen() == 0 {
        return Err(ProviderError::Protocol("empty response".into()));
    }
    Ok(acc.finish())
}

struct St {
    body: BoxStream<'static, reqwest::Result<Bytes>>,
    parser: SseParser,
    acc: Option<TurnAccumulator>,
    queue: VecDeque<Result<StreamEvent, ProviderError>>,
    idle: Duration,
    finished: bool,
    meter: Option<Arc<dyn Meter>>,
}

impl St {
    fn feed(&mut self, events: Vec<super::sse::SseEvent>) {
        for e in events {
            let Some(acc) = self.acc.as_mut() else { return };
            if let Some(m) = &self.meter {
                m.data(&e.data);
            }
            match acc.push_data(&e.data) {
                Ok(evs) => self.queue.extend(evs.into_iter().map(Ok)),
                Err(err) => {
                    self.queue.push_back(Err(err));
                    self.finished = true;
                    return;
                }
            }
            if acc.is_done() {
                self.complete();
                return;
            }
        }
    }

    fn complete(&mut self) {
        if let Some(m) = self.meter.take() {
            m.end();
        }
        if let Some(acc) = self.acc.take() {
            self.queue.push_back(if acc.chunks_seen() == 0 {
                Err(ProviderError::Protocol(
                    "stream ended without any data".into(),
                ))
            } else {
                Ok(StreamEvent::Done(acc.finish()))
            });
        }
        self.finished = true;
    }
}

impl Client {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_meter(meter: Arc<dyn Meter>) -> Self {
        Self {
            meter: Some(meter),
            ..Self::default()
        }
    }

    pub fn with_sleeper(mut self, sleeper: Sleeper) -> Self {
        self.sleeper = Some(sleeper);
        self
    }

    pub fn with_wait_notice(mut self, notice: WaitNotice) -> Self {
        self.on_wait = Some(notice);
        self
    }

    async fn sleep(&self, d: Duration) {
        match &self.sleeper {
            Some(f) => f(d).await,
            None => tokio::time::sleep(d).await,
        }
    }

    // 429s wait (Retry-After, else exponential) and retry; a daily-quota 429 ends at once.
    async fn send_retrying(
        &self,
        rb: reqwest::RequestBuilder,
        headers_within: Option<Duration>,
    ) -> Result<reqwest::Response, ProviderError> {
        for attempt in 1..=MAX_ATTEMPTS {
            let req = rb
                .try_clone()
                .ok_or_else(|| ProviderError::Config("request body cannot be retried".into()))?;
            let resp = match headers_within {
                Some(t) => tokio::time::timeout(t, req.send())
                    .await
                    .map_err(|_| ProviderError::Timeout("no response headers".into()))?,
                None => req.send().await,
            }
            .map_err(map_reqwest)?;
            if resp.status().as_u16() != 429 {
                return ensure_ok(resp).await;
            }
            let wait = retry_after(&resp);
            let body = resp.text().await.unwrap_or_default();
            if super::error::is_daily_quota(&body) {
                return Err(ProviderError::DailyQuota(super::error::excerpt(&body)));
            }
            if attempt == MAX_ATTEMPTS {
                return Err(ProviderError::RateLimited(super::error::excerpt(&body)));
            }
            let wait = wait
                .unwrap_or_else(|| Duration::from_secs(2u64 << (attempt - 1)))
                .min(MAX_WAIT);
            if let Some(n) = &self.on_wait {
                n(wait.as_secs().max(1));
            }
            self.sleep(wait).await;
        }
        Err(ProviderError::RateLimited(String::new()))
    }

    pub async fn chat_stream(
        &self,
        cfg: &ProviderConfig,
        key: Option<&str>,
        req: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent, ProviderError>>, ProviderError> {
        let http = http_client(cfg, false)?;
        let rb = decorate(http.post(cfg.endpoint("chat/completions")), cfg, key)
            .json(&build_body(cfg, req, true));
        let idle = Duration::from_secs(cfg.timeout_s.max(1));
        if let Some(m) = &self.meter {
            m.request();
        }
        let resp = self.send_retrying(rb, Some(idle)).await?;

        let is_sse = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("event-stream"));
        if !is_sse {
            let text = resp.text().await.map_err(map_reqwest)?;
            let items: Vec<Result<StreamEvent, ProviderError>> =
                if text.trim_start().starts_with("data:") {
                    let mut st = St {
                        body: stream::empty().boxed(),
                        parser: SseParser::new(),
                        acc: Some(TurnAccumulator::new()),
                        queue: VecDeque::new(),
                        idle,
                        finished: false,
                        meter: self.meter.clone(),
                    };
                    let evs = st.parser.push(text.as_bytes());
                    st.feed(evs);
                    let evs = st.parser.finish();
                    st.feed(evs);
                    st.complete();
                    st.queue.into_iter().collect()
                } else {
                    if let Some(m) = &self.meter {
                        m.data(&text);
                        m.end();
                    }
                    let mut acc = TurnAccumulator::new();
                    match serde_json::from_str::<Value>(text.trim()) {
                        Err(_) => vec![Err(ProviderError::Protocol(format!(
                            "response is not JSON: {}",
                            super::error::excerpt(&text)
                        )))],
                        Ok(v) => match acc.push_value(&v) {
                            Err(e) => vec![Err(e)],
                            Ok(evs) => {
                                let mut items: Vec<_> = evs.into_iter().map(Ok).collect();
                                items.push(if acc.chunks_seen() == 0 {
                                    Err(ProviderError::Protocol("empty response".into()))
                                } else {
                                    Ok(StreamEvent::Done(acc.finish()))
                                });
                                items
                            }
                        },
                    }
                };
            return Ok(stream::iter(items).boxed());
        }

        let st = St {
            body: resp.bytes_stream().boxed(),
            parser: SseParser::new(),
            acc: Some(TurnAccumulator::new()),
            queue: VecDeque::new(),
            idle,
            finished: false,
            meter: self.meter.clone(),
        };
        Ok(stream::unfold(st, |mut st| async move {
            loop {
                if let Some(item) = st.queue.pop_front() {
                    return Some((item, st));
                }
                if st.finished {
                    return None;
                }
                match tokio::time::timeout(st.idle, st.body.next()).await {
                    Err(_) => {
                        st.queue.push_back(Err(ProviderError::Timeout(format!(
                            "no data for {}s",
                            st.idle.as_secs()
                        ))));
                        st.finished = true;
                    }
                    Ok(None) => {
                        let evs = st.parser.finish();
                        st.feed(evs);
                        st.complete();
                    }
                    Ok(Some(Err(e))) => {
                        st.queue.push_back(Err(map_reqwest(e)));
                        st.finished = true;
                    }
                    Ok(Some(Ok(b))) => {
                        let evs = st.parser.push(&b);
                        st.feed(evs);
                    }
                }
            }
        })
        .boxed())
    }

    pub async fn chat(
        &self,
        cfg: &ProviderConfig,
        key: Option<&str>,
        req: &ChatRequest,
    ) -> Result<AssembledTurn, ProviderError> {
        let http = http_client(cfg, true)?;
        let rb = decorate(http.post(cfg.endpoint("chat/completions")), cfg, key)
            .json(&build_body(cfg, req, false));
        let resp = self.send_retrying(rb, None).await?;
        let text = resp.text().await.map_err(map_reqwest)?;
        assemble_body(&text)
    }

    pub async fn list_models(
        &self,
        cfg: &ProviderConfig,
        key: Option<&str>,
    ) -> Result<Vec<String>, ProviderError> {
        let http = http_client(cfg, true)?;
        let resp = decorate(http.get(cfg.endpoint("models")), cfg, key)
            .send()
            .await
            .map_err(map_reqwest)?;
        if matches!(resp.status().as_u16(), 404 | 405 | 501) {
            return Ok(vec![]);
        }
        let text = ensure_ok(resp).await?.text().await.map_err(map_reqwest)?;
        let v: Value = serde_json::from_str(text.trim()).map_err(|_| {
            ProviderError::Protocol(format!(
                "/models is not JSON: {}",
                super::error::excerpt(&text)
            ))
        })?;
        if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
            return Err(ProviderError::Api(super::accumulate::error_message(e)));
        }
        Ok(parse_model_ids(&v))
    }
}

pub fn parse_model_ids(v: &Value) -> Vec<String> {
    let list = v
        .get("data")
        .or_else(|| v.get("models"))
        .and_then(Value::as_array)
        .or_else(|| v.as_array());
    list.into_iter()
        .flatten()
        .filter_map(|m| match m {
            Value::String(s) => Some(s.clone()),
            _ => ["id", "name", "model"]
                .iter()
                .find_map(|k| m.get(*k).and_then(Value::as_str))
                .map(str::to_string),
        })
        .collect()
}

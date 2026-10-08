use std::sync::Mutex;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::Meter;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RequestStat {
    pub ttft_ms: Option<f64>,
    pub total_ms: f64,
    pub completion_tokens: Option<u64>,
    pub decode_ms: Option<f64>,
    pub prompt_tokens: Option<u64>,
    pub prompt_ms: Option<f64>,
    pub chunks: u32,
    #[serde(default)]
    pub server_timed: bool,
}

struct Open {
    t0: Instant,
    first: Option<Instant>,
    chunks: u32,
    usage: (Option<u64>, Option<u64>),
    timings: Option<Value>,
}

#[derive(Default)]
pub struct EvalMeter {
    state: Mutex<(Option<Open>, Vec<RequestStat>)>,
}

fn has_output(delta: &Value) -> bool {
    let nonempty = |k: &str| match delta.get(k) {
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        _ => false,
    };
    nonempty("content")
        || nonempty("reasoning_content")
        || nonempty("reasoning")
        || nonempty("tool_calls")
}

fn finish(o: Open) -> RequestStat {
    let end = Instant::now();
    let ms = |a: Instant, b: Instant| b.duration_since(a).as_secs_f64() * 1000.0;
    let t = o.timings.as_ref();
    let num = |k: &str| t.and_then(|t| t.get(k)).and_then(Value::as_f64);
    // llama-server reports its own decode timings; elsewhere fall back to wall time from the first token.
    let (completion_tokens, decode_ms) = match (num("predicted_n"), num("predicted_ms")) {
        (Some(n), Some(d)) => (Some(n as u64), Some(d)),
        _ => (
            o.usage.1.or((o.chunks > 0).then_some(u64::from(o.chunks))),
            o.first.map(|f| ms(f, end)),
        ),
    };
    RequestStat {
        ttft_ms: o.first.map(|f| ms(o.t0, f)),
        total_ms: ms(o.t0, end),
        completion_tokens,
        decode_ms,
        prompt_tokens: num("prompt_n").map(|n| n as u64).or(o.usage.0),
        prompt_ms: num("prompt_ms"),
        chunks: o.chunks,
        server_timed: num("predicted_ms").is_some(),
    }
}

impl EvalMeter {
    pub fn take(&self) -> Vec<RequestStat> {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(o) = g.0.take() {
            let s = finish(o);
            g.1.push(s);
        }
        std::mem::take(&mut g.1)
    }
}

impl Meter for EvalMeter {
    fn request(&self) {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(o) = g.0.take() {
            let s = finish(o);
            g.1.push(s);
        }
        g.0 = Some(Open {
            t0: Instant::now(),
            first: None,
            chunks: 0,
            usage: (None, None),
            timings: None,
        });
    }

    fn data(&self, data: &str) {
        let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
            return;
        };
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(o) = g.0.as_mut() else { return };
        let out = v
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|c| {
                c.get("delta")
                    .or_else(|| c.get("message"))
                    .is_some_and(has_output)
            });
        if out {
            o.chunks += 1;
            o.first.get_or_insert_with(Instant::now);
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            o.usage = (
                u.get("prompt_tokens").and_then(Value::as_u64),
                u.get("completion_tokens").and_then(Value::as_u64),
            );
        }
        if let Some(t) = v.get("timings").filter(|t| t.is_object()) {
            o.timings = Some(t.clone());
        }
    }

    fn end(&self) {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(o) = g.0.take() {
            let s = finish(o);
            g.1.push(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_server_timings() {
        let m = EvalMeter::default();
        m.request();
        m.data(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#);
        m.data(r#"{"choices":[{"delta":{"content":"Hi"}}]}"#);
        m.data(r#"{"choices":[{"delta":{"tool_calls":[{"index":0}]}}]}"#);
        m.data(r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"timings":{"prompt_n":900,"prompt_ms":1500.0,"predicted_n":40,"predicted_ms":2000.0}}"#);
        m.end();
        let s = m.take();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].chunks, 2);
        assert!(s[0].ttft_ms.is_some());
        assert_eq!(s[0].completion_tokens, Some(40));
        assert_eq!(s[0].decode_ms, Some(2000.0));
        assert_eq!(
            (s[0].prompt_tokens, s[0].prompt_ms),
            (Some(900), Some(1500.0))
        );
        assert!(s[0].server_timed);
    }

    #[test]
    fn falls_back_to_chunks_and_closes_unfinished_requests() {
        let m = EvalMeter::default();
        m.request();
        m.data(r#"{"choices":[{"delta":{"content":"a"}}]}"#);
        m.data(r#"{"choices":[{"delta":{"content":"b"}}]}"#);
        m.request();
        let s = m.take();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].completion_tokens, Some(2));
        assert_eq!(s[1].ttft_ms, None);
    }
}

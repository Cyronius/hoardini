use futures::StreamExt;
use serde_json::json;
use std::collections::HashSet;
use std::time::Instant;

use super::client::Client;
use super::config::ProviderConfig;
use super::error::ProviderError;
use super::types::{Caps, ChatMessage, ChatRequest, StreamEvent, TestReport, ToolDef};

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

fn tokens(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn score(target: &str, cand: &str) -> f64 {
    let (t, c) = (target.to_lowercase(), cand.to_lowercase());
    if t == c {
        return 1000.0;
    }
    let mut s = 0.0;
    if t.len() >= 3 && (c.contains(&t) || t.contains(&c)) {
        s += 500.0;
    }
    let (tt, ct) = (tokens(&t), tokens(&c));
    let inter = tt.intersection(&ct).count() as f64;
    let union = tt.union(&ct).count().max(1) as f64;
    s += 300.0 * inter / union;
    let (tc, cc): (Vec<char>, Vec<char>) = (t.chars().collect(), c.chars().collect());
    let sim = 1.0 - levenshtein(&tc, &cc) as f64 / tc.len().max(cc.len()).max(1) as f64;
    if sim >= 0.5 {
        s += 200.0 * sim;
    }
    s
}

pub fn suggest_models(target: &str, available: &[String]) -> Vec<String> {
    let mut scored: Vec<(f64, &String)> = available
        .iter()
        .map(|a| (score(target, a), a))
        .filter(|(s, _)| *s > 0.0)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().take(3).map(|(_, a)| a.clone()).collect()
}

fn ping_tool() -> ToolDef {
    ToolDef {
        name: "ping".into(),
        description: "Send a short message to the user.".into(),
        parameters: json!({"type": "object",
            "properties": {"message": {"type": "string"}}, "required": ["message"]}),
    }
}

impl Client {
    pub async fn test_connection(&self, cfg: &ProviderConfig, key: Option<&str>) -> TestReport {
        let mut rep = TestReport::default();
        let t0 = Instant::now();
        let listed = self.list_models(cfg, key).await;
        rep.latency_ms = Some(t0.elapsed().as_millis() as u64);
        match listed {
            Ok(m) => rep.models = m,
            Err(e) => {
                rep.error = Some(e.to_string());
                return rep;
            }
        }
        let model = cfg
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty());
        if let Some(m) = model
            && !rep.models.is_empty()
        {
            let found = rep.models.iter().any(|x| x == m);
            rep.model_found = Some(found);
            if !found {
                rep.suggestions = suggest_models(m, &rep.models);
                let hint = if rep.suggestions.is_empty() {
                    String::new()
                } else {
                    format!(" Did you mean: {}?", rep.suggestions.join(", "))
                };
                rep.error = Some(format!(
                    "Model '{m}' is not in the server's model list; servers like LM Studio answer unknown ids from a different model.{hint}"
                ));
                return rep;
            }
        }
        match self.probe_caps(cfg, key).await {
            Ok(c) => {
                rep.caps = Some(c);
                rep.ok = true;
            }
            Err(e) => rep.error = Some(e.to_string()),
        }
        rep
    }

    async fn probe_caps(
        &self,
        cfg: &ProviderConfig,
        key: Option<&str>,
    ) -> Result<Caps, ProviderError> {
        let mut cfg = cfg.clone();
        cfg.params.max_tokens = Some(cfg.params.max_tokens.unwrap_or(0).max(512));
        cfg.params.temperature = Some(0.0);
        let mut req = ChatRequest {
            messages: vec![ChatMessage::text(
                "user",
                "Call the ping tool with the message \"hi\".",
            )],
            tools: vec![ping_tool()],
            tool_choice: Some(json!("auto")),
            include_usage: false,
        };
        let mut caps = Caps::default();

        let streamed = async {
            let mut s = self.chat_stream(&cfg, key, &req).await?;
            let (mut started, mut turn) = (false, None);
            while let Some(ev) = s.next().await {
                match ev? {
                    StreamEvent::ToolCallStart { .. } => started = true,
                    StreamEvent::Done(t) => turn = Some(t),
                    _ => {}
                }
            }
            Ok::<_, ProviderError>((started, turn))
        }
        .await;
        match streamed {
            Ok((started, Some(turn))) => {
                caps.streaming = true;
                caps.streamed_tool_calls = started;
                caps.tools = !turn.tool_calls.is_empty();
            }
            Ok((_, None)) | Err(_) => {
                let turn = self.chat(&cfg, key, &req).await?;
                caps.tools = !turn.tool_calls.is_empty();
            }
        }

        if caps.tools {
            req.messages = vec![ChatMessage::text("user", "Say hello. Do not use any tool.")];
            req.tool_choice = Some(json!({"type": "function", "function": {"name": "ping"}}));
            caps.tool_choice = self
                .chat(&cfg, key, &req)
                .await
                .map(|t| !t.tool_calls.is_empty())
                .unwrap_or(false);
        }
        Ok(caps)
    }
}

use serde_json::Value;

use super::error::{ProviderError, excerpt};
use super::sse::SseParser;
use super::types::{AssembledToolCall, AssembledTurn, StreamEvent, Usage};

struct Slot {
    index: Option<u64>,
    id: String,
    id_generated: bool,
    name: String,
    args: String,
    started: bool,
    pending: String,
}

#[derive(Default)]
pub struct TurnAccumulator {
    text: String,
    reasoning: String,
    slots: Vec<Slot>,
    finish_reason: Option<String>,
    usage: Option<Usage>,
    done: bool,
    chunks: usize,
}

pub fn error_message(e: &Value) -> String {
    match e {
        Value::String(s) => s.clone(),
        Value::Object(o) => {
            let msg = o.get("message").and_then(Value::as_str);
            let kind = o.get("type").or_else(|| o.get("code")).and_then(|v| {
                v.as_str()
                    .map(str::to_string)
                    .or_else(|| v.as_i64().map(|n| n.to_string()))
            });
            match (msg, kind) {
                (Some(m), Some(k)) => format!("{m} ({k})"),
                (Some(m), None) => m.to_string(),
                _ => excerpt(&e.to_string()),
            }
        }
        _ => excerpt(&e.to_string()),
    }
}

fn text_of(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let s: String = parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect();
            Some(s)
        }
        _ => None,
    }
}

fn parse_usage(v: &Value) -> Option<Usage> {
    let o = v.as_object()?;
    let n = |keys: &[&str]| keys.iter().find_map(|k| o.get(*k).and_then(Value::as_u64));
    let u = Usage {
        prompt_tokens: n(&["prompt_tokens", "input_tokens"]),
        completion_tokens: n(&["completion_tokens", "output_tokens"]),
        total_tokens: n(&["total_tokens"]),
    };
    (u != Usage::default()).then_some(u)
}

impl TurnAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    pub fn chunks_seen(&self) -> usize {
        self.chunks
    }

    pub fn push_data(&mut self, data: &str) -> Result<Vec<StreamEvent>, ProviderError> {
        let d = data.trim();
        if d == "[DONE]" {
            self.done = true;
            return Ok(vec![]);
        }
        let v: Value = serde_json::from_str(d).map_err(|_| {
            ProviderError::Protocol(format!("non-JSON stream data: {}", excerpt(d)))
        })?;
        self.push_value(&v)
    }

    pub fn push_value(&mut self, v: &Value) -> Result<Vec<StreamEvent>, ProviderError> {
        if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
            return Err(ProviderError::Api(error_message(e)));
        }
        self.chunks += 1;
        let mut ev = Vec::new();
        if let Some(u) = v.get("usage").and_then(parse_usage) {
            self.usage = Some(u.clone());
            ev.push(StreamEvent::Usage(u));
        }
        let choices = v.get("choices").and_then(Value::as_array);
        for choice in choices.into_iter().flatten() {
            if choice
                .get("index")
                .and_then(Value::as_u64)
                .is_some_and(|i| i != 0)
            {
                continue;
            }
            let (delta, whole) = match (choice.get("delta"), choice.get("message")) {
                (Some(d), _) if d.is_object() => (Some(d), false),
                (_, Some(m)) if m.is_object() => (Some(m), true),
                _ => (None, false),
            };
            if let Some(d) = delta {
                self.apply_delta(d, whole, &mut ev);
            }
            if let Some(fr) = choice
                .get("finish_reason")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                self.finish_reason = Some(fr.to_string());
                ev.push(StreamEvent::Finish(fr.to_string()));
            }
        }
        Ok(ev)
    }

    fn apply_delta(&mut self, d: &Value, whole: bool, ev: &mut Vec<StreamEvent>) {
        if let Some(r) = text_of(d.get("reasoning_content"))
            .filter(|s| !s.is_empty())
            .or_else(|| text_of(d.get("reasoning")).filter(|s| !s.is_empty()))
        {
            self.reasoning.push_str(&r);
            ev.push(StreamEvent::Reasoning(r));
        }
        if let Some(t) = text_of(d.get("content")).filter(|s| !s.is_empty()) {
            self.text.push_str(&t);
            ev.push(StreamEvent::Text(t));
        }
        if let Some(calls) = d.get("tool_calls").and_then(Value::as_array) {
            for (pos, tc) in calls.iter().enumerate() {
                self.apply_tool_call(tc, pos, whole, ev);
            }
        }
    }

    fn find_slot(
        &self,
        index: Option<u64>,
        id: Option<&str>,
        name: &str,
        pos: usize,
    ) -> Option<usize> {
        if let Some(i) = index {
            let p = self.slots.iter().position(|s| s.index == Some(i))?;
            let s = &self.slots[p];
            let conflict = id.is_some_and(|id| !s.id_generated && s.id != id);
            return (!conflict).then_some(p);
        }
        if let Some(id) = id {
            return self.slots.iter().position(|s| s.id == id);
        }
        let last = self.slots.len().checked_sub(1)?;
        let s = &self.slots[last];
        let looks_new = pos > 0 || (!name.is_empty() && !s.name.is_empty() && !s.args.is_empty());
        (!looks_new).then_some(last)
    }

    fn apply_tool_call(&mut self, tc: &Value, pos: usize, whole: bool, ev: &mut Vec<StreamEvent>) {
        let mut index = tc.get("index").and_then(Value::as_u64);
        if index.is_none() && whole {
            index = Some(pos as u64);
        }
        let id = tc
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let f = tc.get("function");
        let name = f
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let args = match f.and_then(|f| f.get("arguments")) {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
        };

        let si = match self.find_slot(index, id, name, pos) {
            Some(i) => i,
            None => {
                self.slots.push(Slot {
                    index,
                    id: id.map_or_else(
                        || format!("call_{}", uuid::Uuid::now_v7().simple()),
                        str::to_string,
                    ),
                    id_generated: id.is_none(),
                    name: String::new(),
                    args: String::new(),
                    started: false,
                    pending: String::new(),
                });
                self.slots.len() - 1
            }
        };
        let s = &mut self.slots[si];
        if let Some(id) = id
            && s.id_generated
        {
            s.id = id.to_string();
            s.id_generated = false;
        }
        if s.name.is_empty() {
            s.name = name.to_string();
        } else if !name.is_empty() && s.name != name {
            s.name.push_str(name);
        }
        if !args.is_empty() {
            s.args.push_str(&args);
            if s.started {
                ev.push(StreamEvent::ToolCallArgs {
                    index: si,
                    delta: args,
                });
            } else {
                s.pending.push_str(&args);
            }
        }
        if !s.started && !s.name.is_empty() {
            s.started = true;
            ev.push(StreamEvent::ToolCallStart {
                index: si,
                id: s.id.clone(),
                name: s.name.clone(),
            });
            if !s.pending.is_empty() {
                ev.push(StreamEvent::ToolCallArgs {
                    index: si,
                    delta: std::mem::take(&mut s.pending),
                });
            }
        }
    }

    pub fn finish(self) -> AssembledTurn {
        AssembledTurn {
            text: self.text,
            reasoning: self.reasoning,
            tool_calls: self
                .slots
                .into_iter()
                .filter(|s| !s.name.is_empty())
                .map(|s| AssembledToolCall {
                    id: s.id,
                    name: s.name,
                    arguments: if s.args.trim().is_empty() {
                        "{}".into()
                    } else {
                        s.args
                    },
                })
                .collect(),
            finish_reason: self.finish_reason,
            usage: self.usage,
        }
    }
}

// Whole-body replay for tests and the recorder: SSE bytes in, assembled turn out.
pub fn assemble_sse(bytes: &[u8], chunk_size: usize) -> Result<AssembledTurn, ProviderError> {
    let mut parser = SseParser::new();
    let mut acc = TurnAccumulator::new();
    let mut events = Vec::new();
    for c in bytes.chunks(chunk_size.max(1)) {
        events.extend(parser.push(c));
    }
    events.extend(parser.finish());
    for e in events {
        acc.push_data(&e.data)?;
        if acc.is_done() {
            break;
        }
    }
    if acc.chunks_seen() == 0 {
        return Err(ProviderError::Protocol("stream carried no data".into()));
    }
    Ok(acc.finish())
}

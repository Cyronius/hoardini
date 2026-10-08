use serde_json::{Value, json};

use super::config::ProviderConfig;
use super::types::{ChatRequest, ToolDef};

pub fn sanitize_body(body: &mut Value) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    if obj
        .get("model")
        .is_some_and(|m| m.as_str().is_none_or(|s| s.trim().is_empty()))
    {
        obj.remove("model");
    }
    let Some(msgs) = obj.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for m in msgs {
        if let Some(mo) = m.as_object_mut()
            && mo.get("role").and_then(Value::as_str) == Some("assistant")
        {
            mo.remove("reasoning_content");
            mo.remove("reasoning");
        }
    }
}

pub fn deep_merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(slot) => deep_merge(slot, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (slot, v) => *slot = v.clone(),
    }
}

fn tool_json(t: &ToolDef) -> Value {
    json!({"type": "function", "function": {
        "name": t.name, "description": t.description, "parameters": t.parameters}})
}

pub fn build_body(cfg: &ProviderConfig, req: &ChatRequest, stream: bool) -> Value {
    let mut body = json!({
        "model": cfg.model.clone().unwrap_or_default(),
        "messages": req.messages,
        "stream": stream,
    });
    if let Some(o) = body.as_object_mut() {
        if let Some(t) = cfg.params.temperature {
            o.insert("temperature".into(), json!(t));
        }
        if let Some(t) = cfg.params.max_tokens {
            o.insert("max_tokens".into(), json!(t));
        }
        if let Some(t) = cfg.params.top_p {
            o.insert("top_p".into(), json!(t));
        }
        if !req.tools.is_empty() {
            o.insert(
                "tools".into(),
                Value::Array(req.tools.iter().map(tool_json).collect()),
            );
            if let Some(tc) = &req.tool_choice {
                o.insert("tool_choice".into(), tc.clone());
            }
        }
        if stream && req.include_usage {
            o.insert("stream_options".into(), json!({"include_usage": true}));
        }
    }
    sanitize_body(&mut body);
    deep_merge(&mut body, &Value::Object(cfg.extra_body.clone()));
    body
}

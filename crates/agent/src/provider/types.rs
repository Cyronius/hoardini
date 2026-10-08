use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallFn {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallMsg {
    pub id: String,
    #[serde(rename = "type", default = "fn_type")]
    pub kind: String,
    pub function: ToolCallFn,
}

fn fn_type() -> String {
    "function".into()
}

impl ToolCallMsg {
    pub fn new(id: &str, name: &str, arguments: &str) -> Self {
        Self {
            id: id.into(),
            kind: fn_type(),
            function: ToolCallFn {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallMsg>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

impl ChatMessage {
    pub fn text(role: &str, content: &str) -> Self {
        Self {
            role: role.into(),
            content: Some(content.into()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDef>,
    pub tool_choice: Option<Value>,
    // Only set for servers known to accept stream_options.
    pub include_usage: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Text(String),
    Reasoning(String),
    ToolCallStart {
        index: usize,
        id: String,
        name: String,
    },
    ToolCallArgs {
        index: usize,
        delta: String,
    },
    Usage(Usage),
    Finish(String),
    Done(AssembledTurn),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssembledToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AssembledTurn {
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<AssembledToolCall>,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
}

impl AssembledTurn {
    pub fn to_message(&self) -> ChatMessage {
        ChatMessage {
            role: "assistant".into(),
            content: (!self.text.is_empty() || self.tool_calls.is_empty())
                .then(|| self.text.clone()),
            tool_calls: (!self.tool_calls.is_empty()).then(|| {
                self.tool_calls
                    .iter()
                    .map(|c| ToolCallMsg::new(&c.id, &c.name, &c.arguments))
                    .collect()
            }),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caps {
    pub tools: bool,
    pub streaming: bool,
    pub streamed_tool_calls: bool,
    pub tool_choice: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TestReport {
    pub ok: bool,
    pub error: Option<String>,
    pub models: Vec<String>,
    pub model_found: Option<bool>,
    pub suggestions: Vec<String>,
    pub caps: Option<Caps>,
    pub latency_ms: Option<u64>,
}

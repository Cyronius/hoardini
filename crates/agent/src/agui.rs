// AG-UI wire types, @ag-ui/core 0.0.47 schema. SEAM (wave 2): the agent-loop owner may extend these.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunAgentInput {
    pub thread_id: String,
    pub run_id: String,
    #[serde(default)]
    pub messages: Vec<AgMessage>,
    #[serde(default)]
    pub tools: Vec<Value>,
    #[serde(default)]
    pub context: Vec<Value>,
    #[serde(default)]
    pub state: Value,
    #[serde(default)]
    pub forwarded_props: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgToolCall {
    pub id: String,
    #[serde(rename = "type", default = "function_type")]
    pub kind: String,
    pub function: AgFunctionCall,
}

fn function_type() -> String {
    "function".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgMessage {
    pub id: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<AgToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    // A tool message's card payload (AGENT-TOOL-DATA); the server sends it, never reads it back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Event {
    #[serde(rename_all = "camelCase")]
    RunStarted { thread_id: String, run_id: String },
    #[serde(rename_all = "camelCase")]
    RunFinished { thread_id: String, run_id: String },
    #[serde(rename_all = "camelCase")]
    RunError {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    TextMessageStart { message_id: String, role: String },
    #[serde(rename_all = "camelCase")]
    TextMessageContent { message_id: String, delta: String },
    #[serde(rename_all = "camelCase")]
    TextMessageEnd { message_id: String },
    #[serde(rename_all = "camelCase")]
    ToolCallStart {
        tool_call_id: String,
        tool_call_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_message_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallArgs { tool_call_id: String, delta: String },
    #[serde(rename_all = "camelCase")]
    ToolCallEnd { tool_call_id: String },
    #[serde(rename_all = "camelCase")]
    ToolCallResult {
        message_id: String,
        tool_call_id: String,
        content: String,
        role: String,
    },
    #[serde(rename_all = "camelCase")]
    ThinkingStart {},
    #[serde(rename_all = "camelCase")]
    ThinkingEnd {},
    #[serde(rename_all = "camelCase")]
    ThinkingTextMessageStart {},
    #[serde(rename_all = "camelCase")]
    ThinkingTextMessageContent { delta: String },
    #[serde(rename_all = "camelCase")]
    ThinkingTextMessageEnd {},
    #[serde(rename_all = "camelCase")]
    Custom { name: String, value: Value },
}

impl Event {
    pub fn to_sse(&self) -> String {
        format!(
            "data: {}\n\n",
            serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
        )
    }
}

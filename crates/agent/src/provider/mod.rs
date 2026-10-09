pub mod accumulate;
pub mod client;
pub mod config;
pub mod error;
pub mod models;
pub mod sanitize;
pub mod sse;
pub mod types;

pub use accumulate::{TurnAccumulator, assemble_sse};
pub use client::{Client, Meter, Sleeper, WaitNotice};
pub use config::{Params, Preset, ProviderConfig, ProviderView, presets};
pub use error::{ProviderError, is_context_overflow, is_daily_quota};
pub use futures::StreamExt;
pub use models::suggest_models;
pub use sanitize::{build_body, deep_merge};
pub use sse::{SseEvent, SseParser};
pub use types::{
    AssembledToolCall, AssembledTurn, Caps, ChatMessage, ChatRequest, StreamEvent, TestReport,
    ToolCallMsg, ToolDef, Usage,
};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("provider reported an error: {0}")]
    Api(String),
    #[error("could not reach the provider: {0}")]
    Transport(String),
    #[error("provider timed out: {0}")]
    Timeout(String),
    #[error("unexpected provider response: {0}")]
    Protocol(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("daily quota used up: {0}")]
    DailyQuota(String),
    #[error("invalid provider configuration: {0}")]
    Config(String),
}

pub fn excerpt(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= 400 {
        return t.to_string();
    }
    let cut: String = t.chars().take(400).collect();
    format!("{cut}...")
}

impl ProviderError {
    pub fn http(status: u16, body: &str) -> Self {
        Self::Http {
            status,
            body: excerpt(body),
        }
    }
}

const OVERFLOW_MARKERS: &[&str] = &[
    "context length",
    "context_length",
    "context window",
    "maximum context",
    "context size",
    "exceeds the available context",
    "exceed_context_size",
    "n_ctx",
    "prompt is too long",
    "input is too long",
    "too many tokens",
    "reduce the length",
    "token limit",
    "tokens exceed",
];

pub fn is_context_overflow(err: &ProviderError) -> bool {
    let text = match err {
        ProviderError::Http { status, body } => {
            if !matches!(*status, 400 | 413 | 422 | 500) {
                return false;
            }
            body.as_str()
        }
        ProviderError::Api(m) => m.as_str(),
        _ => return false,
    };
    let lower = text.to_lowercase();
    OVERFLOW_MARKERS.iter().any(|m| lower.contains(m))
}

const DAILY_MARKERS: &[&str] = &["per day", "per-day", "perday", "per_day", "daily", "a day"];

pub fn is_daily_quota(body: &str) -> bool {
    let lower = body.to_lowercase();
    DAILY_MARKERS.iter().any(|m| lower.contains(m))
}

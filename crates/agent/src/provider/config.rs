use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use super::types::Caps;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Params {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
}

fn d_ctx() -> u32 {
    32768
}
fn d_timeout() -> u64 {
    120
}
fn d_steps() -> u32 {
    20
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub model: Option<String>,
    // A `secret://` reference, never a key.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "d_ctx")]
    pub context_window: u32,
    #[serde(default = "d_timeout")]
    pub timeout_s: u64,
    #[serde(default = "d_steps")]
    pub max_tool_steps: u32,
    #[serde(default)]
    pub small: bool,
    #[serde(default)]
    pub params: Params,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub extra_body: Map<String, Value>,
}

impl ProviderConfig {
    pub fn new(name: &str, base_url: &str) -> Self {
        Self {
            id: None,
            name: name.into(),
            base_url: base_url.into(),
            model: None,
            api_key: None,
            context_window: d_ctx(),
            timeout_s: d_timeout(),
            max_tool_steps: d_steps(),
            small: false,
            params: Params::default(),
            headers: BTreeMap::new(),
            extra_body: Map::new(),
        }
    }

    // Forms send 0 for a blank field; taken literally that is a 1 s timeout and one tool step.
    pub fn fill_defaults(&mut self) {
        if self.context_window == 0 {
            self.context_window = d_ctx();
        }
        if self.timeout_s == 0 {
            self.timeout_s = d_timeout();
        }
        if self.max_tool_steps == 0 {
            self.max_tool_steps = d_steps();
        }
    }

    pub fn endpoint(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim().trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderView {
    #[serde(flatten)]
    pub config: ProviderConfig,
    pub id: String,
    pub is_active: bool,
    pub has_key: bool,
    pub caps: Option<Caps>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub needs_key: bool,
    pub default_model: Option<&'static str>,
    pub key_url: Option<&'static str>,
}

pub fn presets() -> Vec<Preset> {
    let p = |id, name, base_url, needs_key| Preset {
        id,
        name,
        base_url,
        needs_key,
        default_model: None,
        key_url: None,
    };
    let free = |mut x: Preset, model, key_url| {
        x.default_model = Some(model);
        x.key_url = Some(key_url);
        x
    };
    vec![
        free(
            p("cerebras", "Cerebras", "https://api.cerebras.ai/v1", true),
            "gpt-oss-120b",
            "https://cloud.cerebras.ai/platform/",
        ),
        free(
            p(
                "gemini",
                "Google Gemini",
                "https://generativelanguage.googleapis.com/v1beta/openai/",
                true,
            ),
            "gemini-3.5-flash-lite",
            "https://aistudio.google.com/apikey",
        ),
        p("openai", "OpenAI", "https://api.openai.com/v1", true),
        free(
            p(
                "openrouter",
                "OpenRouter",
                "https://openrouter.ai/api/v1",
                true,
            ),
            "google/gemma-4-31b-it:free",
            "https://openrouter.ai/settings/keys",
        ),
        free(
            p("mistral", "Mistral", "https://api.mistral.ai/v1", true),
            "mistral-small-2603",
            "https://console.mistral.ai/api-keys",
        ),
        free(
            p("zai", "Z.ai", "https://api.z.ai/api/paas/v4", true),
            "glm-4.7-flash",
            "https://z.ai/manage-apikey/apikey-list",
        ),
        free(
            p(
                "ollama-cloud",
                "Ollama Cloud",
                "https://ollama.com/v1",
                true,
            ),
            "gemma4:31b",
            "https://ollama.com/settings/keys",
        ),
        // The account ID goes into the URL; the setup form asks for it (UI-FIRST-RUN).
        free(
            p(
                "cloudflare",
                "Cloudflare Workers AI",
                "https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1",
                true,
            ),
            "@cf/zai-org/glm-4.7-flash",
            "https://dash.cloudflare.com/profile/api-tokens",
        ),
        p(
            "anthropic",
            "Anthropic (OpenAI-compatible)",
            "https://api.anthropic.com/v1/",
            true,
        ),
        p("groq", "Groq", "https://api.groq.com/openai/v1", true),
        p("together", "Together", "https://api.together.xyz/v1", true),
        p("ollama", "Ollama", "http://127.0.0.1:11434/v1", false),
        p("lmstudio", "LM Studio", "http://127.0.0.1:1234/v1", false),
        p("llamacpp", "llama.cpp", "http://127.0.0.1:8080/v1", false),
        p("vllm", "vLLM", "http://127.0.0.1:8000/v1", false),
    ]
}

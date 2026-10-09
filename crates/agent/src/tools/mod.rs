use std::sync::Arc;

use hoardini_core::store::Db;
use hoardini_core::vault::SecretStore;
use serde_json::Value;

use crate::provider::ToolDef;
use crate::run::IndexControl;

pub mod util;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Read,
    Propose,
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> Value;
    fn kind(&self) -> Kind {
        Kind::Read
    }
    // Part of the reduced toolset for profiles flagged `small` (design 5.12).
    fn small(&self) -> bool {
        false
    }
    // Checked once per run; a tool whose setup is missing (e.g. web search off) is not offered.
    fn available(&self, _conn: &rusqlite::Connection) -> bool {
        true
    }
    fn run(&self, ctx: &ToolCtx, args: Value) -> Result<Outcome, ToolError>;
}

#[derive(Clone)]
pub struct ToolCtx {
    pub db: Db,
    pub index: Arc<dyn IndexControl>,
    pub secrets: Arc<dyn SecretStore>,
    pub profile: String,
    pub conversation_id: Option<String>,
    pub tool_call_id: String,
}

// Card payloads carry this version so the UI can tell an old stored shape from a new one.
pub const DATA_VERSION: u64 = 1;

#[derive(Debug)]
pub enum Outcome {
    Text {
        content: String,
        custom: Vec<(String, Value)>,
        // The UI's card payload; stored in `message.ui_json` and never sent to the model.
        data: Option<Value>,
    },
    Proposal {
        proposal_id: String,
    },
}

impl Outcome {
    pub fn text(content: impl Into<String>) -> Outcome {
        Outcome::Text {
            content: util::cap(content.into()),
            custom: Vec::new(),
            data: None,
        }
    }

    pub fn with_data(self, data: Value) -> Outcome {
        match self {
            Outcome::Text {
                content, custom, ..
            } => Outcome::Text {
                content,
                custom,
                data: Some(data),
            },
            other => other,
        }
    }
}

#[derive(Debug)]
pub enum ToolError {
    // The model's call did not parse or validate; it gets one retry.
    BadArgs(String),
    Failed(String),
}

impl ToolError {
    pub fn message(&self) -> String {
        match self {
            ToolError::BadArgs(m) => {
                format!("Error: invalid arguments: {m}. Fix the arguments and call the tool again.")
            }
            ToolError::Failed(m) => format!("Error: {m}"),
        }
    }
}

// Adding a tool: write tools/<name>.rs with a unit struct implementing Tool, then list it here.
macro_rules! registry {
    ($($m:ident => [$($t:ident),+]),* $(,)?) => {
        $(mod $m;)*
        fn all() -> Vec<Arc<dyn Tool>> {
            vec![$($(Arc::new($m::$t) as Arc<dyn Tool>,)+)*]
        }
    };
}

registry! {
    index_status => [IndexStatus],
    refresh => [Refresh],
    top_files => [TopFiles],
    top_dirs => [TopDirs],
    changes => [Changes],
    inspect => [Inspect],
    recall => [Recall],
    remember => [Remember],
    destinations => [Destinations],
    duplicates => [Duplicates],
    estimate_compression => [EstimateCompression],
    web_search => [WebSearch],
    suggest_next => [SuggestNext],
    propose => [ProposeDelete, ProposeArchive, ProposeCompress, ProposeDedupe],
}

pub use index_status::refresh_note;

#[derive(Clone)]
pub struct Registry {
    tools: Vec<Arc<dyn Tool>>,
}

impl Registry {
    pub fn full() -> Registry {
        Registry { tools: all() }
    }

    pub fn small() -> Registry {
        Registry {
            tools: all().into_iter().filter(|t| t.small()).collect(),
        }
    }

    pub fn for_profile(small: bool) -> Registry {
        if small {
            Registry::small()
        } else {
            Registry::full()
        }
    }

    pub fn available(self, db: &Db) -> Registry {
        let Ok(conn) = db.conn() else {
            return self;
        };
        Registry {
            tools: self
                .tools
                .into_iter()
                .filter(|t| t.available(&conn))
                .collect(),
        }
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.tools.iter().map(|t| t.name()).collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name).cloned()
    }

    pub fn defs(&self) -> Vec<ToolDef> {
        self.tools
            .iter()
            .map(|t| ToolDef {
                name: t.name().into(),
                description: t.description().into(),
                parameters: t.parameters(),
            })
            .collect()
    }

    // Blocking: tools hit SQLite and the filesystem metadata; async callers use spawn_blocking.
    pub fn call(&self, ctx: &ToolCtx, name: &str, raw_args: &str) -> Result<Outcome, ToolError> {
        let tool = self.get(name).ok_or_else(|| {
            ToolError::BadArgs(format!(
                "there is no tool named {name:?}; available: {}",
                self.names().join(", ")
            ))
        })?;
        tool.run(ctx, parse_args(raw_args)?)
    }
}

pub fn parse_args(raw: &str) -> Result<Value, ToolError> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    let v: Value = serde_json::from_str(t)
        .map_err(|e| ToolError::BadArgs(format!("arguments are not valid JSON ({e})")))?;
    match v {
        Value::Object(_) => Ok(v),
        Value::Null => Ok(Value::Object(Default::default())),
        // Some models double-encode the arguments object as a JSON string.
        Value::String(s) => match serde_json::from_str::<Value>(&s) {
            Ok(inner @ Value::Object(_)) => Ok(inner),
            _ => Err(ToolError::BadArgs("arguments must be a JSON object".into())),
        },
        _ => Err(ToolError::BadArgs("arguments must be a JSON object".into())),
    }
}

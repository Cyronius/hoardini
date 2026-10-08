// cargo run -p hoardini-agent --example smoke -- <folder> [base_url] [model] [question] [--small]
use std::io::Write;
use std::sync::Arc;

use hoardini_agent::agui::{AgMessage, Event, RunAgentInput};
use hoardini_agent::provider::{Client, ProviderConfig};
use hoardini_agent::run::{AgentEnv, IndexControl, new_id, run_agent};
use hoardini_core::index::{Mode, RefreshOptions, WinLister, refresh};
use hoardini_core::store::Db;
use hoardini_core::vault::SecretStore;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::sync::mpsc;

struct NoSecrets;

impl SecretStore for NoSecrets {
    fn get(&self, _r: &str) -> anyhow::Result<Option<SecretString>> {
        Ok(None)
    }
    fn set(&self, _r: &str, _v: SecretString) -> anyhow::Result<()> {
        anyhow::bail!("read-only")
    }
    fn delete(&self, _r: &str) -> anyhow::Result<()> {
        Ok(())
    }
    fn refs(&self) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

struct Idle;

impl IndexControl for Idle {
    fn status(&self) -> Value {
        json!({"state": "idle"})
    }
    fn refresh(&self, _mode: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let small = std::env::args().any(|a| a == "--small");
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a != "--small")
        .collect();
    let Some(folder) = args.first() else {
        anyhow::bail!("usage: smoke <folder> [base_url] [model] [question] [--small]");
    };
    let base = args
        .get(1)
        .map_or("http://localhost:1234/v1", String::as_str);
    let model = args.get(2).map_or("minicpm5-2b", String::as_str);
    let question = args.get(3).map_or(
        "What are the biggest files in the indexed folder, and can any of them go?",
        String::as_str,
    );
    let dir = std::env::temp_dir().join(format!("hoardini-smoke-{}", new_id()));
    let db = Db::open(dir.join("smoke.db"))?;
    let opts = RefreshOptions {
        mode: Mode::Fast,
        ..RefreshOptions::default()
    };
    let out = refresh(&db.conn()?, &WinLister, folder, &opts)?;
    eprintln!("indexed {folder}: {out:?}");
    let env = Arc::new(AgentEnv::new(
        db,
        Client::new(),
        Arc::new(NoSecrets),
        Arc::new(Idle),
    ));
    let mut p = ProviderConfig::new("lmstudio", base);
    p.model = Some(model.into());
    p.small = small;
    p.timeout_s = 300;
    p.params.temperature = Some(0.2);
    let input = RunAgentInput {
        thread_id: new_id(),
        run_id: new_id(),
        messages: vec![AgMessage {
            id: new_id(),
            role: "user".into(),
            content: Some(question.into()),
            tool_calls: None,
            tool_call_id: None,
        }],
        ..Default::default()
    };
    let (tx, mut rx) = mpsc::channel(1024);
    let task = tokio::spawn(run_agent(env, p, input, tx));
    let mut stdout = std::io::stdout();
    while let Some(ev) = rx.recv().await {
        match &ev {
            Event::TextMessageContent { delta, .. } => {
                let _ = write!(stdout, "{delta}");
                let _ = stdout.flush();
            }
            Event::ThinkingTextMessageContent { .. } => {}
            Event::TextMessageEnd { .. } => println!(),
            other => println!("{}", serde_json::to_string(other)?),
        }
    }
    task.await??;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

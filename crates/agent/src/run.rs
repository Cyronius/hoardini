// SEAM (wave 2): signatures are the contract between the agent loop and the server.
use std::sync::Arc;

use hoardini_core::store::Db;
use hoardini_core::vault::SecretStore;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::agui::{Event, RunAgentInput};
use crate::provider::{Client, ProviderConfig};

// Implemented by the server's indexer supervisor; the agent only asks.
pub trait IndexControl: Send + Sync {
    fn status(&self) -> Value;
    fn refresh(&self, mode: &str) -> anyhow::Result<bool>;
}

pub struct AgentEnv {
    pub db: Db,
    pub client: Client,
    pub secrets: Arc<dyn SecretStore>,
    pub index: Arc<dyn IndexControl>,
}

// Streams AG-UI events into `tx`; the server aborts the task to cancel a run.
pub async fn run_agent(
    _env: Arc<AgentEnv>,
    _provider: ProviderConfig,
    input: RunAgentInput,
    tx: mpsc::Sender<Event>,
) -> anyhow::Result<()> {
    let _ = tx
        .send(Event::RunError {
            message: "agent loop not implemented yet".into(),
            code: None,
        })
        .await;
    let _ = input;
    Ok(())
}

// SEAM (wave 2): types are the contract; bodies are filled by the policy/actions owner.
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::store::Db;
use crate::vault::SecretStore;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct JobProgress {
    pub done_bytes: i64,
    pub total_bytes: i64,
    pub done_items: i64,
    pub total_items: i64,
    pub current: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub proposal_id: Option<String>,
    pub status: String,
    pub progress: JobProgress,
    pub freed_bytes: i64,
    pub error: Option<String>,
    pub created_ms: i64,
    pub finished_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApproveResult {
    pub approved: bool,
    pub items: Vec<i64>,
    pub job_id: String,
}

// Jobs run on std threads (file I/O is blocking); listeners get every Job update.
pub struct Executor {
    pub db: Db,
    pub secrets: Arc<dyn SecretStore>,
    pub events: tokio::sync::broadcast::Sender<Job>,
}

impl Executor {
    pub fn new(db: Db, secrets: Arc<dyn SecretStore>) -> Executor {
        let (events, _) = tokio::sync::broadcast::channel(256);
        Executor {
            db,
            secrets,
            events,
        }
    }

    // The only entry point that changes the filesystem: revalidates, then starts a job.
    pub fn approve(&self, _proposal_id: &str, _item_ids: &[i64]) -> anyhow::Result<ApproveResult> {
        anyhow::bail!("Executor::approve not implemented yet")
    }

    pub fn jobs(&self) -> Vec<Job> {
        Vec::new()
    }

    pub fn cancel(&self, _job_id: &str) -> bool {
        false
    }

    pub fn restore(&self, _archive_entry_id: i64) -> anyhow::Result<String> {
        anyhow::bail!("Executor::restore not implemented yet")
    }

    pub fn vanished_total(&self) -> i64 {
        0
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Job> {
        self.events.subscribe()
    }
}

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;

use crate::destinations::rclone::Endpoint;
use crate::policy::PolicyEnv;
use crate::proposals::{Proposal, ProposalKind, StoredItem};
use crate::vault::SecretStore;

use super::{archive, compress, dedupe, delete};

pub type CopyHook = Arc<dyn Fn(&Path) + Send + Sync>;

#[derive(Debug, thiserror::Error)]
#[error("cancelled")]
pub(crate) struct Cancelled;

pub(crate) struct JobCx<'a> {
    pub conn: &'a Connection,
    pub proposal: &'a Proposal,
    pub items: &'a [StoredItem],
    pub secrets: &'a dyn SecretStore,
    pub env: &'a PolicyEnv,
    pub hook: Option<CopyHook>,
    pub rc: Option<Endpoint>,
}

pub(crate) struct ItemCx<'a> {
    pub conn: &'a Connection,
    pub item: &'a StoredItem,
    pub action_id: i64,
    pub cancel: &'a AtomicBool,
    pub progress: &'a mut dyn FnMut(i64),
    // Set even on failure: a half-finished folder delete still freed something.
    pub freed: i64,
    pub rescan: bool,
}

impl ItemCx<'_> {
    pub fn check_cancel(&self) -> anyhow::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        Ok(())
    }
}

pub(crate) struct Outcome {
    pub undo: serde_json::Value,
    pub detail: Option<String>,
}

pub(crate) trait Handler: Send + Sync {
    fn weight(&self, item: &StoredItem) -> i64 {
        item.alloc
    }
    fn begin(&self, job: &JobCx<'_>) -> anyhow::Result<Box<dyn Runner>>;
}

pub(crate) trait Runner {
    fn run(&mut self, cx: &mut ItemCx<'_>) -> anyhow::Result<Outcome>;
}

pub(crate) fn handler_for(kind: ProposalKind) -> Option<Box<dyn Handler>> {
    match kind {
        ProposalKind::Delete => Some(Box::new(delete::DeleteHandler)),
        ProposalKind::Archive => Some(Box::new(archive::ArchiveHandler)),
        ProposalKind::Compress => Some(Box::new(compress::CompressHandler)),
        ProposalKind::Dedupe => Some(Box::new(dedupe::DedupeHandler)),
    }
}

pub fn supports(kind: ProposalKind) -> bool {
    handler_for(kind).is_some()
}

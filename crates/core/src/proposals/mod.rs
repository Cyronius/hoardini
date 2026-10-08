// SEAM (wave 2): types are the contract; bodies are filled by the policy/actions owner.
use serde::{Deserialize, Serialize};

use crate::policy::Verdict;
use crate::store::Db;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProposalKind {
    Delete,
    Archive,
    Compress,
    Dedupe,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DedupeParams {
    pub group_id: i64,
    pub keep: String,
    pub mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewItem {
    pub path: String,
    pub verdict: Option<Verdict>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewProposal {
    pub conversation_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub kind: ProposalKind,
    pub destination_id: Option<String>,
    pub format: Option<String>,
    pub dedupe: Option<DedupeParams>,
    pub summary: String,
    pub items: Vec<NewItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DestinationRef {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProposalItem {
    pub id: i64,
    pub path: String,
    pub kind: String,
    pub alloc: i64,
    pub verdict: Option<Verdict>,
    pub cap: Option<Verdict>,
    pub reason: Option<String>,
    pub block_reason: Option<String>,
    pub selected: bool,
    pub status: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub kind: ProposalKind,
    pub status: String,
    pub total_alloc: i64,
    pub created_ms: i64,
    pub job_id: Option<String>,
    pub destination: Option<DestinationRef>,
    pub format: Option<String>,
    pub summary: String,
    pub conversation_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub items: Vec<ProposalItem>,
}

// Runs the policy gate (deny list, protect memories, in-use, verdict caps) and stores the proposal.
pub fn create(_db: &Db, _req: NewProposal) -> anyhow::Result<Proposal> {
    anyhow::bail!("proposals::create not implemented yet")
}

pub fn get(_db: &Db, _id: &str) -> anyhow::Result<Option<Proposal>> {
    anyhow::bail!("proposals::get not implemented yet")
}

pub fn decline(_db: &Db, _id: &str) -> anyhow::Result<Proposal> {
    anyhow::bail!("proposals::decline not implemented yet")
}

pub fn list_for_conversation(_db: &Db, _conversation_id: &str) -> anyhow::Result<Vec<Proposal>> {
    anyhow::bail!("proposals::list_for_conversation not implemented yet")
}

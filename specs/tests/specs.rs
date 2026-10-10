#![allow(clippy::duplicate_mod)] // policy/support.rs is shared by three test roots
#[path = "../act/tests/act.rs"]
mod act;
#[path = "../agent/tests/agent.rs"]
mod agent;
#[path = "../agent/tests/approval.rs"]
mod agent_approval;
#[path = "../evid/tests/evid.rs"]
mod evid;
#[path = "../hist/tests/hist.rs"]
mod hist;
#[path = "../index/tests/index.rs"]
mod index;
#[path = "../mem/tests/mem.rs"]
mod mem;
#[path = "../policy/tests/policy.rs"]
mod policy;
#[path = "../prov/tests/prov.rs"]
mod prov;
#[path = "../review/tests/review.rs"]
mod review;
#[path = "../runtime/tests/runtime.rs"]
mod runtime;
#[path = "../srch/tests/srch.rs"]
mod srch;
#[path = "../srv/tests/srv.rs"]
mod srv;
#[path = "../vault/tests/vault.rs"]
mod vault;

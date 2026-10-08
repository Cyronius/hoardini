// SEAM (wave 2): filled by the evidence owner.
use std::path::Path;

use super::Evidence;
use crate::store::Db;

pub fn inspect(_db: &Db, _path: &Path) -> anyhow::Result<Evidence> {
    anyhow::bail!("evidence::inspect not implemented yet")
}

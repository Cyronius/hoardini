mod deny;
mod env;
mod gate;

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub use deny::{deny_reason, is_volume_root};
pub use env::PolicyEnv;
pub use gate::{
    Action, Examined, FsState, IN_USE_FILE_CAP, ItemKind, examine, fs_state, in_use_reason,
    resolve, revalidate,
};

// Ordered least to most protective; a cap is the minimum protection a verdict may show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Safe,
    Redownload,
    // In sync with a cloud copy: the local copy can go online-only, never be deleted for it.
    Synced,
    Review,
    Keep,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Safe => "safe",
            Verdict::Redownload => "redownload",
            Verdict::Synced => "synced",
            Verdict::Review => "review",
            Verdict::Keep => "keep",
        }
    }

    pub fn parse(s: &str) -> Option<Verdict> {
        match s {
            "safe" => Some(Verdict::Safe),
            "redownload" => Some(Verdict::Redownload),
            "synced" => Some(Verdict::Synced),
            "review" => Some(Verdict::Review),
            "keep" => Some(Verdict::Keep),
            _ => None,
        }
    }

    pub fn capped(self, cap: Option<Verdict>) -> Verdict {
        match cap {
            Some(c) if c > self => c,
            _ => self,
        }
    }
}

// Evidence shows this next to the facts; proposals store it per item.
pub fn cap_for(
    conn: &Connection,
    path: &Path,
    catalog_cap: Option<Verdict>,
    in_use: bool,
) -> anyhow::Result<Option<Verdict>> {
    cap_for_with(conn, &PolicyEnv::system(), path, catalog_cap, in_use)
}

pub fn cap_for_with(
    conn: &Connection,
    env: &PolicyEnv,
    path: &Path,
    catalog_cap: Option<Verdict>,
    in_use: bool,
) -> anyhow::Result<Option<Verdict>> {
    cap_for_action(conn, env, path, catalog_cap, in_use, false)
}

// `online_only`: an in-sync cloud copy counts; for every other action only non-sync copies do.
pub(crate) fn cap_for_action(
    conn: &Connection,
    env: &PolicyEnv,
    path: &Path,
    catalog_cap: Option<Verdict>,
    in_use: bool,
    online_only: bool,
) -> anyhow::Result<Option<Verdict>> {
    let p = crate::index::normalize_path(&path.to_string_lossy());
    if in_use || deny_reason(conn, env, &p)?.is_some() {
        return Ok(Some(Verdict::Keep));
    }
    let found = env.catalog().lookup_match(&p);
    let mut cap = catalog_cap.max(found.as_ref().and_then(|m| m.cap));
    let review = match &found {
        None => true,
        // One blob can back several models; only the whole model folder is a safe unit.
        Some(m) => m.shared_blobs && !std::fs::symlink_metadata(&p).is_ok_and(|md| md.is_dir()),
    };
    if review {
        // A verified copy is what an unknown location lacks; a shared blob stays `review` whatever its copy.
        let lifted =
            found.is_none() && crate::copycheck::identical_for(conn, &p, online_only).is_some();
        cap = cap.max(Some(match (lifted, online_only) {
            (true, true) => Verdict::Synced,
            (true, false) => Verdict::Redownload,
            (false, _) => Verdict::Review,
        }));
    }
    Ok(cap)
}

mod deny;
mod env;
mod gate;

use std::path::Path;

use rusqlite::Connection;

use crate::copycheck::CopyResult;
use serde::{Deserialize, Serialize};

pub use deny::{Area, area_of, deny_reason, is_volume_root};
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
    Ok(cap_and_copy(conn, env, path, catalog_cap, in_use, false)?.0)
}

// `online_only`: an in-sync cloud copy counts; for every other action only non-sync copies do.
pub(crate) fn cap_and_copy(
    conn: &Connection,
    env: &PolicyEnv,
    path: &Path,
    catalog_cap: Option<Verdict>,
    in_use: bool,
    online_only: bool,
) -> anyhow::Result<(Option<Verdict>, Option<CopyResult>)> {
    let p = crate::index::normalize_path(&path.to_string_lossy());
    if in_use || deny_reason(conn, env, &p)?.is_some() {
        return Ok((Some(Verdict::Keep), None));
    }
    // Nothing is deleted on an online-only card, so the catalog's caps about deleting do not apply there.
    if online_only {
        let copy =
            crate::copycheck::identical_for(conn, env.cloud().clone(), &env.profile, &p, true);
        let cap = if copy.is_some() {
            Verdict::Synced
        } else {
            Verdict::Review
        };
        return Ok((Some(cap), copy));
    }
    let found = env.catalog().lookup_match(&p);
    let mut cap = catalog_cap.max(found.as_ref().and_then(|m| m.cap));
    // The user saying a folder can go is the knowledge an unknown location lacks.
    let disposable = || {
        crate::memory::location::user_answer_covering(conn, &env.profile, &p)
            .ok()
            .flatten()
            .and_then(|m| m.detail)
            .is_some_and(|d| d.verdict == Some(Verdict::Safe))
    };
    let review = match &found {
        None => !disposable(),
        // One blob can back several models; only the whole model folder is a safe unit.
        Some(m) => m.shared_blobs && !std::fs::symlink_metadata(&p).is_ok_and(|md| md.is_dir()),
    };
    let mut copy = None;
    if review {
        // A verified copy is what an unknown location lacks; a shared blob stays `review` whatever its copy.
        if found.is_none() {
            copy = crate::copycheck::identical_for(
                conn,
                env.cloud().clone(),
                &env.profile,
                &p,
                online_only,
            );
        }
        cap = cap.max(Some(match (copy.is_some(), online_only) {
            (true, true) => Verdict::Synced,
            (true, false) => Verdict::Redownload,
            (false, _) => Verdict::Review,
        }));
    }
    Ok((cap, copy))
}

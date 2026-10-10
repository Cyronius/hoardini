use std::path::Path;

use serde_json::json;

use super::{Confidence, CopyCheck, CopyResult, Cx, Find, Found, Subject, pending, with_note};
use crate::cloud::{Cloud, CloudState, root_for};

pub struct SyncCheck;

impl SyncCheck {
    pub const NAME: &'static str = "sync";
}

const WALK_CAP: usize = 20_000;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct FolderSync {
    pub files: usize,
    pub not_in_sync: usize,
    pub local_bytes: u64,
    pub truncated: bool,
}

pub(crate) fn file_in_sync(state: Option<CloudState>) -> bool {
    state.is_some_and(|s| s.in_sync)
}

// Every file under the folder (bounded); a file that is not a placeholder counts as not in sync.
pub fn folder_sync(cloud: &dyn Cloud, dir: &Path) -> FolderSync {
    let mut out = FolderSync::default();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = hoardini_win::list_dir(&d) else {
            continue;
        };
        for e in entries {
            let p = d.join(&e.name);
            if e.is_dir() {
                if !e.is_link() {
                    stack.push(p);
                }
                continue;
            }
            if e.is_link() || crate::cloud::never_synced(&e.name) {
                continue;
            }
            out.files += 1;
            if out.files > WALK_CAP {
                out.truncated = true;
                return out;
            }
            let st = cloud.state(&p).ok().flatten();
            if !file_in_sync(st) {
                out.not_in_sync += 1;
            }
            out.local_bytes += st.map_or(e.alloc, |s| s.local_bytes);
        }
    }
    out
}

impl CopyCheck for SyncCheck {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn applies(&self, cx: &Cx<'_>, s: &Subject) -> bool {
        root_for(&cx.env.cloud.roots(), &s.path).is_some()
    }

    fn covers_dir(&self, cx: &Cx<'_>, path: &str) -> bool {
        root_for(&cx.env.cloud.roots(), path).is_some()
    }

    // A file inside can fall out of sync without the folder's own id, size or mtime moving.
    fn still_valid(&self, _cx: &Cx<'_>, s: &Subject, _r: &CopyResult) -> bool {
        !s.dir
    }

    fn find(&self, cx: &Cx<'_>, s: &Subject) -> anyhow::Result<Find> {
        let roots = cx.env.cloud.roots();
        let Some(root) = root_for(&roots, &s.path) else {
            return Ok(Find::Skip);
        };
        let detail = json!({"provider": root.provider, "root": root.path});
        if s.dir {
            let f = folder_sync(cx.env.cloud.as_ref(), Path::new(&s.path));
            let detail = json!({
                "provider": root.provider, "root": root.path, "files": f.files,
                "not_in_sync": f.not_in_sync, "local_bytes": f.local_bytes,
            });
            if f.files > 0 && f.not_in_sync == 0 && !f.truncated {
                return Ok(Find::Copy(Found {
                    locator: root.provider.clone(),
                    detail,
                }));
            }
            let note = format!(
                "{} of {} files not in sync with {} yet",
                f.not_in_sync, f.files, root.provider
            );
            return Ok(Find::NoCopy(pending(with_note(detail, note), "sync")));
        }
        let st = cx.env.cloud.state(Path::new(&s.path))?;
        match st {
            Some(st) if st.in_sync => {
                let mut d = detail;
                d["local_bytes"] = json!(st.local_bytes);
                d["pinned"] = json!(st.pinned);
                Ok(Find::Copy(Found {
                    locator: root.provider.clone(),
                    detail: d,
                }))
            }
            _ => {
                let note = format!("not in sync with {} yet", root.provider);
                Ok(Find::NoCopy(pending(with_note(detail, note), "sync")))
            }
        }
    }

    fn verify(&self, _cx: &Cx<'_>, _s: &Subject, f: Found) -> anyhow::Result<CopyResult> {
        let note = format!("synced to {}", f.locator);
        Ok(CopyResult::new(
            Self::NAME,
            Some(f.locator),
            Confidence::Identical,
            with_note(f.detail, note),
        ))
    }
}

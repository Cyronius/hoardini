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
    pub pinned: usize,
    pub local_bytes: u64,
    pub truncated: bool,
}

// A file or folder the user set to "Always keep on this device".
pub fn is_pinned(detail: &serde_json::Value) -> bool {
    detail["pinned"].as_bool() == Some(true)
        || detail["pinned_files"].as_u64().is_some_and(|n| n > 0)
}

pub(crate) fn file_in_sync(state: Option<CloudState>) -> bool {
    state.is_some_and(|s| s.in_sync)
}

// A file that is no placeholder has no cloud copy yet.
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
            out.pinned += usize::from(st.is_some_and(|s| s.pinned));
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

    // Deeper folders would each walk the same files again on every pass.
    fn covers_dir(&self, cx: &Cx<'_>, path: &str) -> bool {
        let roots = cx.env.cloud.roots();
        root_for(&roots, path).is_some_and(|r| {
            path.get(r.path.len()..)
                .is_some_and(|rest| rest.matches('\\').count() <= 2)
        })
    }

    // Pinning, freeing or falling out of sync moves no id, size or mtime, and the ask is cheap.
    fn still_valid(&self, _cx: &Cx<'_>, _s: &Subject, _r: &CopyResult) -> bool {
        false
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
                "not_in_sync": f.not_in_sync, "pinned_files": f.pinned, "local_bytes": f.local_bytes,
            });
            if f.truncated {
                let note =
                    format!("more than {WALK_CAP} files; each one is checked when it is freed");
                return Ok(Find::NoCopy(with_note(detail, note)));
            }
            if f.files > 0 && f.not_in_sync == 0 {
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
        let note = if is_pinned(&f.detail) {
            format!(
                "synced to {}, kept on this device by your choice",
                f.locator
            )
        } else {
            format!("synced to {}", f.locator)
        };
        Ok(CopyResult::new(
            Self::NAME,
            Some(f.locator),
            Confidence::Identical,
            with_note(f.detail, note),
        ))
    }
}

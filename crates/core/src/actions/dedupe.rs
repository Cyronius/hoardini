use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::delete::remove_file_force;
use super::handler::{Cancelled, Handler, ItemCx, JobCx, Outcome, Runner};
use crate::dupes::full_hash;

pub(crate) struct DedupeHandler;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Hardlink,
    Delete,
}

struct KeepHash {
    len: u64,
    mtime: Option<std::time::SystemTime>,
    hash: String,
}

struct DedupeRunner {
    keep: PathBuf,
    mode: Mode,
    cached: Option<KeepHash>,
}

impl Handler for DedupeHandler {
    fn begin(&self, job: &JobCx<'_>) -> anyhow::Result<Box<dyn Runner>> {
        let params = job
            .proposal
            .dedupe
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("dedupe proposal has no keep/mode parameters"))?;
        let mode = match params.mode.as_str() {
            "hardlink" => Mode::Hardlink,
            "delete" => Mode::Delete,
            other => anyhow::bail!("unknown dedupe mode {other:?}"),
        };
        let keep = PathBuf::from(&params.keep);
        let md = std::fs::symlink_metadata(&keep).map_err(|e| {
            anyhow::anyhow!("the copy to keep ({}) is not readable: {e}", params.keep)
        })?;
        if !md.is_file() || hoardini_win::is_cloud_placeholder(md.file_attributes()) {
            anyhow::bail!(
                "the copy to keep ({}) is not a plain local file",
                params.keep
            );
        }
        if job
            .items
            .iter()
            .any(|i| i.path.eq_ignore_ascii_case(&params.keep))
        {
            anyhow::bail!("the copy to keep is also listed for removal; nothing was changed");
        }
        Ok(Box::new(DedupeRunner {
            keep,
            mode,
            cached: None,
        }))
    }
}

impl DedupeRunner {
    // Re-hashed whenever the keeper's size or mtime moved, so a stale hash never blesses a removal.
    fn keep_hash(&mut self, cx: &ItemCx<'_>) -> anyhow::Result<(u64, String)> {
        let md = std::fs::metadata(&self.keep)?;
        let mtime = md.modified().ok();
        if let Some(k) = &self.cached
            && k.len == md.len()
            && k.mtime == mtime
        {
            return Ok((k.len, k.hash.clone()));
        }
        let (n, hash) = full_hash(&self.keep, cx.cancel).map_err(map_io)?;
        self.cached = Some(KeepHash {
            len: n,
            mtime,
            hash: hash.clone(),
        });
        Ok((n, hash))
    }
}

fn map_io(e: std::io::Error) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::Interrupted {
        Cancelled.into()
    } else {
        e.into()
    }
}

fn sibling(extra: &Path) -> PathBuf {
    let mut s = extra.as_os_str().to_os_string();
    s.push(format!(".hoardini-link-{}", std::process::id()));
    PathBuf::from(s)
}

fn same_volume(a: &Path, b: &Path) -> bool {
    match (
        hoardini_win::volume_root_of(a),
        hoardini_win::volume_root_of(b),
    ) {
        (Ok(x), Ok(y)) => x
            .to_string_lossy()
            .eq_ignore_ascii_case(&y.to_string_lossy()),
        _ => false,
    }
}

// The extra's name is never absent: the link exists under a temp name first, then replaces it in one rename.
fn replace_with_link(keep: &Path, extra: &Path) -> anyhow::Result<()> {
    let tmp = sibling(extra);
    let _ = std::fs::remove_file(&tmp);
    std::fs::hard_link(keep, &tmp).map_err(|e| {
        anyhow::anyhow!(
            "could not create a hard link next to {}: {e}",
            extra.display()
        )
    })?;
    let swap = || -> std::io::Result<()> {
        match std::fs::rename(&tmp, extra) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                let mut perms = std::fs::metadata(extra)?.permissions();
                if !perms.readonly() {
                    return Err(e);
                }
                #[allow(clippy::permissions_set_readonly_false)]
                perms.set_readonly(false);
                std::fs::set_permissions(extra, perms)?;
                std::fs::rename(&tmp, extra)
            }
            r => r,
        }
    };
    if let Err(e) = swap() {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::anyhow!(
            "could not replace {} with the link: {e}",
            extra.display()
        ));
    }
    Ok(())
}

impl Runner for DedupeRunner {
    fn run(&mut self, cx: &mut ItemCx<'_>) -> anyhow::Result<Outcome> {
        cx.check_cancel()?;
        let extra = Path::new(&cx.item.path);
        let id_keep = hoardini_win::file_id_of(&self.keep)?;
        let id_extra = hoardini_win::file_id_of(extra)?;
        if id_keep == id_extra {
            return Ok(Outcome {
                undo: serde_json::json!({"kind": "none", "reason": "already the same file; nothing changed"}),
                detail: Some("Already a hard link to the kept copy".into()),
            });
        }
        if self.mode == Mode::Hardlink && !same_volume(&self.keep, extra) {
            anyhow::bail!(
                "{} is on a different volume than the kept copy; hard links need the same volume",
                extra.display()
            );
        }
        let (klen, khash) = self.keep_hash(cx)?;
        let (elen, ehash) = full_hash(extra, cx.cancel).map_err(map_io)?;
        (cx.progress)(cx.item.alloc);
        if klen != elen || khash != ehash {
            anyhow::bail!(
                "not a duplicate anymore: {} differs from the kept copy (BLAKE3)",
                extra.display()
            );
        }
        let before = hoardini_win::alloc_size_by_open(extra)
            .map(|n| i64::try_from(n).unwrap_or(i64::MAX))
            .unwrap_or(cx.item.alloc);
        cx.check_cancel()?;
        let undo = match self.mode {
            Mode::Hardlink => {
                replace_with_link(&self.keep, extra)?;
                cx.rescan = true;
                serde_json::json!({
                    "kind": "none",
                    "reason": "hard-linked to the kept copy; copy the file to a new name to break the link",
                    "keep": self.keep.to_string_lossy(),
                    "path": cx.item.path,
                })
            }
            Mode::Delete => {
                remove_file_force(extra)?;
                serde_json::json!({
                    "kind": "none",
                    "reason": "permanent delete of an exact duplicate; the kept copy has the same content",
                    "keep": self.keep.to_string_lossy(),
                })
            }
        };
        cx.freed = before;
        Ok(Outcome { undo, detail: None })
    }
}

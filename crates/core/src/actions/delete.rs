use std::path::Path;

use super::handler::{Handler, ItemCx, JobCx, Outcome, Runner};
use crate::policy::ItemKind;

pub(crate) struct DeleteHandler;

impl Handler for DeleteHandler {
    fn begin(&self, _job: &JobCx<'_>) -> anyhow::Result<Box<dyn Runner>> {
        Ok(Box::new(DeleteRunner))
    }
}

struct DeleteRunner;

pub(crate) fn remove_file_force(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            let mut perms = std::fs::symlink_metadata(path)?.permissions();
            if !perms.readonly() {
                return Err(e);
            }
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            std::fs::set_permissions(path, perms)?;
            std::fs::remove_file(path)
        }
        r => r,
    }
}

// Links go by their own name; remove_dir_all never follows links inside (Rust >= 1.58.1).
pub(crate) fn remove_item(path: &Path, kind: ItemKind) -> std::io::Result<()> {
    match kind {
        ItemKind::File => remove_file_force(path),
        ItemKind::Dir => std::fs::remove_dir_all(path),
        ItemKind::Link => {
            if std::fs::symlink_metadata(path)?.is_dir() {
                std::fs::remove_dir(path)
            } else {
                std::fs::remove_file(path)
            }
        }
    }
}

pub(crate) fn remaining_alloc(path: &Path) -> i64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = hoardini_win::list_dir(&dir) else {
            continue;
        };
        for e in entries {
            if e.is_dir() {
                if !e.is_reparse() {
                    stack.push(dir.join(&e.name));
                }
            } else if !e.is_link() {
                total += e.alloc.max(if e.needs_alloc_by_open() {
                    e.logical
                } else {
                    0
                });
            }
        }
    }
    crate::index::to_i64(total)
}

impl Runner for DeleteRunner {
    fn run(&mut self, cx: &mut ItemCx<'_>) -> anyhow::Result<Outcome> {
        cx.check_cancel()?;
        let path = Path::new(&cx.item.path);
        if let Err(e) = remove_item(path, cx.item.kind) {
            if cx.item.kind == ItemKind::Dir && path.exists() {
                cx.freed = (cx.item.alloc - remaining_alloc(path)).max(0);
                cx.rescan = true;
            }
            return Err(anyhow::anyhow!("delete failed: {e}"));
        }
        cx.freed = cx.item.alloc;
        (cx.progress)(cx.item.alloc);
        Ok(Outcome {
            undo: serde_json::json!({"kind": "none", "reason": "permanent delete; there is no undo"}),
            detail: None,
        })
    }
}

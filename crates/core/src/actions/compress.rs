use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use super::delete::{remaining_alloc, remove_file_force};
use super::handler::{Cancelled, CopyHook, Handler, ItemCx, JobCx, Outcome, Runner};
use crate::compress::pack::{self, Format};
use crate::policy::ItemKind;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(crate) struct CompressHandler;

#[derive(Clone, Copy)]
enum Method {
    Archive(Format),
    Lzx,
    Lznt1,
}

struct CompressRunner {
    method: Method,
    hook: Option<CopyHook>,
}

impl Handler for CompressHandler {
    fn weight(&self, item: &crate::proposals::StoredItem) -> i64 {
        item.logical.max(item.alloc)
    }

    fn begin(&self, job: &JobCx<'_>) -> anyhow::Result<Box<dyn Runner>> {
        let f = job
            .proposal
            .format
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("compress proposal has no format"))?;
        let method = match f.trim().to_ascii_lowercase().as_str() {
            "lzx" | "wof-lzx" => Method::Lzx,
            "lznt1" => Method::Lznt1,
            other => Method::Archive(
                Format::parse(other)
                    .ok_or_else(|| anyhow::anyhow!("unknown compression format {f:?}"))?,
            ),
        };
        Ok(Box::new(CompressRunner {
            method,
            hook: job.hook.clone(),
        }))
    }
}

fn alloc_of(path: &Path, kind: ItemKind) -> Option<i64> {
    match kind {
        ItemKind::Dir => Some(remaining_alloc(path)),
        _ => hoardini_win::alloc_size_by_open(path)
            .ok()
            .map(|n| i64::try_from(n).unwrap_or(i64::MAX)),
    }
}

// compact /s walks into junctions and recalls cloud files, so a folder must hold neither.
fn refuse_links_inside(root: &Path) -> anyhow::Result<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in hoardini_win::list_dir(&dir)? {
            let p = dir.join(&e.name);
            if e.is_reparse() && e.reparse_tag != hoardini_win::IO_REPARSE_TAG_WOF {
                anyhow::bail!(
                    "{} is a link or reparse point; compact would follow it",
                    p.display()
                );
            }
            if e.is_placeholder() {
                anyhow::bail!(
                    "{} is cloud-only; compressing would download it",
                    p.display()
                );
            }
            if e.is_dir() {
                stack.push(p);
            }
        }
    }
    Ok(())
}

fn map_cancel(e: anyhow::Error) -> anyhow::Error {
    if e.is::<pack::Cancelled>() {
        Cancelled.into()
    } else {
        e
    }
}

impl CompressRunner {
    fn archive(&self, cx: &mut ItemCx<'_>, fmt: Format) -> anyhow::Result<Outcome> {
        let root = PathBuf::from(&cx.item.path);
        let before = alloc_of(&root, cx.item.kind).unwrap_or(cx.item.alloc);
        let packed = pack::pack(
            &root,
            fmt,
            cx.cancel,
            &mut *cx.progress,
            self.hook.as_deref().map(|h| h as &dyn Fn(&Path)),
        )
        .map_err(map_cancel)?;
        // The archive is complete and tested; from here the originals may go.
        cx.rescan = true;
        let mut kept = Vec::new();
        for e in packed.entries.iter().filter(|e| !e.is_dir) {
            let unchanged = std::fs::symlink_metadata(&e.src).is_ok_and(|md| {
                md.len() == e.size
                    && md
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64)
                        == e.mtime_ms
            });
            if !unchanged {
                kept.push(e.src.to_string_lossy().into_owned());
                continue;
            }
            remove_file_force(&e.src).map_err(|err| {
                anyhow::anyhow!("archived, but could not delete {}: {err}", e.src.display())
            })?;
        }
        // Deepest first; a folder that still has content (a changed file) simply stays.
        for e in packed.entries.iter().rev().filter(|e| e.is_dir) {
            let _ = std::fs::remove_dir(&e.src);
        }
        let after = alloc_of(&packed.archive, ItemKind::File).unwrap_or(0);
        let left = if root.exists() {
            alloc_of(&root, cx.item.kind).unwrap_or(0)
        } else {
            0
        };
        cx.freed = (before - after - left).max(0);
        let parent = root.parent().map(|p| p.to_string_lossy().into_owned());
        Ok(Outcome {
            undo: serde_json::json!({
                "kind": "extract",
                "reason": "the archive is the result; extract it to get the original back",
                "archive": packed.archive.to_string_lossy(),
                "format": fmt.ext(),
                "extract_into": parent,
            }),
            detail: if kept.is_empty() {
                Some(format!("Archived to {}", packed.archive.display()))
            } else {
                Some(format!(
                    "Archived to {}; {} changed file(s) were left in place",
                    packed.archive.display(),
                    kept.len()
                ))
            },
        })
    }

    fn in_place(&self, cx: &mut ItemCx<'_>, lzx: bool) -> anyhow::Result<Outcome> {
        let path = Path::new(&cx.item.path);
        let is_dir = cx.item.kind == ItemKind::Dir;
        if is_dir {
            refuse_links_inside(path)?;
        }
        let before = alloc_of(path, cx.item.kind).unwrap_or(cx.item.alloc);
        let mut args: Vec<String> = vec!["/c".into()];
        let mut undo_args: Vec<String> = vec!["/u".into()];
        if is_dir {
            args.push(format!("/s:{}", cx.item.path));
            undo_args.push(format!("/s:{}", cx.item.path));
        }
        args.push("/i".into());
        undo_args.push("/i".into());
        if lzx {
            args.push("/exe:lzx".into());
            undo_args.push("/exe".into());
        }
        if !is_dir {
            args.push(cx.item.path.clone());
            undo_args.push(cx.item.path.clone());
        }
        let mut child = Command::new("compact.exe")
            .args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("could not start compact.exe: {e}"))?;
        cx.rescan = true;
        let status = loop {
            if let Some(s) = child.try_wait()? {
                break s;
            }
            if cx.check_cancel().is_err() {
                let _ = child.kill();
                let _ = child.wait();
                cx.freed = (before - alloc_of(path, cx.item.kind).unwrap_or(before)).max(0);
                return Err(Cancelled.into());
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let after = alloc_of(path, cx.item.kind).unwrap_or(before);
        cx.freed = (before - after).max(0);
        (cx.progress)(cx.item.logical.max(cx.item.alloc));
        if !status.success() {
            anyhow::bail!("compact.exe exited with {status}");
        }
        let mut argv = vec!["compact.exe".to_string()];
        argv.extend(undo_args);
        Ok(Outcome {
            undo: serde_json::json!({
                "kind": "command",
                "reason": "run compact /u to decompress",
                "argv": argv,
            }),
            detail: None,
        })
    }
}

impl Runner for CompressRunner {
    fn run(&mut self, cx: &mut ItemCx<'_>) -> anyhow::Result<Outcome> {
        cx.check_cancel()?;
        match self.method {
            Method::Archive(fmt) => self.archive(cx, fmt),
            Method::Lzx => self.in_place(cx, true),
            Method::Lznt1 => self.in_place(cx, false),
        }
    }
}

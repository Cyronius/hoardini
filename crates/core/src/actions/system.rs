use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rusqlite::Connection;

use super::{Ctx, Job, journal, patch_index};
use crate::proposals::{self, Proposal, StoredItem};
use crate::syscleanup::{OpResult, SystemOp, helper_params};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const ERROR_CANCELLED: i32 = 1223;
const WSL_SHUTDOWN_WAIT: Duration = Duration::from_secs(60);

pub trait SystemRunner: Send + Sync {
    fn local(&self, op: &SystemOp) -> anyhow::Result<OpResult>;
    // All elevated operations behind one administrator prompt; `done(n)` reports how many have ended.
    fn elevated(
        &self,
        ops: &[SystemOp],
        done: &mut dyn FnMut(usize),
    ) -> anyhow::Result<Vec<OpResult>>;
}

pub struct WindowsRunner;

fn total_free() -> u64 {
    hoardini_win::fixed_volumes()
        .unwrap_or_default()
        .iter()
        .filter_map(|v| hoardini_win::volume_info(v).ok())
        .map(|v| v.free_bytes)
        .sum()
}

fn shutdown_wsl() {
    let Ok(exe) = hoardini_win::system_tool("wsl.exe") else {
        return;
    };
    let Ok(mut child) = Command::new(exe)
        .arg("--shutdown")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let start = Instant::now();
    while start.elapsed() < WSL_SHUTDOWN_WAIT {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
}

fn read_results(path: &Path) -> Vec<OpResult> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

impl SystemRunner for WindowsRunner {
    fn local(&self, op: &SystemOp) -> anyhow::Result<OpResult> {
        if *op != SystemOp::RecycleBin {
            anyhow::bail!("{} needs the administrator prompt", op.name());
        }
        // SHEmptyRecycleBinW fails on an empty bin, so an empty one is reported, not attempted.
        if hoardini_win::recycle_bin()?.items == 0 {
            return Ok(OpResult {
                ok: true,
                detail: Some("the Recycle Bin was already empty".into()),
                ..OpResult::default()
            });
        }
        let before = total_free();
        hoardini_win::empty_recycle_bin()?;
        Ok(OpResult {
            ok: true,
            freed: total_free().saturating_sub(before),
            ..OpResult::default()
        })
    }

    fn elevated(
        &self,
        ops: &[SystemOp],
        done: &mut dyn FnMut(usize),
    ) -> anyhow::Result<Vec<OpResult>> {
        // WSL is per user, so it is stopped from this side of the prompt.
        if ops
            .iter()
            .any(|o| matches!(o, SystemOp::CompactVhdx { .. }))
        {
            shutdown_wsl();
        }
        let result = std::env::temp_dir().join(format!(
            "hoardini-cleanup-{}.jsonl",
            uuid::Uuid::now_v7().simple()
        ));
        let params = helper_params(&result, ops)?;
        let exe = std::env::current_exe()?;
        let code = hoardini_win::run_elevated(&exe, &params, || done(read_results(&result).len()))
            .map_err(|e| {
                if e.raw_os_error() == Some(ERROR_CANCELLED) {
                    anyhow::anyhow!("The administrator prompt was declined; nothing ran")
                } else {
                    anyhow::anyhow!("could not start the elevated helper: {e}")
                }
            })?;
        let mut results = read_results(&result);
        let _ = std::fs::remove_file(&result);
        if results.is_empty() && code != 0 {
            anyhow::bail!("the elevated helper exited with {code} before running anything");
        }
        results.truncate(ops.len());
        results.resize_with(ops.len(), || {
            OpResult::failed("the elevated helper stopped before this ran")
        });
        Ok(results)
    }
}

struct Run<'a> {
    ctx: &'a Ctx,
    conn: &'a Connection,
    job: &'a mut Job,
    done: usize,
    failed: usize,
    cancelled: bool,
}

impl Run<'_> {
    fn settle(
        &mut self,
        item: &StoredItem,
        op: &SystemOp,
        action_id: i64,
        res: OpResult,
    ) -> anyhow::Result<()> {
        let freed = i64::try_from(res.freed).unwrap_or(i64::MAX);
        if res.ok {
            journal::finish(self.conn, action_id, "done", freed, Some(&op.undo()), None)?;
            if matches!(op, SystemOp::Hibernation | SystemOp::CompactVhdx { .. }) {
                patch_index(self.conn, &item.path, true);
            }
            proposals::set_item(self.conn, item.id, "done", res.detail.as_deref())?;
            self.done += 1;
        } else {
            let msg = res.error.unwrap_or_else(|| "failed".into());
            journal::finish(self.conn, action_id, "failed", freed, None, Some(&msg))?;
            proposals::set_item(self.conn, item.id, "failed", Some(&msg))?;
            self.failed += 1;
        }
        self.job.freed_bytes += freed;
        self.job.progress.done_items += 1;
        self.job.progress.done_bytes += item.alloc;
        self.ctx.publish(Some(self.conn), self.job);
        Ok(())
    }

    fn cancel(&mut self, item: &StoredItem, action_id: i64) -> anyhow::Result<()> {
        journal::finish(
            self.conn,
            action_id,
            "cancelled",
            0,
            None,
            Some("cancelled"),
        )?;
        proposals::set_item(self.conn, item.id, "skipped", Some("Cancelled"))?;
        self.job.progress.done_items += 1;
        self.cancelled = true;
        Ok(())
    }
}

// The user's own operations run first; the rest go to one elevated helper, so there is one prompt per job.
pub(crate) fn run_job(
    ctx: &Ctx,
    conn: &Connection,
    job: &mut Job,
    p: &Proposal,
    items: &[StoredItem],
    cancel: &AtomicBool,
) -> anyhow::Result<()> {
    let runner: Arc<dyn SystemRunner> = ctx
        .shared
        .system
        .lock()
        .clone()
        .unwrap_or_else(|| Arc::new(WindowsRunner));
    let mut work: Vec<(&StoredItem, SystemOp)> = Vec::new();
    for it in items {
        match p
            .items
            .iter()
            .find(|i| i.id == it.id)
            .and_then(|i| i.op.as_ref())
        {
            Some(v) => work.push((it, v.op.clone())),
            None => proposals::set_item(conn, it.id, "failed", Some("no operation recorded"))?,
        }
    }
    job.progress.total_items = work.len() as i64;
    job.progress.total_bytes = work.iter().map(|(it, _)| it.alloc).sum();
    job.status = "running".into();
    proposals::set_status(conn, &p.id, "running")?;
    ctx.publish(Some(conn), job);

    let mut actions = Vec::with_capacity(work.len());
    for (it, _) in &work {
        actions.push(journal::begin(
            conn,
            &job.id,
            Some(&p.id),
            Some(it.id),
            p.kind.as_str(),
            &it.path,
        )?);
    }
    let mut run = Run {
        ctx,
        conn,
        job,
        done: 0,
        failed: 0,
        cancelled: false,
    };
    for (k, (it, op)) in work
        .iter()
        .enumerate()
        .filter(|(_, (_, op))| !op.elevated())
    {
        if cancel.load(Ordering::Relaxed) {
            run.cancel(it, actions[k])?;
            continue;
        }
        run.job.progress.current = Some(op.label());
        ctx.publish(None, run.job);
        let res = runner
            .local(op)
            .unwrap_or_else(|e| OpResult::failed(format!("{e:#}")));
        run.settle(it, op, actions[k], res)?;
    }
    let elevated: Vec<usize> = (0..work.len()).filter(|&k| work[k].1.elevated()).collect();
    if !elevated.is_empty() && cancel.load(Ordering::Relaxed) {
        for &k in &elevated {
            run.cancel(work[k].0, actions[k])?;
        }
    } else if !elevated.is_empty() {
        let ops: Vec<SystemOp> = elevated.iter().map(|&k| work[k].1.clone()).collect();
        run.job.progress.current = Some("Waiting for the administrator prompt".into());
        ctx.publish(None, run.job);
        let shown = run.job.clone();
        let mut seen = 0usize;
        let mut on_done = |n: usize| {
            if n > seen && n <= ops.len() {
                seen = n;
                let mut now = shown.clone();
                now.progress.done_items += n as i64;
                now.progress.current = ops.get(n).map(SystemOp::label);
                ctx.publish(None, &now);
            }
        };
        let results = runner
            .elevated(&ops, &mut on_done)
            .unwrap_or_else(|e| vec![OpResult::failed(format!("{e:#}")); ops.len()]);
        for (&k, res) in elevated.iter().zip(results) {
            run.settle(work[k].0, &work[k].1, actions[k], res)?;
        }
    }
    let approved: i64 = conn.query_row(
        "SELECT COUNT(*) FROM proposal_item WHERE proposal_id = ?1 AND selected = 1",
        [&p.id],
        |r| r.get(0),
    )?;
    let status = if run.done > 0 && run.done as i64 == approved {
        "done"
    } else if run.done > 0 {
        "partial"
    } else {
        "failed"
    };
    proposals::set_status(conn, &p.id, status)?;
    run.job.status = if run.cancelled {
        "cancelled"
    } else if run.done == 0 && run.failed > 0 {
        "failed"
    } else {
        "done"
    }
    .into();
    Ok(())
}

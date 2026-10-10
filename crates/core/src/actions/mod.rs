mod archive;
mod compress;
mod dedupe;
mod delete;
mod handler;
mod install;
mod journal;
mod online;
mod rclone;
mod system;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rusqlite::{Connection, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::destinations::rclone::Endpoint;
use crate::index::{self, WinLister};
use crate::memory;
use crate::policy::{self, PolicyEnv};
use crate::proposals::{self, StoredItem};
use crate::store::{Db, now_ms};
use crate::vault::SecretStore;

pub use handler::{CopyHook, supports};
pub use system::{SystemRunner, WindowsRunner};

use handler::{Cancelled, ItemCx, JobCx, handler_for};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct JobProgress {
    pub done_bytes: i64,
    pub total_bytes: i64,
    pub done_items: i64,
    pub total_items: i64,
    pub current: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub proposal_id: Option<String>,
    pub status: String,
    pub progress: JobProgress,
    pub freed_bytes: i64,
    pub error: Option<String>,
    pub created_ms: i64,
    pub finished_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApproveResult {
    pub approved: bool,
    pub items: Vec<i64>,
    pub job_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub id: i64,
    pub original_path: String,
    pub destination_id: String,
    pub dest_uri: String,
    pub size: i64,
    pub archived_ms: i64,
    pub restored_ms: Option<i64>,
}

struct Live {
    job: Job,
    cancel: Arc<AtomicBool>,
}

struct Shared {
    env: PolicyEnv,
    live: Mutex<HashMap<String, Live>>,
    // Jobs run one at a time; a second approval waits as "queued".
    run_lock: Mutex<()>,
    hook: Mutex<Option<CopyHook>>,
    rclone: Mutex<Option<Endpoint>>,
    system: Mutex<Option<Arc<dyn SystemRunner>>>,
}

// Process-wide so a second Executor's startup recovery never fails a job that is still running here.
static RUNNING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

struct RunningGuard(String);

impl RunningGuard {
    fn new(id: &str) -> RunningGuard {
        RUNNING.lock().insert(id.to_string());
        RunningGuard(id.to_string())
    }
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        RUNNING.lock().remove(&self.0);
    }
}

// Jobs run on std threads (file I/O is blocking); listeners get every Job update.
pub struct Executor {
    pub db: Db,
    pub secrets: Arc<dyn SecretStore>,
    pub events: tokio::sync::broadcast::Sender<Job>,
    shared: Arc<Shared>,
}

#[derive(Clone)]
struct Ctx {
    db: Db,
    secrets: Arc<dyn SecretStore>,
    events: tokio::sync::broadcast::Sender<Job>,
    shared: Arc<Shared>,
}

impl Ctx {
    fn publish(&self, conn: Option<&Connection>, job: &Job) {
        if let Some(l) = self.shared.live.lock().get_mut(&job.id) {
            l.job = job.clone();
        }
        if let Some(conn) = conn
            && let Err(e) = persist_job(conn, job)
        {
            tracing::warn!(job = %job.id, error = %e, "could not persist job state");
        }
        let _ = self.events.send(job.clone());
    }
}

fn persist_job(conn: &Connection, job: &Job) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE job SET status = ?2, progress_json = ?3, finished_ms = ?4, error = ?5 WHERE id = ?1",
        params![
            job.id,
            job.status,
            serde_json::to_string(&job.progress)?,
            job.finished_ms,
            job.error
        ],
    )?;
    Ok(())
}

fn insert_job(conn: &Connection, job: &Job) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO job(id, kind, proposal_id, status, progress_json, created_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            job.id,
            job.kind,
            job.proposal_id,
            job.status,
            serde_json::to_string(&job.progress)?,
            job.created_ms
        ],
    )?;
    Ok(())
}

fn new_job(kind: &str, proposal_id: Option<&str>) -> Job {
    Job {
        id: uuid::Uuid::now_v7().to_string(),
        kind: kind.to_string(),
        proposal_id: proposal_id.map(str::to_string),
        status: "queued".into(),
        progress: JobProgress::default(),
        freed_bytes: 0,
        error: None,
        created_ms: now_ms(),
        finished_ms: None,
    }
}

// Our own change, so the index is patched now instead of waiting for the next refresh.
fn patch_index(conn: &Connection, path: &str, rescan: bool) {
    let had = matches!(index::node_id_by_path(conn, path), Ok(Some(_)));
    if had && let Err(e) = index::patch_removed(conn, path) {
        tracing::debug!(%path, error = %e, "index patch (remove) skipped");
    }
    if (!had || rescan)
        && let Err(e) = index::patch_rescan(conn, &WinLister, path)
    {
        tracing::debug!(%path, error = %e, "index patch (rescan) skipped");
    }
}

fn recover_interrupted(db: &Db) -> anyhow::Result<()> {
    let conn = db.conn()?;
    let running: HashSet<String> = RUNNING.lock().clone();
    let now = now_ms();
    let stale: Vec<(String, Option<String>)> = conn
        .prepare("SELECT id, proposal_id FROM job WHERE status IN ('queued', 'running')")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (job_id, proposal_id) in stale.into_iter().filter(|(id, _)| !running.contains(id)) {
        conn.execute(
            "UPDATE job SET status = 'failed', error = 'interrupted: Hoardini closed while this job ran', finished_ms = ?2 WHERE id = ?1",
            params![job_id, now],
        )?;
        conn.execute(
            "UPDATE action SET status = 'failed', error = 'interrupted', finished_ms = ?2 WHERE job_id = ?1 AND status = 'running'",
            params![job_id, now],
        )?;
        if let Some(pid) = proposal_id {
            conn.execute(
                "UPDATE proposal_item SET status = 'skipped', detail = 'Interrupted before it ran' WHERE proposal_id = ?1 AND status = 'pending' AND selected = 1",
                [&pid],
            )?;
            let done: i64 = conn.query_row(
                "SELECT COUNT(*) FROM proposal_item WHERE proposal_id = ?1 AND status = 'done'",
                [&pid],
                |r| r.get(0),
            )?;
            proposals::set_status(&conn, &pid, if done > 0 { "partial" } else { "failed" })?;
        }
    }
    Ok(())
}

impl Executor {
    pub fn new(db: Db, secrets: Arc<dyn SecretStore>) -> Executor {
        Executor::with_policy(db, secrets, PolicyEnv::system())
    }

    pub fn with_policy(db: Db, secrets: Arc<dyn SecretStore>, env: PolicyEnv) -> Executor {
        let (events, _) = tokio::sync::broadcast::channel(256);
        if let Err(e) = recover_interrupted(&db) {
            tracing::warn!(error = %e, "could not recover interrupted jobs");
        }
        Executor {
            db,
            secrets,
            events,
            shared: Arc::new(Shared {
                env,
                live: Mutex::new(HashMap::new()),
                run_lock: Mutex::new(()),
                hook: Mutex::new(None),
                rclone: Mutex::new(None),
                system: Mutex::new(None),
            }),
        }
    }

    fn ctx(&self) -> Ctx {
        Ctx {
            db: self.db.clone(),
            secrets: self.secrets.clone(),
            events: self.events.clone(),
            shared: self.shared.clone(),
        }
    }

    #[doc(hidden)]
    // Fault injection for ACT-ARCHIVE-VERIFY: runs on each finished copy just before it is verified.
    pub fn set_copy_hook(&self, hook: Option<CopyHook>) {
        *self.shared.hook.lock() = hook;
    }

    #[doc(hidden)]
    // Points rclone work at a fake rc server instead of launching the daemon.
    pub fn set_rclone_endpoint(&self, ep: Option<Endpoint>) {
        *self.shared.rclone.lock() = ep;
    }

    #[doc(hidden)]
    // Replaces the Windows tools and the admin prompt for system_cleanup jobs.
    pub fn set_system_runner(&self, runner: Option<Arc<dyn SystemRunner>>) {
        *self.shared.system.lock() = runner;
    }

    // The only entry point that changes the filesystem: revalidates, then starts a job.
    pub fn approve(&self, proposal_id: &str, item_ids: &[i64]) -> anyhow::Result<ApproveResult> {
        let mut conn = self.db.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let p = proposals::load(&tx, proposal_id)?
            .ok_or_else(|| anyhow::anyhow!("unknown proposal {proposal_id}"))?;
        if p.status != "pending" {
            anyhow::bail!("proposal {proposal_id} is {}, not pending", p.status);
        }
        if !supports(p.kind) {
            anyhow::bail!("{} proposals cannot run yet", p.kind.as_str());
        }
        let mut chosen: Vec<i64> = Vec::new();
        for id in item_ids {
            let item = p.items.iter().find(|i| i.id == *id).ok_or_else(|| {
                anyhow::anyhow!("item {id} is not part of proposal {proposal_id}")
            })?;
            if item.status != "pending" {
                anyhow::bail!(
                    "item {id} cannot be approved: {}",
                    item.block_reason.as_deref().unwrap_or(&item.status)
                );
            }
            if !chosen.contains(id) {
                chosen.push(*id);
            }
        }
        if chosen.is_empty() {
            anyhow::bail!("select at least one item to approve");
        }
        let now = now_ms();
        for item in p.items.iter().filter(|i| i.status == "pending") {
            if chosen.contains(&item.id) {
                tx.execute(
                    "UPDATE proposal_item SET selected = 1 WHERE id = ?1",
                    [item.id],
                )?;
            } else {
                tx.execute(
                    "UPDATE proposal_item SET selected = 0, status = 'skipped', detail = 'Not selected' WHERE id = ?1",
                    [item.id],
                )?;
            }
        }
        let mut job = new_job(p.kind.as_str(), Some(proposal_id));
        tx.execute(
            "UPDATE proposal SET status = 'approved', decided_ms = ?2, job_id = ?3, touched = 1 WHERE id = ?1",
            params![proposal_id, now, job.id],
        )?;
        insert_job(&tx, &job)?;
        tx.commit()?;

        // The proposal is claimed; from here a failure must not leave it approved with no job running.
        let running = RunningGuard::new(&job.id);
        let runnable = self.revalidate_chosen(&conn, p.kind.action(), proposal_id, &chosen);
        let cancel = Arc::new(AtomicBool::new(false));
        job.progress.total_items = runnable.len() as i64;
        self.shared.live.lock().insert(
            job.id.clone(),
            Live {
                job: job.clone(),
                cancel: cancel.clone(),
            },
        );
        let ctx = self.ctx();
        let pid = proposal_id.to_string();
        let thread_job = job.clone();
        let spawned = std::thread::Builder::new()
            .name("hoardini-job".into())
            .spawn(move || run_proposal_job(ctx, thread_job, pid, cancel, running));
        if let Err(e) = spawned {
            self.shared.live.lock().remove(&job.id);
            job.status = "failed".into();
            job.error = Some(format!("could not start the job: {e}"));
            job.finished_ms = Some(now_ms());
            let _ = proposals::set_status(&conn, proposal_id, "failed");
            follow_up(&self.db, &self.shared.env, proposal_id);
            let _ = persist_job(&conn, &job);
            return Err(anyhow::anyhow!("could not start the job: {e}"));
        }
        Ok(ApproveResult {
            approved: true,
            items: runnable,
            job_id: job.id,
        })
    }

    // Errors here only cost the early answer; the job revalidates each item again before acting.
    fn revalidate_chosen(
        &self,
        conn: &Connection,
        action: policy::Action,
        proposal_id: &str,
        chosen: &[i64],
    ) -> Vec<i64> {
        // Operations have no file state to compare; the Windows tools check their own preconditions.
        if action == policy::Action::SystemCleanup {
            return chosen.to_vec();
        }
        let items = match proposals::stored_items(conn, proposal_id) {
            Ok(items) => items,
            Err(e) => {
                tracing::warn!(error = %e, "approve-time revalidation skipped");
                return chosen.to_vec();
            }
        };
        let mut runnable = Vec::new();
        for item in items.into_iter().filter(|i| chosen.contains(&i.id)) {
            match policy::revalidate(
                conn,
                &self.shared.env,
                action,
                &item.path,
                &item.baseline(),
                item.cap,
            ) {
                Ok(Some(detail)) => {
                    if let Err(e) = proposals::set_item(conn, item.id, "skipped", Some(&detail)) {
                        tracing::warn!(error = %e, "could not mark item skipped");
                        runnable.push(item.id);
                    }
                }
                Ok(None) => runnable.push(item.id),
                Err(e) => {
                    tracing::warn!(error = %e, path = %item.path, "approve-time revalidation failed");
                    runnable.push(item.id);
                }
            }
        }
        runnable
    }

    pub fn jobs(&self) -> Vec<Job> {
        let mut out = match self.db.conn().and_then(|c| load_jobs(&c)) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "could not load jobs");
                Vec::new()
            }
        };
        let live = self.shared.live.lock();
        for j in out.iter_mut() {
            if let Some(l) = live.get(&j.id) {
                *j = l.job.clone();
            }
        }
        out
    }

    pub fn cancel(&self, job_id: &str) -> bool {
        match self.shared.live.lock().get(job_id) {
            Some(l) if matches!(l.job.status.as_str(), "queued" | "running") => {
                l.cancel.store(true, Ordering::Relaxed);
                true
            }
            _ => false,
        }
    }

    pub fn restore(&self, archive_entry_id: i64) -> anyhow::Result<String> {
        let conn = self.db.conn()?;
        let entry = archive::load_entry(&conn, archive_entry_id)?
            .ok_or_else(|| anyhow::anyhow!("unknown archive entry {archive_entry_id}"))?;
        if std::fs::symlink_metadata(&entry.original_path).is_ok() {
            anyhow::bail!(
                "something already exists at {}; move it away before restoring",
                entry.original_path
            );
        }
        let remote = entry
            .dest_uri
            .starts_with(crate::destinations::rclone::URI_PREFIX);
        if let Some(dest) = crate::destinations::get(&conn, &entry.destination_id)?
            && !remote
        {
            crate::destinations::connect(&dest, &*self.secrets)?;
        }
        if !remote && !Path::new(&entry.dest_uri).exists() {
            anyhow::bail!("the archived copy at {} is not reachable", entry.dest_uri);
        }
        let mut job = new_job("restore", None);
        job.progress.total_items = 1;
        job.progress.total_bytes = entry.size;
        insert_job(&conn, &job)?;
        let cancel = Arc::new(AtomicBool::new(false));
        self.shared.live.lock().insert(
            job.id.clone(),
            Live {
                job: job.clone(),
                cancel: cancel.clone(),
            },
        );
        let ctx = self.ctx();
        let id = job.id.clone();
        let running = RunningGuard::new(&id);
        let spawned = std::thread::Builder::new()
            .name("hoardini-restore".into())
            .spawn(move || run_restore_job(ctx, job, entry, cancel, running));
        if let Err(e) = spawned {
            self.shared.live.lock().remove(&id);
            let _ = conn.execute(
                "UPDATE job SET status = 'failed', error = ?2, finished_ms = ?3 WHERE id = ?1",
                params![id, format!("could not start the job: {e}"), now_ms()],
            );
            return Err(anyhow::anyhow!("could not start the restore: {e}"));
        }
        Ok(id)
    }

    pub fn vanished_total(&self) -> i64 {
        let total = self.db.conn().and_then(|c| {
            Ok(c.query_row(
                "SELECT COALESCE(SUM(freed_bytes), 0) FROM action",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        });
        total.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not sum freed bytes");
            0
        })
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Job> {
        self.events.subscribe()
    }
}

pub fn archive_entries(
    conn: &Connection,
    q: Option<&str>,
    limit: usize,
) -> anyhow::Result<Vec<ArchiveEntry>> {
    let pattern = q
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!("%{}%", s.replace(['%', '_'], "")));
    let mut st = conn.prepare(
        "SELECT id, original_path, destination_id, dest_uri, size, archived_ms, restored_ms FROM archive_entry
         WHERE ?1 IS NULL OR original_path LIKE ?1 OR dest_uri LIKE ?1
         ORDER BY archived_ms DESC, id DESC LIMIT ?2",
    )?;
    Ok(st
        .query_map(params![pattern, limit.max(1) as i64], |r| {
            Ok(ArchiveEntry {
                id: r.get(0)?,
                original_path: r.get(1)?,
                destination_id: r.get(2)?,
                dest_uri: r.get(3)?,
                size: r.get(4)?,
                archived_ms: r.get(5)?,
                restored_ms: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_jobs(conn: &Connection) -> anyhow::Result<Vec<Job>> {
    let mut st = conn.prepare(
        "SELECT j.id, j.kind, j.proposal_id, j.status, j.progress_json, j.created_ms, j.finished_ms, j.error,
                (SELECT COALESCE(SUM(a.freed_bytes), 0) FROM action a WHERE a.job_id = j.id)
         FROM job j ORDER BY j.created_ms DESC LIMIT 100",
    )?;
    Ok(st
        .query_map([], |r| {
            let progress: Option<String> = r.get(4)?;
            Ok(Job {
                id: r.get(0)?,
                kind: r.get(1)?,
                proposal_id: r.get(2)?,
                status: r.get(3)?,
                progress: progress
                    .and_then(|p| serde_json::from_str(&p).ok())
                    .unwrap_or_default(),
                created_ms: r.get(5)?,
                finished_ms: r.get(6)?,
                error: r.get(7)?,
                freed_bytes: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn finish_job(ctx: &Ctx, conn: Option<&Connection>, job: &mut Job) {
    job.finished_ms = Some(now_ms());
    job.progress.current = None;
    ctx.publish(conn, job);
    ctx.shared.live.lock().remove(&job.id);
}

fn run_proposal_job(
    ctx: Ctx,
    mut job: Job,
    proposal_id: String,
    cancel: Arc<AtomicBool>,
    _running: RunningGuard,
) {
    let conn = match ctx.db.conn() {
        Ok(c) => c,
        Err(e) => {
            job.status = "failed".into();
            job.error = Some(e.to_string());
            finish_job(&ctx, None, &mut job);
            return;
        }
    };
    let turn = ctx.shared.run_lock.lock();
    if let Err(e) = run_items(&ctx, &conn, &mut job, &proposal_id, &cancel) {
        job.status = "failed".into();
        job.error = Some(e.to_string());
        let _ = conn.execute(
            "UPDATE proposal_item SET status = 'failed', detail = ?2 WHERE proposal_id = ?1 AND status = 'pending' AND selected = 1",
            params![proposal_id, e.to_string()],
        );
        let _ = proposals::set_status(&conn, &proposal_id, "failed");
    }
    drop(turn);
    // Before the terminal broadcast, so the client's refetch on job end already sees follow_up_id and learned.
    follow_up(&ctx.db, &ctx.shared.env, &proposal_id);
    if let Err(e) = memory::learn::learn_from_job(&conn, &ctx.shared.env, &proposal_id, now_ms()) {
        tracing::warn!(proposal = %proposal_id, error = %e, "could not learn from the finished job");
    }
    finish_job(&ctx, Some(&conn), &mut job);
}

fn follow_up(db: &Db, env: &PolicyEnv, proposal_id: &str) {
    if let Err(e) = proposals::create_follow_up(db, env, proposal_id) {
        tracing::warn!(proposal = %proposal_id, error = %e, "could not create the follow-up proposal");
    }
}

const PUBLISH_EVERY: Duration = Duration::from_millis(200);

fn run_items(
    ctx: &Ctx,
    conn: &Connection,
    job: &mut Job,
    proposal_id: &str,
    cancel: &AtomicBool,
) -> anyhow::Result<()> {
    let p = proposals::load(conn, proposal_id)?
        .ok_or_else(|| anyhow::anyhow!("proposal {proposal_id} vanished"))?;
    let items: Vec<StoredItem> = proposals::stored_items(conn, proposal_id)?
        .into_iter()
        .filter(|i| i.selected && i.status == "pending")
        .collect();
    if p.kind == proposals::ProposalKind::SystemCleanup {
        return system::run_job(ctx, conn, job, &p, &items, cancel);
    }
    let handler = handler_for(p.kind)
        .ok_or_else(|| anyhow::anyhow!("{} proposals cannot run yet", p.kind.as_str()))?;
    job.progress.total_items = items.len() as i64;
    job.progress.total_bytes = items.iter().map(|i| handler.weight(i)).sum();
    job.status = "running".into();
    proposals::set_status(conn, proposal_id, "running")?;
    ctx.publish(Some(conn), job);

    let env = &ctx.shared.env;
    let hook = ctx.shared.hook.lock().clone();
    let jcx = JobCx {
        conn,
        proposal: &p,
        items: &items,
        secrets: &*ctx.secrets,
        env,
        hook,
        rc: ctx.shared.rclone.lock().clone(),
    };
    let mut runner = match handler.begin(&jcx) {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            for it in &items {
                proposals::set_item(conn, it.id, "failed", Some(&msg))?;
            }
            proposals::set_status(conn, proposal_id, "failed")?;
            job.status = "failed".into();
            job.error = Some(msg);
            return Ok(());
        }
    };
    let action = p.kind.action();
    let (mut done, mut failed, mut cancelled) = (0usize, 0usize, false);
    for it in &items {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            proposals::set_item(conn, it.id, "skipped", Some("Cancelled"))?;
            continue;
        }
        job.progress.current = Some(it.path.clone());
        ctx.publish(None, job);
        if let Some(detail) =
            policy::revalidate(conn, env, action, &it.path, &it.baseline(), it.cap)?
        {
            proposals::set_item(conn, it.id, "skipped", Some(&detail))?;
            job.progress.done_items += 1;
            ctx.publish(Some(conn), job);
            continue;
        }
        let action_id = journal::begin(
            conn,
            &job.id,
            Some(proposal_id),
            Some(it.id),
            p.kind.as_str(),
            &it.path,
        )?;
        let bytes_before = job.progress.done_bytes;
        let (res, freed, rescan) = {
            let mut last = Instant::now();
            let mut on_bytes = |n: i64| {
                job.progress.done_bytes += n;
                if last.elapsed() >= PUBLISH_EVERY {
                    ctx.publish(None, job);
                    last = Instant::now();
                }
            };
            let mut icx = ItemCx {
                conn,
                item: it,
                action_id,
                cancel,
                progress: &mut on_bytes,
                freed: 0,
                rescan: false,
            };
            let res = runner.run(&mut icx);
            (res, icx.freed, icx.rescan)
        };
        job.progress.done_bytes = bytes_before + handler.weight(it);
        match res {
            Ok(out) => {
                journal::finish(conn, action_id, "done", freed, Some(&out.undo), None)?;
                patch_index(conn, &it.path, rescan);
                proposals::set_item(conn, it.id, "done", out.detail.as_deref())?;
                done += 1;
            }
            Err(e) if e.is::<Cancelled>() => {
                cancelled = true;
                journal::finish(conn, action_id, "cancelled", freed, None, Some("cancelled"))?;
                if freed > 0 || rescan {
                    patch_index(conn, &it.path, true);
                }
                proposals::set_item(conn, it.id, "skipped", Some("Cancelled"))?;
            }
            Err(e) => {
                let msg = format!("{e:#}");
                journal::finish(conn, action_id, "failed", freed, None, Some(&msg))?;
                if freed > 0 || rescan {
                    patch_index(conn, &it.path, true);
                }
                proposals::set_item(conn, it.id, "failed", Some(&msg))?;
                failed += 1;
            }
        }
        job.freed_bytes += freed;
        job.progress.done_items += 1;
        ctx.publish(Some(conn), job);
    }
    // Judged against everything the user approved, including items revalidation skipped.
    let approved: i64 = conn.query_row(
        "SELECT COUNT(*) FROM proposal_item WHERE proposal_id = ?1 AND selected = 1",
        [proposal_id],
        |r| r.get(0),
    )?;
    let status = if done > 0 && done as i64 == approved {
        "done"
    } else if done > 0 {
        "partial"
    } else {
        "failed"
    };
    proposals::set_status(conn, proposal_id, status)?;
    job.status = if cancelled {
        "cancelled"
    } else if done == 0 && failed > 0 {
        "failed"
    } else {
        "done"
    }
    .into();
    Ok(())
}

fn run_restore_job(
    ctx: Ctx,
    mut job: Job,
    entry: archive::EntryRow,
    cancel: Arc<AtomicBool>,
    _running: RunningGuard,
) {
    let conn = match ctx.db.conn() {
        Ok(c) => c,
        Err(e) => {
            job.status = "failed".into();
            job.error = Some(e.to_string());
            finish_job(&ctx, None, &mut job);
            return;
        }
    };
    let turn = ctx.shared.run_lock.lock();
    job.status = "running".into();
    job.progress.current = Some(entry.original_path.clone());
    ctx.publish(Some(&conn), &job);
    let result = (|| -> anyhow::Result<()> {
        let action_id =
            journal::begin(&conn, &job.id, None, None, "restore", &entry.original_path)?;
        let undo = serde_json::json!({
            "kind": "none",
            "reason": "restored; the archived copy stays at the destination",
            "archive_entry_id": entry.id,
            "dest_uri": entry.dest_uri,
        });
        journal::set_undo(&conn, action_id, &undo)?;
        let res = {
            let mut last = Instant::now();
            let mut on_bytes = |n: i64| {
                job.progress.done_bytes += n;
                if last.elapsed() >= PUBLISH_EVERY {
                    ctx.publish(None, &job);
                    last = Instant::now();
                }
            };
            let mut copier = archive::Copier {
                cancel: &cancel,
                progress: &mut on_bytes,
                hook: None,
            };
            if entry
                .dest_uri
                .starts_with(crate::destinations::rclone::URI_PREFIX)
            {
                let rc = ctx.shared.rclone.lock().clone();
                rclone::restore(&conn, &entry, &mut copier, &*ctx.secrets, rc.as_ref())
            } else {
                archive::restore_entry(&conn, &entry, &mut copier)
            }
        };
        match res {
            Ok(()) => {
                journal::finish(&conn, action_id, "done", 0, None, None)?;
                if let Err(e) = index::patch_rescan(&conn, &WinLister, &entry.original_path) {
                    tracing::debug!(error = %e, "index patch after restore skipped");
                }
                Ok(())
            }
            Err(e) => {
                let status = if e.is::<Cancelled>() {
                    "cancelled"
                } else {
                    "failed"
                };
                journal::finish(&conn, action_id, status, 0, None, Some(&format!("{e:#}")))?;
                Err(e)
            }
        }
    })();
    drop(turn);
    match result {
        Ok(()) => {
            job.status = "done".into();
            job.progress.done_items = 1;
            job.progress.done_bytes = entry.size;
        }
        Err(e) if e.is::<Cancelled>() => job.status = "cancelled".into(),
        Err(e) => {
            job.status = "failed".into();
            job.error = Some(format!("{e:#}"));
        }
    }
    finish_job(&ctx, Some(&conn), &mut job);
}

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use hoardini_agent::run::IndexControl;
use hoardini_core::index;
use hoardini_core::store::{Db, get_meta};
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Notify, watch};

const JOURNAL_EVERY_S: u64 = 180;
// A timer-driven journal refresh shows in the header only once it runs longer than this.
const QUIET_FOR: Duration = Duration::from_secs(4);
const NEEDS_FULL_EXIT: i32 = 3;

#[derive(Default)]
struct State {
    refreshing: bool,
    mode: Option<&'static str>,
    progress: Option<Value>,
    quiet_since: Option<Instant>,
    // A user refresh asked for during a quiet run; it starts when that run ends.
    queued: Option<&'static str>,
    // The volume whose child runs now, and the volumes this pass has already taken.
    current: Option<String>,
    done: Vec<String>,
    // A pass started for newly added drives covers only those.
    only: Option<Vec<String>>,
    // Removed while their child may run; that child is killed before their rows are deleted.
    dropping: Vec<String>,
}

impl State {
    fn visible(&self) -> bool {
        self.refreshing && self.quiet_since.is_none_or(|t| t.elapsed() >= QUIET_FOR)
    }
}

#[derive(Default)]
struct Presence {
    posted: bool,
    streams: usize,
}

pub struct Supervisor {
    db: Db,
    exe: PathBuf,
    state: Mutex<State>,
    want_fast: AtomicBool,
    switch: Notify,
    change: Arc<watch::Sender<u64>>,
    rt: tokio::runtime::Handle,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    // Exists only while someone is looking, so an unwatched app has no timer at all.
    timer: Mutex<Option<tokio::task::JoinHandle<()>>>,
    journal_on: AtomicBool,
    presence: Mutex<Presence>,
    // Overlapping PUT /api/volumes calls would each judge the other's drives added or removed.
    pub volume_edits: tokio::sync::Mutex<()>,
    // Children join this job so a hard-killed server cannot orphan an indexer.
    job: Option<hoardini_win::KillOnCloseJob>,
    me: Weak<Supervisor>,
}

enum ChildEnd {
    Finished,
    SwitchedToFast,
    Dropped,
    NeedsFull(String),
}

enum Next {
    Volume(String),
    Again(&'static str),
    Finished,
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn norm_mode(mode: &str) -> Option<&'static str> {
    match mode {
        "background" => Some("background"),
        "fast" => Some("fast"),
        _ => None,
    }
}

// The Fast / Lean switch; Lean runs the indexer in background mode.
pub fn index_speed(conn: &Connection) -> &'static str {
    match get_meta(conn, "index_speed")
        .ok()
        .flatten()
        .as_deref()
        .map(str::trim)
    {
        Some("fast") => "fast",
        _ => "lean",
    }
}

// Time until the next journal refresh is due; an index that never completed is due now.
pub fn journal_wait(age: Option<Duration>, every: Duration) -> Duration {
    age.map_or(Duration::ZERO, |a| every.saturating_sub(a))
}

// Held by each open status stream; the last one closing counts as nobody looking.
pub struct Watcher(Weak<Supervisor>);

impl Drop for Watcher {
    fn drop(&mut self) {
        let Some(sup) = self.0.upgrade() else {
            return;
        };
        if let Ok(mut p) = sup.presence.lock() {
            p.streams = p.streams.saturating_sub(1);
            if p.streams == 0 {
                p.posted = false;
            }
        }
        sup.reschedule();
    }
}

impl Supervisor {
    pub fn new(
        db: Db,
        exe: PathBuf,
        change: Arc<watch::Sender<u64>>,
        rt: tokio::runtime::Handle,
    ) -> Arc<Supervisor> {
        let job = match hoardini_win::KillOnCloseJob::new() {
            Ok(j) => Some(j),
            Err(e) => {
                tracing::warn!(error = %e, "no job object; an indexer could outlive a crashed server");
                None
            }
        };
        Arc::new_cyclic(|me| Supervisor {
            db,
            exe,
            state: Mutex::new(State::default()),
            want_fast: AtomicBool::new(false),
            switch: Notify::new(),
            change,
            rt,
            task: Mutex::new(None),
            timer: Mutex::new(None),
            journal_on: AtomicBool::new(false),
            presence: Mutex::new(Presence::default()),
            volume_edits: tokio::sync::Mutex::new(()),
            job,
            me: me.clone(),
        })
    }

    fn bump(&self) {
        self.change.send_modify(|n| *n += 1);
    }

    pub fn snapshot(&self) -> Value {
        let (refreshing, mode, progress) = match self.state.lock() {
            Ok(s) if s.visible() => (true, s.mode, s.progress.clone()),
            _ => (false, None, None),
        };
        let (last, generation) = self
            .db
            .conn()
            .ok()
            .and_then(|c| index::index_status(&c, None).ok().flatten())
            .map(|s| (s.last_refresh_ms, s.generation))
            .unwrap_or((None, 0));
        json!({
            "state": if refreshing { "refreshing" } else { "idle" },
            "mode": mode,
            "progress": progress,
            "last_refresh_ms": last,
            "generation": generation,
        })
    }

    fn spawn_run(&self, mode: &'static str, kind: &'static str) -> anyhow::Result<()> {
        self.want_fast.store(false, Ordering::SeqCst);
        let Some(me) = self.me.upgrade() else {
            anyhow::bail!("supervisor is shutting down");
        };
        let handle = self.rt.spawn(async move { me.run(mode, kind).await });
        if let Ok(mut t) = self.task.lock() {
            *t = Some(handle);
        }
        Ok(())
    }

    // The very first full scan always runs fast; after that the Fast / Lean switch decides.
    pub fn auto_mode(&self) -> &'static str {
        let Ok(conn) = self.db.conn() else {
            return "background";
        };
        let never = index::index_status(&conn, None)
            .ok()
            .flatten()
            .is_none_or(|s| s.last_refresh_ms.is_none());
        if never || index_speed(&conn) == "fast" {
            "fast"
        } else {
            "background"
        }
    }

    // Fast takes over a Lean refresh; Lean during a Fast one lets it finish.
    pub fn speed_changed(&self, speed: &str) -> bool {
        if speed != "fast" {
            return false;
        }
        let Ok(mut st) = self.state.lock() else {
            return false;
        };
        if !st.refreshing {
            return false;
        }
        if st.queued.is_some() {
            st.queued = Some("fast");
            st.mode = Some("fast");
            drop(st);
            self.bump();
            return true;
        }
        // Killing a journal refresh would force the next one to be full; it is short, so it finishes.
        if st.quiet_since.is_some() || st.mode != Some("background") {
            return false;
        }
        st.mode = Some("fast");
        drop(st);
        self.want_fast.store(true, Ordering::SeqCst);
        self.switch.notify_one();
        self.bump();
        true
    }

    pub fn start_refresh(&self, mode: &str) -> anyhow::Result<bool> {
        let Some(mode) = norm_mode(mode) else {
            anyhow::bail!("mode must be \"background\" or \"fast\"");
        };
        let Ok(mut st) = self.state.lock() else {
            anyhow::bail!("indexer state unavailable");
        };
        if !st.refreshing {
            *st = State {
                refreshing: true,
                mode: Some(mode),
                ..State::default()
            };
            drop(st);
            self.spawn_run(mode, "auto")?;
            self.bump();
            return Ok(true);
        }
        // Killing a journal refresh would force the next one to be full, so the request waits for it.
        if st.quiet_since.is_some() || st.queued.is_some() {
            if st.queued == Some(mode) || st.queued == Some("fast") {
                return Ok(false);
            }
            st.queued = Some(mode);
            st.quiet_since = None;
            st.mode = Some(mode);
            drop(st);
            self.bump();
            return Ok(true);
        }
        if mode == "fast" && st.mode == Some("background") {
            st.mode = Some("fast");
            drop(st);
            self.want_fast.store(true, Ordering::SeqCst);
            self.switch.notify_one();
            self.bump();
            return Ok(true);
        }
        Ok(false)
    }

    fn start_quiet_journal(&self) {
        let mode = self.auto_mode();
        let Ok(mut st) = self.state.lock() else {
            return;
        };
        if st.refreshing {
            return;
        }
        *st = State {
            refreshing: true,
            mode: Some(mode),
            quiet_since: Some(Instant::now()),
            ..State::default()
        };
        drop(st);
        if let Err(e) = self.spawn_run(mode, "journal") {
            tracing::warn!(error = %e, "journal refresh did not start");
        }
    }

    fn journal_every(&self) -> Duration {
        let s = self
            .db
            .conn()
            .ok()
            .and_then(|c| get_meta(&c, "journal_every_s").ok().flatten())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|s| *s > 0)
            .unwrap_or(JOURNAL_EVERY_S);
        Duration::from_secs(s)
    }

    fn index_age(&self) -> Option<Duration> {
        let conn = self.db.conn().ok()?;
        let age = index::index_status(&conn, None).ok()??.age_ms?;
        Some(Duration::from_millis(u64::try_from(age).unwrap_or(0)))
    }

    // The change journal keeps the index current between full refreshes, but only while someone looks.
    pub fn enable_journal(&self) {
        self.journal_on.store(true, Ordering::SeqCst);
        self.reschedule();
    }

    pub fn set_looking(&self, looking: bool) {
        if let Ok(mut p) = self.presence.lock() {
            p.posted = looking;
        }
        self.reschedule();
    }

    pub fn watch(&self) -> Watcher {
        if let Ok(mut p) = self.presence.lock() {
            p.streams += 1;
        }
        self.reschedule();
        Watcher(self.me.clone())
    }

    fn looking(&self) -> bool {
        self.presence
            .lock()
            .is_ok_and(|p| p.posted && p.streams > 0)
    }

    // Only the timer stops when nobody looks; a refresh it already started runs to the end.
    fn reschedule(&self) {
        let Ok(mut t) = self.timer.lock() else {
            return;
        };
        let want = self.journal_on.load(Ordering::SeqCst) && self.looking();
        let running = t.as_ref().is_some_and(|h| !h.is_finished());
        if want && !running {
            *t = Some(self.spawn_journal_timer());
        } else if !want && let Some(h) = t.take() {
            h.abort();
        }
    }

    fn spawn_journal_timer(&self) -> tokio::task::JoinHandle<()> {
        let me = self.me.clone();
        self.rt.spawn(async move {
            loop {
                let Some(sup) = me.upgrade() else {
                    break;
                };
                let every = sup.journal_every();
                let wait = journal_wait(sup.index_age(), every);
                if wait.is_zero() {
                    sup.start_quiet_journal();
                }
                drop(sup);
                tokio::time::sleep(if wait.is_zero() { every } else { wait }).await;
            }
        })
    }

    // Aborting the task drops the child handle, and kill_on_drop ends the process.
    pub fn shutdown(&self) {
        self.journal_on.store(false, Ordering::SeqCst);
        for slot in [&self.timer, &self.task] {
            if let Ok(mut t) = slot.lock()
                && let Some(h) = t.take()
            {
                h.abort();
            }
        }
    }

    // Picking the next volume and ending the pass share one lock, so a drive added meanwhile is never missed.
    fn next_step(&self) -> Next {
        let Ok(mut st) = self.state.lock() else {
            return Next::Finished;
        };
        let watched = self
            .db
            .conn()
            .map(|c| index::watched(&c))
            .unwrap_or_default();
        let pick = watched.into_iter().find(|w| {
            !st.done.iter().any(|d| same(d, w))
                && st
                    .only
                    .as_ref()
                    .is_none_or(|o| o.iter().any(|x| same(x, w)))
        });
        if let Some(v) = pick {
            st.done.push(v.clone());
            st.current = Some(v.clone());
            return Next::Volume(v);
        }
        st.current = None;
        match st.queued.take() {
            Some(m) => {
                st.progress = None;
                st.done.clear();
                st.only = None;
                Next::Again(m)
            }
            None => {
                *st = State::default();
                Next::Finished
            }
        }
    }

    fn dropping(&self, volume: &str) -> bool {
        self.state
            .lock()
            .is_ok_and(|st| st.dropping.iter().any(|d| same(d, volume)))
    }

    fn child_mode(&self, volume: &str, mode: &'static str) -> &'static str {
        let first = self
            .db
            .conn()
            .is_ok_and(|c| index::never_indexed(&c, volume));
        let m = if first { "fast" } else { mode };
        if let Ok(mut st) = self.state.lock()
            && st.queued.is_none()
        {
            st.mode = Some(m);
        }
        m
    }

    async fn run(self: Arc<Self>, first_mode: &'static str, first_kind: &'static str) {
        let (mut mode, mut kind) = (first_mode, first_kind);
        loop {
            let volume = match self.next_step() {
                Next::Volume(v) => v,
                Next::Again(m) => {
                    (mode, kind) = (m, "auto");
                    self.bump();
                    continue;
                }
                Next::Finished => break,
            };
            // An unplugged drive keeps its index until it is back.
            if !index::connected(&volume) {
                tracing::info!(volume, "volume not connected; skipped");
                continue;
            }
            let mut k = kind;
            loop {
                if self.dropping(&volume) {
                    break;
                }
                if self.want_fast.swap(false, Ordering::SeqCst) {
                    mode = "fast";
                }
                let m = self.child_mode(&volume, mode);
                match self.run_child(&volume, m, k).await {
                    Ok(ChildEnd::SwitchedToFast) => continue,
                    Ok(ChildEnd::NeedsFull(reason)) if k != "full" => {
                        tracing::info!(
                            volume,
                            reason,
                            "journal refresh not possible; running a full refresh"
                        );
                        k = "full";
                        continue;
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(volume, error = %e, "indexer child failed"),
                }
                break;
            }
        }
        self.bump();
    }

    // Starts the first scan of newly watched roots, or folds them into the pass already running.
    pub fn volumes_added(&self, roots: &[String]) -> anyhow::Result<()> {
        if roots.is_empty() {
            return Ok(());
        }
        let Ok(mut st) = self.state.lock() else {
            anyhow::bail!("indexer state unavailable");
        };
        st.dropping.retain(|d| !roots.iter().any(|r| same(r, d)));
        if st.refreshing {
            st.done.retain(|d| !roots.iter().any(|r| same(r, d)));
            if let Some(only) = st.only.as_mut() {
                only.extend(roots.iter().cloned());
            }
            st.quiet_since = None;
            drop(st);
            self.bump();
            return Ok(());
        }
        *st = State {
            refreshing: true,
            mode: Some("fast"),
            only: Some(roots.to_vec()),
            ..State::default()
        };
        drop(st);
        self.spawn_run("fast", "auto")?;
        self.bump();
        Ok(())
    }

    // Returns once no child runs on these roots; the caller then deletes their rows.
    pub async fn release(&self, roots: &[String]) -> bool {
        let busy = || {
            self.state.lock().is_ok_and(|st| {
                st.current
                    .as_ref()
                    .is_some_and(|c| roots.iter().any(|r| same(r, c)))
            })
        };
        if let Ok(mut st) = self.state.lock() {
            st.dropping.extend(roots.iter().cloned());
        }
        self.switch.notify_one();
        let deadline = Instant::now() + Duration::from_secs(30);
        while busy() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let released = !busy();
        if let Ok(mut st) = self.state.lock() {
            st.dropping.retain(|d| !roots.iter().any(|r| same(r, d)));
        }
        released
    }

    async fn run_child(&self, volume: &str, mode: &str, kind: &str) -> anyhow::Result<ChildEnd> {
        let mut cmd = Command::new(&self.exe);
        // --mode stays the 7th argument; the SRV-ONE-REFRESH fake indexer reads it positionally.
        cmd.arg("index")
            .arg("--db")
            .arg(self.db.path())
            .arg("--volume")
            .arg(volume)
            .arg("--mode")
            .arg(mode)
            .arg("--kind")
            .arg(kind)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = cmd.spawn()?;
        if let (Some(job), Some(h)) = (&self.job, child.raw_handle())
            && let Err(e) = job.assign(h)
        {
            tracing::warn!(error = %e, "indexer not in the kill-on-close job");
        }
        let Some(stdout) = child.stdout.take() else {
            anyhow::bail!("indexer stdout unavailable");
        };
        let mut lines = BufReader::new(stdout).lines();
        let mut needs_full = None;
        loop {
            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(l)) => {
                        if let Some(reason) = self.on_line(&l) {
                            needs_full = Some(reason);
                        }
                    }
                    Ok(None) | Err(_) => break,
                },
                _ = self.switch.notified() => {
                    if self.dropping(volume) {
                        let _ = child.kill().await;
                        return Ok(ChildEnd::Dropped);
                    }
                    if self.want_fast.load(Ordering::SeqCst) && mode != "fast" {
                        let _ = child.kill().await;
                        return Ok(ChildEnd::SwitchedToFast);
                    }
                }
            }
        }
        let status = child.wait().await?;
        if needs_full.is_some() || status.code() == Some(NEEDS_FULL_EXIT) {
            return Ok(ChildEnd::NeedsFull(needs_full.unwrap_or_default()));
        }
        if !status.success() {
            tracing::warn!(volume, code = ?status.code(), "indexer exited with failure");
        }
        Ok(ChildEnd::Finished)
    }

    // Returns the reason when the child reports that only a full refresh can run.
    fn on_line(&self, line: &str) -> Option<String> {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return None;
        };
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        match v.get("type").and_then(Value::as_str) {
            Some("progress") => {
                let num = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
                let p = json!({
                    "phase": text("phase"),
                    "dirs": num("dirs"),
                    "files": num("files"),
                    "bytes": num("bytes"),
                    "fraction": v.get("fraction").and_then(Value::as_f64),
                });
                if let Ok(mut st) = self.state.lock() {
                    st.progress = Some(p);
                }
                self.bump();
            }
            Some("start") => self.bump(),
            Some("done") => {
                tracing::info!(
                    kind = text("kind"),
                    fallback = ?v.get("fallback"),
                    duration_ms = ?v.get("duration_ms"),
                    dirty_dirs = ?v.get("dirty_dirs"),
                    "index refresh done"
                );
                self.bump();
            }
            Some("needs_full") => return Some(text("reason")),
            Some("error") => tracing::warn!(message = text("message"), "indexer reported an error"),
            _ => {}
        }
        None
    }
}

impl IndexControl for Supervisor {
    fn status(&self) -> Value {
        self.snapshot()
    }
    fn refresh(&self, mode: &str) -> anyhow::Result<bool> {
        self.start_refresh(mode)
    }
}

use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TryRecvError, bounded};
use hoardini_win::{DirEntry, VolumeInfo};
use parking_lot::{Condvar, Mutex};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::dedupe::SeenSet;
use super::lister::{DirLister, list_fixed};
use super::writer::{Chain, Dedupe, Work, Writer, load_chain};
use super::{KIND_DIR, normalize_path, threshold_setting, to_i64, to_u64};
use crate::history::{self, ScanRecord};
use crate::store::now_ms;

const HOTSPOT_DIRS: usize = 16;
const COMMIT_EVERY: Duration = Duration::from_secs(1);
const IDLE_COMMIT_AFTER: Duration = Duration::from_millis(50);
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Background,
    Fast,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Background => "background",
            Mode::Fast => "fast",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Progress {
    pub phase: &'static str,
    pub dirs: u64,
    pub files: u64,
    pub bytes: u64,
    pub fraction: Option<f64>,
}

pub enum Event<'a> {
    Start {
        volume: &'a str,
        generation: i64,
        mode: Mode,
    },
    Progress(&'a Progress),
}

pub struct RefreshOptions<'a> {
    pub mode: Mode,
    pub workers: usize,
    pub threshold: Option<u64>,
    pub hotspots_first: bool,
    pub batch_rows: usize,
    pub cancel: Option<Arc<AtomicBool>>,
    pub on_event: Option<&'a dyn Fn(Event<'_>)>,
}

impl Default for RefreshOptions<'_> {
    fn default() -> Self {
        RefreshOptions {
            mode: Mode::Background,
            workers: 2,
            threshold: None,
            hotspots_first: true,
            batch_rows: 5000,
            cancel: None,
            on_event: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RefreshOutcome {
    pub volume_id: i64,
    pub root: String,
    pub generation: i64,
    pub completed: bool,
    pub scan_id: Option<i64>,
    pub duration_ms: i64,
    pub dirs: u64,
    pub files: u64,
    pub denied: u64,
    pub rows_deleted: u64,
    pub threshold: u64,
    pub indexed_bytes: u64,
    pub total_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
    pub unaccounted_bytes: Option<i64>,
}

struct Listed {
    work: Work,
    entries: io::Result<Vec<DirEntry>>,
}

#[derive(Default)]
struct QState {
    hot: VecDeque<Work>,
    stack: Vec<Work>,
    rest: Vec<Work>,
    outstanding: usize,
    closed: bool,
}

// LIFO keeps the pending set near depth x fanout instead of holding a whole tree level.
#[derive(Default)]
struct WorkQueue {
    state: Mutex<QState>,
    cv: Condvar,
}

impl WorkQueue {
    fn push(&self, w: Work) {
        let mut s = self.state.lock();
        s.stack.push(w);
        s.outstanding += 1;
        self.cv.notify_one();
    }

    fn push_hot(&self, w: Work) {
        let mut s = self.state.lock();
        s.hot.push_back(w);
        s.outstanding += 1;
        self.cv.notify_one();
    }

    // Waits until every hotspot subtree is done before it hands out the rest of the volume.
    fn push_rest(&self, w: Work) {
        let mut s = self.state.lock();
        s.rest.push(w);
        s.outstanding += 1;
        self.cv.notify_one();
    }

    fn pop(&self) -> Option<Work> {
        let mut s = self.state.lock();
        loop {
            if s.closed {
                return None;
            }
            if let Some(w) = s.hot.pop_front() {
                return Some(w);
            }
            if let Some(w) = s.stack.pop() {
                return Some(w);
            }
            if s.outstanding == s.rest.len()
                && let Some(w) = s.rest.pop()
            {
                return Some(w);
            }
            if s.outstanding == 0 {
                return None;
            }
            self.cv.wait(&mut s);
        }
    }

    fn done_one(&self) {
        let mut s = self.state.lock();
        s.outstanding = s.outstanding.saturating_sub(1);
        if s.outstanding <= s.rest.len() {
            self.cv.notify_all();
        }
    }

    fn outstanding(&self) -> usize {
        self.state.lock().outstanding
    }

    fn close(&self) {
        self.state.lock().closed = true;
        self.cv.notify_all();
    }
}

fn worker_loop(q: &WorkQueue, lister: &dyn DirLister, tx: Sender<Listed>, cancel: &AtomicBool) {
    while let Some(work) = q.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let entries = list_fixed(lister, &work.path);
        if tx.send(Listed { work, entries }).is_err() {
            break;
        }
    }
}

struct Begin {
    volume_id: i64,
    root_id: i64,
    generation: i64,
    prev_dirs: Option<u64>,
}

fn begin_generation(conn: &Connection, root: &str, vinfo: Option<&VolumeInfo>) -> Result<Begin> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let r = (|| -> Result<Begin> {
        conn.execute(
            "INSERT INTO volume(root) VALUES (?1) ON CONFLICT(root) DO NOTHING",
            [root],
        )?;
        let (volume_id, root_node, generation): (i64, Option<i64>, i64) = conn.query_row(
            "SELECT id, root_node_id, gen FROM volume WHERE root = ?1",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let generation = generation + 1;
        let existing = match root_node {
            Some(id) => conn
                .query_row("SELECT id FROM node WHERE id = ?1", [id], |r| {
                    r.get::<_, i64>(0)
                })
                .optional()?,
            None => None,
        };
        let root_id = match existing {
            Some(id) => id,
            None => {
                conn.execute(
                    "INSERT INTO node(parent_id, volume_id, name, kind, seen_gen) VALUES (NULL, ?1, ?2, ?3, ?4)",
                    params![volume_id, root, KIND_DIR, generation],
                )?;
                conn.last_insert_rowid()
            }
        };
        conn.execute(
            "UPDATE node SET seen_gen = ?1 WHERE id = ?2",
            params![generation, root_id],
        )?;
        conn.execute(
            "UPDATE volume SET gen = ?1, root_node_id = ?2, serial = COALESCE(?3, serial), \
             fs_name = COALESCE(?4, fs_name), label = COALESCE(?5, label) WHERE id = ?6",
            params![
                generation,
                root_id,
                vinfo.map(|v| to_i64(v.serial)),
                vinfo.map(|v| v.fs_name.clone()),
                vinfo.map(|v| v.label.clone()),
                volume_id
            ],
        )?;
        let prev_dirs: Option<i64> = conn
            .query_row(
                "SELECT dir_count FROM scan WHERE volume_id = ?1 ORDER BY finished_ms DESC, id DESC LIMIT 1",
                [volume_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(Begin {
            volume_id,
            root_id,
            generation,
            prev_dirs: prev_dirs.map(to_u64),
        })
    })();
    match r {
        Ok(b) => {
            conn.execute_batch("COMMIT")?;
            Ok(b)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

struct LoopCtx<'a, 'o> {
    q: &'a WorkQueue,
    cancel: &'a AtomicBool,
    opts: &'a RefreshOptions<'o>,
    prev_dirs: Option<u64>,
    root_id: i64,
    has_hot: bool,
}

fn emit_progress(w: &Writer, ctx: &LoopCtx, phase: &'static str) {
    if let Some(cb) = ctx.opts.on_event {
        let fraction = ctx
            .prev_dirs
            .filter(|&d| d > 0)
            .map(|d| (w.stats.dirs as f64 / d as f64).min(0.99));
        cb(Event::Progress(&Progress {
            phase,
            dirs: w.stats.dirs,
            files: w.stats.files,
            bytes: w.stats.bytes,
            fraction,
        }));
    }
}

fn writer_loop(w: &mut Writer, rx: Receiver<Listed>, ctx: &LoopCtx) -> Result<bool> {
    let mut last_commit = Instant::now();
    let mut last_progress = Instant::now();
    let mut root_done = false;
    let mut subdirs = Vec::new();
    loop {
        if ctx.q.outstanding() == 0 {
            return Ok(true);
        }
        if ctx.cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let listed = match rx.try_recv() {
            Ok(l) => l,
            Err(TryRecvError::Empty) => {
                // Release the write lock while idle so the app's patches are not starved.
                if w.in_tx() && last_commit.elapsed() >= IDLE_COMMIT_AFTER {
                    w.commit()?;
                    last_commit = Instant::now();
                }
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(l) => l,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => {
                        bail!("indexer workers stopped unexpectedly")
                    }
                }
            }
            Err(TryRecvError::Disconnected) => bail!("indexer workers stopped unexpectedly"),
        };
        if listed.work.node_id == ctx.root_id
            && let Err(e) = &listed.entries
        {
            bail!("cannot list {}: {e}", listed.work.path);
        }
        w.apply_listing(&listed.work, listed.entries, &mut subdirs)?;
        if listed.work.node_id == ctx.root_id {
            root_done = true;
        }
        for s in subdirs.drain(..) {
            ctx.q.push(s);
        }
        ctx.q.done_one();
        if w.writes >= ctx.opts.batch_rows || last_commit.elapsed() >= COMMIT_EVERY {
            w.commit()?;
            last_commit = Instant::now();
        }
        if last_progress.elapsed() >= PROGRESS_EVERY {
            let phase = if ctx.has_hot && !root_done {
                "hotspots"
            } else {
                "walk"
            };
            emit_progress(w, ctx, phase);
            last_progress = Instant::now();
        }
    }
}

pub fn refresh(
    conn: &Connection,
    lister: &dyn DirLister,
    root: &str,
    opts: &RefreshOptions,
) -> Result<RefreshOutcome> {
    let started_ms = now_ms();
    let t0 = Instant::now();
    let root = normalize_path(root);
    let vinfo = lister.volume_info(Path::new(&root)).ok();
    let threshold = match opts.threshold {
        Some(t) => t,
        None => threshold_setting(conn)?,
    };
    let begin = begin_generation(conn, &root, vinfo.as_ref())?;
    if let Some(cb) = opts.on_event {
        cb(Event::Start {
            volume: &root,
            generation: begin.generation,
            mode: opts.mode,
        });
    }
    let ntfs = vinfo
        .as_ref()
        .is_some_and(|v| v.fs_name.eq_ignore_ascii_case("NTFS"));
    let mut w = Writer::new(
        conn,
        begin.volume_id,
        begin.generation,
        threshold,
        Dedupe::Walk(SeenSet::new(ntfs)),
    );
    let queue = WorkQueue::default();
    if opts.hotspots_first {
        for path in history::hotspot_dirs(conn, begin.volume_id, HOTSPOT_DIRS)? {
            let Some(id) = super::node_id_by_path(conn, &path)? else {
                continue;
            };
            if id == begin.root_id || w.hot_ids.contains(&id) {
                continue;
            }
            let kind: i64 =
                conn.query_row("SELECT kind FROM node WHERE id = ?1", [id], |r| r.get(0))?;
            if kind != KIND_DIR {
                continue;
            }
            let chain = load_chain(conn, id)?;
            queue.push_hot(Work {
                node_id: id,
                path: super::path_of(conn, id)?.unwrap_or(path),
                chain,
                is_new: false,
            });
            w.hot_ids.insert(id);
        }
    }
    queue.push_rest(Work {
        node_id: begin.root_id,
        path: root.clone(),
        chain: Chain::root(begin.root_id),
        is_new: false,
    });

    let workers = opts.workers.max(1);
    let local_cancel = Arc::new(AtomicBool::new(false));
    let cancel = opts.cancel.as_ref().unwrap_or(&local_cancel);
    let ctx = LoopCtx {
        q: &queue,
        cancel,
        opts,
        prev_dirs: begin.prev_dirs,
        root_id: begin.root_id,
        has_hot: !w.hot_ids.is_empty(),
    };
    let (tx, rx) = bounded::<Listed>(workers * 4);
    let walked = std::thread::scope(|s| {
        for _ in 0..workers {
            let tx = tx.clone();
            let q = &queue;
            let c: &AtomicBool = cancel;
            s.spawn(move || worker_loop(q, lister, tx, c));
        }
        drop(tx);
        let r = writer_loop(&mut w, rx, &ctx);
        queue.close();
        r
    });
    let completed = match walked {
        Ok(c) => c,
        Err(e) => {
            w.rollback();
            return Err(e);
        }
    };
    w.commit()?;

    let mut out = RefreshOutcome {
        volume_id: begin.volume_id,
        root: root.clone(),
        generation: begin.generation,
        completed,
        scan_id: None,
        duration_ms: 0,
        dirs: w.stats.dirs,
        files: w.stats.files,
        denied: w.stats.denied,
        rows_deleted: 0,
        threshold,
        indexed_bytes: 0,
        total_bytes: vinfo.as_ref().map(|v| v.total_bytes),
        free_bytes: vinfo.as_ref().map(|v| v.free_bytes),
        unaccounted_bytes: None,
    };
    if !completed {
        out.indexed_bytes = root_alloc(conn, begin.root_id)?;
        out.duration_ms = t0.elapsed().as_millis() as i64;
        return Ok(out);
    }

    emit_progress(&w, &ctx, "finalize");
    w.ensure_tx()?;
    let fin = (|| -> Result<()> {
        w.resolve_groups(false)?;
        out.rows_deleted = w.delete_unseen()?;
        w.flush_deltas()?;
        out.indexed_bytes = root_alloc(conn, begin.root_id)?;
        let same_volume = vinfo
            .as_ref()
            .is_some_and(|v| normalize_path(&v.root).eq_ignore_ascii_case(&root));
        out.unaccounted_bytes = vinfo.as_ref().filter(|_| same_volume).map(|v| {
            to_i64(v.total_bytes.saturating_sub(v.free_bytes)) - to_i64(out.indexed_bytes)
        });
        let finished_ms = now_ms();
        conn.execute(
            "UPDATE volume SET threshold = ?1, last_refresh_ms = ?2, total_bytes = COALESCE(?3, total_bytes), \
             free_bytes = COALESCE(?4, free_bytes) WHERE id = ?5",
            params![
                to_i64(threshold),
                finished_ms,
                out.total_bytes.map(to_i64),
                out.free_bytes.map(to_i64),
                begin.volume_id
            ],
        )?;
        let files: i64 = conn.query_row(
            "SELECT sub_count FROM node WHERE id = ?1",
            [begin.root_id],
            |r| r.get(0),
        )?;
        out.scan_id = Some(history::record_scan(
            conn,
            &ScanRecord {
                volume_id: begin.volume_id,
                generation: begin.generation,
                kind: "full".into(),
                mode: opts.mode.as_str().into(),
                started_ms,
                finished_ms,
                total_bytes: out.total_bytes,
                free_bytes: out.free_bytes,
                indexed_bytes: out.indexed_bytes,
                unaccounted_bytes: out.unaccounted_bytes,
                file_count: to_u64(files),
                dir_count: w.stats.dirs,
                denied_count: w.stats.denied,
            },
        )?);
        Ok(())
    })();
    if let Err(e) = fin {
        w.rollback();
        return Err(e);
    }
    w.commit()?;
    out.duration_ms = t0.elapsed().as_millis() as i64;
    Ok(out)
}

fn root_alloc(conn: &Connection, root_id: i64) -> Result<u64> {
    let v: i64 = conn.query_row("SELECT sub_alloc FROM node WHERE id = ?1", [root_id], |r| {
        r.get(0)
    })?;
    Ok(to_u64(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::FakeFs;

    #[test]
    fn unreadable_root_fails_without_a_scan() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::store::Db::open(dir.path().join("w.db")).unwrap();
        let conn = db.conn().unwrap();
        let fs = FakeFs::new(r"C:\");
        fs.file(r"C:\a.bin", 1 << 20);
        let err = refresh(&conn, &fs, r"C:\missing", &RefreshOptions::default()).unwrap_err();
        assert!(err.to_string().contains(r"C:\missing"), "{err}");
        let scans: i64 = conn
            .query_row("SELECT COUNT(*) FROM scan", [], |r| r.get(0))
            .unwrap();
        assert_eq!(scans, 0);
    }
}

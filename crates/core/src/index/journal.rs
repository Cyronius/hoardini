use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use hoardini_win::{DirEntry, FILE_ATTRIBUTE_DIRECTORY, UsnJournal, UsnRecord};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::lister::{DirLister, list_fixed};
use super::walk::root_alloc;
use super::writer::{Chain, Dedupe, Delta, Work, Writer, classify, load_chain};
use super::{
    DEFAULT_THRESHOLD, Event, FLAG_DENIED, FileId, KIND_DIR, KIND_FILE, Progress, RefreshOptions,
    RefreshOutcome, is_ancestor_path, join_path, normalize_path, path_of, refresh, split_parent,
    to_i64, to_u64,
};
use crate::history::{self, ScanRecord};
use crate::store::now_ms;

pub(crate) const USN_REASON_DATA_OVERWRITE: u32 = 0x1;
pub(crate) const USN_REASON_DATA_EXTEND: u32 = 0x2;
pub(crate) const USN_REASON_DATA_TRUNCATION: u32 = 0x4;
pub(crate) const USN_REASON_FILE_CREATE: u32 = 0x100;
pub(crate) const USN_REASON_FILE_DELETE: u32 = 0x200;
pub(crate) const USN_REASON_SECURITY_CHANGE: u32 = 0x800;
pub(crate) const USN_REASON_RENAME_OLD_NAME: u32 = 0x1000;
pub(crate) const USN_REASON_RENAME_NEW_NAME: u32 = 0x2000;
pub(crate) const USN_REASON_BASIC_INFO_CHANGE: u32 = 0x8000;
pub(crate) const USN_REASON_HARD_LINK_CHANGE: u32 = 0x1_0000;
pub(crate) const USN_REASON_REPARSE_POINT_CHANGE: u32 = 0x10_0000;
pub(crate) const USN_REASON_CLOSE: u32 = 0x8000_0000;

pub(crate) const ERROR_JOURNAL_NOT_ACTIVE: i32 = 1179;
pub(crate) const ERROR_JOURNAL_ENTRY_DELETED: i32 = 1181;
const ERROR_INVALID_PARAMETER: i32 = 87;
const ERROR_JOURNAL_DELETE_IN_PROGRESS: i32 = 1178;

const RENAMED: u32 = USN_REASON_RENAME_OLD_NAME | USN_REASON_RENAME_NEW_NAME;
// These change which entries a directory holds, and with that its own times in its parent's listing.
const STRUCTURAL: u32 =
    USN_REASON_FILE_CREATE | USN_REASON_FILE_DELETE | RENAMED | USN_REASON_HARD_LINK_CHANGE;

const COMMIT_EVERY: Duration = Duration::from_secs(1);
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
// One history entry per day of journal refreshes keeps retention and the hotspot window meaningful.
const COALESCE_MS: i64 = 86_400_000;
const NO_ID: [u8; 16] = [0; 16];

pub trait JournalSource {
    fn query(&self) -> io::Result<UsnJournal>;
    fn read(&self, journal_id: u64, from: i64) -> io::Result<(Vec<UsnRecord>, i64)>;
    fn path_by_file_id(&self, id: [u8; 16]) -> io::Result<String>;
    fn final_path(&self, path: &str) -> io::Result<String> {
        Ok(normalize_path(path))
    }
}

#[derive(Clone, Debug)]
pub struct WinJournal {
    volume: PathBuf,
}

impl WinJournal {
    pub fn for_path(path: &str) -> io::Result<WinJournal> {
        Ok(WinJournal {
            volume: hoardini_win::volume_root_of(Path::new(path))?,
        })
    }
}

impl JournalSource for WinJournal {
    fn query(&self) -> io::Result<UsnJournal> {
        hoardini_win::usn_query(&self.volume)
    }

    fn read(&self, journal_id: u64, from: i64) -> io::Result<(Vec<UsnRecord>, i64)> {
        hoardini_win::usn_read(&self.volume, journal_id, from)
    }

    fn path_by_file_id(&self, id: [u8; 16]) -> io::Result<String> {
        let p = hoardini_win::path_by_file_id(&self.volume, id)?;
        Ok(normalize_path(&p.to_string_lossy()))
    }

    // Records resolve to final paths, so an index root given in another spelling must be compared as one.
    fn final_path(&self, path: &str) -> io::Result<String> {
        Ok(normalize_path(
            &std::fs::canonicalize(path)?.to_string_lossy(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NeedsFull {
    NoPosition,
    NoJournal,
    JournalChanged,
    Wrapped,
    ThresholdChanged,
}

impl NeedsFull {
    pub fn as_str(&self) -> &'static str {
        match self {
            NeedsFull::NoPosition => "no_position",
            NeedsFull::NoJournal => "no_journal",
            NeedsFull::JournalChanged => "journal_changed",
            NeedsFull::Wrapped => "wrapped",
            NeedsFull::ThresholdChanged => "threshold_changed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RefreshKind {
    Auto,
    Full,
    Journal,
}

impl RefreshKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RefreshKind::Auto => "auto",
            RefreshKind::Full => "full",
            RefreshKind::Journal => "journal",
        }
    }
}

impl FromStr for RefreshKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<RefreshKind> {
        match s {
            "auto" => Ok(RefreshKind::Auto),
            "full" => Ok(RefreshKind::Full),
            "journal" => Ok(RefreshKind::Journal),
            other => bail!("unknown refresh kind {other:?}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JournalRefresh {
    pub outcome: RefreshOutcome,
    pub from_usn: i64,
    pub next_usn: i64,
    pub records: u64,
    pub pages: u64,
    pub dirty_dirs: u64,
    pub moved_dirs: u64,
    pub new_dirs: u64,
    pub resolved_ids: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum JournalOutcome {
    Done(Box<JournalRefresh>),
    NeedsFull(NeedsFull),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum RefreshRun {
    Full {
        outcome: RefreshOutcome,
        fallback: Option<NeedsFull>,
    },
    Journal(JournalRefresh),
    NeedsFull(NeedsFull),
}

pub fn refresh_kind(
    conn: &Connection,
    lister: &dyn DirLister,
    root: &str,
    kind: RefreshKind,
    opts: &RefreshOptions,
) -> Result<RefreshRun> {
    if kind == RefreshKind::Full {
        return Ok(RefreshRun::Full {
            outcome: refresh(conn, lister, root, opts)?,
            fallback: None,
        });
    }
    let why = match opts.journal {
        None => NeedsFull::NoJournal,
        Some(_) if kind == RefreshKind::Auto && threshold_changed(conn, root, opts.threshold)? => {
            NeedsFull::ThresholdChanged
        }
        Some(src) => match journal_refresh(conn, lister, src, root, opts)? {
            JournalOutcome::Done(j) => return Ok(RefreshRun::Journal(*j)),
            JournalOutcome::NeedsFull(why) => why,
        },
    };
    if kind == RefreshKind::Journal {
        return Ok(RefreshRun::NeedsFull(why));
    }
    Ok(RefreshRun::Full {
        outcome: refresh(conn, lister, root, opts)?,
        fallback: Some(why),
    })
}

// A journal refresh keeps the stored threshold; only a full refresh can apply a new one.
fn threshold_changed(conn: &Connection, root: &str, over: Option<u64>) -> Result<bool> {
    let stored: Option<i64> = conn
        .query_row(
            "SELECT threshold FROM volume WHERE root = ?1 COLLATE NOCASE",
            [normalize_path(root)],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let want = over.unwrap_or(DEFAULT_THRESHOLD);
    Ok(stored.is_some_and(|s| to_u64(s) != want))
}

struct Vol {
    id: i64,
    root: String,
    root_id: i64,
    generation: i64,
    threshold: u64,
    journal_id: u64,
    next_usn: i64,
}

fn load_volume(conn: &Connection, root: &str) -> Result<Option<Vol>> {
    type Raw = (
        i64,
        String,
        Option<i64>,
        i64,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    );
    let row: Option<Raw> = conn
        .query_row(
            "SELECT id, root, root_node_id, gen, threshold, usn_journal_id, usn_next FROM volume \
             WHERE root = ?1 COLLATE NOCASE",
            [root],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()?;
    let Some((id, root, Some(root_id), generation, Some(t), Some(jid), Some(next))) = row else {
        return Ok(None);
    };
    let exists = conn
        .query_row("SELECT 1 FROM node WHERE id = ?1", [root_id], |_| Ok(()))
        .optional()?
        .is_some();
    Ok(exists.then_some(Vol {
        id,
        root,
        root_id,
        generation,
        threshold: to_u64(t),
        journal_id: jid as u64,
        next_usn: next,
    }))
}

fn forget_position(conn: &Connection, volume_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE volume SET usn_journal_id = NULL, usn_next = NULL WHERE id = ?1",
        [volume_id],
    )?;
    Ok(())
}

#[derive(Default)]
struct Seen {
    reasons: u32,
    last: u32,
    dir: bool,
}

#[derive(Default)]
struct Kids {
    reasons: u32,
    ids: Vec<[u8; 16]>,
}

#[derive(Default)]
struct Records {
    files: HashMap<[u8; 16], Seen>,
    parents: HashMap<[u8; 16], Kids>,
    count: u64,
    pages: u64,
    next: i64,
}

impl Records {
    fn add(&mut self, r: &UsnRecord) {
        self.count += 1;
        let s = self.files.entry(r.file_id).or_default();
        s.reasons |= r.reason;
        s.last = r.reason;
        s.dir |= r.attrs & FILE_ATTRIBUTE_DIRECTORY != 0;
        let k = self.parents.entry(r.parent_id).or_default();
        k.reasons |= r.reason;
        if k.ids.last() != Some(&r.file_id) {
            k.ids.push(r.file_id);
        }
    }
}

enum Read {
    Records(Records),
    Cancelled,
    Unusable(NeedsFull),
}

fn unusable(e: &io::Error) -> Option<NeedsFull> {
    match e.raw_os_error()? {
        ERROR_JOURNAL_ENTRY_DELETED => Some(NeedsFull::Wrapped),
        ERROR_JOURNAL_DELETE_IN_PROGRESS | ERROR_JOURNAL_NOT_ACTIVE => {
            Some(NeedsFull::JournalChanged)
        }
        ERROR_INVALID_PARAMETER => Some(NeedsFull::NoPosition),
        _ => None,
    }
}

fn read_records(
    src: &dyn JournalSource,
    journal_id: u64,
    from: i64,
    until: i64,
    cancel: &AtomicBool,
) -> Result<Read> {
    let mut recs = Records {
        next: from,
        ..Records::default()
    };
    while recs.next < until {
        if cancel.load(Ordering::Relaxed) {
            return Ok(Read::Cancelled);
        }
        let (page, next) = match src.read(journal_id, recs.next) {
            Ok(v) => v,
            Err(e) => {
                return match unusable(&e) {
                    Some(why) => Ok(Read::Unusable(why)),
                    None => Err(anyhow::Error::new(e).context("reading the change journal")),
                };
            }
        };
        recs.pages += 1;
        for r in &page {
            recs.add(r);
        }
        if next <= recs.next {
            break;
        }
        recs.next = next;
    }
    for k in recs.parents.values_mut() {
        k.ids.sort_unstable();
        k.ids.dedup();
    }
    Ok(Read::Records(recs))
}

fn depth(path: &str) -> usize {
    path.split('\\').filter(|c| !c.is_empty()).count()
}

fn chain_has(chain: &Chain, id: i64) -> bool {
    let mut cur = Some(chain);
    while let Some(c) = cur {
        if c.id == id {
            return true;
        }
        cur = c.parent.as_deref();
    }
    false
}

struct Run<'a> {
    conn: &'a Connection,
    lister: &'a dyn DirLister,
    source: &'a dyn JournalSource,
    cancel: &'a AtomicBool,
    on_event: Option<&'a dyn Fn(Event<'_>)>,
    batch_rows: usize,
    root: String,
    canon_root: String,
    root_id: i64,
    recs: Records,
    dirty: HashSet<i64>,
    structural: HashSet<i64>,
    kid_parents: HashMap<i64, Vec<[u8; 16]>>,
    moved: u64,
    new_dirs: u64,
    planned: u64,
    resolved: u64,
    listing_from: Option<Instant>,
    last_commit: Instant,
    last_progress: Instant,
}

impl Run<'_> {
    fn dir_node(&self, fid: &[u8; 16], volume_id: i64) -> Result<Option<i64>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT id FROM node WHERE volume_id = ?1 AND file_id = ?2 AND kind = 0 LIMIT 1",
            )?
            .query_row(params![volume_id, FileId(*fid)], |r| r.get(0))
            .optional()?)
    }

    fn rows_of(&self, fid: &[u8; 16], volume_id: i64) -> Result<Vec<(Option<i64>, i64)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT parent_id, kind FROM node WHERE volume_id = ?1 AND file_id = ?2",
        )?;
        let rows = st.query_map(params![volume_id, FileId(*fid)], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn index_path(&self, real: &str) -> Option<String> {
        let p = normalize_path(real);
        if p.eq_ignore_ascii_case(&self.canon_root) {
            return Some(self.root.clone());
        }
        if !is_ancestor_path(&self.canon_root, &p) {
            return None;
        }
        let rest = p[self.canon_root.len()..].trim_start_matches('\\');
        Some(join_path(&self.root, rest))
    }

    fn deepest_dir(&self, path: &str) -> Result<(i64, bool)> {
        let mut id = self.root_id;
        if !is_ancestor_path(&self.root, path) {
            return Ok((id, path.eq_ignore_ascii_case(&self.root)));
        }
        let mut st = self.conn.prepare_cached(
            "SELECT id, kind FROM node WHERE parent_id = ?1 AND name = ?2 COLLATE NOCASE \
             ORDER BY name = ?2 DESC, kind LIMIT 1",
        )?;
        for comp in path[self.root.len()..]
            .split('\\')
            .filter(|c| !c.is_empty())
        {
            match st
                .query_row(params![id, comp], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
                })
                .optional()?
            {
                Some((c, KIND_DIR)) => id = c,
                _ => return Ok((id, false)),
            }
        }
        Ok((id, true))
    }

    fn mark(&mut self, id: i64, reasons: u32, parent_fid: Option<[u8; 16]>) {
        self.dirty.insert(id);
        if reasons & STRUCTURAL != 0 {
            self.structural.insert(id);
        }
        if let Some(f) = parent_fid {
            self.kid_parents.entry(id).or_default().push(f);
        }
    }

    fn apply(&mut self, w: &mut Writer) -> Result<bool> {
        let volume_id = w.volume_id;
        let mut via_path: Vec<(String, u32, [u8; 16])> = Vec::new();
        let parents: Vec<([u8; 16], u32)> = self
            .recs
            .parents
            .iter()
            .map(|(k, v)| (*k, v.reasons))
            .collect();
        for (pfid, reasons) in parents {
            if pfid == NO_ID {
                continue;
            }
            match self.dir_node(&pfid, volume_id)? {
                Some(id) => self.mark(id, reasons, Some(pfid)),
                None => {
                    // Cheap rejection of the rest of the volume: an unknown parent outside the root is skipped.
                    self.resolved += 1;
                    if let Ok(real) = self.source.path_by_file_id(pfid)
                        && let Some(p) = self.index_path(&real)
                    {
                        via_path.push((p, reasons, pfid));
                    }
                }
            }
        }
        let files: Vec<([u8; 16], u32, u32, bool)> = self
            .recs
            .files
            .iter()
            .map(|(k, s)| (*k, s.reasons, s.last, s.dir))
            .collect();
        let mut moves = Vec::new();
        for (fid, reasons, last, dir) in files {
            if fid == NO_ID {
                continue;
            }
            if dir {
                let Some(id) = self.dir_node(&fid, volume_id)? else {
                    continue;
                };
                if reasons & USN_REASON_SECURITY_CHANGE != 0 {
                    self.dirty.insert(id);
                }
                if reasons & RENAMED != 0
                    && last & USN_REASON_FILE_DELETE == 0
                    && id != self.root_id
                {
                    moves.push((id, fid));
                }
            } else {
                // Hard-link siblings change with the file, but the record names only one parent.
                for (parent, kind) in self.rows_of(&fid, volume_id)? {
                    if kind != KIND_DIR
                        && let Some(p) = parent
                    {
                        self.dirty.insert(p);
                    }
                }
            }
        }
        w.ensure_tx()?;
        self.apply_moves(w, moves)?;
        for (p, reasons, pfid) in via_path {
            let (id, exact) = self.deepest_dir(&p)?;
            if exact {
                self.mark(id, reasons, Some(pfid));
            } else {
                self.mark(id, STRUCTURAL, None);
            }
        }
        let structural: Vec<i64> = self.structural.iter().copied().collect();
        for id in structural {
            let parent: Option<i64> = self
                .conn
                .query_row("SELECT parent_id FROM node WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?
                .flatten();
            if let Some(p) = parent {
                self.dirty.insert(p);
            }
        }
        self.list_dirty(w)
    }

    fn apply_moves(&mut self, w: &mut Writer, candidates: Vec<(i64, [u8; 16])>) -> Result<()> {
        let mut moves: Vec<(usize, String, i64)> = Vec::new();
        for (node, fid) in candidates {
            self.resolved += 1;
            let Ok(real) = self.source.path_by_file_id(fid) else {
                continue;
            };
            if let Some(p) = self.index_path(&real) {
                moves.push((depth(&p), p, node));
            }
        }
        // Shallow first, so a moved parent already sits at its new path when its children are checked.
        moves.sort();
        let mut pending: HashSet<i64> = moves.iter().map(|m| m.2).collect();
        for (_, target, node) in moves {
            pending.remove(&node);
            let Some(cur) = path_of(self.conn, node)? else {
                continue;
            };
            if cur == target {
                continue;
            }
            let Some((parent_path, name)) = split_parent(&target) else {
                continue;
            };
            let (np, exact) = self.deepest_dir(&parent_path)?;
            // A new parent is not indexed yet; its walk picks the subtree up and the old parent drops it.
            if !exact {
                continue;
            }
            let chain = load_chain(self.conn, np)?;
            if chain_has(&chain, node) {
                continue;
            }
            let clashes: Vec<(i64, Delta)> = self
                .conn
                .prepare_cached(
                    "SELECT id, sub_alloc, sub_logical, sub_count FROM node \
                     WHERE parent_id = ?1 AND name = ?2 COLLATE NOCASE AND kind = 0 AND id <> ?3",
                )?
                .query_map(params![np, name, node], |r| {
                    Ok((
                        r.get(0)?,
                        Delta {
                            alloc: r.get(1)?,
                            logical: r.get(2)?,
                            count: r.get(3)?,
                        },
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?;
            for (cid, c) in clashes {
                if pending.contains(&cid) {
                    // ':' cannot occur in a Windows name, so the parked row clashes with nothing.
                    self.conn.execute(
                        "UPDATE node SET name = ?1 WHERE id = ?2",
                        params![format!(":moving:{cid}"), cid],
                    )?;
                } else {
                    w.delete_rows(cid)?;
                    w.add_delta(&chain, -c);
                }
            }
            let (old_parent, c): (Option<i64>, Delta) = self.conn.query_row(
                "SELECT parent_id, sub_alloc, sub_logical, sub_count FROM node WHERE id = ?1",
                [node],
                |r| {
                    Ok((
                        r.get(0)?,
                        Delta {
                            alloc: r.get(1)?,
                            logical: r.get(2)?,
                            count: r.get(3)?,
                        },
                    ))
                },
            )?;
            if let Some(op) = old_parent {
                let old_chain = load_chain(self.conn, op)?;
                w.add_delta(&old_chain, -c);
            }
            self.conn.execute(
                "UPDATE node SET parent_id = ?1, name = ?2 WHERE id = ?3",
                params![np, name, node],
            )?;
            w.add_delta(&chain, c);
            w.regroup_links_under(node)?;
            self.moved += 1;
        }
        Ok(())
    }

    // The stored fold carries hard-link exclusions that recounting one directory cannot reproduce.
    fn small_fold(&self, id: i64, entries: &[DirEntry], w: &Writer) -> Result<Option<Delta>> {
        let (old, flags): (Delta, i64) = self.conn.query_row(
            "SELECT small_bytes, small_logical, small_count, flags FROM node WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    Delta {
                        alloc: r.get(0)?,
                        logical: r.get(1)?,
                        count: r.get(2)?,
                    },
                    r.get(3)?,
                ))
            },
        )?;
        if flags & FLAG_DENIED != 0 {
            return Ok(None);
        }
        let rows: HashSet<[u8; 16]> = self
            .conn
            .prepare_cached(
                "SELECT file_id FROM node WHERE parent_id = ?1 AND kind = 1 AND file_id IS NOT NULL",
            )?
            .query_map([id], |r| r.get::<_, FileId>(0).map(|f| f.0))?
            .collect::<rusqlite::Result<_>>()?;
        let created = |s: Option<&Seen>| s.is_some_and(|s| s.reasons & USN_REASON_FILE_CREATE != 0);
        let mut listed: HashSet<[u8; 16]> = HashSet::new();
        let mut added = Delta::default();
        for e in entries {
            if classify(e) != KIND_FILE {
                continue;
            }
            let fresh_id = listed.insert(e.file_id);
            let seen = self
                .recs
                .files
                .get(&e.file_id)
                .filter(|_| e.file_id != NO_ID);
            if e.file_id != NO_ID && seen.is_none() {
                continue;
            }
            let small = e.alloc.max(e.logical) < w.threshold;
            if created(seen) {
                if small && fresh_id {
                    added += Delta {
                        alloc: to_i64(e.alloc),
                        logical: to_i64(e.logical),
                        count: 1,
                    };
                }
                continue;
            }
            if small || !rows.contains(&e.file_id) {
                return Ok(None);
            }
        }
        for pfid in self.kid_parents.get(&id).into_iter().flatten() {
            let Some(kids) = self.recs.parents.get(pfid) else {
                continue;
            };
            for k in &kids.ids {
                let seen = self.recs.files.get(k);
                if listed.contains(k) || seen.is_some_and(|s| s.dir) || created(seen) {
                    continue;
                }
                if self.rows_of(k, w.volume_id)?.is_empty() {
                    return Ok(None);
                }
            }
        }
        Ok(Some(old + added))
    }

    fn tick(&mut self, w: &mut Writer) -> Result<()> {
        if w.writes >= self.batch_rows || self.last_commit.elapsed() >= COMMIT_EVERY {
            w.commit()?;
            self.last_commit = Instant::now();
        }
        if let Some(cb) = self.on_event
            && self.last_progress.elapsed() >= PROGRESS_EVERY
        {
            let fraction = (w.stats.dirs as f64 / self.planned.max(1) as f64).min(0.99);
            cb(Event::Progress(&Progress {
                phase: "walk",
                dirs: w.stats.dirs,
                files: w.stats.files,
                bytes: w.stats.bytes,
                fraction: Some(fraction),
            }));
            self.last_progress = Instant::now();
        }
        Ok(())
    }

    fn list_dirty(&mut self, w: &mut Writer) -> Result<bool> {
        self.listing_from = Some(Instant::now());
        let mut heap = BinaryHeap::new();
        for &id in &self.dirty {
            if let Some(p) = path_of(self.conn, id)? {
                heap.push(Reverse((depth(&p), id)));
            }
        }
        self.planned = heap.len() as u64;
        let mut done = HashSet::new();
        let mut sub = Vec::new();
        while let Some(Reverse((d, id))) = heap.pop() {
            if !done.insert(id) {
                continue;
            }
            if self.cancel.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let Some((kind, parent)) = self
                .conn
                .query_row(
                    "SELECT kind, parent_id FROM node WHERE id = ?1",
                    [id],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)),
                )
                .optional()?
            else {
                continue;
            };
            if kind != KIND_DIR {
                continue;
            }
            let Some(path) = path_of(self.conn, id)? else {
                continue;
            };
            let listing = list_fixed(self.lister, &path);
            match &listing {
                Err(e) if id == self.root_id => bail!("cannot list {path}: {e}"),
                // Gone since the records were read; its parent's listing drops it, now or next time.
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    if let Some(p) = parent
                        && !done.contains(&p)
                    {
                        heap.push(Reverse((d.saturating_sub(1), p)));
                    }
                    continue;
                }
                _ => {}
            }
            let fold = match &listing {
                Ok(entries) => self.small_fold(id, entries, w)?,
                Err(_) => None,
            };
            let mut work = Work::existing(id, path, load_chain(self.conn, id)?);
            w.apply_listing_with(&mut work, listing, &mut sub, fold)?;
            let fresh: Vec<Work> = sub.drain(..).filter(|s| s.is_new).collect();
            for s in fresh {
                if !self.walk_new(w, s)? {
                    return Ok(false);
                }
            }
            self.tick(w)?;
        }
        Ok(true)
    }

    fn walk_new(&mut self, w: &mut Writer, start: Work) -> Result<bool> {
        let mut stack = vec![start];
        let mut sub = Vec::new();
        while let Some(mut work) = stack.pop() {
            if self.cancel.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let listing = list_fixed(self.lister, &work.path);
            w.apply_listing(&mut work, listing, &mut sub)?;
            self.new_dirs += 1;
            stack.append(&mut sub);
            self.tick(w)?;
        }
        Ok(true)
    }
}

pub fn journal_refresh(
    conn: &Connection,
    lister: &dyn DirLister,
    source: &dyn JournalSource,
    volume: &str,
    opts: &RefreshOptions,
) -> Result<JournalOutcome> {
    let started_ms = now_ms();
    let t0 = Instant::now();
    let Some(v) = load_volume(conn, &normalize_path(volume))? else {
        return Ok(JournalOutcome::NeedsFull(NeedsFull::NoPosition));
    };
    super::bulk_indexes(conn, false)?;
    let Ok(j) = source.query() else {
        return Ok(JournalOutcome::NeedsFull(NeedsFull::NoJournal));
    };
    if j.journal_id != v.journal_id {
        return Ok(JournalOutcome::NeedsFull(NeedsFull::JournalChanged));
    }
    if v.next_usn < j.first_usn || v.next_usn > j.next_usn {
        return Ok(JournalOutcome::NeedsFull(NeedsFull::Wrapped));
    }
    let local_cancel = AtomicBool::new(false);
    let cancel: &AtomicBool = opts.cancel.as_deref().unwrap_or(&local_cancel);
    let vinfo = lister.volume_info(Path::new(&v.root)).ok();
    let mut out = RefreshOutcome {
        volume_id: v.id,
        root: v.root.clone(),
        generation: v.generation,
        completed: false,
        scan_id: None,
        duration_ms: 0,
        dirs: 0,
        files: 0,
        denied: 0,
        rows_deleted: 0,
        threshold: v.threshold,
        indexed_bytes: root_alloc(conn, v.root_id)?,
        total_bytes: vinfo.as_ref().map(|i| i.total_bytes),
        free_bytes: vinfo.as_ref().map(|i| i.free_bytes),
        unaccounted_bytes: None,
    };
    let mut result = JournalRefresh {
        outcome: out.clone(),
        from_usn: v.next_usn,
        next_usn: v.next_usn,
        records: 0,
        pages: 0,
        dirty_dirs: 0,
        moved_dirs: 0,
        new_dirs: 0,
        resolved_ids: 0,
    };
    let recs = match read_records(source, v.journal_id, v.next_usn, j.next_usn, cancel)? {
        Read::Records(r) => r,
        Read::Unusable(why) => return Ok(JournalOutcome::NeedsFull(why)),
        Read::Cancelled => {
            result.outcome.duration_ms = t0.elapsed().as_millis() as i64;
            return Ok(JournalOutcome::Done(Box::new(result)));
        }
    };
    if let Some(cb) = opts.on_event {
        cb(Event::Start {
            volume: &v.root,
            generation: v.generation,
            mode: opts.mode,
            kind: "journal",
        });
    }
    result.records = recs.count;
    result.pages = recs.pages;
    let next_usn = recs.next;
    let read_at = Instant::now();
    let canon_root = source
        .final_path(&v.root)
        .unwrap_or_else(|_| v.root.clone());
    let mut run = Run {
        conn,
        lister,
        source,
        cancel,
        on_event: opts.on_event,
        batch_rows: opts.batch_rows.max(1),
        root: v.root.clone(),
        canon_root,
        root_id: v.root_id,
        recs,
        dirty: HashSet::new(),
        structural: HashSet::new(),
        kid_parents: HashMap::new(),
        moved: 0,
        new_dirs: 0,
        planned: 0,
        resolved: 0,
        listing_from: None,
        last_commit: Instant::now(),
        last_progress: Instant::now(),
    };
    let mut w = Writer::new(conn, v.id, v.generation, v.threshold, Dedupe::Db);
    let applied = run.apply(&mut w);
    result.dirty_dirs = run.planned;
    result.moved_dirs = run.moved;
    result.new_dirs = run.new_dirs;
    result.resolved_ids = run.resolved;
    let listed_from = run.listing_from.unwrap_or(read_at);
    tracing::debug!(
        read_ms = (read_at - t0).as_millis() as u64,
        resolve_ms = (listed_from - read_at).as_millis() as u64,
        list_ms = listed_from.elapsed().as_millis() as u64,
        records = result.records,
        resolved = result.resolved_ids,
        "journal refresh phases"
    );
    let completed = match applied {
        Ok(c) => c,
        Err(e) => {
            w.rollback();
            let _ = forget_position(conn, v.id);
            return Err(e);
        }
    };
    out.dirs = w.stats.dirs;
    out.files = w.stats.files;
    out.denied = w.stats.denied;
    if !completed {
        // A half-walked new subtree has rows that no record will point at again.
        w.commit()?;
        forget_position(conn, v.id)?;
        out.rows_deleted = w.stats.deleted;
        out.indexed_bytes = root_alloc(conn, v.root_id)?;
        out.duration_ms = t0.elapsed().as_millis() as i64;
        result.outcome = out;
        return Ok(JournalOutcome::Done(Box::new(result)));
    }
    w.ensure_tx()?;
    let fin = (|| -> Result<Option<i64>> {
        w.resolve_groups(true)?;
        w.flush_deltas()?;
        out.indexed_bytes = root_alloc(conn, v.root_id)?;
        let same_volume = vinfo
            .as_ref()
            .is_some_and(|i| normalize_path(&i.root).eq_ignore_ascii_case(&v.root));
        out.unaccounted_bytes = vinfo.as_ref().filter(|_| same_volume).map(|i| {
            to_i64(i.total_bytes.saturating_sub(i.free_bytes)) - to_i64(out.indexed_bytes)
        });
        let finished_ms = now_ms();
        // Guarded so a full refresh that began meanwhile keeps its cleared position.
        let stored = conn.execute(
            "UPDATE volume SET usn_next = ?1, last_refresh_ms = ?2, total_bytes = COALESCE(?3, total_bytes), \
             free_bytes = COALESCE(?4, free_bytes) WHERE id = ?5 AND gen = ?6 AND usn_journal_id = ?7 \
             AND usn_next = ?8",
            params![
                next_usn,
                finished_ms,
                out.total_bytes.map(to_i64),
                out.free_bytes.map(to_i64),
                v.id,
                v.generation,
                v.journal_id as i64,
                v.next_usn
            ],
        )?;
        if stored == 0 {
            return Ok(None);
        }
        let files: i64 = conn.query_row(
            "SELECT sub_count FROM node WHERE id = ?1",
            [v.root_id],
            |r| r.get(0),
        )?;
        record_journal_scan(
            conn,
            ScanRecord {
                volume_id: v.id,
                generation: v.generation,
                kind: "journal".into(),
                mode: opts.mode.as_str().into(),
                started_ms,
                finished_ms,
                total_bytes: out.total_bytes,
                free_bytes: out.free_bytes,
                indexed_bytes: out.indexed_bytes,
                unaccounted_bytes: out.unaccounted_bytes,
                file_count: to_u64(files),
                dir_count: 0,
                denied_count: 0,
            },
        )
        .map(Some)
    })();
    match fin {
        Ok(scan_id) => {
            out.scan_id = scan_id;
            out.completed = true;
        }
        Err(e) => {
            w.rollback();
            let _ = forget_position(conn, v.id);
            return Err(e);
        }
    }
    w.commit()?;
    out.rows_deleted = w.stats.deleted;
    out.duration_ms = t0.elapsed().as_millis() as i64;
    result.outcome = out;
    result.next_usn = next_usn;
    Ok(JournalOutcome::Done(Box::new(result)))
}

// Directory and denied counts carry over: a journal refresh sees only the directories it re-lists.
fn record_journal_scan(conn: &Connection, mut rec: ScanRecord) -> Result<i64> {
    type Prev = (i64, String, i64, Option<i64>, Option<i64>);
    let prev: Option<Prev> = conn
        .query_row(
            "SELECT id, kind, started_ms, dir_count, denied_count FROM scan WHERE volume_id = ?1 \
             AND finished_ms IS NOT NULL ORDER BY finished_ms DESC, id DESC LIMIT 1",
            [rec.volume_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    if let Some((id, kind, started, dirs, denied)) = prev {
        rec.dir_count = dirs.map(to_u64).unwrap_or(0);
        rec.denied_count = denied.map(to_u64).unwrap_or(0);
        if kind == "journal" && started >= rec.finished_ms - COALESCE_MS {
            rec.started_ms = started;
            conn.execute("DELETE FROM scan_dir WHERE scan_id = ?1", [id])?;
            conn.execute("DELETE FROM scan_file WHERE scan_id = ?1", [id])?;
            conn.execute("DELETE FROM scan WHERE id = ?1", [id])?;
        }
    }
    history::record_scan(conn, &rec).context("recording the journal refresh")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(usn: i64, id: u8, parent: u8, reason: u32, attrs: u32) -> UsnRecord {
        let mut file_id = [0u8; 16];
        file_id[0] = id;
        let mut parent_id = [0u8; 16];
        parent_id[0] = parent;
        UsnRecord {
            usn,
            file_id,
            parent_id,
            reason,
            attrs,
        }
    }

    #[test]
    fn records_aggregate_by_file_and_parent() {
        let mut r = Records::default();
        r.add(&rec(1, 7, 5, USN_REASON_FILE_CREATE, 0x20));
        r.add(&rec(
            2,
            7,
            5,
            USN_REASON_DATA_EXTEND | USN_REASON_CLOSE,
            0x20,
        ));
        r.add(&rec(3, 9, 5, USN_REASON_RENAME_OLD_NAME, 0x10));
        r.add(&rec(4, 9, 6, USN_REASON_RENAME_NEW_NAME, 0x10));
        r.add(&rec(
            5,
            7,
            5,
            USN_REASON_FILE_DELETE | USN_REASON_CLOSE,
            0x20,
        ));
        assert_eq!(r.count, 5);
        let f = &r.files[&rec(0, 7, 0, 0, 0).file_id];
        assert!(!f.dir);
        assert_eq!(f.last, USN_REASON_FILE_DELETE | USN_REASON_CLOSE);
        assert_ne!(f.reasons & USN_REASON_DATA_EXTEND, 0);
        assert!(r.files[&rec(0, 9, 0, 0, 0).file_id].dir);
        let five = &r.parents[&rec(0, 0, 5, 0, 0).parent_id];
        assert_eq!(
            five.ids.len(),
            3,
            "consecutive records of one file collapse"
        );
        assert_ne!(five.reasons & STRUCTURAL, 0);
        assert_eq!(r.parents[&rec(0, 0, 6, 0, 0).parent_id].ids.len(), 1);
    }

    #[test]
    fn journal_errors_map_to_fallbacks() {
        let e = |c| io::Error::from_raw_os_error(c);
        assert_eq!(
            unusable(&e(ERROR_JOURNAL_ENTRY_DELETED)),
            Some(NeedsFull::Wrapped)
        );
        assert_eq!(
            unusable(&e(ERROR_JOURNAL_NOT_ACTIVE)),
            Some(NeedsFull::JournalChanged)
        );
        assert_eq!(unusable(&e(5)), None);
        assert_eq!(
            "journal".parse::<RefreshKind>().unwrap(),
            RefreshKind::Journal
        );
        assert!("sometimes".parse::<RefreshKind>().is_err());
    }

    #[test]
    fn depth_counts_components() {
        assert_eq!(depth(r"C:\"), 1);
        assert_eq!(depth(r"C:\a"), 2);
        assert_eq!(depth(r"C:\a\b"), 3);
    }
}

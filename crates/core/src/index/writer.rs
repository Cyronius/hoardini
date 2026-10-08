use std::collections::{HashMap, HashSet};
use std::io;
use std::ops::{Add, AddAssign, Neg, Sub};
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use hoardini_win::{DirEntry, FILE_ATTRIBUTE_REPARSE_POINT};
use rusqlite::{Connection, OptionalExtension, params};

use super::dedupe::SeenSet;
use super::{
    FLAG_DENIED, FLAG_HARDLINK_DUP, FLAG_PLACEHOLDER, FileId, KIND_DIR, KIND_FILE, KIND_LINK,
    PathCache, join_path, to_i64,
};

const NAME_SURROGATE: u32 = 0x2000_0000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Delta {
    pub alloc: i64,
    pub logical: i64,
    pub count: i64,
}

impl Delta {
    pub fn is_zero(&self) -> bool {
        *self == Delta::default()
    }
}

impl Add for Delta {
    type Output = Delta;
    fn add(self, o: Delta) -> Delta {
        Delta {
            alloc: self.alloc + o.alloc,
            logical: self.logical + o.logical,
            count: self.count + o.count,
        }
    }
}

impl AddAssign for Delta {
    fn add_assign(&mut self, o: Delta) {
        *self = *self + o;
    }
}

impl Sub for Delta {
    type Output = Delta;
    fn sub(self, o: Delta) -> Delta {
        self + -o
    }
}

impl Neg for Delta {
    type Output = Delta;
    fn neg(self) -> Delta {
        Delta {
            alloc: -self.alloc,
            logical: -self.logical,
            count: -self.count,
        }
    }
}

pub(crate) fn contrib(kind: i64, flags: i64, alloc: i64, logical: i64, sub: Delta) -> Delta {
    match kind {
        KIND_DIR => sub,
        KIND_FILE if flags & FLAG_HARDLINK_DUP == 0 => Delta {
            alloc,
            logical,
            count: 1,
        },
        _ => Delta::default(),
    }
}

pub(crate) fn classify(e: &DirEntry) -> i64 {
    let reparse = e.attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    let surrogate = e.reparse_tag & NAME_SURROGATE != 0;
    if e.is_dir() {
        // A tagless reparse dir comes from a lister that cannot read tags; never risk following it.
        if reparse && (surrogate || e.reparse_tag == 0) {
            KIND_LINK
        } else {
            KIND_DIR
        }
    } else if reparse && surrogate {
        KIND_LINK
    } else {
        KIND_FILE
    }
}

pub(crate) struct Chain {
    pub id: i64,
    pub parent: Option<Arc<Chain>>,
}

impl Chain {
    pub fn root(id: i64) -> Arc<Chain> {
        Arc::new(Chain { id, parent: None })
    }

    pub fn child(self: &Arc<Self>, id: i64) -> Arc<Chain> {
        Arc::new(Chain {
            id,
            parent: Some(self.clone()),
        })
    }
}

pub(crate) fn load_chain(conn: &Connection, id: i64) -> Result<Arc<Chain>> {
    let mut ids = vec![id];
    let mut st = conn.prepare_cached("SELECT parent_id FROM node WHERE id = ?1")?;
    let mut cur = id;
    while let Some(Some(p)) = st
        .query_row([cur], |r| r.get::<_, Option<i64>>(0))
        .optional()?
    {
        if ids.len() > 4096 {
            bail!("node {id}: parent chain does not end");
        }
        ids.push(p);
        cur = p;
    }
    let mut chain: Option<Arc<Chain>> = None;
    for id in ids.into_iter().rev() {
        chain = Some(Arc::new(Chain { id, parent: chain }));
    }
    chain.ok_or_else(|| anyhow!("empty chain"))
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Prev {
    pub small: Delta,
    pub flags: i64,
}

pub(crate) struct NewDir {
    parent: i64,
    name: String,
    vals: Vals,
}

// A new dir's row is written once, when it is listed; `prev`/`new_dir` are void after a foreign commit.
pub(crate) struct Work {
    pub node_id: i64,
    pub path: String,
    pub chain: Arc<Chain>,
    pub is_new: bool,
    prev: Option<Prev>,
    new_dir: Option<Box<NewDir>>,
    epoch: u64,
}

impl Work {
    pub fn existing(node_id: i64, path: String, chain: Arc<Chain>) -> Work {
        Work {
            node_id,
            path,
            chain,
            is_new: false,
            prev: None,
            new_dir: None,
            epoch: 0,
        }
    }
}

struct Row {
    id: i64,
    kind: i64,
    alloc: i64,
    logical: i64,
    mtime: Option<i64>,
    atime: Option<i64>,
    ctime: Option<i64>,
    attrs: i64,
    reparse_tag: Option<i64>,
    file_id: Option<[u8; 16]>,
    flags: i64,
    sub: Delta,
    small: Delta,
}

impl Row {
    fn contrib(&self) -> Delta {
        contrib(self.kind, self.flags, self.alloc, self.logical, self.sub)
    }
}

#[derive(Default)]
struct Existing(HashMap<String, Vec<Row>>);

impl Existing {
    fn take(&mut self, name: &str, kind: i64) -> Option<Row> {
        let rows = self.0.get_mut(name)?;
        let i = rows.iter().position(|r| r.kind == kind)?;
        Some(rows.swap_remove(i))
    }

    fn into_rows(self) -> impl Iterator<Item = Row> {
        self.0.into_values().flatten()
    }
}

struct Vals {
    alloc: i64,
    logical: i64,
    mtime: Option<i64>,
    atime: Option<i64>,
    ctime: Option<i64>,
    attrs: i64,
    reparse_tag: Option<i64>,
    file_id: Option<[u8; 16]>,
    flags: i64,
    kind: i64,
}

impl Vals {
    // Nothing reads a directory's access or creation time, and listing it moves its access time.
    fn from_entry(e: &DirEntry, kind: i64, flags: i64) -> Vals {
        let (alloc, logical) = if kind == KIND_FILE {
            (to_i64(e.alloc), to_i64(e.logical))
        } else {
            (0, 0)
        };
        let t = |v: i64| (v != 0 && kind != KIND_DIR).then_some(v);
        Vals {
            alloc,
            logical,
            mtime: (e.mtime_ms != 0).then_some(e.mtime_ms),
            atime: t(e.atime_ms),
            ctime: t(e.ctime_ms),
            attrs: i64::from(e.attrs),
            reparse_tag: (e.reparse_tag != 0).then_some(i64::from(e.reparse_tag)),
            file_id: (e.file_id != [0; 16]).then_some(e.file_id),
            flags,
            kind,
        }
    }

    fn same_as(&self, r: &Row) -> bool {
        self.alloc == r.alloc
            && self.logical == r.logical
            && self.mtime == r.mtime
            && self.atime == r.atime
            && self.ctime == r.ctime
            && self.attrs == r.attrs
            && self.reparse_tag == r.reparse_tag
            && self.file_id == r.file_id
            && self.flags == r.flags
    }
}

pub(crate) enum Dedupe {
    Walk(SeenSet),
    Db,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stats {
    pub dirs: u64,
    pub files: u64,
    pub bytes: u64,
    pub denied: u64,
    pub deleted: u64,
}

pub(crate) struct Writer<'c> {
    conn: &'c Connection,
    pub volume_id: i64,
    pub generation: i64,
    pub threshold: u64,
    dedupe: Dedupe,
    pub hot_ids: HashSet<i64>,
    regroup: HashSet<[u8; 16]>,
    deltas: HashMap<i64, Delta>,
    chains: HashMap<i64, Arc<Chain>>,
    // A walk deletes nothing until it completes; these rows were missing from their parent's listing.
    gone: Vec<i64>,
    data_version: Option<i64>,
    epoch: u64,
    in_tx: bool,
    pub writes: usize,
    pub stats: Stats,
}

const CHILD_COLS: &str = "id, name, kind, alloc, logical, mtime, atime, ctime, attrs, reparse_tag, \
     file_id, flags, sub_alloc, sub_logical, sub_count, small_bytes, small_logical, small_count";

fn fid(id: &Option<[u8; 16]>) -> Option<FileId> {
    id.map(FileId)
}

impl<'c> Writer<'c> {
    pub fn new(
        conn: &'c Connection,
        volume_id: i64,
        generation: i64,
        threshold: u64,
        dedupe: Dedupe,
    ) -> Writer<'c> {
        Writer {
            conn,
            volume_id,
            generation,
            threshold,
            dedupe,
            hot_ids: HashSet::new(),
            regroup: HashSet::new(),
            deltas: HashMap::new(),
            chains: HashMap::new(),
            gone: Vec::new(),
            data_version: None,
            epoch: 0,
            in_tx: false,
            writes: 0,
            stats: Stats::default(),
        }
    }

    pub fn in_tx(&self) -> bool {
        self.in_tx
    }

    // IMMEDIATE so a concurrent writer waits on busy_timeout instead of failing on lock upgrade.
    pub fn ensure_tx(&mut self) -> Result<()> {
        if !self.in_tx {
            self.conn.execute_batch("BEGIN IMMEDIATE")?;
            self.in_tx = true;
            let v: i64 = self
                .conn
                .query_row("PRAGMA data_version", [], |r| r.get(0))?;
            if self.data_version.replace(v).is_some_and(|old| old != v) {
                self.epoch += 1;
            }
        }
        Ok(())
    }

    pub fn commit(&mut self) -> Result<()> {
        if self.in_tx {
            self.flush_deltas()?;
            self.conn.execute_batch("COMMIT")?;
            self.in_tx = false;
        }
        self.writes = 0;
        Ok(())
    }

    pub fn rollback(&mut self) {
        if self.in_tx {
            let _ = self.conn.execute_batch("ROLLBACK");
            self.in_tx = false;
        }
        self.deltas.clear();
        self.writes = 0;
    }

    pub fn add_delta(&mut self, chain: &Arc<Chain>, d: Delta) {
        if d.is_zero() {
            return;
        }
        let mut cur = Some(chain);
        while let Some(c) = cur {
            *self.deltas.entry(c.id).or_default() += d;
            cur = c.parent.as_ref();
        }
    }

    pub fn flush_deltas(&mut self) -> Result<()> {
        let conn = self.conn;
        let mut st = conn.prepare_cached(
            "UPDATE node SET sub_alloc = sub_alloc + ?1, sub_logical = sub_logical + ?2, \
             sub_count = sub_count + ?3 WHERE id = ?4",
        )?;
        for (id, d) in self.deltas.drain() {
            if !d.is_zero() {
                st.execute(params![d.alloc, d.logical, d.count, id])?;
                self.writes += 1;
            }
        }
        Ok(())
    }

    pub fn chain_cached(&mut self, id: i64) -> Result<Arc<Chain>> {
        if let Some(c) = self.chains.get(&id) {
            return Ok(c.clone());
        }
        let c = load_chain(self.conn, id)?;
        self.chains.insert(id, c.clone());
        Ok(c)
    }

    fn self_row(&self, id: i64) -> Result<Option<Prev>> {
        let mut st = self.conn.prepare_cached(
            "SELECT small_bytes, small_logical, small_count, flags FROM node WHERE id = ?1",
        )?;
        Ok(st
            .query_row([id], |r| {
                Ok(Prev {
                    small: Delta {
                        alloc: r.get(0)?,
                        logical: r.get(1)?,
                        count: r.get(2)?,
                    },
                    flags: r.get(3)?,
                })
            })
            .optional()?)
    }

    fn dir_row(&self, parent: i64, name: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT id FROM node WHERE parent_id = ?1 AND name = ?2 AND kind = 0 LIMIT 1",
            )?
            .query_row(params![parent, name], |r| r.get(0))
            .optional()?)
    }

    fn children(&self, id: i64) -> Result<Existing> {
        let _s = super::prof::span(super::prof::Phase::Children);
        let mut st = self.conn.prepare_cached(&format!(
            "SELECT {CHILD_COLS} FROM node WHERE parent_id = ?1"
        ))?;
        let mut out = Existing::default();
        let rows = st.query_map([id], |r| {
            let row = Row {
                id: r.get(0)?,
                kind: r.get(2)?,
                alloc: r.get(3)?,
                logical: r.get(4)?,
                mtime: r.get(5)?,
                atime: r.get(6)?,
                ctime: r.get(7)?,
                attrs: r.get(8)?,
                reparse_tag: r.get(9)?,
                file_id: r.get::<_, Option<FileId>>(10)?.map(|f| f.0),
                flags: r.get(11)?,
                sub: Delta {
                    alloc: r.get(12)?,
                    logical: r.get(13)?,
                    count: r.get(14)?,
                },
                small: Delta {
                    alloc: r.get(15)?,
                    logical: r.get(16)?,
                    count: r.get(17)?,
                },
            };
            Ok((r.get::<_, String>(1)?, row))
        })?;
        for row in rows {
            let (name, row) = row?;
            out.0.entry(name).or_default().push(row);
        }
        Ok(out)
    }

    fn insert(&mut self, parent: i64, name: &str, v: &Vals) -> Result<i64> {
        let _s = super::prof::span(super::prof::Phase::Write);
        let mut st = self.conn.prepare_cached(
            "INSERT INTO node(parent_id, volume_id, name, kind, alloc, logical, mtime, atime, ctime, \
             attrs, reparse_tag, file_id, flags, seen_gen) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )?;
        st.execute(params![
            parent,
            self.volume_id,
            name,
            v.kind,
            v.alloc,
            v.logical,
            v.mtime,
            v.atime,
            v.ctime,
            v.attrs,
            v.reparse_tag,
            fid(&v.file_id),
            v.flags,
            self.generation
        ])?;
        self.writes += 1;
        Ok(self.conn.last_insert_rowid())
    }

    // `work.chain` is the parent's chain until here; the row starts with its own listing's totals.
    fn insert_dir(
        &mut self,
        work: &mut Work,
        nd: &NewDir,
        small: Delta,
        sub: Delta,
        flags: i64,
    ) -> Result<()> {
        let _s = super::prof::span(super::prof::Phase::Write);
        let v = &nd.vals;
        self.conn
            .prepare_cached(
                "INSERT INTO node(parent_id, volume_id, name, kind, mtime, attrs, reparse_tag, file_id, \
                 flags, small_count, small_bytes, small_logical, sub_alloc, sub_logical, sub_count, seen_gen) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            )?
            .execute(params![
                nd.parent,
                self.volume_id,
                nd.name,
                KIND_DIR,
                v.mtime,
                v.attrs,
                v.reparse_tag,
                fid(&v.file_id),
                flags,
                small.count,
                small.alloc,
                small.logical,
                sub.alloc,
                sub.logical,
                sub.count,
                self.generation
            ])?;
        self.writes += 1;
        let id = self.conn.last_insert_rowid();
        let parent_chain = work.chain.clone();
        self.add_delta(&parent_chain, sub);
        work.node_id = id;
        work.chain = parent_chain.child(id);
        Ok(())
    }

    fn upsert_existing(&mut self, row: &Row, v: &Vals) -> Result<()> {
        if v.same_as(row) {
            return Ok(());
        }
        let _s = super::prof::span(super::prof::Phase::Write);
        self.conn
            .prepare_cached(
                "UPDATE node SET alloc = ?1, logical = ?2, mtime = ?3, atime = ?4, ctime = ?5, \
                 attrs = ?6, reparse_tag = ?7, file_id = ?8, flags = ?9 WHERE id = ?10",
            )?
            .execute(params![
                v.alloc,
                v.logical,
                v.mtime,
                v.atime,
                v.ctime,
                v.attrs,
                v.reparse_tag,
                fid(&v.file_id),
                v.flags,
                row.id
            ])?;
        self.writes += 1;
        Ok(())
    }

    fn set_self(&mut self, id: i64, small: Delta, flags: i64) -> Result<()> {
        let _s = super::prof::span(super::prof::Phase::Write);
        self.conn
            .prepare_cached(
                "UPDATE node SET small_bytes = ?1, small_logical = ?2, small_count = ?3, flags = ?4 \
                 WHERE id = ?5",
            )?
            .execute(params![small.alloc, small.logical, small.count, flags, id])?;
        self.writes += 1;
        Ok(())
    }

    // The caller owns the ancestor delta.
    pub fn delete_rows(&mut self, id: i64) -> Result<()> {
        let conn = self.conn;
        let mut ids = conn.prepare_cached(
            "WITH RECURSIVE sub(id) AS (SELECT ?1 UNION ALL \
             SELECT n.id FROM node n JOIN sub ON n.parent_id = sub.id) \
             SELECT n.file_id FROM node n JOIN sub ON n.id = sub.id \
             WHERE n.kind = 1 AND n.file_id IS NOT NULL",
        )?;
        for key in ids.query_map([id], |r| r.get::<_, FileId>(0))? {
            self.regroup.insert(key?.0);
        }
        let n = conn
            .prepare_cached(
                "WITH RECURSIVE sub(id) AS (SELECT ?1 UNION ALL \
                 SELECT n.id FROM node n JOIN sub ON n.parent_id = sub.id) \
                 DELETE FROM node WHERE id IN (SELECT id FROM sub)",
            )?
            .execute([id])?;
        self.writes += n;
        self.stats.deleted += n as u64;
        Ok(())
    }

    // A walk defers the deletion to its end; any other run deletes at once.
    fn drop_row(&mut self, row: &Row, delta: &mut Delta) -> Result<()> {
        if matches!(self.dedupe, Dedupe::Walk(_)) {
            self.gone.push(row.id);
        } else {
            *delta = *delta - row.contrib();
            self.delete_rows(row.id)?;
        }
        Ok(())
    }

    pub fn apply_listing(
        &mut self,
        work: &mut Work,
        listing: io::Result<Vec<DirEntry>>,
        push: &mut Vec<Work>,
    ) -> Result<()> {
        self.apply_listing_with(work, listing, push, None)
    }

    // Moved files keep their ids but change paths, and the smallest path owns a hard-link group.
    pub fn regroup_links_under(&mut self, id: i64) -> Result<()> {
        let conn = self.conn;
        let mut st = conn.prepare_cached(
            "WITH RECURSIVE sub(id) AS (SELECT ?1 UNION ALL \
             SELECT n.id FROM node n JOIN sub ON n.parent_id = sub.id) \
             SELECT n.file_id FROM node n JOIN sub ON n.id = sub.id \
             WHERE n.kind = 1 AND n.file_id IS NOT NULL AND EXISTS (SELECT 1 FROM node m \
             WHERE m.volume_id = n.volume_id AND m.file_id = n.file_id AND m.id <> n.id)",
        )?;
        for key in st.query_map([id], |r| r.get::<_, FileId>(0))? {
            self.regroup.insert(key?.0);
        }
        Ok(())
    }

    // small_fold: the caller derived the fold from the stored one, keeping its hard-link exclusions.
    pub fn apply_listing_with(
        &mut self,
        work: &mut Work,
        listing: io::Result<Vec<DirEntry>>,
        push: &mut Vec<Work>,
        small_fold: Option<Delta>,
    ) -> Result<()> {
        self.ensure_tx()?;
        let mut new_dir = work.new_dir.take();
        let mut prev = work.prev.take();
        if work.epoch != self.epoch {
            prev = None;
            // The app may have patched this directory in since its parent was listed.
            if let Some(nd) = &new_dir
                && let Some(id) = self.dir_row(nd.parent, &nd.name)?
            {
                work.node_id = id;
                work.chain = work.chain.child(id);
                new_dir = None;
            }
        }
        let old = match (&new_dir, prev) {
            (Some(_), _) => Prev::default(),
            (None, Some(p)) => p,
            (None, None) => match self.self_row(work.node_id)? {
                Some(p) => p,
                None => return Ok(()),
            },
        };
        self.stats.dirs += 1;
        let entries = match listing {
            Ok(e) => e,
            Err(err) => {
                tracing::debug!(path = %work.path, error = %err, "cannot list directory");
                self.stats.denied += 1;
                if let Some(nd) = new_dir {
                    return self.insert_dir(
                        work,
                        &nd,
                        Delta::default(),
                        Delta::default(),
                        FLAG_DENIED,
                    );
                }
                let mut delta = -old.small;
                for row in self.children(work.node_id)?.into_rows() {
                    self.drop_row(&row, &mut delta)?;
                }
                self.set_self(work.node_id, Delta::default(), old.flags | FLAG_DENIED)?;
                self.add_delta(&work.chain, delta);
                return Ok(());
            }
        };

        let mut existing = match new_dir {
            Some(_) => Existing::default(),
            None => self.children(work.node_id)?,
        };
        let mut small = Delta::default();
        let mut delta = Delta::default();
        let mut small_links: HashSet<[u8; 16]> = HashSet::new();
        let mut new_rows: Vec<(String, Vals)> = Vec::new();
        let mut old_dirs: Vec<(String, Row)> = Vec::new();
        let mut new_dirs: Vec<(String, Vals)> = Vec::new();
        for e in entries {
            match classify(&e) {
                KIND_DIR => {
                    let vals = Vals::from_entry(&e, KIND_DIR, 0);
                    match existing.take(&e.name, KIND_DIR) {
                        Some(row) => {
                            let vals = Vals {
                                flags: row.flags,
                                ..vals
                            };
                            self.upsert_existing(&row, &vals)?;
                            if !self.hot_ids.contains(&row.id) {
                                old_dirs.push((e.name, row));
                            }
                        }
                        None => new_dirs.push((e.name, vals)),
                    }
                }
                KIND_LINK => {
                    let vals = Vals::from_entry(&e, KIND_LINK, 0);
                    match existing.take(&e.name, KIND_LINK) {
                        Some(row) => self.upsert_existing(&row, &vals)?,
                        None => new_rows.push((e.name, vals)),
                    }
                }
                _ => {
                    self.stats.files += 1;
                    let row_sized = e.alloc.max(e.logical) >= self.threshold;
                    let key = (e.file_id != [0; 16]).then_some(e.file_id);
                    let prev = if row_sized {
                        existing.take(&e.name, KIND_FILE)
                    } else {
                        None
                    };
                    let dup = match (&mut self.dedupe, key) {
                        (Dedupe::Walk(seen), Some(k)) => {
                            let first = seen.first_sighting(&k);
                            if !first && row_sized {
                                self.regroup.insert(k);
                            }
                            !first
                        }
                        (Dedupe::Db, Some(k)) if row_sized => {
                            self.regroup.insert(k);
                            prev.as_ref().is_some_and(|r| {
                                r.flags & FLAG_HARDLINK_DUP != 0 && r.file_id == Some(k)
                            })
                        }
                        (Dedupe::Db, Some(k)) => !small_links.insert(k),
                        _ => false,
                    };
                    if !dup {
                        self.stats.bytes += e.alloc;
                    }
                    if row_sized {
                        let mut flags = 0;
                        if e.is_placeholder() {
                            flags |= FLAG_PLACEHOLDER;
                        }
                        if dup {
                            flags |= FLAG_HARDLINK_DUP;
                        }
                        let vals = Vals::from_entry(&e, KIND_FILE, flags);
                        let new_c =
                            contrib(KIND_FILE, flags, vals.alloc, vals.logical, Delta::default());
                        match prev {
                            Some(row) => {
                                delta = delta + new_c - row.contrib();
                                self.upsert_existing(&row, &vals)?;
                            }
                            None => {
                                delta += new_c;
                                new_rows.push((e.name, vals));
                            }
                        }
                    } else if !dup {
                        small += Delta {
                            alloc: to_i64(e.alloc),
                            logical: to_i64(e.logical),
                            count: 1,
                        };
                    }
                }
            }
        }
        for row in existing.into_rows() {
            self.drop_row(&row, &mut delta)?;
        }
        if let Some(s) = small_fold
            && old.flags & FLAG_DENIED == 0
        {
            small = s;
        }
        delta = delta + small - old.small;
        match new_dir {
            Some(nd) => self.insert_dir(work, &nd, small, delta, 0)?,
            None => {
                let flags = old.flags & !FLAG_DENIED;
                if small != old.small || flags != old.flags {
                    self.set_self(work.node_id, small, flags)?;
                }
                self.add_delta(&work.chain, delta);
            }
        }
        for (name, vals) in new_rows {
            self.insert(work.node_id, &name, &vals)?;
        }
        for (name, row) in old_dirs {
            push.push(Work {
                node_id: row.id,
                path: join_path(&work.path, &name),
                chain: work.chain.child(row.id),
                is_new: false,
                prev: Some(Prev {
                    small: row.small,
                    flags: row.flags,
                }),
                new_dir: None,
                epoch: self.epoch,
            });
        }
        for (name, vals) in new_dirs {
            push.push(Work {
                node_id: 0,
                path: join_path(&work.path, &name),
                chain: work.chain.clone(),
                is_new: true,
                prev: None,
                new_dir: Some(Box::new(NewDir {
                    parent: work.node_id,
                    name,
                    vals,
                })),
                epoch: self.epoch,
            });
        }
        Ok(())
    }

    // Smallest path owns the bytes so ownership does not depend on which worker listed first.
    pub fn resolve_groups(&mut self, promote_if_none: bool) -> Result<()> {
        let keys: Vec<[u8; 16]> = self.regroup.drain().collect();
        let conn = self.conn;
        let mut paths = PathCache::default();
        for key in keys {
            let rows: Vec<(i64, i64, i64, i64, i64)> = {
                let mut st = conn.prepare_cached(
                    "SELECT id, parent_id, flags, alloc, logical FROM node \
                     WHERE volume_id = ?1 AND file_id = ?2 AND kind = 1",
                )?;
                st.query_map(params![self.volume_id, FileId(key)], |r| {
                    Ok((
                        r.get(0)?,
                        r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?
            };
            if rows.is_empty() {
                continue;
            }
            let counted = rows.iter().filter(|r| r.2 & FLAG_HARDLINK_DUP == 0).count();
            // In a walk, zero counted rows means the first sighting was folded as a small file.
            if counted == 0 && !promote_if_none {
                continue;
            }
            let mut owner: Option<((String, String), i64)> = None;
            for r in &rows {
                let p = paths.get(conn, r.0)?.unwrap_or_default();
                let k = (p.to_lowercase(), p);
                if owner.as_ref().is_none_or(|(ok, _)| k < *ok) {
                    owner = Some((k, r.0));
                }
            }
            let owner_id = owner.map(|o| o.1).unwrap_or(rows[0].0);
            for (id, parent, flags, alloc, logical) in rows {
                let want_dup = id != owner_id;
                if (flags & FLAG_HARDLINK_DUP != 0) == want_dup {
                    continue;
                }
                let nf = if want_dup {
                    flags | FLAG_HARDLINK_DUP
                } else {
                    flags & !FLAG_HARDLINK_DUP
                };
                conn.prepare_cached("UPDATE node SET flags = ?1 WHERE id = ?2")?
                    .execute(params![nf, id])?;
                self.writes += 1;
                let d = Delta {
                    alloc,
                    logical,
                    count: 1,
                };
                let chain = self.chain_cached(parent)?;
                self.add_delta(&chain, if want_dup { -d } else { d });
            }
        }
        Ok(())
    }

    // Runs before resolve_groups, so that only rows of the finished walk remain to own a link group.
    pub fn delete_gone(&mut self) -> Result<u64> {
        if self.gone.is_empty() {
            return Ok(0);
        }
        let gone: HashSet<i64> = self.gone.drain(..).collect();
        let conn = self.conn;
        let mut found: Vec<(i64, i64, Delta)> = Vec::new();
        {
            let mut st = conn.prepare_cached(
                "SELECT parent_id, kind, flags, alloc, logical, sub_alloc, sub_logical, sub_count \
                 FROM node WHERE id = ?1",
            )?;
            for &id in &gone {
                let row = st
                    .query_row([id], |r| {
                        let sub = Delta {
                            alloc: r.get(5)?,
                            logical: r.get(6)?,
                            count: r.get(7)?,
                        };
                        Ok((
                            r.get::<_, Option<i64>>(0)?,
                            contrib(r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, sub),
                        ))
                    })
                    .optional()?;
                if let Some((Some(parent), c)) = row {
                    found.push((id, parent, c));
                }
            }
        }
        conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS gone(id INTEGER PRIMARY KEY); DELETE FROM temp.gone;",
        )?;
        for (id, parent, c) in found {
            let chain = self.chain_cached(parent)?;
            let mut cur = Some(&chain);
            let mut under_gone = false;
            while let Some(link) = cur {
                under_gone |= gone.contains(&link.id);
                cur = link.parent.as_ref();
            }
            // A gone ancestor's own delta already covers this row.
            if under_gone {
                continue;
            }
            conn.prepare_cached("INSERT OR IGNORE INTO temp.gone(id) VALUES (?1)")?
                .execute([id])?;
            self.add_delta(&chain, -c);
        }
        let n = conn.execute(
            "WITH RECURSIVE sub(id) AS (SELECT id FROM temp.gone UNION ALL \
             SELECT n.id FROM node n JOIN sub ON n.parent_id = sub.id) \
             DELETE FROM node WHERE id IN (SELECT id FROM sub)",
            [],
        )?;
        conn.execute("DELETE FROM temp.gone", [])?;
        self.writes += n;
        self.stats.deleted += n as u64;
        Ok(n as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoardini_win::{
        FILE_ATTRIBUTE_DIRECTORY, IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
        IO_REPARSE_TAG_WOF,
    };

    fn entry(attrs: u32, tag: u32) -> DirEntry {
        DirEntry {
            name: "x".into(),
            attrs,
            reparse_tag: tag,
            alloc: 0,
            logical: 0,
            ctime_ms: 0,
            atime_ms: 0,
            mtime_ms: 0,
            file_id: [0; 16],
        }
    }

    #[test]
    fn classifies_links_dirs_and_files() {
        let d = FILE_ATTRIBUTE_DIRECTORY;
        let r = FILE_ATTRIBUTE_REPARSE_POINT;
        assert_eq!(classify(&entry(d, 0)), KIND_DIR);
        assert_eq!(
            classify(&entry(d | r, IO_REPARSE_TAG_MOUNT_POINT)),
            KIND_LINK
        );
        assert_eq!(classify(&entry(d | r, IO_REPARSE_TAG_SYMLINK)), KIND_LINK);
        assert_eq!(classify(&entry(d | r, 0)), KIND_LINK);
        assert_eq!(classify(&entry(d | r, 0x9000_601A)), KIND_DIR);
        assert_eq!(classify(&entry(r, IO_REPARSE_TAG_WOF)), KIND_FILE);
        assert_eq!(classify(&entry(r, 0)), KIND_FILE);
        assert_eq!(classify(&entry(r, IO_REPARSE_TAG_SYMLINK)), KIND_LINK);
    }

    // A patch from another connection lands between a parent's listing and its children's.
    #[test]
    fn a_patch_between_parent_and_child_listings_keeps_totals() {
        use crate::index::{
            DirLister, FakeFs, RefreshOptions, patch_rescan, refresh, verify_totals,
        };
        use std::path::Path;

        let dir = tempfile::tempdir().unwrap();
        let db = crate::store::Db::open(dir.path().join("w.db")).unwrap();
        let conn = db.conn().unwrap();
        let fs = FakeFs::new(r"C:\");
        fs.file(r"C:\a\x.txt", 10 << 10);
        fs.file(r"C:\a\y.txt", 20 << 10);
        fs.file(r"C:\b\z.bin", 3 << 20);
        let out = refresh(&conn, &fs, r"C:\", &RefreshOptions::default()).unwrap();
        fs.file(r"C:\c\new.bin", 2 << 20);
        let root_id: i64 = conn
            .query_row("SELECT root_node_id FROM volume", [], |r| r.get(0))
            .unwrap();
        let mut w = Writer::new(
            &conn,
            out.volume_id,
            out.generation,
            1 << 20,
            Dedupe::Walk(SeenSet::new(true)),
        );
        let mut root = Work::existing(root_id, r"C:\".into(), Chain::root(root_id));
        let mut subs = Vec::new();
        w.apply_listing(&mut root, fs.list_dir(Path::new(r"C:\")), &mut subs)
            .unwrap();
        w.commit().unwrap();
        assert!(subs.iter().any(|s| s.prev.is_some()) && subs.iter().any(|s| s.is_new));

        fs.remove(r"C:\a\y.txt");
        let other = db.conn().unwrap();
        patch_rescan(&other, &fs, r"C:\a\y.txt").unwrap();
        patch_rescan(&other, &fs, r"C:\c").unwrap();

        let mut more = Vec::new();
        for mut s in subs {
            let listing = fs.list_dir(Path::new(&s.path));
            w.apply_listing(&mut s, listing, &mut more).unwrap();
        }
        w.commit().unwrap();
        assert!(verify_totals(&conn).unwrap().is_empty());
        let c_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM node WHERE parent_id = ?1 AND name = 'c'",
                [root_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(c_rows, 1, "the walk adopts the row the patch made");
    }
}

use std::collections::{HashMap, HashSet};
use std::io;
use std::ops::{Add, AddAssign, Neg, Sub};
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use hoardini_win::{DirEntry, FILE_ATTRIBUTE_REPARSE_POINT};
use rusqlite::{Connection, OptionalExtension, params};

use super::dedupe::SeenSet;
use super::{
    FLAG_DENIED, FLAG_HARDLINK_DUP, FLAG_PLACEHOLDER, KIND_DIR, KIND_FILE, KIND_LINK, PathCache,
    join_path, to_i64,
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

pub(crate) struct Work {
    pub node_id: i64,
    pub path: String,
    pub chain: Arc<Chain>,
    pub is_new: bool,
}

struct Row {
    id: i64,
    name: String,
    kind: i64,
    alloc: i64,
    logical: i64,
    mtime: Option<i64>,
    atime: Option<i64>,
    ctime: Option<i64>,
    attrs: i64,
    reparse_tag: Option<i64>,
    file_id: Option<Vec<u8>>,
    flags: i64,
    seen_gen: i64,
    sub: Delta,
}

impl Row {
    fn contrib(&self) -> Delta {
        contrib(self.kind, self.flags, self.alloc, self.logical, self.sub)
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
    fn from_entry(e: &DirEntry, kind: i64, flags: i64) -> Vals {
        let (alloc, logical) = if kind == KIND_FILE {
            (to_i64(e.alloc), to_i64(e.logical))
        } else {
            (0, 0)
        };
        let t = |v: i64| (v != 0).then_some(v);
        Vals {
            alloc,
            logical,
            mtime: t(e.mtime_ms),
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
            && self.file_id.as_ref().map(|f| f.as_slice()) == r.file_id.as_deref()
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
    in_tx: bool,
    pub writes: usize,
    pub stats: Stats,
}

const CHILD_COLS: &str = "id, name, kind, alloc, logical, mtime, atime, ctime, attrs, reparse_tag, \
     file_id, flags, seen_gen, sub_alloc, sub_logical, sub_count";

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

    fn self_row(&self, id: i64) -> Result<Option<(Delta, i64, i64)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT small_bytes, small_logical, small_count, flags, seen_gen FROM node WHERE id = ?1",
        )?;
        Ok(st
            .query_row([id], |r| {
                Ok((
                    Delta {
                        alloc: r.get(0)?,
                        logical: r.get(1)?,
                        count: r.get(2)?,
                    },
                    r.get(3)?,
                    r.get(4)?,
                ))
            })
            .optional()?)
    }

    fn children(&self, id: i64) -> Result<Vec<Row>> {
        let mut st = self.conn.prepare_cached(&format!(
            "SELECT {CHILD_COLS} FROM node WHERE parent_id = ?1"
        ))?;
        let rows = st.query_map([id], |r| {
            Ok(Row {
                id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                alloc: r.get(3)?,
                logical: r.get(4)?,
                mtime: r.get(5)?,
                atime: r.get(6)?,
                ctime: r.get(7)?,
                attrs: r.get(8)?,
                reparse_tag: r.get(9)?,
                file_id: r.get(10)?,
                flags: r.get(11)?,
                seen_gen: r.get(12)?,
                sub: Delta {
                    alloc: r.get(13)?,
                    logical: r.get(14)?,
                    count: r.get(15)?,
                },
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn insert(&mut self, parent: i64, name: &str, v: &Vals) -> Result<i64> {
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
            v.file_id,
            v.flags,
            self.generation
        ])?;
        self.writes += 1;
        Ok(self.conn.last_insert_rowid())
    }

    fn upsert_existing(&mut self, row: &Row, v: &Vals) -> Result<()> {
        if v.same_as(row) {
            if row.seen_gen != self.generation {
                self.conn
                    .prepare_cached("UPDATE node SET seen_gen = ?1 WHERE id = ?2")?
                    .execute(params![self.generation, row.id])?;
                self.writes += 1;
            }
            return Ok(());
        }
        self.conn
            .prepare_cached(
                "UPDATE node SET alloc = ?1, logical = ?2, mtime = ?3, atime = ?4, ctime = ?5, \
                 attrs = ?6, reparse_tag = ?7, file_id = ?8, flags = ?9, seen_gen = ?10 WHERE id = ?11",
            )?
            .execute(params![
                v.alloc,
                v.logical,
                v.mtime,
                v.atime,
                v.ctime,
                v.attrs,
                v.reparse_tag,
                v.file_id,
                v.flags,
                self.generation,
                row.id
            ])?;
        self.writes += 1;
        Ok(())
    }

    fn set_self(&mut self, id: i64, small: Delta, flags: i64) -> Result<()> {
        self.conn
            .prepare_cached(
                "UPDATE node SET small_bytes = ?1, small_logical = ?2, small_count = ?3, flags = ?4, \
                 seen_gen = ?5 WHERE id = ?6",
            )?
            .execute(params![
                small.alloc,
                small.logical,
                small.count,
                flags,
                self.generation,
                id
            ])?;
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
        for key in ids.query_map([id], |r| r.get::<_, Vec<u8>>(0))? {
            if let Ok(k) = <[u8; 16]>::try_from(key?.as_slice()) {
                self.regroup.insert(k);
            }
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

    pub fn apply_listing(
        &mut self,
        work: &Work,
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
        for key in st.query_map([id], |r| r.get::<_, Vec<u8>>(0))? {
            if let Ok(k) = <[u8; 16]>::try_from(key?.as_slice()) {
                self.regroup.insert(k);
            }
        }
        Ok(())
    }

    // small_fold: the caller derived the fold from the stored one, keeping its hard-link exclusions.
    pub fn apply_listing_with(
        &mut self,
        work: &Work,
        listing: io::Result<Vec<DirEntry>>,
        push: &mut Vec<Work>,
        small_fold: Option<Delta>,
    ) -> Result<()> {
        self.ensure_tx()?;
        let Some((old_small, old_flags, old_seen)) = self.self_row(work.node_id)? else {
            return Ok(());
        };
        self.stats.dirs += 1;
        let walk = matches!(self.dedupe, Dedupe::Walk(_));
        let entries = match listing {
            Ok(e) => e,
            Err(err) => {
                tracing::debug!(path = %work.path, error = %err, "cannot list directory");
                self.stats.denied += 1;
                let mut delta = -old_small;
                if walk {
                    // A child listed early as a hotspot must not outlive an unreadable parent.
                    let n = self
                        .conn
                        .prepare_cached(
                            "UPDATE node SET seen_gen = 0 WHERE parent_id = ?1 AND seen_gen = ?2",
                        )?
                        .execute(params![work.node_id, self.generation])?;
                    self.writes += n;
                } else {
                    for row in self.children(work.node_id)? {
                        delta = delta - row.contrib();
                        self.delete_rows(row.id)?;
                    }
                }
                self.set_self(work.node_id, Delta::default(), old_flags | FLAG_DENIED)?;
                self.add_delta(&work.chain, delta);
                return Ok(());
            }
        };

        let mut existing: HashMap<(String, i64), Row> = self
            .children(work.node_id)?
            .into_iter()
            .map(|r| ((r.name.clone(), r.kind), r))
            .collect();
        let mut small = Delta::default();
        let mut delta = Delta::default();
        let mut small_links: HashSet<[u8; 16]> = HashSet::new();
        for e in entries {
            match classify(&e) {
                KIND_DIR => {
                    let vals = Vals::from_entry(&e, KIND_DIR, 0);
                    let (id, is_new) = match existing.remove(&(e.name.clone(), KIND_DIR)) {
                        Some(row) => {
                            let vals = Vals {
                                flags: row.flags,
                                ..vals
                            };
                            self.upsert_existing(&row, &vals)?;
                            (row.id, false)
                        }
                        None => (self.insert(work.node_id, &e.name, &vals)?, true),
                    };
                    if !self.hot_ids.contains(&id) {
                        push.push(Work {
                            node_id: id,
                            path: join_path(&work.path, &e.name),
                            chain: work.chain.child(id),
                            is_new,
                        });
                    }
                }
                KIND_LINK => {
                    let vals = Vals::from_entry(&e, KIND_LINK, 0);
                    match existing.remove(&(e.name.clone(), KIND_LINK)) {
                        Some(row) => self.upsert_existing(&row, &vals)?,
                        None => {
                            self.insert(work.node_id, &e.name, &vals)?;
                        }
                    }
                }
                _ => {
                    self.stats.files += 1;
                    let row_sized = e.alloc.max(e.logical) >= self.threshold;
                    let key = (e.file_id != [0; 16]).then_some(e.file_id);
                    let prev = existing.remove(&(e.name.clone(), KIND_FILE));
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
                                r.flags & FLAG_HARDLINK_DUP != 0
                                    && r.file_id.as_deref() == Some(&k[..])
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
                                self.insert(work.node_id, &e.name, &vals)?;
                            }
                        }
                    } else {
                        if let Some(row) = prev {
                            existing.insert((row.name.clone(), KIND_FILE), row);
                        }
                        if !dup {
                            small += Delta {
                                alloc: to_i64(e.alloc),
                                logical: to_i64(e.logical),
                                count: 1,
                            };
                        }
                    }
                }
            }
        }
        for row in existing.into_values() {
            if !walk {
                delta = delta - row.contrib();
                self.delete_rows(row.id)?;
            } else if row.seen_gen == self.generation {
                // Seen earlier this generation (a hotspot) but gone from its parent's listing now.
                self.conn
                    .prepare_cached("UPDATE node SET seen_gen = 0 WHERE id = ?1")?
                    .execute([row.id])?;
                self.writes += 1;
            }
        }
        if let Some(s) = small_fold
            && old_flags & FLAG_DENIED == 0
        {
            small = s;
        }
        delta = delta + small - old_small;
        let flags = old_flags & !FLAG_DENIED;
        if small != old_small || flags != old_flags || old_seen != self.generation {
            self.set_self(work.node_id, small, flags)?;
        }
        self.add_delta(&work.chain, delta);
        Ok(())
    }

    // Smallest path owns the bytes so ownership does not depend on which worker listed first.
    pub fn resolve_groups(&mut self, promote_if_none: bool) -> Result<()> {
        let keys: Vec<[u8; 16]> = self.regroup.drain().collect();
        let this_gen_only = matches!(self.dedupe, Dedupe::Walk(_));
        let conn = self.conn;
        let mut paths = PathCache::default();
        for key in keys {
            let rows: Vec<(i64, i64, i64, i64, i64)> = {
                let mut st = conn.prepare_cached(
                    "SELECT id, parent_id, flags, alloc, logical FROM node \
                     WHERE volume_id = ?1 AND file_id = ?2 AND kind = 1 AND (?3 = 0 OR seen_gen = ?4)",
                )?;
                st.query_map(
                    params![self.volume_id, &key[..], this_gen_only, self.generation],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                        ))
                    },
                )?
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

    pub fn delete_unseen(&mut self) -> Result<u64> {
        let conn = self.conn;
        let roots: Vec<(i64, i64, Delta)> = {
            let mut st = conn.prepare(
                "SELECT n.id, n.parent_id, n.kind, n.flags, n.alloc, n.logical, n.sub_alloc, \
                 n.sub_logical, n.sub_count FROM node n JOIN node p ON p.id = n.parent_id \
                 WHERE n.volume_id = ?1 AND n.seen_gen <> ?2 AND p.seen_gen = ?2",
            )?;
            st.query_map(params![self.volume_id, self.generation], |r| {
                let sub = Delta {
                    alloc: r.get(6)?,
                    logical: r.get(7)?,
                    count: r.get(8)?,
                };
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    contrib(r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, sub),
                ))
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        if roots.is_empty() {
            return Ok(0);
        }
        conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS gone(id INTEGER PRIMARY KEY); DELETE FROM temp.gone;",
        )?;
        {
            let mut ins = conn.prepare_cached("INSERT OR IGNORE INTO temp.gone(id) VALUES (?1)")?;
            for (id, _, _) in &roots {
                ins.execute([id])?;
            }
        }
        for (_, parent, c) in roots {
            let chain = self.chain_cached(parent)?;
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
}

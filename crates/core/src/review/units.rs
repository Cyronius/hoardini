use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use crate::catalog::{Entry, UnitKind};
use crate::index::{
    self, DirLister, NodeInfo, NodeKind, PathCache, is_ancestor_path, normalize_path, to_u64,
};
use crate::memory::{self, knowledge, location};
use crate::policy::{Area, PolicyEnv, Verdict, area_of};

const GIB: u64 = 1 << 30;
pub const DESCEND_BYTES: u64 = GIB;
pub const DESCEND_PCT: u64 = 10;
pub const STORE_WEIGHT_PCT: u64 = 80;
// Keeps the weight scan on node_file_alloc's range; smaller weight files count as other bytes.
pub const WEIGHT_MIN_BYTES: u64 = 16 << 20;
pub const SPLIT_DEPTH: usize = 3;
const HOUR_MS: i64 = 3_600_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Why {
    Catalog,
    ModelStore,
    Leaf,
    Kept,
    Disposable,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Part {
    pub path: String,
    pub size: u64,
    pub mtime_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Unit {
    pub path: String,
    pub kind: UnitKind,
    pub why: Why,
    pub catalog_id: Option<String>,
    pub container: Option<String>,
    pub size: u64,
    pub is_dir: bool,
    pub mtime_ms: Option<i64>,
    pub parts: Vec<Part>,
}

struct Walker<'a> {
    conn: &'a Connection,
    env: &'a PolicyEnv,
    lister: &'a dyn DirLister,
    now: i64,
    kept_locations: Vec<String>,
    disposable: Vec<String>,
    weight_sub: HashMap<i64, u64>,
    weight_direct: HashMap<i64, u64>,
    seen: HashSet<String>,
    out: Vec<Unit>,
}

pub fn resolve(
    conn: &Connection,
    env: &PolicyEnv,
    lister: &dyn DirLister,
    root: &str,
    now: i64,
) -> anyhow::Result<Vec<Unit>> {
    let root = normalize_path(&env.expand(root));
    let node = index::node_by_path(conn, &root)?
        .ok_or_else(|| anyhow::anyhow!("{root} is not in the index"))?;
    let mut w = Walker {
        conn,
        env,
        lister,
        now,
        kept_locations: Vec::new(),
        disposable: Vec::new(),
        weight_sub: HashMap::new(),
        weight_direct: HashMap::new(),
        seen: HashSet::new(),
        out: Vec::new(),
    };
    // Keep rows from the deny list or the catalog are re-derived by the walk itself, so only the others stop it.
    for m in location::locations(conn)? {
        let verdict = m.detail.as_ref().and_then(|d| d.verdict);
        if verdict == Some(Verdict::Keep) && !matches!(m.source.as_str(), "deny" | "catalog") {
            w.kept_locations.push(location::expanded(env, &m.key));
        } else if verdict == Some(Verdict::Safe) && m.source == "user" {
            w.disposable.push(location::expanded(env, &m.key));
        }
    }
    w.index_weights(node.id)?;
    w.visit(&node, None)?;
    Ok(w.out)
}

impl Walker<'_> {
    fn index_weights(&mut self, root_id: i64) -> anyhow::Result<()> {
        let weights = knowledge::weight_extensions(self.conn)?;
        let volume: i64 =
            self.conn
                .query_row("SELECT volume_id FROM node WHERE id = ?1", [root_id], |r| {
                    r.get(0)
                })?;
        let mut st = self.conn.prepare(
            "SELECT parent_id, name, alloc FROM node WHERE kind = 1 AND alloc >= ?1 \
             AND volume_id = ?2 AND (flags & 4) = 0",
        )?;
        let files = st
            .query_map(
                rusqlite::params![index::to_i64(WEIGHT_MIN_BYTES), volume],
                |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut parent_of = self
            .conn
            .prepare_cached("SELECT parent_id FROM node WHERE id = ?1")?;
        let mut parents: HashMap<i64, Option<i64>> = HashMap::new();
        let mut configs: HashMap<i64, bool> = HashMap::new();
        let mut paths = PathCache::default();
        for (parent, name, alloc) in files {
            let Some(parent) = parent else {
                continue;
            };
            let ext = match name.rfind('.') {
                Some(i) if i > 0 => name[i + 1..].to_ascii_lowercase(),
                _ => continue,
            };
            let weight = if ext == "bin" {
                match configs.get(&parent) {
                    Some(b) => *b,
                    None => {
                        let has = paths.get(self.conn, parent)?.is_some_and(|dir| {
                            self.lister.list_dir(Path::new(&dir)).is_ok_and(|es| {
                                es.iter()
                                    .any(|e| e.name.eq_ignore_ascii_case("config.json"))
                            })
                        });
                        configs.insert(parent, has);
                        has
                    }
                }
            } else {
                weights.contains(&ext)
            };
            if !weight {
                continue;
            }
            let bytes = to_u64(alloc);
            let mut cur = Some(parent);
            let mut level = 0;
            while let Some(c) = cur {
                *self.weight_sub.entry(c).or_default() += bytes;
                if level == 1 {
                    *self.weight_direct.entry(c).or_default() += bytes;
                }
                level += 1;
                cur = match parents.get(&c) {
                    Some(p) => *p,
                    None => {
                        let p: Option<i64> = parent_of.query_row([c], |r| r.get(0))?;
                        parents.insert(c, p);
                        p
                    }
                };
            }
        }
        Ok(())
    }

    fn is_model_store(&self, node: &NodeInfo) -> bool {
        let sub = self.weight_sub.get(&node.id).copied().unwrap_or(0);
        let direct = self.weight_direct.get(&node.id).copied().unwrap_or(0);
        sub > 0
            && sub * 100 >= node.size * STORE_WEIGHT_PCT
            && direct * 100 >= sub * STORE_WEIGHT_PCT
    }

    fn protected(&self, path: &str) -> anyhow::Result<bool> {
        Ok(memory::protect_covering_quiet(self.conn, path, &self.env.profile)?.is_some())
    }

    fn guarded(&self, node: &NodeInfo) -> anyhow::Result<Option<Why>> {
        if self.protected(&node.path)? {
            return Ok(Some(Why::Kept));
        }
        if node.kind == NodeKind::Dir
            && self
                .disposable
                .iter()
                .any(|d| d.eq_ignore_ascii_case(&node.path))
        {
            return Ok(Some(Why::Disposable));
        }
        Ok(self.kept(&node.path).then_some(Why::Kept))
    }

    fn kept(&self, path: &str) -> bool {
        self.kept_locations
            .iter()
            .any(|k| k.eq_ignore_ascii_case(path) || is_ancestor_path(k, path))
    }

    fn push(
        &mut self,
        node: &NodeInfo,
        kind: UnitKind,
        why: Why,
        entry: Option<&Entry>,
        container: Option<&str>,
    ) {
        self.out.push(Unit {
            path: node.path.clone(),
            kind,
            why,
            catalog_id: entry.map(|e| e.id.clone()),
            container: container.map(str::to_string),
            size: node.size,
            is_dir: node.kind == NodeKind::Dir,
            mtime_ms: node.mtime_ms,
            parts: Vec::new(),
        });
    }

    fn visit(&mut self, node: &NodeInfo, container: Option<&str>) -> anyhow::Result<()> {
        if node.kind == NodeKind::Link || !self.seen.insert(node.path.to_lowercase()) {
            return Ok(());
        }
        let catalog = self.env.catalog();
        let exact = catalog.lookup_exact(&node.path);
        if self.protected(&node.path)? {
            self.push(node, UnitKind::Whole, Why::Kept, exact, container);
            return Ok(());
        }
        if node.kind == NodeKind::Dir
            && self
                .disposable
                .iter()
                .any(|d| d.eq_ignore_ascii_case(&node.path))
        {
            self.push(node, UnitKind::Whole, Why::Disposable, exact, container);
            return Ok(());
        }
        if self.kept(&node.path) {
            self.push(node, UnitKind::Whole, Why::Kept, exact, container);
            return Ok(());
        }
        if let Some(e) = exact
            && e.verdict() != Verdict::Keep
            && e.unit != UnitKind::Container
        {
            return self.by_catalog(node, e, container);
        }
        let system = matches!(area_of(self.env, &node.path), Some(Area::System { .. }));
        if system || exact.is_some_and(|e| e.verdict() == Verdict::Keep) {
            self.push(node, UnitKind::Whole, Why::Kept, exact, container);
            return self.inside_kept(node);
        }
        if node.kind == NodeKind::File {
            self.push(node, UnitKind::Whole, Why::Leaf, None, container);
            return Ok(());
        }
        if exact.is_none() && self.is_model_store(node) {
            for c in index::children(self.conn, node.id)? {
                if c.kind != NodeKind::Link {
                    self.push(
                        &c,
                        UnitKind::Children,
                        Why::ModelStore,
                        None,
                        Some(&node.path),
                    );
                }
            }
            return Ok(());
        }
        self.descend(node, container)
    }

    fn descend(&mut self, node: &NodeInfo, container: Option<&str>) -> anyhow::Result<()> {
        let big: Vec<NodeInfo> = index::children(self.conn, node.id)?
            .into_iter()
            .filter(|c| {
                c.kind != NodeKind::Link
                    && c.size > 0
                    && (c.size >= DESCEND_BYTES || c.size * 100 >= node.size * DESCEND_PCT)
            })
            .collect();
        if big.is_empty() {
            self.push(node, UnitKind::Whole, Why::Leaf, None, container);
            return Ok(());
        }
        for c in &big {
            self.visit(c, Some(&node.path))?;
        }
        Ok(())
    }

    // A kept folder can still hold catalog locations named literally, such as Windows\Temp.
    fn inside_kept(&mut self, node: &NodeInfo) -> anyhow::Result<()> {
        let inner: Vec<String> = self
            .env
            .catalog()
            .literal_paths_under(&node.path)
            .into_iter()
            .filter(|(_, e)| e.verdict() != Verdict::Keep && e.unit != UnitKind::Container)
            .map(|(p, _)| p)
            .collect();
        for p in inner {
            if let Some(n) = index::node_by_path(self.conn, &p)? {
                self.visit(&n, Some(&node.path))?;
            }
        }
        Ok(())
    }

    fn by_catalog(
        &mut self,
        node: &NodeInfo,
        e: &Entry,
        container: Option<&str>,
    ) -> anyhow::Result<()> {
        if node.kind == NodeKind::File {
            self.push(node, UnitKind::Whole, Why::Catalog, Some(e), container);
            return Ok(());
        }
        match e.unit {
            UnitKind::Children => {
                let mut level = vec![node.clone()];
                for _ in 0..e.unit_depth.unwrap_or(1).max(1) {
                    let mut next = Vec::new();
                    for n in level {
                        if n.kind == NodeKind::Dir {
                            next.extend(index::children(self.conn, n.id)?);
                        } else {
                            next.push(n);
                        }
                    }
                    level = next;
                }
                for c in level.iter().filter(|c| c.kind != NodeKind::Link) {
                    match self.guarded(c)? {
                        Some(why) => {
                            let own = self.env.catalog().lookup_exact(&c.path);
                            self.push(c, UnitKind::Whole, why, own, Some(&node.path));
                        }
                        None => self.push(
                            c,
                            UnitKind::Children,
                            Why::Catalog,
                            Some(e),
                            Some(&node.path),
                        ),
                    }
                }
            }
            UnitKind::Contents => {
                let mut parts = self.contents(node, e.min_age_hours)?;
                // A part the user protects or keeps is a kept unit of its own, never an item of the folder's.
                let mut held = Vec::new();
                for p in &parts {
                    if self.protected(&p.path)? || self.kept(&p.path) {
                        held.push(p.path.clone());
                    }
                }
                parts.retain(|p| !held.contains(&p.path));
                for h in &held {
                    if let Some(n) = index::node_by_path(self.conn, h)? {
                        self.push(&n, UnitKind::Whole, Why::Kept, None, Some(&node.path));
                    }
                }
                self.out.push(Unit {
                    path: node.path.clone(),
                    kind: UnitKind::Contents,
                    why: Why::Catalog,
                    catalog_id: Some(e.id.clone()),
                    container: container.map(str::to_string),
                    size: parts.iter().map(|p| p.size).sum(),
                    is_dir: true,
                    mtime_ms: node.mtime_ms,
                    parts,
                });
            }
            UnitKind::Whole | UnitKind::Container => {
                self.push(node, UnitKind::Whole, Why::Catalog, Some(e), container)
            }
        }
        Ok(())
    }

    fn contents(&self, node: &NodeInfo, min_age_hours: Option<u32>) -> anyhow::Result<Vec<Part>> {
        let cutoff = min_age_hours.map(|h| self.now - i64::from(h) * HOUR_MS);
        let mut out = Vec::new();
        self.parts(node, cutoff, 0, &mut out)?;
        Ok(out)
    }

    // A child counts as modified when it or anything indexed inside it was.
    fn parts(
        &self,
        node: &NodeInfo,
        cutoff: Option<i64>,
        depth: usize,
        out: &mut Vec<Part>,
    ) -> anyhow::Result<()> {
        let kids = index::children(self.conn, node.id)?;
        let newest: HashMap<i64, Option<i64>> = match cutoff {
            None => HashMap::new(),
            Some(_) => {
                let mut st = self.conn.prepare(
                    "WITH RECURSIVE d(id, top) AS (SELECT id, id FROM node WHERE parent_id = ?1 \
                     UNION ALL SELECT n.id, d.top FROM node n JOIN d ON n.parent_id = d.id) \
                     SELECT d.top, MAX(n.mtime) FROM d JOIN node n ON n.id = d.id GROUP BY d.top",
                )?;
                st.query_map([node.id], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?))
                })?
                .collect::<rusqlite::Result<_>>()?
            }
        };
        for k in kids.into_iter().filter(|k| k.kind != NodeKind::Link) {
            let last = newest.get(&k.id).copied().flatten();
            let old = match cutoff {
                None => true,
                Some(c) => last.is_some_and(|t| t <= c),
            };
            if old {
                out.push(Part {
                    mtime_ms: last.or(k.mtime_ms),
                    path: k.path,
                    size: k.size,
                });
            } else if k.kind == NodeKind::Dir && k.size >= DESCEND_BYTES && depth < SPLIT_DEPTH {
                // Split, not skipped, so one live session's folder does not hide the old ones beside it.
                self.parts(&k, cutoff, depth + 1, out)?;
            }
        }
        Ok(())
    }
}

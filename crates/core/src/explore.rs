use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::catalog::{Category, UnitKind};
use crate::index::{self, NodeInfo, NodeKind, children_at, ext_of, normalize_path, path_of};
use crate::memory::knowledge::{self, Detail, MODEL_WEIGHTS, WEIGHT_EXTS};
use crate::memory::location::{self, DOCUMENT_EXTS, PHOTO_VIDEO_EXTS};
use crate::policy::{Area, PolicyEnv, Verdict, area_of};
use crate::store::{get_meta, set_meta};

pub const MIN_BLOCK_PX: u64 = 9;
pub const MAX_NODES: usize = 5000;
pub const MAX_TYPES: usize = 200;
const FOCUS_KEY: &str = "explore_focus";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TagState {
    Proposed,
    Review,
    Keep,
    System,
    User,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Tag {
    pub state: TagState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MapKind {
    Dir,
    File,
    Link,
    Small,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct More {
    pub count: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MapNode {
    pub path: String,
    pub name: String,
    pub kind: MapKind,
    pub size: u64,
    pub files: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<MapNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more: Option<More>,
    pub tag: Tag,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Crumb {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Map {
    pub tree: MapNode,
    pub ancestors: Vec<Crumb>,
    pub min_bytes: u64,
    pub nodes: usize,
    pub capped: bool,
}

#[derive(Clone, Debug)]
struct Known {
    state: TagState,
    label: Option<String>,
    verdict: Option<Verdict>,
}

impl Known {
    fn tag(&self) -> Tag {
        Tag {
            state: self.state,
            label: self.label.clone(),
            verdict: self.verdict,
        }
    }
}

const USER_CATEGORIES: &[&str] = &["photos", "documents", "user", "user-data"];

fn from_detail(d: &Detail) -> Option<Known> {
    if d.category == "container" {
        return None;
    }
    let state = if matches!(d.category.as_str(), "system" | "protected") {
        TagState::System
    } else {
        match d.verdict {
            Some(Verdict::Keep) if USER_CATEGORIES.contains(&d.category.as_str()) => TagState::User,
            Some(Verdict::Keep) => TagState::Keep,
            Some(_) => TagState::Review,
            None if USER_CATEGORIES.contains(&d.category.as_str()) => TagState::User,
            None => TagState::Unknown,
        }
    };
    let label = d
        .label
        .clone()
        .or_else(|| d.app.clone())
        .or_else(|| (!d.category.is_empty()).then(|| d.category.clone()));
    Some(Known {
        state,
        label,
        verdict: d.verdict,
    })
}

fn from_catalog(e: &crate::catalog::Entry) -> Known {
    let verdict = e.verdict();
    let state = match (e.category, verdict) {
        (Category::System, _) => TagState::System,
        (Category::UserData, Verdict::Keep) => TagState::User,
        (_, Verdict::Keep) => TagState::Keep,
        _ => TagState::Review,
    };
    Known {
        state,
        label: Some(e.name.clone()),
        verdict: Some(verdict),
    }
}

#[derive(Clone, Default)]
struct Ctx {
    proposed: Option<Known>,
    known: Option<Known>,
    system: Option<String>,
}

// Everything the tags need, read once so a map of thousands of nodes costs no query per node.
struct Tagger<'a> {
    env: &'a PolicyEnv,
    rows: HashMap<String, Known>,
    containers: HashMap<String, String>,
    protect: HashMap<String, Known>,
    pending: HashMap<String, Known>,
}

impl<'a> Tagger<'a> {
    fn load(conn: &Connection, env: &'a PolicyEnv) -> Result<Tagger<'a>> {
        let mut rows = HashMap::new();
        let mut containers = HashMap::new();
        for m in location::locations(conn)? {
            if m.forgotten_ms.is_some() {
                continue;
            }
            let Some(d) = &m.detail else { continue };
            let key = location::expanded(env, &m.key).to_lowercase();
            match from_detail(d) {
                Some(k) => {
                    rows.insert(key, k);
                }
                None => {
                    if let Some(l) = d.label.clone() {
                        containers.insert(key, l);
                    }
                }
            }
        }
        let mut protect = HashMap::new();
        let mut st = conn.prepare(
            "SELECT key, text FROM memory WHERE scope = 'path' AND kind = 'protect' AND forgotten_ms IS NULL",
        )?;
        for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (key, text) = r?;
            protect.insert(
                location::expanded(env, &key).to_lowercase(),
                Known {
                    state: TagState::Keep,
                    label: Some(text),
                    verdict: Some(Verdict::Keep),
                },
            );
        }
        let mut pending = HashMap::new();
        let mut st = conn.prepare(
            "SELECT i.path, i.reason, i.verdict, p.params_json FROM proposal_item i JOIN proposal p ON p.id = i.proposal_id \
             WHERE p.status = 'pending' AND i.status = 'pending' ORDER BY p.created_ms, i.seq",
        )?;
        let items = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for it in items {
            let (path, reason, verdict, params) = it?;
            let summary = serde_json::from_str::<serde_json::Value>(&params)
                .ok()
                .and_then(|v| v.get("summary")?.as_str().map(str::to_string))
                .filter(|s| !s.trim().is_empty());
            pending
                .entry(normalize_path(&path).to_lowercase())
                .or_insert(Known {
                    state: TagState::Proposed,
                    label: reason.filter(|r| !r.trim().is_empty()).or(summary),
                    verdict: verdict.as_deref().and_then(Verdict::parse),
                });
        }
        Ok(Tagger {
            env,
            rows,
            containers,
            protect,
            pending,
        })
    }

    // The nearest knowledge wins: a deeper row or catalog entry overrides what a folder above says.
    fn step(&self, parent: &Ctx, path: &str) -> (Ctx, Tag) {
        let key = path.to_lowercase();
        let proposed = self
            .pending
            .get(&key)
            .cloned()
            .or_else(|| parent.proposed.clone());
        let mut self_label = self.containers.get(&key).cloned();
        let exact = self
            .protect
            .get(&key)
            .or_else(|| self.rows.get(&key))
            .cloned()
            .or_else(|| {
                let e = self.env.catalog().lookup_exact(path)?;
                if e.unit == UnitKind::Container {
                    self_label.get_or_insert_with(|| e.name.clone());
                    return None;
                }
                Some(from_catalog(e))
            });
        let known = exact.or_else(|| parent.known.clone());
        let system = match (&known, &parent.system) {
            (Some(_), _) => None,
            (None, Some(s)) => Some(s.clone()),
            (None, None) => match area_of(self.env, path) {
                Some(Area::System { label }) => Some(label),
                Some(Area::Container { label }) => {
                    self_label.get_or_insert(label);
                    None
                }
                None => None,
            },
        };
        let tag = if let Some(p) = &proposed {
            p.tag()
        } else if let Some(k) = &known {
            k.tag()
        } else if let Some(label) = &system {
            Tag {
                state: TagState::System,
                label: Some(label.clone()),
                verdict: Some(Verdict::Keep),
            }
        } else {
            Tag {
                state: TagState::Unknown,
                label: self_label,
                verdict: None,
            }
        };
        (
            Ctx {
                proposed,
                known,
                system,
            },
            tag,
        )
    }
}

fn chain(conn: &Connection, node_id: i64) -> Result<Vec<(i64, String)>> {
    let mut ids = Vec::new();
    let mut cur = Some(node_id);
    let mut st = conn.prepare_cached("SELECT parent_id FROM node WHERE id = ?1")?;
    while let Some(id) = cur {
        ids.push(id);
        if ids.len() > 4096 {
            anyhow::bail!("node {node_id}: parent chain does not end");
        }
        cur = st
            .query_row([id], |r| r.get::<_, Option<i64>>(0))
            .optional()?
            .flatten();
    }
    ids.reverse();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(p) = path_of(conn, id)? {
            out.push((id, p));
        }
    }
    Ok(out)
}

fn name_of(path: &str) -> String {
    match path.trim_end_matches('\\').rsplit_once('\\') {
        Some((_, n)) if !n.is_empty() => n.to_string(),
        _ => path.trim_end_matches('\\').to_string(),
    }
}

struct Slot {
    node: MapNode,
    id: i64,
    small: (u64, u64),
    ctx: Ctx,
    kids: Vec<usize>,
    more: Option<More>,
}

enum Entry {
    Row(NodeInfo),
    Small { count: u64, bytes: u64 },
}

impl Entry {
    fn size(&self) -> u64 {
        match self {
            Entry::Row(n) if n.kind == NodeKind::File && n.hardlink_dup => 0,
            Entry::Row(n) => n.size,
            Entry::Small { bytes, .. } => *bytes,
        }
    }
    fn count(&self) -> u64 {
        match self {
            Entry::Row(_) => 1,
            Entry::Small { count, .. } => *count,
        }
    }
}

pub fn min_bytes(size: u64, width: u32, height: u32) -> u64 {
    let area = u128::from(width.max(1)) * u128::from(height.max(1));
    let need = (u128::from(size) * u128::from(MIN_BLOCK_PX)).div_ceil(area);
    u64::try_from(need).unwrap_or(u64::MAX).max(1)
}

// Biggest folders open first, so the node cap cuts the smallest blocks.
pub fn map(
    conn: &Connection,
    env: &PolicyEnv,
    root: &NodeInfo,
    width: u32,
    height: u32,
    max_nodes: usize,
) -> Result<Map> {
    let tagger = Tagger::load(conn, env)?;
    let line = chain(conn, root.id)?;
    let mut ctx = Ctx::default();
    let mut tag = Tag {
        state: TagState::Unknown,
        label: None,
        verdict: None,
    };
    for (_, p) in &line {
        (ctx, tag) = tagger.step(&ctx, p);
    }
    let ancestors: Vec<Crumb> = line[..line.len().saturating_sub(1)]
        .iter()
        .map(|(_, p)| Crumb {
            path: p.clone(),
            name: name_of(p),
        })
        .collect();
    let min = min_bytes(root.size, width, height);
    let max_nodes = max_nodes.max(1);
    let mut slots = vec![Slot {
        node: MapNode {
            path: root.path.clone(),
            name: name_of(&root.path),
            kind: MapKind::Dir,
            size: root.size,
            files: root.files,
            ext: None,
            children: None,
            more: None,
            tag,
        },
        id: root.id,
        small: (root.small_count, root.small_bytes),
        ctx,
        kids: Vec::new(),
        more: None,
    }];
    let mut heap = BinaryHeap::new();
    heap.push((root.size, Reverse(0usize)));
    let mut capped = false;
    while let Some((_, Reverse(i))) = heap.pop() {
        if slots.len() >= max_nodes {
            capped = true;
            break;
        }
        let (id, path) = (slots[i].id, slots[i].node.path.clone());
        let parent_ctx = slots[i].ctx.clone();
        let mut entries: Vec<Entry> = children_at(conn, id, &path)?
            .into_iter()
            .map(Entry::Row)
            .collect();
        let (count, bytes) = slots[i].small;
        if bytes > 0 {
            entries.push(Entry::Small { count, bytes });
        }
        entries.sort_by(|a, b| {
            b.size()
                .cmp(&a.size())
                .then_with(|| {
                    matches!(a, Entry::Small { .. }).cmp(&matches!(b, Entry::Small { .. }))
                })
                .then_with(|| match (a, b) {
                    (Entry::Row(x), Entry::Row(y)) => x.name.cmp(&y.name),
                    _ => std::cmp::Ordering::Equal,
                })
        });
        let listed: u64 = entries.iter().map(Entry::size).sum();
        let mut more = More::default();
        let mut kids = Vec::new();
        for e in entries {
            let size = e.size();
            if size == 0 || size < min || slots.len() >= max_nodes {
                if size >= min && size > 0 {
                    capped = true;
                }
                more.count += e.count();
                more.bytes += size;
                continue;
            }
            let (node, id, small, ctx, open) = match e {
                Entry::Row(n) => {
                    let (ctx, tag) = tagger.step(&parent_ctx, &n.path);
                    let kind = match n.kind {
                        NodeKind::Dir => MapKind::Dir,
                        NodeKind::File => MapKind::File,
                        NodeKind::Link => MapKind::Link,
                    };
                    let open = n.kind == NodeKind::Dir;
                    let small = (n.small_count, n.small_bytes);
                    let node = MapNode {
                        ext: (n.kind == NodeKind::File).then(|| ext_of(&n.name)),
                        path: n.path,
                        name: n.name,
                        kind,
                        size,
                        files: n.files,
                        children: None,
                        more: None,
                        tag,
                    };
                    (node, n.id, small, ctx, open)
                }
                Entry::Small { count, bytes } => {
                    let tag = slots[i].node.tag.clone();
                    let node = MapNode {
                        path: path.clone(),
                        name: "small files".into(),
                        kind: MapKind::Small,
                        size: bytes,
                        files: count,
                        ext: None,
                        children: None,
                        more: None,
                        tag,
                    };
                    (node, id, (0, 0), parent_ctx.clone(), false)
                }
            };
            let k = slots.len();
            if open {
                heap.push((size, Reverse(k)));
            }
            slots.push(Slot {
                node,
                id,
                small,
                ctx,
                kids: Vec::new(),
                more: None,
            });
            kids.push(k);
        }
        // An index that disagrees with itself still adds up: the gap is counted with the rest.
        more.bytes += slots[i].node.size.saturating_sub(listed);
        slots[i].kids = kids;
        slots[i].more = Some(more);
    }
    if !heap.is_empty() {
        capped = true;
    }
    let nodes = slots.len();
    let mut taken: Vec<Option<Slot>> = slots.into_iter().map(Some).collect();
    let tree = assemble(&mut taken, 0);
    Ok(Map {
        tree,
        ancestors,
        min_bytes: min,
        nodes,
        capped,
    })
}

fn assemble(slots: &mut [Option<Slot>], i: usize) -> MapNode {
    let Some(s) = slots[i].take() else {
        unreachable!("each slot has one parent")
    };
    let mut node = s.node;
    if let Some(more) = s.more {
        let kids: Vec<MapNode> = s.kids.iter().map(|&k| assemble(slots, k)).collect();
        node.size = kids.iter().map(|k| k.size).sum::<u64>() + more.bytes;
        node.children = Some(kids);
        node.more = Some(more);
    }
    node
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TypeRow {
    pub ext: Option<String>,
    pub bytes: u64,
    pub files: u64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub small: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub use_: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

fn fixed_category(ext: &str) -> Option<&'static str> {
    let has = |list: &[&str]| list.iter().any(|e| e.eq_ignore_ascii_case(ext));
    if has(PHOTO_VIDEO_EXTS) {
        Some("photos")
    } else if has(DOCUMENT_EXTS) {
        Some("documents")
    } else if has(WEIGHT_EXTS) {
        Some(MODEL_WEIGHTS)
    } else {
        None
    }
}

pub fn types(conn: &Connection, node_id: i64) -> Result<Vec<TypeRow>> {
    let mix = index::ext_mix(conn, node_id)?;
    let mut known: HashMap<String, (String, Detail)> = HashMap::new();
    for m in knowledge::filetypes(conn)? {
        if m.forgotten_ms.is_some() {
            continue;
        }
        if let Some(ext) = m.key.rsplit("*.").next() {
            known.insert(
                ext.to_ascii_lowercase(),
                (m.text.clone(), m.detail.unwrap_or_default()),
            );
        }
    }
    let mut out: Vec<TypeRow> = Vec::new();
    let mut rest = (0u64, 0u64, 0u64);
    for t in mix.types {
        if out.len() >= MAX_TYPES {
            rest.0 += 1;
            rest.1 += t.bytes;
            rest.2 += t.files;
            continue;
        }
        let row = known.get(&t.ext);
        let category = row
            .map(|(_, d)| d.category.clone())
            .filter(|c| !c.is_empty())
            .or_else(|| fixed_category(&t.ext).map(str::to_string));
        out.push(TypeRow {
            label: row.map(|(text, _)| text.clone()),
            app: row.and_then(|(_, d)| d.app.clone()),
            use_: row.and_then(|(_, d)| d.use_.clone()),
            category,
            ext: Some(t.ext),
            bytes: t.bytes,
            files: t.files,
            small: false,
            other: None,
        });
    }
    if mix.small_files > 0 || mix.small_bytes > 0 {
        let at = out
            .iter()
            .position(|r| r.bytes < mix.small_bytes)
            .unwrap_or(out.len());
        out.insert(
            at,
            TypeRow {
                ext: None,
                bytes: mix.small_bytes,
                files: mix.small_files,
                small: true,
                other: None,
                label: None,
                app: None,
                use_: None,
                category: None,
            },
        );
    }
    if rest.0 > 0 {
        out.push(TypeRow {
            ext: None,
            bytes: rest.1,
            files: rest.2,
            small: false,
            other: Some(rest.0),
            label: None,
            app: None,
            use_: None,
            category: None,
        });
    }
    Ok(out)
}

pub fn focus(conn: &Connection) -> Option<String> {
    get_meta(conn, FOCUS_KEY)
        .ok()
        .flatten()
        .filter(|p| !p.trim().is_empty())
}

pub fn set_focus(conn: &Connection, path: Option<&str>) -> Result<()> {
    set_meta(
        conn,
        FOCUS_KEY,
        path.map(normalize_path).as_deref().unwrap_or(""),
    )
}

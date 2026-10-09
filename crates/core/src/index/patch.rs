use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension};

use super::lister::{DirLister, list_fixed};
use super::writer::{Dedupe, Delta, Work, Writer, contrib, load_chain};
use super::{
    DEFAULT_THRESHOLD, KIND_DIR, NodeInfo, node_by_path, node_id_by_path, normalize_path,
    split_parent, to_u64,
};

struct Target {
    volume_id: i64,
    generation: i64,
    threshold: u64,
}

fn target_of(conn: &Connection, node_id: i64) -> Result<Target> {
    let (volume_id, generation, threshold): (i64, i64, Option<i64>) = conn.query_row(
        "SELECT v.id, v.gen, v.threshold FROM node n JOIN volume v ON v.id = n.volume_id WHERE n.id = ?1",
        [node_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let threshold = threshold.map_or(DEFAULT_THRESHOLD, to_u64);
    Ok(Target {
        volume_id,
        generation,
        threshold,
    })
}

fn with_writer<T>(
    conn: &Connection,
    t: &Target,
    f: impl FnOnce(&mut Writer) -> Result<T>,
) -> Result<T> {
    let mut w = Writer::new(conn, t.volume_id, t.generation, t.threshold, Dedupe::Db);
    w.ensure_tx()?;
    let r = f(&mut w).and_then(|v| {
        w.resolve_groups(true)?;
        w.commit()?;
        Ok(v)
    });
    if r.is_err() {
        w.rollback();
    }
    r
}

// A file below the threshold has no row; patch_rescan its path instead so the parent's fold is redone.
pub fn patch_removed(conn: &Connection, path: &str) -> Result<bool> {
    let Some(id) = node_id_by_path(conn, path)? else {
        return Ok(false);
    };
    let (parent, c): (Option<i64>, Delta) = conn.query_row(
        "SELECT parent_id, kind, flags, alloc, logical, sub_alloc, sub_logical, sub_count FROM node WHERE id = ?1",
        [id],
        |r| {
            let sub = Delta {
                alloc: r.get(5)?,
                logical: r.get(6)?,
                count: r.get(7)?,
            };
            Ok((
                r.get(0)?,
                contrib(r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, sub),
            ))
        },
    )?;
    let Some(parent) = parent else {
        bail!("{path} is a volume root; refresh it instead");
    };
    let t = target_of(conn, id)?;
    with_writer(conn, &t, |w| {
        w.delete_rows(id)?;
        let chain = load_chain(conn, parent)?;
        w.add_delta(&chain, -c);
        Ok(true)
    })
}

// Listing the parent, not just `path`, is what lets a removed path drop out.
pub fn patch_rescan(
    conn: &Connection,
    lister: &dyn DirLister,
    path: &str,
) -> Result<Option<NodeInfo>> {
    let path = normalize_path(path);
    let own = node_id_by_path(conn, &path)?;
    let is_root = match own {
        Some(id) => conn
            .query_row("SELECT parent_id FROM node WHERE id = ?1", [id], |r| {
                r.get::<_, Option<i64>>(0)
            })
            .optional()?
            .flatten()
            .is_none(),
        None => false,
    };
    let (start_id, start_path, only) = if is_root {
        (
            own.ok_or_else(|| anyhow!("missing root"))?,
            path.clone(),
            None,
        )
    } else {
        let (parent, name) =
            split_parent(&path).ok_or_else(|| anyhow!("{path} is not in the index"))?;
        let pid = node_id_by_path(conn, &parent)?
            .ok_or_else(|| anyhow!("{parent} is not in the index"))?;
        let kind: i64 =
            conn.query_row("SELECT kind FROM node WHERE id = ?1", [pid], |r| r.get(0))?;
        if kind != KIND_DIR {
            bail!("{parent} is not an indexed directory");
        }
        (pid, parent, Some(name.to_string()))
    };
    let t = target_of(conn, start_id)?;
    with_writer(conn, &t, |w| {
        let chain = load_chain(conn, start_id)?;
        let mut stack = vec![(Work::existing(start_id, start_path, chain), only)];
        let mut sub = Vec::new();
        while let Some((mut work, only)) = stack.pop() {
            let listing = list_fixed(lister, &work.path);
            w.apply_listing(&mut work, listing, &mut sub)?;
            for s in sub.drain(..) {
                let keep = match &only {
                    None => true,
                    Some(name) => {
                        s.is_new
                            || split_parent(&s.path)
                                .is_some_and(|(_, n)| n.eq_ignore_ascii_case(name))
                    }
                };
                if keep {
                    stack.push((s, None));
                }
            }
        }
        Ok(())
    })?;
    node_by_path(conn, &path)
}

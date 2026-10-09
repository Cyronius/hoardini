use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use super::query::{compact_summary_for_prompt, index_status, summary_for_prompt};
use super::{is_ancestor_path, normalize_path, to_u64};
use crate::store::{get_meta, set_meta};

const WATCHED_KEY: &str = "indexed_volumes";
const SELECTED_KEY: &str = "selected_volume";
// Roots whose rows are being deleted; a crash mid-delete is finished at the next start.
const FORGETTING_KEY: &str = "volumes_forgetting";
const FORGET_CHUNK: i64 = 20_000;

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn contains(list: &[String], root: &str) -> bool {
    list.iter().any(|x| same(x, root))
}

fn dedupe(roots: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in roots {
        if !r.is_empty() && !contains(&out, &r) {
            out.push(r);
        }
    }
    out
}

pub fn system_root() -> String {
    normalize_path(&std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()))
}

fn meta_list(conn: &Connection, key: &str) -> Vec<String> {
    get_meta(conn, key)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .map(|v| dedupe(v.iter().map(|r| normalize_path(r))))
        .unwrap_or_default()
}

pub fn watched(conn: &Connection) -> Vec<String> {
    let w = meta_list(conn, WATCHED_KEY);
    if w.is_empty() { vec![system_root()] } else { w }
}

pub fn selected(conn: &Connection) -> String {
    let w = watched(conn);
    let pick = |r: &str| w.iter().find(|x| same(x, r)).cloned();
    get_meta(conn, SELECTED_KEY)
        .ok()
        .flatten()
        .and_then(|s| pick(&normalize_path(&s)))
        .or_else(|| pick(&system_root()))
        .unwrap_or_else(|| w[0].clone())
}

// Before the selected drive has an index, callers fall back to the most recently refreshed one.
pub fn focus(conn: &Connection) -> Option<String> {
    let s = selected(conn);
    stored_volume(conn, &s).ok().flatten().map(|_| s)
}

pub fn connected(root: &str) -> bool {
    Path::new(root).is_dir()
}

// A volume whose index never completed gets its first scan, which always runs fast.
pub fn never_indexed(conn: &Connection, root: &str) -> bool {
    index_status(conn, Some(root))
        .ok()
        .flatten()
        .is_none_or(|s| s.last_refresh_ms.is_none())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeChange {
    pub watched: Vec<String>,
    pub selected: String,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

fn is_drive_path(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
}

// Err is the reason shown to the user; nothing is stored then.
pub fn plan_change(
    current: &[String],
    watched: &[String],
    selected: &str,
    exists: &dyn Fn(&str) -> bool,
) -> std::result::Result<VolumeChange, String> {
    let watched = dedupe(watched.iter().map(|r| normalize_path(r)));
    if watched.is_empty() {
        return Err("at least one drive must stay watched".into());
    }
    if let Some(bad) = watched.iter().find(|r| !is_drive_path(r)) {
        return Err(format!("{bad} is not a drive path like D:\\"));
    }
    let selected = normalize_path(selected);
    let Some(selected) = watched.iter().find(|r| same(r, &selected)).cloned() else {
        return Err(format!("{selected} is not among the watched drives"));
    };
    let added: Vec<String> = watched
        .iter()
        .filter(|r| !contains(current, r))
        .cloned()
        .collect();
    if let Some(gone) = added.iter().find(|r| !exists(r)) {
        return Err(format!("{gone} is not connected"));
    }
    let removed = current
        .iter()
        .filter(|r| !contains(&watched, r))
        .cloned()
        .collect();
    Ok(VolumeChange {
        watched,
        selected,
        added,
        removed,
    })
}

pub fn store_change(conn: &Connection, c: &VolumeChange) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    set_meta(&tx, WATCHED_KEY, &serde_json::to_string(&c.watched)?)?;
    set_meta(&tx, SELECTED_KEY, &c.selected)?;
    if !c.removed.is_empty() {
        let mut pending = meta_list(&tx, FORGETTING_KEY);
        pending.extend(c.removed.iter().cloned());
        set_meta(
            &tx,
            FORGETTING_KEY,
            &serde_json::to_string(&dedupe(pending))?,
        )?;
    }
    tx.commit()?;
    Ok(())
}

// Deletes everything indexed for this root; nodes go in chunks so other writers never wait long.
pub fn forget(conn: &Connection, root: &str) -> Result<u64> {
    let root = normalize_path(root);
    let prefix = if root.ends_with('\\') {
        root.clone()
    } else {
        format!("{root}\\")
    };
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM dup_member WHERE path = ?1 COLLATE NOCASE \
         OR substr(path, 1, length(?2)) = ?2 COLLATE NOCASE",
        params![root, prefix],
    )?;
    tx.execute(
        "DELETE FROM dup_group WHERE (SELECT COUNT(*) FROM dup_member m WHERE m.group_id = dup_group.id) < 2",
        [],
    )?;
    tx.execute(
        "DELETE FROM dup_member WHERE group_id NOT IN (SELECT id FROM dup_group)",
        [],
    )?;
    tx.commit()?;
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM volume WHERE root = ?1 COLLATE NOCASE")?
        .query_map([&root], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut nodes = 0u64;
    for id in ids {
        conn.execute(
            "DELETE FROM evidence_cache WHERE (SELECT volume_id FROM node WHERE id = evidence_cache.node_id) = ?1",
            [id],
        )?;
        loop {
            let n = conn.execute(
                "DELETE FROM node WHERE id IN (SELECT id FROM node WHERE volume_id = ?1 LIMIT ?2)",
                params![id, FORGET_CHUNK],
            )?;
            nodes += n as u64;
            if n == 0 {
                break;
            }
        }
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM scan_dir WHERE scan_id IN (SELECT id FROM scan WHERE volume_id = ?1)",
            [id],
        )?;
        tx.execute(
            "DELETE FROM scan_file WHERE scan_id IN (SELECT id FROM scan WHERE volume_id = ?1)",
            [id],
        )?;
        tx.execute("DELETE FROM scan WHERE volume_id = ?1", [id])?;
        tx.execute("DELETE FROM volume WHERE id = ?1", [id])?;
        tx.commit()?;
    }
    let rest: Vec<String> = meta_list(conn, FORGETTING_KEY)
        .into_iter()
        .filter(|r| !same(r, &root))
        .collect();
    set_meta(conn, FORGETTING_KEY, &serde_json::to_string(&rest)?)?;
    Ok(nodes)
}

// Finishes deletes a crash interrupted; a root watched again since is left to its next full scan.
pub fn finish_forgetting(conn: &Connection) -> Result<()> {
    let w = watched(conn);
    for root in meta_list(conn, FORGETTING_KEY) {
        if contains(&w, &root) {
            let rest: Vec<String> = meta_list(conn, FORGETTING_KEY)
                .into_iter()
                .filter(|r| !same(r, &root))
                .collect();
            set_meta(conn, FORGETTING_KEY, &serde_json::to_string(&rest)?)?;
        } else {
            forget(conn, &root)?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredVolume {
    pub label: Option<String>,
    pub fs_name: Option<String>,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

// What the last refresh saw, for a drive that is not connected now.
pub fn stored_volume(conn: &Connection, root: &str) -> Result<Option<StoredVolume>> {
    Ok(conn
        .query_row(
            "SELECT label, fs_name, total_bytes, free_bytes FROM volume WHERE root = ?1 COLLATE NOCASE",
            [normalize_path(root)],
            |r| {
                Ok(StoredVolume {
                    label: r.get(0)?,
                    fs_name: r.get(1)?,
                    total_bytes: to_u64(r.get::<_, Option<i64>>(2)?.unwrap_or(0)),
                    free_bytes: to_u64(r.get::<_, Option<i64>>(3)?.unwrap_or(0)),
                })
            },
        )
        .optional()?)
}

// The index rows that describe a watched root: its own, else any indexed folder under it.
fn indexed_for(roots: &[String], watched: &str) -> Vec<String> {
    let exact: Vec<String> = roots.iter().filter(|r| same(r, watched)).cloned().collect();
    if !exact.is_empty() {
        return exact;
    }
    roots
        .iter()
        .filter(|r| is_ancestor_path(watched, r))
        .cloned()
        .collect()
}

pub fn prompt_summary(conn: &Connection, small: bool) -> Result<String> {
    let roots: Vec<String> = conn
        .prepare("SELECT root FROM volume WHERE root_node_id IS NOT NULL ORDER BY id")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let watched = watched(conn);
    let selected = selected(conn);
    if !watched.iter().any(|w| !indexed_for(&roots, w).is_empty()) {
        return summary_for_prompt(conn, None);
    }
    let order = std::iter::once(selected.clone())
        .chain(watched.into_iter().filter(|w| !same(w, &selected)))
        .take(if small { 1 } else { usize::MAX });
    let mut parts = Vec::new();
    for (i, w) in order.enumerate() {
        let mut part = String::new();
        let online = connected(&w);
        let found = indexed_for(&roots, &w);
        if found.is_empty() {
            part.push_str(&if online {
                format!("{w}: no index yet; its first scan has not finished.")
            } else {
                format!("{w}: no index yet; it is not connected.")
            });
        } else if !online {
            part.push_str(&format!(
                "{w} is not connected right now; its index is from before it was unplugged.\n"
            ));
        }
        for r in found {
            let s = if i == 0 {
                summary_for_prompt(conn, Some(&r))?
            } else {
                compact_summary_for_prompt(conn, &r)?
            };
            part.push_str(s.trim_end());
            part.push('\n');
        }
        parts.push(part.trim_end().to_string());
    }
    Ok(format!(
        "The user is looking at {selected}; a question that names no drive is about it.\n{}",
        parts.join("\n\n")
    ))
}

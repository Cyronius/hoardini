use rusqlite::{Connection, OptionalExtension, params};

use super::Evidence;
use crate::store::now_ms;

pub struct Key {
    pub node_id: i64,
    pub file_id: Option<Vec<u8>>,
    pub mtime: Option<i64>,
}

pub fn key_for(conn: &Connection, node_id: i64, mtime: Option<i64>) -> Key {
    let file_id = conn
        .query_row("SELECT file_id FROM node WHERE id = ?1", [node_id], |r| {
            r.get::<_, Option<crate::index::FileId>>(0)
                .map(|f| f.map(|f| f.0.to_vec()))
        })
        .optional()
        .ok()
        .flatten()
        .flatten();
    Key {
        node_id,
        file_id,
        mtime,
    }
}

pub fn load(conn: &Connection, key: &Key) -> Option<Evidence> {
    let (file_id, mtime, json): (Option<Vec<u8>>, Option<i64>, String) = conn
        .query_row(
            "SELECT file_id, mtime, json FROM evidence_cache WHERE node_id = ?1",
            [key.node_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .ok()??;
    if file_id != key.file_id || mtime != key.mtime {
        return None;
    }
    serde_json::from_str(&json).ok()
}

pub fn store(conn: &Connection, key: &Key, ev: &Evidence) {
    let Ok(json) = serde_json::to_string(ev) else {
        return;
    };
    let _ = conn.execute(
        "INSERT OR REPLACE INTO evidence_cache (node_id, file_id, mtime, json, created_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![key.node_id, key.file_id, key.mtime, json, now_ms()],
    );
}

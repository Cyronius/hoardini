use rusqlite::{Connection, params};

use crate::store::now_ms;

pub(crate) fn begin(
    conn: &Connection,
    job_id: &str,
    proposal_id: Option<&str>,
    item_id: Option<i64>,
    kind: &str,
    path: &str,
) -> anyhow::Result<i64> {
    conn.execute(
        "INSERT INTO action(job_id, proposal_id, item_id, kind, path, status, started_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)",
        params![job_id, proposal_id, item_id, kind, path, now_ms()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub(crate) fn set_undo(
    conn: &Connection,
    action_id: i64,
    undo: &serde_json::Value,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE action SET undo_json = ?2 WHERE id = ?1",
        params![action_id, undo.to_string()],
    )?;
    Ok(())
}

pub(crate) fn finish(
    conn: &Connection,
    action_id: i64,
    status: &str,
    freed: i64,
    undo: Option<&serde_json::Value>,
    error: Option<&str>,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE action SET status = ?2, freed_bytes = ?3, undo_json = COALESCE(?4, undo_json), error = ?5, finished_ms = ?6
         WHERE id = ?1",
        params![
            action_id,
            status,
            freed,
            undo.map(|u| u.to_string()),
            error,
            now_ms()
        ],
    )?;
    Ok(())
}

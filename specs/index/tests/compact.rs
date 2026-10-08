use hoardini_core::index::FileId;

use super::*;

fn wide_id(seed: u8) -> [u8; 16] {
    let mut id = [seed; 16];
    id[15] = 0x80;
    id
}

// Sign bit set in the low half: the stored INTEGER is negative and must still round-trip.
fn high_seq_id(record: u64) -> [u8; 16] {
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&(record | (0x8001u64 << 48)).to_le_bytes());
    id
}

fn compact_tree() -> FakeFs {
    let fs = sample_tree();
    fs.file_with_id(r"C:\refs\wide.bin", 4 * MIB, 4 * MIB, wide_id(7));
    fs.hard_link(r"C:\refs\wide.bin", r"C:\refs2\wide-link.bin");
    fs.file_with_id(r"C:\refs\seq.bin", 2 * MIB, 2 * MIB, high_seq_id(4242));
    fs
}

fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

// Traces: INDEX-COMPACT
#[test]
fn directory_rows_keep_mtime_only() {
    let fs = compact_tree();
    let e = env();
    run(&e, &fs, r"C:\", 3);
    let conn = e.db.conn().unwrap();
    let dir = node_by_path(&conn, r"C:\Users\josha\models")
        .unwrap()
        .unwrap();
    assert!(dir.mtime_ms.is_some());
    assert_eq!((dir.atime_ms, dir.ctime_ms), (None, None));
    let file = node_by_path(&conn, r"C:\Users\josha\models\a.gguf")
        .unwrap()
        .unwrap();
    assert!(file.mtime_ms.is_some() && file.atime_ms.is_some() && file.ctime_ms.is_some());
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM node WHERE kind = 0 AND (atime IS NOT NULL OR ctime IS NOT NULL)"
        ),
        0
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM node WHERE kind = 0 AND mtime IS NULL AND parent_id IS NOT NULL"
        ),
        0,
        "only the root, which no listing describes, lacks a time"
    );
}

// Traces: INDEX-COMPACT
#[test]
fn ntfs_ids_are_integers_and_wide_ids_stay_128_bit() {
    let fs = compact_tree();
    let e = env();
    run(&e, &fs, r"C:\", 3);
    let conn = e.db.conn().unwrap();
    let stored = |path: &str| -> (String, FileId) {
        let id = node_by_path(&conn, path).unwrap().unwrap().id;
        conn.query_row(
            "SELECT typeof(file_id), file_id FROM node WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    for path in [
        r"C:\Users\josha\models\a.gguf",
        r"C:\code",
        r"C:\refs\seq.bin",
    ] {
        let (ty, id) = stored(path);
        assert_eq!(ty, "integer", "{path}");
        assert_eq!(Some(id.0), fs.file_id(path), "{path}");
    }
    let (ty, id) = stored(r"C:\refs2\wide-link.bin");
    assert_eq!(ty, "blob");
    assert_eq!(id.0, wide_id(7));
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM node WHERE typeof(file_id) = 'blob'"
        ),
        2
    );
    let size = |p: &str| node_by_path(&conn, p).unwrap().unwrap().size;
    assert_eq!(
        size(r"C:\refs") + size(r"C:\refs2"),
        4 * MIB + 2 * MIB,
        "a wide-id hard link counts once"
    );
    assert!(
        node_by_path(&conn, r"C:\refs\wide.bin")
            .unwrap()
            .unwrap()
            .hardlink_dup,
        "the smaller path (refs2) owns the bytes"
    );
    assert_consistent(&e);
}

// Traces: INDEX-COMPACT
#[test]
fn a_v1_index_upgrades_in_place() {
    let fs = compact_tree();
    let built = env();
    run(&built, &fs, r"C:\", 3);
    let want = snapshot(&built.db.conn().unwrap(), r"C:\").unwrap();

    let dir = tempfile::tempdir().unwrap();
    let old_path = dir.path().join("v1.db");
    {
        let old = rusqlite::Connection::open(&old_path).unwrap();
        old.execute_batch(include_str!("../../../crates/core/src/store/schema_v1.sql"))
            .unwrap();
        old.execute_batch("PRAGMA user_version = 1").unwrap();
        old.execute(
            "ATTACH DATABASE ?1 AS src",
            [built.db.path().to_string_lossy()],
        )
        .unwrap();
        old.execute_batch(
            "INSERT INTO volume SELECT * FROM src.volume; INSERT INTO node SELECT * FROM src.node; \
             UPDATE node SET atime = mtime, ctime = mtime WHERE kind = 0;",
        )
        .unwrap();
        let ints: Vec<(i64, i64)> = old
            .prepare("SELECT id, file_id FROM node WHERE typeof(file_id) = 'integer'")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(ints.iter().any(|&(_, v)| v < 0), "covers the sign bit");
        for (id, v) in ints {
            let mut blob = [0u8; 16];
            blob[..8].copy_from_slice(&v.to_le_bytes());
            old.execute(
                "UPDATE node SET file_id = ?1 WHERE id = ?2",
                rusqlite::params![blob.to_vec(), id],
            )
            .unwrap();
        }
    }

    let db = Db::open(&old_path).unwrap();
    let conn = db.conn().unwrap();
    assert_eq!(count(&conn, "PRAGMA user_version"), 2);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &want);
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM node WHERE typeof(file_id) = 'blob'"
        ),
        2,
        "only the wide ids stay blobs"
    );
    let out = refresh(&conn, &fs, r"C:\", &opts(2)).unwrap();
    assert!(out.completed);
    assert_eq!(out.rows_deleted, 0);
    assert_same(&snapshot(&conn, r"C:\").unwrap(), &want);
    assert!(verify_totals(&conn).unwrap().is_empty());
}

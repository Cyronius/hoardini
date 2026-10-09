use hoardini_core::index::{
    TopFilesQuery, VolumeChange, finish_forgetting, focus, forget, index_status, plan_change,
    selected, store_change, system_root, top_files, watched,
};
use hoardini_core::store::set_meta;
use rusqlite::{Connection, params};

use super::*;

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn count(conn: &Connection, sql: &str, id: i64) -> i64 {
    conn.query_row(sql, [id], |r| r.get(0)).unwrap()
}

fn volume_id(conn: &Connection, root: &str) -> i64 {
    conn.query_row("SELECT id FROM volume WHERE root = ?1", [root], |r| {
        r.get(0)
    })
    .unwrap()
}

// Two refreshes each, so both drives have scan history with folder and file summaries.
fn two_drives(e: &Env) {
    for root in [r"C:\", r"D:\"] {
        let fs = FakeFs::new(root);
        fs.file(&format!(r"{root}big\a.bin"), 2 * GIB);
        fs.file(&format!(r"{root}big\b.bin"), GIB);
        fs.file(&format!(r"{root}small.txt"), KIB);
        run(e, &fs, root, 2);
        fs.file(&format!(r"{root}big\c.bin"), GIB);
        run(e, &fs, root, 2);
    }
}

fn add_dupes(conn: &Connection) {
    let groups = [
        (1, [r"C:\big\a.bin", r"C:\big\b.bin"]),
        (2, [r"C:\big\c.bin", r"D:\big\c.bin"]),
        (3, [r"D:\big\a.bin", r"d:\BIG\b.bin"]),
    ];
    for (id, paths) in groups {
        conn.execute(
            "INSERT INTO dup_group(id, size, hash, created_ms) VALUES (?1, 1, ?2, 0)",
            params![id, format!("h{id}")],
        )
        .unwrap();
        for p in paths {
            conn.execute(
                "INSERT INTO dup_member(group_id, path) VALUES (?1, ?2)",
                params![id, p],
            )
            .unwrap();
        }
    }
}

fn add_evidence(conn: &Connection, path: &str) -> i64 {
    let id = node_by_path(conn, path).unwrap().unwrap().id;
    conn.execute(
        "INSERT INTO evidence_cache(node_id, json, created_ms) VALUES (?1, '{}', 0)",
        [id],
    )
    .unwrap();
    id
}

// Traces: INDEX-VOLUMES
#[test]
fn a_volume_change_is_validated_and_normalized() {
    let yes = |_: &str| true;
    let c = plan_change(&s(&[r"C:\"]), &s(&["c:", r"C:\", "d:/"]), "d:", &yes).unwrap();
    assert_eq!(
        c,
        VolumeChange {
            watched: s(&[r"C:\", r"D:\"]),
            selected: r"D:\".into(),
            added: s(&[r"D:\"]),
            removed: vec![],
        }
    );
    let r = plan_change(&s(&[r"C:\", r"D:\"]), &s(&[r"D:\"]), r"d:\", &yes).unwrap();
    assert_eq!((r.added, r.removed), (vec![], s(&[r"C:\"])));

    let why = |w: &[&str], sel: &str| plan_change(&s(&[r"C:\"]), &s(w), sel, &yes).unwrap_err();
    assert!(why(&[], r"C:\").contains("at least one"));
    assert!(why(&[r"C:\"], r"D:\").contains("not among the watched"));
    assert!(why(&[r"relative\x"], r"relative\x").contains("drive path"));
    assert!(why(&[r"\\nas\share"], r"\\nas\share").contains("drive path"));

    let unplugged = |r: &str| r != r"E:\";
    let keep = plan_change(
        &s(&[r"C:\", r"E:\"]),
        &s(&[r"C:\", r"E:\"]),
        r"E:\",
        &unplugged,
    );
    assert!(
        keep.is_ok(),
        "an unplugged drive may stay watched and selected"
    );
    let add = plan_change(&s(&[r"C:\"]), &s(&[r"C:\", r"E:\"]), r"C:\", &unplugged);
    assert!(add.unwrap_err().contains("not connected"));
}

// Traces: INDEX-VOLUMES
#[test]
fn watched_and_selected_round_trip_with_system_drive_defaults() {
    let e = env();
    let conn = e.db.conn().unwrap();
    let sys = system_root();
    assert_eq!(watched(&conn), vec![sys.clone()]);
    assert_eq!(selected(&conn), sys);

    let c = plan_change(&watched(&conn), &s(&[&sys, r"Q:\"]), "q:", &|_| true).unwrap();
    store_change(&conn, &c).unwrap();
    assert_eq!(watched(&conn), s(&[&sys, r"Q:\"]));
    assert_eq!(selected(&conn), r"Q:\");

    set_meta(&conn, "selected_volume", r"Z:\").unwrap();
    assert_eq!(
        selected(&conn),
        sys,
        "a stale selection falls back to the system drive"
    );
    set_meta(&conn, "indexed_volumes", r#"["Q:\\", "R:\\"]"#).unwrap();
    assert_eq!(selected(&conn), r"Q:\", "then to the first watched drive");
}

// Traces: INDEX-VOLUMES
#[test]
fn forgetting_a_drive_deletes_its_rows_and_nothing_else() {
    let e = env();
    two_drives(&e);
    let conn = e.db.conn().unwrap();
    add_dupes(&conn);
    let c_ev = add_evidence(&conn, r"C:\big\a.bin");
    let d_ev = add_evidence(&conn, r"D:\big\a.bin");
    let (c, d) = (volume_id(&conn, r"C:\"), volume_id(&conn, r"D:\"));
    let scans = "SELECT COUNT(*) FROM scan WHERE volume_id = ?1";
    let dir_rows =
        "SELECT COUNT(*) FROM scan_dir WHERE scan_id IN (SELECT id FROM scan WHERE volume_id = ?1)";
    let file_rows = "SELECT COUNT(*) FROM scan_file WHERE scan_id IN (SELECT id FROM scan WHERE volume_id = ?1)";
    let nodes = "SELECT COUNT(*) FROM node WHERE volume_id = ?1";
    for q in [scans, dir_rows, file_rows, nodes] {
        assert!(count(&conn, q, d) > 0, "D has rows for {q}");
    }
    let d_scan_ids: Vec<i64> = conn
        .prepare("SELECT id FROM scan WHERE volume_id = ?1")
        .unwrap()
        .query_map([d], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let c_before: Vec<i64> = [scans, dir_rows, file_rows, nodes]
        .iter()
        .map(|q| count(&conn, q, c))
        .collect();

    let deleted = forget(&conn, "d:").unwrap();
    assert!(deleted > 0);

    let gone = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(gone(r"SELECT COUNT(*) FROM volume WHERE root = 'D:\'"), 0);
    assert_eq!(count(&conn, nodes, d), 0);
    assert_eq!(count(&conn, scans, d), 0);
    for id in d_scan_ids {
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM scan_dir WHERE scan_id = ?1",
                id
            ),
            0
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM scan_file WHERE scan_id = ?1",
                id
            ),
            0
        );
    }
    let ev = "SELECT COUNT(*) FROM evidence_cache WHERE node_id = ?1";
    assert_eq!((count(&conn, ev, c_ev), count(&conn, ev, d_ev)), (1, 0));
    let groups: Vec<i64> = conn
        .prepare("SELECT id FROM dup_group ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(groups, [1], "a group left with one copy is dropped");
    assert_eq!(gone("SELECT COUNT(*) FROM dup_member"), 2);

    let c_after: Vec<i64> = [scans, dir_rows, file_rows, nodes]
        .iter()
        .map(|q| count(&conn, q, c))
        .collect();
    assert_eq!(c_after, c_before, "C:\\ is untouched");
    let all = top_files(
        &conn,
        &TopFilesQuery {
            limit: 50,
            ..TopFilesQuery::default()
        },
    )
    .unwrap();
    assert!(!all.items.is_empty());
    assert!(all.items.iter().all(|f| f.path.starts_with(r"C:\")));
    let ok: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");
}

// Traces: INDEX-VOLUMES
#[test]
fn a_delete_cut_short_finishes_at_the_next_start() {
    let e = env();
    two_drives(&e);
    let conn = e.db.conn().unwrap();
    set_meta(&conn, "indexed_volumes", r#"["C:\\", "D:\\"]"#).unwrap();
    let c = plan_change(&watched(&conn), &s(&[r"C:\"]), r"C:\", &|_| true).unwrap();
    store_change(&conn, &c).unwrap();
    let d = volume_id(&conn, r"D:\");
    assert!(count(&conn, "SELECT COUNT(*) FROM node WHERE volume_id = ?1", d) > 0);

    finish_forgetting(&conn).unwrap();
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM node WHERE volume_id = ?1", d),
        0
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM volume WHERE id = ?1", d),
        0
    );
    let c_id = volume_id(&conn, r"C:\");
    assert!(
        count(
            &conn,
            "SELECT COUNT(*) FROM node WHERE volume_id = ?1",
            c_id
        ) > 0
    );
    let pending: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'volumes_forgetting'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, "[]");
}

// Traces: INDEX-VOLUMES
#[test]
fn history_and_status_follow_the_selected_drive_once_it_has_an_index() {
    let e = env();
    two_drives(&e);
    let conn = e.db.conn().unwrap();
    set_meta(&conn, "indexed_volumes", r#"["C:\\", "D:\\"]"#).unwrap();
    set_meta(&conn, "selected_volume", r"D:\").unwrap();
    assert_eq!(focus(&conn).as_deref(), Some(r"D:\"));
    let st = index_status(&conn, focus(&conn).as_deref())
        .unwrap()
        .unwrap();
    assert_eq!(st.volume_id, volume_id(&conn, r"D:\"));

    set_meta(&conn, "indexed_volumes", r#"["C:\\", "D:\\", "Q:\\"]"#).unwrap();
    set_meta(&conn, "selected_volume", r"Q:\").unwrap();
    assert_eq!(
        focus(&conn),
        None,
        "a drive with no index yet falls back to the latest refresh"
    );
}

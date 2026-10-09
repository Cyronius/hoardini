use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use hoardini_core::cloud::{Cloud, CloudState, SyncRoot};
use hoardini_core::index::node_by_path;
use hoardini_core::policy::Verdict;
use hoardini_core::proposals::{self, ProposalKind, default_tick, default_tick_for};

use super::support::*;

// A sync root stand-in: the decisions are Hoardini's, the dehydration is recorded instead of done.
struct FakeCloud {
    root: String,
    states: Mutex<HashMap<String, CloudState>>,
    freed: Mutex<Vec<String>>,
}

impl FakeCloud {
    fn new(root: &Path) -> Arc<FakeCloud> {
        Arc::new(FakeCloud {
            root: s(root),
            states: Mutex::default(),
            freed: Mutex::default(),
        })
    }

    fn set(&self, p: &Path, in_sync: bool) {
        self.states.lock().unwrap().insert(
            s(p).to_lowercase(),
            CloudState {
                in_sync,
                pinned: false,
                local_bytes: std::fs::metadata(p).unwrap().len(),
            },
        );
    }

    fn freed(&self) -> Vec<String> {
        self.freed.lock().unwrap().clone()
    }
}

impl Cloud for FakeCloud {
    fn roots(&self) -> Vec<SyncRoot> {
        vec![SyncRoot {
            path: self.root.clone(),
            provider: "OneDrive".into(),
        }]
    }

    fn state(&self, path: &Path) -> std::io::Result<Option<CloudState>> {
        Ok(self
            .states
            .lock()
            .unwrap()
            .get(&s(path).to_lowercase())
            .copied())
    }

    fn free(&self, path: &Path) -> std::io::Result<u64> {
        let mut st = self.states.lock().unwrap();
        let e = st
            .get_mut(&s(path).to_lowercase())
            .ok_or_else(|| std::io::Error::other("not a placeholder"))?;
        let n = e.local_bytes;
        e.local_bytes = 0;
        self.freed.lock().unwrap().push(s(path));
        Ok(n)
    }
}

fn action_rows(fx: &Fx, proposal: &str) -> Vec<(String, String, Option<String>)> {
    let conn = fx.db.conn().unwrap();
    let mut st = conn
        .prepare("SELECT path, status, undo_json FROM action WHERE proposal_id = ?1 ORDER BY id")
        .unwrap();
    st.query_map([proposal], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

// Traces: ACT-ONLINE-ONLY, ACT-JOURNAL, INDEX-ACTION-PATCH, POLICY-DEFAULT-TICK
#[test]
fn online_only_frees_in_sync_files_skips_the_rest_and_never_deletes() {
    let mut fx = Fx::new();
    let root = fx.work.join("OneDrive");
    let cloud = FakeCloud::new(&root);
    fx.env = fx.env.clone().with_cloud(cloud.clone());
    let a = fx.file(r"OneDrive\a.bin", &data(96 * 1024, 1));
    let b = fx.file(r"OneDrive\b.bin", &data(80 * 1024, 2));
    let c = fx.file(r"OneDrive\album\c.jpg", &data(70 * 1024, 3));
    let d = fx.file(r"OneDrive\album\d.jpg", &data(60 * 1024, 4));
    let outside = fx.file("outside.bin", &data(50 * 1024, 5));
    let album = root.join("album");
    for (p, ok) in [
        (&a, true),
        (&b, false),
        (&c, true),
        (&d, false),
        (&outside, true),
    ] {
        cloud.set(p, ok);
    }
    let before: Vec<Vec<u8>> = [&a, &b, &c, &d, &outside]
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    fx.index_work();

    let p = fx
        .propose(ProposalKind::OnlineOnly, &[&a, &b, &album, &outside], None)
        .unwrap();
    assert_eq!(p.kind, ProposalKind::OnlineOnly);
    let out = item(&p, &outside);
    assert_eq!(out.status, "blocked");
    assert!(
        out.block_reason
            .as_deref()
            .unwrap()
            .starts_with("Not in a cloud-synced folder")
    );
    let ia = item(&p, &a);
    assert_eq!(
        (ia.verdict, ia.selected),
        (Some(Verdict::Synced), true),
        "{ia:#?}"
    );
    assert_eq!(ia.copy_note.as_deref(), Some("synced to OneDrive"));
    let ib = item(&p, &b);
    assert_eq!((ib.verdict, ib.selected), (Some(Verdict::Review), false));
    let ialbum = item(&p, &album);
    assert_eq!(
        (ialbum.verdict, ialbum.selected),
        (Some(Verdict::Review), false)
    );

    let ex = fx.executor();
    let res = ex.approve(&p.id, &[ia.id, ib.id, ialbum.id]).unwrap();
    let job = wait(&ex, &res.job_id);
    let after = fx.get(&p.id);
    assert_eq!(after.status, "partial");
    assert_eq!(item(&after, &a).status, "done");
    assert_eq!(
        item(&after, &a).detail.as_deref(),
        Some("1 file(s) now online-only in OneDrive")
    );
    let fb = item(&after, &b);
    assert_eq!(fb.status, "failed");
    assert!(
        fb.detail
            .as_deref()
            .unwrap()
            .contains("nothing here is in sync with OneDrive")
    );
    let fal = item(&after, &album);
    assert_eq!(fal.status, "done");
    assert_eq!(
        fal.detail.as_deref(),
        Some("1 file(s) now online-only in OneDrive; 1 not in sync were left on this PC")
    );

    assert_eq!(cloud.freed(), vec![s(&a), s(&c)]);
    assert_eq!(job.freed_bytes, 96 * 1024 + 70 * 1024);
    for (p, bytes) in [&a, &b, &c, &d, &outside].iter().zip(&before) {
        assert_eq!(
            &std::fs::read(p).unwrap(),
            bytes,
            "{} must stay",
            p.display()
        );
    }

    let rows = action_rows(&fx, &p.id);
    assert_eq!(rows.len(), 3);
    for (path, status, undo) in &rows {
        if path.eq_ignore_ascii_case(&s(&b)) {
            assert_eq!(status, "failed");
            continue;
        }
        assert_eq!(status, "done", "{path}");
        let undo: serde_json::Value = serde_json::from_str(undo.as_deref().unwrap()).unwrap();
        assert_eq!(
            (undo["kind"].as_str(), undo["provider"].as_str()),
            (Some("pin"), Some("OneDrive"))
        );
    }
    let conn = fx.db.conn().unwrap();
    assert!(
        node_by_path(&conn, &s(&a)).unwrap().is_some(),
        "patched, not removed"
    );
    assert!(node_by_path(&conn, &s(&c)).unwrap().is_some());
}

// Traces: POLICY-DEFAULT-TICK
#[test]
fn synced_ticks_only_on_an_online_only_card() {
    assert!(default_tick_for(
        ProposalKind::OnlineOnly,
        false,
        Some(Verdict::Synced),
        false
    ));
    assert!(!default_tick_for(
        ProposalKind::OnlineOnly,
        false,
        Some(Verdict::Safe),
        false
    ));
    assert!(!default_tick_for(
        ProposalKind::OnlineOnly,
        true,
        Some(Verdict::Synced),
        false
    ));
    assert!(!default_tick_for(
        ProposalKind::OnlineOnly,
        false,
        Some(Verdict::Synced),
        true
    ));
    assert!(!default_tick_for(
        ProposalKind::Delete,
        false,
        Some(Verdict::Synced),
        false
    ));
    assert!(!default_tick(false, Some(Verdict::Synced), false));
    assert_eq!(
        proposals::default_tick_for(ProposalKind::Delete, false, Some(Verdict::Safe), false),
        default_tick(false, Some(Verdict::Safe), false)
    );
}

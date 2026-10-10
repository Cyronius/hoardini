use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hoardini_core::actions::Executor;
use hoardini_core::destinations::{self, NewDestination};
use hoardini_core::index::{FakeFs, RefreshOptions, normalize_path, refresh};
use hoardini_core::policy::{PolicyEnv, Verdict};
use hoardini_core::proposals::{self, NewItem, NewProposal, Proposal, ProposalKind};
use hoardini_core::review::{self, Review, Scope};
use hoardini_core::store::Db;
use hoardini_core::vault::DpapiVault;
use serde_json::json;

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;
// FakeFs stamps every node at 1_700_000_000_000, so everything not touched is 400 days old.
const NOW: i64 = 1_700_000_000_000 + 400 * DAY + HOUR;

// Real folders and files on disk, sized by a fake index of the same paths.
pub struct Disk {
    dir: tempfile::TempDir,
    pub root: String,
    pub profile: String,
    pub db: Db,
    pub env: PolicyEnv,
    pub fs: FakeFs,
}

impl Disk {
    pub fn new() -> Disk {
        let dir = tempfile::tempdir().unwrap();
        let drive = dir.path().join("drive");
        std::fs::create_dir_all(&drive).unwrap();
        let root = normalize_path(&drive.to_string_lossy());
        let profile = format!(r"{root}\Users\tester");
        let db = Db::open(dir.path().join("h.db")).unwrap();
        let fs = FakeFs::new(&root);
        fs.set_volume(1000 * GIB, 400 * GIB);
        let env = PolicyEnv::with_paths(Path::new(&profile), &dir.path().join("data"));
        Disk {
            dir,
            root,
            profile,
            db,
            env,
            fs,
        }
    }

    pub fn path(&self, rel: &str) -> String {
        let rel = rel.replace('~', &self.profile);
        if rel.starts_with(&self.profile) {
            rel
        } else {
            format!(r"{}\{rel}", self.root)
        }
    }

    // A real folder whose indexed size is one fake file inside it.
    pub fn folder(&self, rel: &str, file: &str, bytes: u64) -> String {
        let dir = self.path(rel);
        std::fs::create_dir_all(&dir).unwrap();
        self.fs.file(&format!(r"{dir}\{file}"), bytes);
        dir
    }

    pub fn file(&self, rel: &str, bytes: u64) -> String {
        let p = self.path(rel);
        std::fs::create_dir_all(Path::new(&p).parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
        self.fs.file(&p, bytes);
        p
    }

    pub fn fresh(&self, path: &str, ago: i64) {
        self.fs.set_times(path, NOW - ago, NOW - ago);
    }

    pub fn index(&self) -> i64 {
        let out = refresh(
            &self.db.conn().unwrap(),
            &self.fs,
            &self.root,
            &RefreshOptions::default(),
        )
        .unwrap();
        assert!(out.completed);
        self.db
            .conn()
            .unwrap()
            .query_row(
                "SELECT id FROM volume WHERE root = ?1 COLLATE NOCASE",
                [&self.root],
                |r| r.get(0),
            )
            .unwrap()
    }

    pub fn review(&self, scope: Scope, now: i64) -> Review {
        review::build(&self.db, &self.env, &self.fs, scope, now).unwrap()
    }

    pub fn card(&self, r: &Review, card: &str) -> Option<Proposal> {
        r.cards
            .iter()
            .find(|c| c.card == card)
            .map(|c| proposals::get(&self.db, &c.proposal_id).unwrap().unwrap())
    }

    pub fn chat_card(&self, paths: &[&str]) -> Proposal {
        proposals::create_with(
            &self.db,
            &self.env,
            NewProposal {
                conversation_id: Some("main".into()),
                tool_call_id: None,
                kind: ProposalKind::Delete,
                destination_id: None,
                format: None,
                dedupe: None,
                summary: "chat".into(),
                items: paths
                    .iter()
                    .map(|p| NewItem {
                        path: p.to_string(),
                        verdict: None,
                        reason: None,
                    })
                    .collect(),
            },
        )
        .unwrap()
    }

    pub fn nas(&self) {
        destinations::create(
            &self.db.conn().unwrap(),
            NewDestination {
                name: "NAS".into(),
                kind: "smb".into(),
                config: json!({"path": r"\\nas\share\hoardini"}),
            },
        )
        .unwrap();
    }

    pub fn executor(&self) -> Executor {
        let vault = DpapiVault::open(&self.dir.path().join("secrets.vault")).unwrap();
        Executor::with_policy(self.db.clone(), Arc::new(vault), self.env.clone())
    }

    pub fn status(&self, id: &str) -> String {
        proposals::get(&self.db, id).unwrap().unwrap().status
    }
}

fn items(p: &Proposal) -> Vec<(String, bool)> {
    let mut v: Vec<(String, bool)> = p
        .items
        .iter()
        .map(|i| (i.path.clone(), i.selected))
        .collect();
    v.sort();
    v
}

// Traces: REVIEW-BUILD, POLICY-DEFAULT-TICK
#[test]
fn each_unit_lands_in_its_card_with_its_tick_and_reason() {
    let d = Disk::new();
    let temp = d.folder(r"~\AppData\Local\Temp\old-session", "scratch.bin", 5 * GIB);
    let live = d.folder(r"~\AppData\Local\Temp\live", "model.gguf", 2 * GIB);
    d.fresh(&format!(r"{live}\model.gguf"), HOUR);
    let dumps = d.folder(r"~\AppData\Local\CrashDumps\app", "app.dmp", 1536 * MIB);
    let chrome = d.folder(
        r"~\AppData\Local\Google\Chrome\User Data\Default\Cache",
        "data_1",
        2 * GIB,
    );
    let target = d.folder(r"code\proj\target", "app.exe", 3 * GIB);
    d.file(r"code\proj\Cargo.toml", 1024);
    let modules = d.folder(r"code\web\node_modules", "big.js", 1536 * MIB);
    let not_rust = d.folder(r"code\data\target", "frames.bin", 2 * GIB);
    let setup = d.file(r"~\Downloads\setup.exe", 800 * MIB);
    let zip = d.file(r"~\Downloads\photos.zip", 600 * MIB);
    d.file(r"~\Downloads\notes.pdf", 300 * MIB);
    let old_pics = d.folder(r"~\Pictures\2019", "a.jpg", 30 * GIB);
    let new_pics = d.folder(r"~\Pictures\2026", "b.jpg", 6 * GIB);
    d.fresh(&format!(r"{new_pics}\b.jpg"), DAY);
    d.nas();
    let vid = d.index();

    let r = d.review(Scope::Volume(vid), NOW);
    let mut cards: Vec<&str> = r.cards.iter().map(|c| c.card.as_str()).collect();
    cards.sort();
    assert_eq!(cards, ["archive", "build", "caches", "downloads", "temp"]);
    assert_eq!(r.cards[0].card, "archive", "biggest first: {:?}", r.cards);
    assert_eq!(r.cards[0].title, "Send elsewhere");

    let t = d.card(&r, "temp").unwrap();
    assert_eq!(t.kind, ProposalKind::Delete);
    assert_eq!(t.conversation_id.as_deref(), Some("review"));
    assert_eq!(t.review_id.as_deref(), Some(r.id.as_str()));
    assert_eq!(t.card.as_deref(), Some("temp"));
    assert_eq!(items(&t), [(dumps.clone(), true), (temp.clone(), true)]);
    let old = t.items.iter().find(|i| i.path == temp).unwrap();
    assert_eq!(old.verdict, Some(Verdict::Safe));
    assert_eq!(
        old.reason.as_deref(),
        Some("User temp folder: untouched for 400 days.")
    );
    assert_eq!(old.alloc, 5 * GIB as i64);

    let c = d.card(&r, "caches").unwrap();
    assert_eq!(items(&c), [(chrome.clone(), true)]);
    assert!(
        c.items[0]
            .reason
            .as_deref()
            .unwrap()
            .starts_with("Google Chrome cache: "),
        "{:?}",
        c.items[0].reason
    );

    let b = d.card(&r, "build").unwrap();
    assert_eq!(items(&b), [(target.clone(), true), (modules.clone(), true)]);
    assert!(!b.items.iter().any(|i| i.path == not_rust));

    let dl = d.card(&r, "downloads").unwrap();
    assert_eq!(items(&dl), [(zip.clone(), false), (setup.clone(), false)]);
    let exe = dl.items.iter().find(|i| i.path == setup).unwrap();
    assert_eq!(exe.verdict, Some(Verdict::Redownload));

    let a = d.card(&r, "archive").unwrap();
    assert_eq!(a.kind, ProposalKind::Archive);
    assert_eq!(a.destination.as_ref().map(|x| x.name.as_str()), Some("NAS"));
    assert_eq!(items(&a), [(old_pics.clone(), false)]);
    assert_eq!(
        a.items[0].reason.as_deref(),
        Some("Your photos and videos: nothing in it opened or changed for 400 days.")
    );

    let area = |p: &str| r.areas.iter().find(|x| x.path == p);
    let pics = area(&d.path(r"~\Pictures")).expect("the rest of the photos");
    assert_eq!(pics.size, 6 * GIB);
    assert_eq!(pics.hint, None);
    let odd = area(&not_rust).expect("a target with no Cargo.toml");
    assert!(odd.unknown);
    assert!(odd.why_not.starts_with("Named target, but no Cargo.toml"));
}

// Traces: REVIEW-BUILD
#[test]
fn pending_and_recently_declined_items_stay_out_and_small_cards_only_show_for_a_folder() {
    let d = Disk::new();
    let kept_for_chat = d.folder(r"code\a\node_modules", "x.js", 1200 * MIB);
    let declined = d.folder(r"code\b\node_modules", "x.js", 1100 * MIB);
    let long_ago = d.folder(r"code\c\node_modules", "x.js", 1000 * MIB);
    let fresh = d.folder(r"code\d\node_modules", "x.js", 1300 * MIB);
    let pip = d.folder(r"~\AppData\Local\pip\cache", "wheel.whl", 300 * MIB);
    d.folder(r"~\Documents", "big.dat", 2 * GIB);
    let vid = d.index();

    d.chat_card(&[&kept_for_chat]);
    let no = d.chat_card(&[&declined]);
    proposals::decline(&d.db, &no.id).unwrap();
    let old = d.chat_card(&[&long_ago]);
    proposals::decline(&d.db, &old.id).unwrap();
    d.db.conn()
        .unwrap()
        .execute(
            "UPDATE proposal SET decided_ms = ?2 WHERE id = ?1",
            rusqlite::params![old.id, NOW - 31 * DAY],
        )
        .unwrap();

    let r = d.review(Scope::Volume(vid), NOW);
    let b = d.card(&r, "build").unwrap();
    assert_eq!(items(&b), [(long_ago.clone(), true), (fresh.clone(), true)]);
    assert!(
        d.card(&r, "caches").is_none(),
        "300 MiB is under the 500 MiB a card needs"
    );
    assert!(!r.areas.iter().any(|a| a.path == kept_for_chat));

    let folder = d.review(Scope::Folder(d.path(r"~\AppData\Local\pip")), NOW);
    assert_eq!(
        folder.scope_path.as_deref(),
        Some(d.path(r"~\AppData\Local\pip").as_str())
    );
    let c = d
        .card(&folder, "caches")
        .expect("a folder review shows any card");
    assert_eq!(items(&c), [(pip.clone(), true)]);
}

// Traces: REVIEW-BUILD, REVIEW-UNITS
#[test]
fn old_scratch_folders_beside_a_live_one_still_count_and_big_cards_list_fifty() {
    let d = Disk::new();
    let claude = d.path(r"~\AppData\Local\Temp\claude");
    d.folder(&format!(r"{claude}\proj\live"), "model.gguf", 2 * GIB);
    d.fresh(&format!(r"{claude}\proj\live\model.gguf"), HOUR);
    let gone = d.folder(&format!(r"{claude}\proj\old"), "model.gguf", 40 * GIB);
    for i in 0..55u64 {
        d.folder(
            &format!(r"~\AppData\Local\Temp\tmp{i:02}"),
            "x.bin",
            (60 + i) * MIB,
        );
    }
    let vid = d.index();
    let r = d.review(Scope::Volume(vid), NOW);
    let t = d.card(&r, "temp").unwrap();
    assert_eq!(t.items.len(), 50);
    assert_eq!(t.items[0].path, gone);
    assert!(!t.items.iter().any(|i| i.path.contains(r"\live")));
    assert!(
        t.summary
            .ends_with(" 6 smaller items (375 MB) are not listed."),
        "{}",
        t.summary
    );
}

// Traces: REVIEW-SUPERSEDE
#[test]
fn a_new_review_replaces_untouched_cards_and_keeps_touched_ones() {
    let d = Disk::new();
    let temp = d.folder(r"~\AppData\Local\Temp\old", "x.bin", 2 * GIB);
    let target = d.folder(r"code\proj\target", "app.exe", 3 * GIB);
    d.file(r"code\proj\Cargo.toml", 1024);
    let modules = d.folder(r"code\web\node_modules", "x.js", 2 * GIB);
    let vid = d.index();

    let first = d.review(Scope::Volume(vid), NOW);
    let t1 = d.card(&first, "temp").unwrap();
    let b1 = d.card(&first, "build").unwrap();
    assert!(!t1.touched && !b1.touched);
    let only_target: Vec<i64> = b1
        .items
        .iter()
        .filter(|i| i.path == target)
        .map(|i| i.id)
        .collect();
    let b1 = proposals::select(&d.db, &b1.id, &only_target).unwrap();
    assert!(b1.touched);
    assert_eq!(
        items(&b1),
        [(target.clone(), true), (modules.clone(), false)]
    );
    assert!(!review::untouched(&d.db.conn().unwrap(), &first.id).unwrap());

    let folder = d.review(Scope::Folder(d.path("code")), NOW);
    assert!(folder.cards.is_empty(), "its items are in a pending card");
    assert_eq!(
        d.status(&t1.id),
        "pending",
        "a folder review replaces nothing"
    );

    let second = d.review(Scope::Volume(vid), NOW + HOUR);
    assert_eq!(d.status(&t1.id), "superseded");
    assert_eq!(d.status(&b1.id), "pending");
    let t2 = d.card(&second, "temp").unwrap();
    assert_eq!(items(&t2), [(temp.clone(), true)]);
    assert!(d.card(&second, "build").is_none());

    let declined = proposals::decline(&d.db, &t2.id).unwrap();
    assert!(declined.touched);
}

// Traces: REVIEW-SUPERSEDE, REVIEW-TRIGGER
#[test]
fn a_refresh_rebuilds_an_untouched_review_and_reopening_waits_six_hours() {
    let d = Disk::new();
    d.folder(r"~\AppData\Local\Temp\old", "x.bin", 2 * GIB);
    let vid = d.index();
    let conn = d.db.conn().unwrap();

    let first = review::open(&d.db, &d.env, &d.fs, vid, NOW).unwrap();
    let again = review::open(&d.db, &d.env, &d.fs, vid, NOW + 5 * HOUR).unwrap();
    assert_eq!(again.id, first.id);
    assert!(
        review::rebuild_if_untouched(&d.db, &d.env, &d.fs, vid, NOW)
            .unwrap()
            .is_none(),
        "no refresh since it was built"
    );
    let later = review::open(&d.db, &d.env, &d.fs, vid, NOW + 6 * HOUR + 1).unwrap();
    assert_ne!(later.id, first.id);

    d.index();
    assert!(review::needs_rerun(&conn, &later, NOW + 6 * HOUR + 2).unwrap());
    let rebuilt = review::rebuild_if_untouched(&d.db, &d.env, &d.fs, vid, NOW + 7 * HOUR)
        .unwrap()
        .expect("a refresh finished and nothing was touched");
    assert_ne!(rebuilt.id, later.id);
    assert_eq!(d.status(&d.card(&later, "temp").unwrap().id), "superseded");

    let t = d.card(&rebuilt, "temp").unwrap();
    proposals::select(&d.db, &t.id, &[]).unwrap();
    d.index();
    assert!(
        review::rebuild_if_untouched(&d.db, &d.env, &d.fs, vid, NOW + 8 * HOUR)
            .unwrap()
            .is_none(),
        "the user changed a tick"
    );
    let reopened = review::open(&d.db, &d.env, &d.fs, vid, NOW + 8 * HOUR).unwrap();
    assert_ne!(
        reopened.id, rebuilt.id,
        "opening after a refresh builds anew"
    );
    assert_eq!(d.status(&t.id), "pending");
    assert_eq!(review::ready_bytes(&conn, &reopened.id).unwrap(), 0);
}

// Traces: ACT-FOLLOW-UP, REVIEW-SUPERSEDE
#[test]
fn approving_a_review_card_touches_it_and_its_follow_up_stays_in_the_review() {
    let d = Disk::new();
    let a = d.folder(r"~\AppData\Local\Temp\old-a", "x.bin", 2 * GIB);
    let b = d.folder(r"~\AppData\Local\Temp\old-b", "y.bin", GIB);
    let vid = d.index();
    let r = d.review(Scope::Volume(vid), NOW);
    let t = d.card(&r, "temp").unwrap();
    assert_eq!(items(&t), [(a.clone(), true), (b.clone(), true)]);
    let ex = d.executor();
    let first = t.items.iter().find(|i| i.path == a).unwrap().id;
    let job = ex.approve(&t.id, &[first]).unwrap().job_id;
    let start = Instant::now();
    while !ex
        .jobs()
        .iter()
        .any(|j| j.id == job && matches!(j.status.as_str(), "done" | "failed" | "cancelled"))
    {
        assert!(start.elapsed() < Duration::from_secs(60), "job did not end");
        std::thread::sleep(Duration::from_millis(20));
    }
    let p = proposals::get(&d.db, &t.id).unwrap().unwrap();
    assert!(p.touched);
    assert!(!Path::new(&a).exists());
    let f = proposals::get(&d.db, p.follow_up_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(f.review_id.as_deref(), Some(r.id.as_str()));
    assert_eq!(f.card.as_deref(), Some("temp"));
    assert_eq!(f.conversation_id.as_deref(), Some("review"));
    assert_eq!(items(&f), [(b.clone(), false)]);
    let conn = d.db.conn().unwrap();
    assert!(!review::untouched(&conn, &r.id).unwrap());
    let again = review::get(&conn, &r.id).unwrap().unwrap();
    assert_eq!(again.cards.len(), 1, "a follow-up is no card of its own");
    assert_eq!(review::proposal_ids(&conn, &r.id).unwrap().len(), 2);
}

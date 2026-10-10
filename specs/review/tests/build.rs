use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hoardini_core::actions::Executor;
use hoardini_core::cloud::{Cloud, CloudState, SyncRoot};
use hoardini_core::compress;
use hoardini_core::copycheck::{self, Confidence, CopyResult, Subject};
use hoardini_core::destinations::{self, NewDestination};
use hoardini_core::index::{FakeFs, RefreshOptions, WinLister, normalize_path, refresh};
use hoardini_core::memory::knowledge::upsert_with;
use hoardini_core::memory::location::{Answer, answer};
use hoardini_core::memory::{Detail, Kind, Knowledge, Scope as MemScope};
use hoardini_core::policy::{PolicyEnv, Verdict};
use hoardini_core::proposals::{self, NewItem, NewProposal, Proposal, ProposalKind};
use hoardini_core::review::{self, Builds, Opened, Review, Scope};
use hoardini_core::store::Db;
use hoardini_core::syscleanup::{self, SystemFacts};
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
    let build = second
        .cards
        .iter()
        .find(|c| c.card == "build")
        .expect("the touched card is still listed");
    assert_eq!(
        (
            build.proposal_id.as_str(),
            build.title.as_str(),
            build.carried
        ),
        (b1.id.as_str(), "Build output", true),
        "carried over, not made again"
    );
    assert_eq!(second.cards.len(), 2);
    let conn = d.db.conn().unwrap();
    assert!(
        review::proposal_ids(&conn, &second.id)
            .unwrap()
            .contains(&b1.id)
    );
    assert_eq!(
        review::ready_bytes(&conn, &second.id).unwrap(),
        (5 * GIB) as i64,
        "the new temp card and the target still ticked on the carried one"
    );
    assert!(
        review::untouched(&conn, &second.id).unwrap(),
        "a carried card does not make the new review touched"
    );
    let declined = proposals::decline(&d.db, &t2.id).unwrap();
    assert!(declined.touched);
    let third = d.review(Scope::Volume(vid), NOW + 2 * HOUR);
    assert!(
        third
            .cards
            .iter()
            .any(|c| c.proposal_id == b1.id && c.carried),
        "still listed a review later"
    );
    assert!(
        !review::get(&conn, &first.id)
            .unwrap()
            .unwrap()
            .cards
            .iter()
            .any(|c| c.carried),
        "nothing is carried back into an older review"
    );

    proposals::decline(&d.db, &b1.id).unwrap();
    let fourth = d.review(Scope::Volume(vid), NOW + 3 * HOUR);
    assert!(
        !fourth.cards.iter().any(|c| c.proposal_id == b1.id),
        "a declined card is no longer live"
    );
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
    // The job walks the real folders again (POLICY-REVALIDATE), so they are as old on disk as in the index.
    for p in [&a, &b] {
        hoardini_win::set_all_times(Path::new(p), hoardini_core::store::now_ms() - 3 * DAY)
            .unwrap();
    }
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

// A sync root stand-in: in-sync state is set per path, nothing is ever freed.
struct FakeCloud {
    root: String,
    synced: Mutex<Vec<String>>,
}

impl Cloud for FakeCloud {
    fn roots(&self) -> Vec<SyncRoot> {
        vec![SyncRoot {
            path: self.root.clone(),
            provider: "OneDrive".into(),
        }]
    }

    fn state(&self, path: &Path) -> std::io::Result<Option<CloudState>> {
        let p = normalize_path(&path.to_string_lossy()).to_lowercase();
        Ok(self
            .synced
            .lock()
            .unwrap()
            .contains(&p)
            .then_some(CloudState {
                in_sync: true,
                pinned: false,
                local_bytes: 1,
            }))
    }

    fn free(&self, _path: &Path) -> std::io::Result<u64> {
        Err(std::io::Error::other("never freed in a review test"))
    }
}

fn store_copy(d: &Disk, path: &str, check: &str, confidence: Confidence, note: &str) {
    let conn = d.db.conn().unwrap();
    let s = Subject::load(&conn, path).expect("a real file");
    copycheck::store(
        &conn,
        &s,
        &CopyResult::new(
            check,
            Some("somewhere".into()),
            confidence,
            json!({"note": note}),
        ),
    )
    .unwrap();
}

fn job_row(d: &Disk, dir: &str, detail: Detail) {
    upsert_with(
        &d.db.conn().unwrap(),
        Knowledge {
            scope: MemScope::Path,
            key: dir.to_string(),
            kind: Kind::Verdict,
            text: "You cleaned 1 item here.".into(),
            source: "job:p1".into(),
            detail,
        },
        NOW - 30 * DAY,
        &d.profile,
    )
    .unwrap();
}

// Traces: REVIEW-BUILD, REVIEW-UNITS, MEM-LEARN-JOB, POLICY-ITEM-MEMORY
#[test]
fn verified_copies_disposable_answers_and_learned_entries_shape_the_cards() {
    let d = Disk::new();
    let tool = d.file(r"work\setup\tool-2.1.exe", 900 * MIB);
    let twin = d.file(r"work\copy\image.iso", 2 * GIB);
    let junk = d.folder(r"old\junk", "stuff.dat", 2 * GIB);
    let kept = d.folder(r"code\a\node_modules", "x.js", 1200 * MIB);
    let grown = d.folder(r"code\b\node_modules", "x.js", 1600 * MIB);
    let vid = d.index();
    store_copy(
        &d,
        &tool,
        "winget",
        Confidence::Identical,
        "in winget as Contoso.Tool 2.1",
    );
    store_copy(
        &d,
        &twin,
        "dup",
        Confidence::Identical,
        r"identical copy at D:\iso\image.iso",
    );
    answer(
        &d.db.conn().unwrap(),
        &d.env,
        &junk,
        Answer::Disposable,
        NOW - DAY,
    )
    .unwrap();
    // 2024-01-15 12:00 UTC.
    let cleaned_ms = 1_705_320_000_000i64;
    job_row(
        &d,
        &d.path("code"),
        Detail {
            category: "unknown".into(),
            by: Some("job".into()),
            kept: vec![
                json!({"ms": NOW - 30 * DAY, "path": kept, "size": 1200 * MIB, "job": "p1"}),
                json!({"ms": NOW - 30 * DAY, "path": grown, "size": 1000 * MIB, "job": "p1"}),
            ],
            cleaned: vec![json!({
                "ms": cleaned_ms, "bytes": 28 * GIB, "path": grown, "rule": grown,
                "job": "p0", "kind": "delete"
            })],
            ..Detail::default()
        },
    );

    let r = d.review(Scope::Volume(vid), NOW);
    let re = d.card(&r, "redownload").expect("a verified card");
    assert_eq!(re.kind, ProposalKind::Delete);
    assert_eq!(items(&re), [(tool.clone(), true)]);
    assert_eq!(
        re.items[0].reason.as_deref(),
        Some("In winget as Contoso.Tool 2.1.")
    );
    assert_eq!(re.items[0].verdict, Some(Verdict::Redownload));
    assert!(
        !r.cards
            .iter()
            .filter_map(|c| d.card(&r, &c.card))
            .any(|p| p.items.iter().any(|i| i.path == twin)),
        "a duplicate elsewhere is no re-download"
    );

    let title = r.cards.iter().find(|c| c.card == "disposable").unwrap();
    assert_eq!(title.title, "Folders you said can go");
    let dis = d.card(&r, "disposable").unwrap();
    assert_eq!(items(&dis), [(junk.clone(), true)]);
    assert_eq!(
        dis.items[0].reason.as_deref(),
        Some("You said this folder can go.")
    );

    let b = d.card(&r, "build").expect("the build card");
    assert_eq!(
        items(&b),
        [(grown.clone(), true)],
        "a folder left unticked stays out until it grows by half"
    );
    assert_eq!(
        b.items[0].reason.as_deref(),
        Some(
            "node_modules: Installed JavaScript dependencies. You cleaned this on 01-15 (28.0 GB)."
        )
    );
    assert_eq!(b.items[0].memory_note, None, "the reason already says it");
}

// Traces: REVIEW-BUILD, ACT-ONLINE-ONLY
#[test]
fn cold_synced_files_go_online_only_and_never_to_a_delete_card() {
    let mut d = Disk::new();
    let root = d.path(r"~\OneDrive");
    let cloud = Arc::new(FakeCloud {
        root: root.clone(),
        synced: Mutex::new(Vec::new()),
    });
    d.env = d.env.clone().with_cloud(cloud.clone());
    let old = d.file(r"~\OneDrive\Videos\2019.mp4", 3 * GIB);
    let new = d.file(r"~\OneDrive\Videos\2026.mp4", 3 * GIB);
    d.fresh(&new, DAY);
    let modules = d.folder(r"~\OneDrive\code\app\node_modules", "x.js", 1536 * MIB);
    let unsynced = d.file(r"~\OneDrive\Videos\raw.mov", 2 * GIB);
    for p in [&old, &new] {
        cloud
            .synced
            .lock()
            .unwrap()
            .push(normalize_path(p).to_lowercase());
    }
    let vid = d.index();

    let r = d.review(Scope::Volume(vid), NOW);
    let o = d.card(&r, "online_only").expect("an online-only card");
    assert_eq!(o.kind, ProposalKind::OnlineOnly);
    assert_eq!(items(&o), [(old.clone(), true)]);
    assert_eq!(o.items[0].verdict, Some(Verdict::Synced));
    assert_eq!(
        o.items[0].reason.as_deref(),
        Some("Synced to OneDrive; nothing in it opened or changed for 400 days.")
    );
    for c in &r.cards {
        let p = d.card(&r, &c.card).unwrap();
        if p.kind != ProposalKind::OnlineOnly {
            assert!(
                !p.items.iter().any(|i| i.path.starts_with(&root)),
                "{} holds a synced file: {:?}",
                c.card,
                p.items
            );
        }
    }
    assert!(
        d.card(&r, "build").is_none(),
        "{modules} is in a sync folder"
    );
    assert!(
        !o.items.iter().any(|i| i.path == unsynced || i.path == new),
        "not in sync, or opened yesterday"
    );
}

fn compress_fixture(d: &Disk) -> String {
    let corpus = d.path(r"data\corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    for i in 0..3 {
        d.fs.file(&format!(r"{corpus}\part{i}.txt"), GIB);
    }
    corpus
}

// Traces: REVIEW-BUILD, ACT-COMPRESS-CANDIDATES
#[test]
fn a_folder_worth_compressing_is_a_compress_card_once_its_estimate_is_in() {
    let d = Disk::new();
    let corpus = compress_fixture(&d);
    let vid = d.index();
    let first = d.review(Scope::Volume(vid), NOW);
    assert!(d.card(&first, "compress").is_none(), "no estimate yet");

    let conn = d.db.conn().unwrap();
    let c = compress::candidates(&conn, &d.env, &d.root, NOW).unwrap();
    assert_eq!(c[0].path, corpus);
    conn.execute(
        "INSERT INTO compress_estimate(path, size, newest_ms, logical, saved_bytes, ratio, estimated_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, 0.6, 0)",
        rusqlite::params![
            corpus,
            c[0].size as i64,
            c[0].newest_ms,
            (3 * GIB) as i64,
            (1229 * MIB) as i64
        ],
    )
    .unwrap();
    let r = d.review(Scope::Volume(vid), NOW + HOUR);
    let k = d.card(&r, "compress").expect("a compress card");
    assert_eq!(k.kind, ProposalKind::Compress);
    assert_eq!(k.format.as_deref(), Some("lzx"));
    assert_eq!(items(&k), [(corpus.clone(), false)]);
    assert_eq!(
        k.items[0].reason.as_deref(),
        Some("Windows compression would save about 1.2 GB (40% of it); the files stay usable.")
    );
    assert_eq!(k.items[0].saves, Some(1229 * MIB));
    assert_eq!(review::ready_bytes(&conn, &r.id).unwrap(), 0, "unticked");
    proposals::select(&d.db, &k.id, &[k.items[0].id]).unwrap();
    assert_eq!(
        review::ready_bytes(&conn, &r.id).unwrap(),
        (1229 * MIB) as i64,
        "what compressing saves, not the 3 GiB folder"
    );
}

fn sparse(path: &str, len: u64) {
    std::fs::File::create(path).unwrap();
    hoardini_win::set_sparse_and_zero_range(Path::new(path), 0, 0).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(len)
        .unwrap();
}

// Traces: REVIEW-TRIGGER, ACT-COMPRESS-CANDIDATES
#[test]
fn estimates_that_make_a_folder_worth_compressing_rebuild_an_untouched_review() {
    let d = Disk::new();
    let corpus = compress_fixture(&d);
    for i in 0..3 {
        sparse(&format!(r"{corpus}\part{i}.txt"), GIB);
    }
    let vid = d.index();
    let r = d.review(Scope::Volume(vid), NOW);
    assert!(d.card(&r, "compress").is_none());

    assert!(
        review::compress_estimates(&d.db, &d.env, &r, NOW).unwrap(),
        "zeros compress to almost nothing"
    );
    let again = review::rebuild_if_current(&d.db, &d.env, &d.fs, &r.id, NOW + HOUR)
        .unwrap()
        .expect("untouched and still the latest");
    let k = d
        .card(&again, "compress")
        .expect("the new review has the card");
    assert_eq!(items(&k), [(corpus.clone(), false)]);
    assert!(
        !review::compress_estimates(&d.db, &d.env, &again, NOW).unwrap(),
        "nothing new to estimate"
    );
    assert!(
        review::rebuild_if_current(&d.db, &d.env, &d.fs, &r.id, NOW + 2 * HOUR)
            .unwrap()
            .is_none(),
        "a newer review replaced it"
    );
    proposals::select(&d.db, &k.id, &[]).unwrap();
    assert!(
        review::rebuild_if_current(&d.db, &d.env, &d.fs, &again.id, NOW + 2 * HOUR)
            .unwrap()
            .is_none(),
        "the user touched it"
    );
}

// Traces: REVIEW-TRIGGER
#[test]
fn open_answers_at_once_while_one_build_runs_in_the_background() {
    let d = Disk::new();
    d.folder(r"~\AppData\Local\Temp\old", "x.bin", 2 * GIB);
    let vid = d.index();
    let clock = Arc::new(AtomicI64::new(NOW));
    let announced = Arc::new(AtomicUsize::new(0));
    let builds = {
        let (c, a) = (clock.clone(), announced.clone());
        Builds::new(
            move || c.load(Ordering::SeqCst),
            move || {
                a.fetch_add(1, Ordering::SeqCst);
            },
        )
    };
    let started = Arc::new(AtomicUsize::new(0));
    let (go, gate) = std::sync::mpsc::channel::<()>();
    let gate = Arc::new(Mutex::new(gate));
    let job = || {
        let (db, env, started, gate) = (d.db.clone(), d.env.clone(), started.clone(), gate.clone());
        move |now: i64| -> anyhow::Result<()> {
            started.fetch_add(1, Ordering::SeqCst);
            gate.lock().unwrap().recv()?;
            review::build(&db, &env, &WinLister, Scope::Volume(vid), now)?;
            Ok(())
        }
    };
    let conn = d.db.conn().unwrap();

    assert!(matches!(
        builds.open(&conn, vid, job()).unwrap(),
        Opened::Building(None)
    ));
    assert!(builds.building());
    assert!(matches!(
        builds.open(&conn, vid, job()).unwrap(),
        Opened::Building(None)
    ));
    let key = format!("volume:{vid}");
    assert!(
        !builds.start(&key, job()),
        "one build at a time for a drive"
    );
    go.send(()).unwrap();
    builds.wait();
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(announced.load(Ordering::SeqCst), 1);
    let Opened::Ready(first) = builds.open(&conn, vid, job()).unwrap() else {
        panic!("built just now");
    };
    assert_eq!(first.created_ms, NOW);

    clock.store(NOW + 5 * HOUR, Ordering::SeqCst);
    let Opened::Ready(same) = builds.open(&conn, vid, job()).unwrap() else {
        panic!("5 hours is fresh");
    };
    assert_eq!(same.id, first.id);

    clock.store(NOW + 6 * HOUR + 1, Ordering::SeqCst);
    let Opened::Building(Some(shown)) = builds.open(&conn, vid, job()).unwrap() else {
        panic!("stale after 6 hours");
    };
    assert_eq!(shown.id, first.id, "the last review shows meanwhile");
    go.send(()).unwrap();
    builds.wait();
    assert_eq!(announced.load(Ordering::SeqCst), 2);
    let Opened::Ready(second) = builds.open(&conn, vid, job()).unwrap() else {
        panic!("built");
    };
    assert_ne!(second.id, first.id);
    assert_eq!(second.created_ms, NOW + 6 * HOUR + 1);

    assert!(builds.start("x", |_| anyhow::bail!("the index is locked")));
    builds.wait();
    assert!(!builds.building(), "a failed build ends too");
    assert_eq!(announced.load(Ordering::SeqCst), 3);
    assert_eq!(builds.exclusive(|now| now), NOW + 6 * HOUR + 1);
    assert_eq!(announced.load(Ordering::SeqCst), 4);

    let order = Arc::new(Mutex::new(Vec::new()));
    let step = |name: &'static str| {
        let (order, gate) = (order.clone(), gate.clone());
        move |_now: i64| -> anyhow::Result<()> {
            if name == "a" {
                gate.lock().unwrap().recv()?;
            }
            order.lock().unwrap().push(name);
            Ok(())
        }
    };
    assert!(builds.start("volume:a", step("a")));
    assert!(
        builds.start("volume:b", step("b")),
        "another drive waits its turn"
    );
    assert!(!builds.start("volume:b", step("b")), "once");
    go.send(()).unwrap();
    builds.wait();
    assert_eq!(*order.lock().unwrap(), ["a", "b"]);
    assert!(!builds.building());
    assert_eq!(announced.load(Ordering::SeqCst), 6);
}

// Traces: REVIEW-AREAS, EVID-COPY-CHECK
#[test]
fn a_big_file_every_check_missed_is_an_only_copy_area_of_its_own() {
    let d = Disk::new();
    d.file(r"proj\.git\HEAD", 1024);
    let own = d.file(r"proj\weights\mine.bin", 3 * GIB);
    d.file(r"proj\data\x.dat", 2 * GIB);
    let disk = d.file(r"vm\disk.vhdx", 4 * GIB);
    let vid = d.index();
    for p in [&own, &disk] {
        store_copy(
            &d,
            p,
            "dup",
            Confidence::None,
            "no other file of this size on the indexed drives",
        );
    }
    let r = d.review(Scope::Volume(vid), NOW);
    let a = r
        .areas
        .iter()
        .find(|a| a.path == own)
        .expect("a row of its own");
    assert!(a.only_copy);
    assert_eq!(a.size, 3 * GIB);
    assert_eq!(
        a.why_not,
        "Only copy we know of: no other file of this size on the indexed drives."
    );
    let p = r
        .areas
        .iter()
        .find(|a| a.path == d.path("proj"))
        .expect("the project keeps the rest");
    assert!(!p.only_copy);
    assert_eq!(p.size, 2 * GIB);
    let vm = r.areas.iter().find(|a| a.path == disk).unwrap();
    assert!(!vm.only_copy, "a virtual disk says what it is instead");
    assert!(vm.why_not.starts_with("A virtual machine's disk"));
}

// Traces: REVIEW-BUILD, ACT-WINDOWS-CLEANUP-DETECT
#[test]
fn the_windows_card_holds_the_leftovers_and_what_only_windows_tools_free() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("h.db")).unwrap();
    let base = PolicyEnv::with_paths(
        Path::new(r"C:\Users\tester"),
        Path::new(r"C:\ProgramData\hoardini-test"),
    );
    let win = base.system_root.clone();
    let facts = SystemFacts {
        system_drive: r"C:\".into(),
        windows_dir: win.clone(),
        ram_bytes: Some(64 * GIB),
        hibernate_enabled: Some(true),
        ..SystemFacts::default()
    };
    let env = base.with_facts(facts.clone());
    let fs = FakeFs::new(r"C:\");
    fs.set_volume(1000 * GIB, 1000 * GIB - 33 * GIB);
    fs.file(&format!(r"{win}\WinSxS\pack.dat"), 6 * GIB);
    fs.file(&format!(r"{win}\Temp\old\x.tmp"), 2 * GIB);
    fs.file(r"C:\hiberfil.sys", 25 * GIB);
    let out = refresh(&db.conn().unwrap(), &fs, r"C:\", &RefreshOptions::default()).unwrap();
    assert!(out.completed);
    let vid: i64 = db
        .conn()
        .unwrap()
        .query_row("SELECT id FROM volume", [], |r| r.get(0))
        .unwrap();
    let ops = |r: &Review| -> Vec<String> {
        r.cards
            .iter()
            .filter(|c| c.card == "windows")
            .flat_map(|c| proposals::get(&db, &c.proposal_id).unwrap().unwrap().items)
            .map(|i| i.op.unwrap().op.name().to_string())
            .collect()
    };

    let first = review::build(&db, &env, &fs, Scope::Volume(vid), NOW).unwrap();
    assert_eq!(
        ops(&first),
        ["leftovers", "component_store", "hibernation"],
        "the leftovers first, then the detected operations in their own order"
    );
    let card = first.cards.iter().find(|c| c.card == "windows").unwrap();
    assert_eq!(card.title, "Cleanups only Windows can do");
    let p = proposals::get(&db, &card.proposal_id).unwrap().unwrap();
    assert_eq!(p.review_id.as_deref(), Some(first.id.as_str()));
    assert_eq!(p.conversation_id.as_deref(), Some("review"));

    syscleanup::propose(&db, &facts, &["hibernation"], Some("main".into()), None).unwrap();
    let second = review::build(&db, &env, &fs, Scope::Volume(vid), NOW + HOUR).unwrap();
    assert_eq!(
        ops(&second),
        ["leftovers", "component_store"],
        "a pending chat card already holds hibernation"
    );
    let status: String = db
        .conn()
        .unwrap()
        .query_row("SELECT status FROM proposal WHERE id = ?1", [&p.id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(status, "superseded");

    let w = second.cards.iter().find(|c| c.card == "windows").unwrap();
    proposals::decline(&db, &w.proposal_id).unwrap();
    let third = review::build(&db, &env, &fs, Scope::Volume(vid), NOW + 2 * HOUR).unwrap();
    assert!(ops(&third).is_empty(), "declined in the last 30 days");
    assert!(
        !third
            .areas
            .iter()
            .any(|a| a.path.to_lowercase().ends_with(r"\temp")),
        "what a declined card held stays out of Large areas too"
    );
}

fn paths(p: &Proposal) -> Vec<String> {
    p.items.iter().map(|i| i.path.clone()).collect()
}

// Fires `act` once, while the build lists `code\proj` for its `Cargo.toml`: after the replaced cards were read.
fn during_build(d: &Disk, act: impl Fn() + Send + Sync + 'static) -> Arc<AtomicBool> {
    let fired = Arc::new(AtomicBool::new(false));
    let (proj, f) = (d.path(r"code\proj"), fired.clone());
    d.fs.on_list(move |p| {
        if p.eq_ignore_ascii_case(&proj) && !f.swap(true, Ordering::SeqCst) {
            act();
        }
    });
    fired
}

fn race_fixture() -> (Disk, i64, String) {
    let d = Disk::new();
    let temp = d.folder(r"~\AppData\Local\Temp\old", "x.bin", 2 * GIB);
    d.folder(r"code\proj\target", "app.exe", 3 * GIB);
    d.file(r"code\proj\Cargo.toml", 1024);
    let vid = d.index();
    (d, vid, temp)
}

// Traces: REVIEW-SUPERSEDE
#[test]
fn a_card_decided_or_ticked_while_a_review_builds_keeps_its_items_out_of_the_new_one() {
    let (d, vid, _) = race_fixture();
    let t1 = d.card(&d.review(Scope::Volume(vid), NOW), "temp").unwrap();
    let (db, id) = (d.db.clone(), t1.id.clone());
    let fired = during_build(&d, move || {
        proposals::decline(&db, &id).unwrap();
    });
    let second = d.review(Scope::Volume(vid), NOW + HOUR);
    d.fs.clear_hook();
    assert!(fired.load(Ordering::SeqCst));
    assert_eq!(d.status(&t1.id), "declined");
    assert!(
        d.card(&second, "temp").is_none(),
        "declined during the build, so out for 30 days"
    );

    let (d, vid, temp) = race_fixture();
    let t3 = d.card(&d.review(Scope::Volume(vid), NOW), "temp").unwrap();
    let ids: Vec<i64> = t3.items.iter().map(|i| i.id).collect();
    let (db, id) = (d.db.clone(), t3.id.clone());
    during_build(&d, move || {
        proposals::select(&db, &id, &ids).unwrap();
    });
    let fourth = d.review(Scope::Volume(vid), NOW + HOUR);
    d.fs.clear_hook();
    assert_eq!(d.status(&t3.id), "pending");
    let holding: Vec<(String, bool)> = fourth
        .cards
        .iter()
        .filter(|c| paths(&proposals::get(&d.db, &c.proposal_id).unwrap().unwrap()).contains(&temp))
        .map(|c| (c.proposal_id.clone(), c.carried))
        .collect();
    assert_eq!(holding, [(t3.id.clone(), true)], "one live card holds it");
    let conn = d.db.conn().unwrap();
    assert_eq!(
        review::ready_bytes(&conn, &fourth.id).unwrap(),
        (5 * GIB) as i64,
        "the temp folder counts once, beside the build card"
    );
}

// Traces: REVIEW-TRIGGER, REVIEW-SUPERSEDE
#[test]
fn a_review_shows_as_latest_only_once_all_its_cards_are_in() {
    let d = Disk::new();
    for i in 0..40 {
        d.folder(
            &format!(r"~\AppData\Local\Temp\old-{i:02}"),
            "scratch.bin",
            600 * MIB,
        );
    }
    d.folder(r"code\proj\target", "app.exe", 3 * GIB);
    d.file(r"code\proj\Cargo.toml", 1024);
    d.folder(r"code\web\node_modules", "big.js", 1536 * MIB);
    let vid = d.index();
    let first = d.review(Scope::Volume(vid), NOW);
    let old: Vec<String> = first.cards.iter().map(|c| c.proposal_id.clone()).collect();

    let stop = Arc::new(AtomicBool::new(false));
    let seen: Arc<Mutex<Vec<(String, usize, usize)>>> = Arc::default();
    let poller = {
        let (db, stop, seen, old) = (d.db.clone(), stop.clone(), seen.clone(), old.clone());
        std::thread::spawn(move || {
            let c = db.conn().unwrap();
            while !stop.load(Ordering::SeqCst) {
                if let Ok(Some(r)) = review::latest(&c, vid, None) {
                    let pending_old = old
                        .iter()
                        .filter(|id| {
                            c.query_row("SELECT status FROM proposal WHERE id = ?1", [id], |r| {
                                r.get::<_, String>(0)
                            })
                            .is_ok_and(|s| s == "pending")
                        })
                        .count();
                    seen.lock()
                        .unwrap()
                        .push((r.id, r.cards.len(), pending_old));
                }
            }
        })
    };
    let second = d.review(Scope::Volume(vid), NOW + 7 * HOUR);
    stop.store(true, Ordering::SeqCst);
    poller.join().unwrap();
    let seen = seen.lock().unwrap();
    assert!(!seen.is_empty());
    for (id, cards, pending_old) in seen.iter() {
        if *id == second.id {
            assert_eq!(*cards, second.cards.len(), "seen half built");
            assert_eq!(*pending_old, 0, "seen beside the cards it replaces");
        } else {
            assert_eq!(*id, first.id);
        }
    }
}

// Traces: REVIEW-SUPERSEDE
#[test]
fn a_volume_review_replaces_a_folder_reviews_untouched_cards_and_carries_its_touched_ones() {
    let d = Disk::new();
    let target = d.folder(r"code\proj\target", "app.exe", 3 * GIB);
    d.file(r"code\proj\Cargo.toml", 1024);
    let temp = d.folder(r"~\AppData\Local\Temp\old", "x.bin", 2 * GIB);
    let vid = d.index();
    let tf = d
        .card(&d.review(Scope::Folder(d.path("~")), NOW), "temp")
        .unwrap();
    assert_eq!(paths(&tf), std::slice::from_ref(&temp));
    proposals::select(&d.db, &tf.id, &[]).unwrap();
    let folder = d.review(Scope::Folder(d.path("code")), NOW);
    let fb = d.card(&folder, "build").unwrap();
    let again = d.review(Scope::Volume(vid), NOW + HOUR);
    assert_eq!(d.status(&fb.id), "superseded");
    let b = d
        .card(&again, "build")
        .expect("the volume review proposes it");
    assert_eq!(paths(&b), std::slice::from_ref(&target));
    assert_eq!(d.status(&tf.id), "pending");
    let listed = again
        .cards
        .iter()
        .find(|c| c.proposal_id == tf.id)
        .expect("the folder review's touched card is listed");
    assert!(listed.carried);
    assert_eq!(
        again.cards.iter().filter(|c| c.card == "temp").count(),
        1,
        "and not proposed again"
    );
}

// Traces: REVIEW-BUILD
#[test]
fn one_temp_part_in_a_chat_card_leaves_the_other_parts_in_the_temp_card() {
    let d = Disk::new();
    let a = d.folder(r"~\AppData\Local\Temp\old-a", "x.bin", 2 * GIB);
    let b = d.folder(r"~\AppData\Local\Temp\old-b", "y.bin", 2 * GIB);
    let vid = d.index();
    d.chat_card(&[&a]);
    let r = d.review(Scope::Volume(vid), NOW);
    let t = d.card(&r, "temp").expect("a temp card");
    assert_eq!(paths(&t), std::slice::from_ref(&b));
}

// Traces: REVIEW-UNITS, REVIEW-BUILD
#[test]
fn a_protected_temp_part_is_a_kept_unit_of_its_own() {
    use hoardini_core::memory::{Kind as MemKind, NewMemory, remember_with};
    let d = Disk::new();
    let a = d.folder(r"~\AppData\Local\Temp\old-a", "x.bin", 2 * GIB);
    let b = d.folder(r"~\AppData\Local\Temp\old-b", "y.bin", 2 * GIB);
    let vid = d.index();
    remember_with(
        &d.db.conn().unwrap(),
        NewMemory::new(
            MemScope::Path,
            &a,
            MemKind::Protect,
            "My session logs",
            "user",
        ),
        NOW,
        &d.profile,
    )
    .unwrap();
    let units = review::units::resolve(
        &d.db.conn().unwrap(),
        &d.env,
        &d.fs,
        &d.path(r"~\AppData\Local\Temp"),
        NOW,
    )
    .unwrap();
    let kept = units
        .iter()
        .find(|u| u.path == a)
        .expect("a unit of its own");
    assert_eq!(kept.why, review::units::Why::Kept);
    let temp = units
        .iter()
        .find(|u| u.parts.iter().any(|p| p.path == b))
        .unwrap();
    assert!(!temp.parts.iter().any(|p| p.path == a));
    let r = d.review(Scope::Volume(vid), NOW);
    assert_eq!(
        paths(&d.card(&r, "temp").unwrap()),
        std::slice::from_ref(&b)
    );
}

fn store_estimate(d: &Disk, corpus: &str) {
    let conn = d.db.conn().unwrap();
    let c = compress::candidates(&conn, &d.env, &d.root, NOW).unwrap();
    let cc = c.iter().find(|c| c.path == corpus).unwrap();
    conn.execute(
        "INSERT INTO compress_estimate(path, size, newest_ms, logical, saved_bytes, ratio, estimated_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, 0.6, 0)",
        rusqlite::params![
            corpus,
            cc.size as i64,
            cc.newest_ms,
            (3 * GIB) as i64,
            (1229 * MIB) as i64
        ],
    )
    .unwrap();
}

// Traces: REVIEW-TRIGGER, ACT-COMPRESS-CANDIDATES
#[test]
fn estimates_report_no_rebuild_for_a_folder_the_review_leaves_out() {
    let d = Disk::new();
    let corpus = compress_fixture(&d);
    let small = d.path(r"other\small");
    std::fs::create_dir_all(&small).unwrap();
    for i in 0..3 {
        let p = format!(r"{small}\part{i}.txt");
        std::fs::write(&p, b"x").unwrap();
        d.fs.file(&p, GIB);
    }
    let vid = d.index();
    store_estimate(&d, &corpus);
    d.chat_card(&[&corpus]);
    let r = d.review(Scope::Volume(vid), NOW);
    assert!(d.card(&r, "compress").is_none(), "a chat card holds it");
    assert!(
        !review::compress_estimates(&d.db, &d.env, &r, NOW).unwrap(),
        "nothing the review could hold became proposable"
    );
}

// Traces: REVIEW-BUILD, ACT-COMPRESS-CANDIDATES
#[test]
fn a_compress_folder_holding_a_redownload_item_is_left_out() {
    let d = Disk::new();
    let corpus = compress_fixture(&d);
    let part0 = format!(r"{corpus}\part0.txt");
    std::fs::write(&part0, b"x").unwrap();
    let vid = d.index();
    store_copy(
        &d,
        &part0,
        "archive",
        Confidence::Identical,
        r"archived copy at D:\arch\part0.txt (verified when archived)",
    );
    store_estimate(&d, &corpus);
    let r = d.review(Scope::Volume(vid), NOW);
    assert_eq!(
        paths(&d.card(&r, "redownload").unwrap()),
        std::slice::from_ref(&part0)
    );
    assert!(
        d.card(&r, "compress").is_none(),
        "compressing the folder would compress what the other card deletes"
    );
}

// Traces: REVIEW-TRIGGER
#[test]
fn an_open_queued_behind_an_exclusive_build_builds_nothing_once_that_review_is_fresh() {
    let d = Disk::new();
    d.folder(r"~\AppData\Local\Temp\old", "x.bin", 2 * GIB);
    let vid = d.index();
    let builds = Builds::new(|| NOW, || {});
    let (in_ex, started) = std::sync::mpsc::channel::<()>();
    let (go, gate) = std::sync::mpsc::channel::<()>();
    let ex = {
        let (b, db, env) = (builds.clone(), d.db.clone(), d.env.clone());
        std::thread::spawn(move || {
            b.exclusive(|now| {
                in_ex.send(()).unwrap();
                gate.recv().unwrap();
                review::build(&db, &env, &WinLister, Scope::Volume(vid), now).unwrap();
            })
        })
    };
    started.recv().unwrap();
    let conn = d.db.conn().unwrap();
    let job = {
        let (db, env) = (d.db.clone(), d.env.clone());
        move |now: i64| review::build_if_stale(&db, &env, &WinLister, vid, now).map(|_| ())
    };
    assert!(matches!(
        builds.open(&conn, vid, job).unwrap(),
        Opened::Building(None)
    ));
    go.send(()).unwrap();
    ex.join().unwrap();
    builds.wait();
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM review", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        n, 1,
        "the queued build found the exclusive one's review fresh"
    );
    assert!(matches!(
        builds.open(&conn, vid, |_| Ok(())).unwrap(),
        Opened::Ready(_)
    ));
}

// Traces: MEM-LOCATION, REVIEW-UNITS, POLICY-VERDICT-CAP
#[test]
fn a_disposable_answer_stops_counting_once_the_folder_grows_by_half() {
    let d = Disk::new();
    let junk = d.folder(r"old\junk", "stuff.dat", 2 * GIB);
    std::fs::write(format!(r"{junk}\stuff.dat"), b"x").unwrap();
    d.index();
    answer(
        &d.db.conn().unwrap(),
        &d.env,
        &junk,
        Answer::Disposable,
        NOW - DAY,
    )
    .unwrap();
    let inside = format!(r"{junk}\stuff.dat");
    let cap = |d: &Disk| {
        hoardini_core::policy::cap_for_with(
            &d.db.conn().unwrap(),
            &d.env,
            Path::new(&inside),
            None,
            false,
        )
        .unwrap()
    };

    d.fs.file(&format!(r"{junk}\more.dat"), 900 * MIB);
    let vid = d.index();
    let r = d.review(Scope::Volume(vid), NOW);
    let dis = d
        .card(&r, "disposable")
        .expect("grown by under half, still asked for");
    assert_eq!(items(&dis), [(junk.clone(), true)]);
    assert_eq!(cap(&d), None);

    d.fs.file(&format!(r"{junk}\new.dat"), 200 * MIB);
    let vid = d.index();
    let r = d.review(Scope::Volume(vid), NOW + HOUR);
    assert!(
        d.card(&r, "disposable")
            .is_none_or(|p| p.items.iter().all(|i| i.path != junk)),
        "3.1 GB is half again the 2 GB answered for, so nothing is ticked on that answer"
    );
    let area = r
        .areas
        .iter()
        .find(|a| a.path == junk)
        .expect("the folder is an area again");
    assert!(area.unknown, "{area:?}");
    assert_eq!(cap(&d), Some(Verdict::Review));
}

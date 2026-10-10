use std::path::Path;

use hoardini_core::index::{FakeFs, RefreshOptions, refresh};
use hoardini_core::policy::{Action, PolicyEnv, Verdict};
use hoardini_core::proposals::{self, ProposalKind};
use hoardini_core::review::areas::ADMIN_HINT;
use hoardini_core::review::{self, Scope};
use hoardini_core::store::Db;
use hoardini_core::syscleanup::{OpPart, SystemOp};

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
const DAY: i64 = 86_400_000;
const NOW: i64 = 1_700_000_000_000 + 30 * DAY;

// Traces: REVIEW-AREAS
#[test]
fn big_things_no_card_takes_are_listed_with_why_and_a_hint() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("h.db")).unwrap();
    let env = PolicyEnv::with_paths(
        Path::new(r"C:\Users\tester"),
        Path::new(r"C:\ProgramData\hoardini-test"),
    );
    let win = env.system_root.clone();
    let fs = FakeFs::new(r"C:\");
    fs.set_volume(2000 * GIB, 1000 * GIB);
    fs.file(&format!(r"{win}\System32\big.dll"), 10 * GIB);
    fs.file(&format!(r"{win}\WinSxS\pack.dat"), 8 * GIB);
    fs.file(&format!(r"{win}\Temp\setup\x.tmp"), 2 * GIB);
    fs.file(
        &format!(r"{win}\SoftwareDistribution\Download\abc\u.cab"),
        3 * GIB,
    );
    fs.file(r"C:\pagefile.sys", 16 * GIB);
    fs.file(r"C:\hiberfil.sys", 12 * GIB);
    fs.file(r"C:\Program Files\App\app.exe", 5 * GIB);
    fs.file(r"C:\code\covenant-agent\.git\HEAD", 1024);
    fs.file(r"C:\code\covenant-agent\models\tiny\a.dat", 60 * GIB);
    fs.file(r"C:\code\covenant-agent\models\tiny\b.dat", 40 * GIB);
    fs.file(r"C:\code\covenant-agent\data\x.dat", 61 * GIB);
    fs.touch(r"C:\code\covenant-agent\data\x.dat", NOW - 2 * DAY);
    fs.file(r"C:\Users\tester\Pictures\2023\a.jpg", 25 * GIB);
    fs.file(r"C:\Users\tester\Pictures\2024\b.mp4", 15 * GIB);
    fs.file(r"C:\Users\tester\.lmstudio\models\org\q\q.gguf", 20 * GIB);
    fs.file(r"C:\vm\disk.vhdx", 30 * GIB);
    fs.file(r"C:\small\s.dat", 512 * MIB);
    fs.file(r"C:\code\small-proj\.git\objects\pack\p.pack", 2560 * MIB);
    let pd = env.program_data.clone();
    fs.file(
        &format!(r"{pd}\Microsoft\Windows\WER\ReportArchive\r1\big.dmp"),
        1536 * MIB,
    );
    fs.file(
        &format!(r"{pd}\Microsoft\VisualStudio\Packages\pkg\a.vsix"),
        2 * GIB,
    );
    let out = refresh(&db.conn().unwrap(), &fs, r"C:\", &RefreshOptions::default()).unwrap();
    assert!(out.completed);
    let vid: i64 = db
        .conn()
        .unwrap()
        .query_row("SELECT id FROM volume", [], |r| r.get(0))
        .unwrap();

    let r = review::build(&db, &env, &fs, Scope::Volume(vid), NOW).unwrap();
    let cards: Vec<&str> = r.cards.iter().map(|c| c.card.as_str()).collect();
    assert_eq!(cards, ["windows"], "only what needs an administrator");
    let w = proposals::get(&db, &r.cards[0].proposal_id)
        .unwrap()
        .unwrap();
    assert_eq!(w.kind, ProposalKind::SystemCleanup);
    assert_eq!(w.items.len(), 1);
    let op = w.items[0].op.as_ref().unwrap();
    assert_eq!(op.op, SystemOp::Leftovers);
    assert!(op.elevated);
    let w_ = win.to_lowercase();
    let parts: Vec<(String, u64)> = op
        .parts
        .iter()
        .map(|OpPart { path, size }| (path.to_lowercase(), size / MIB))
        .collect();
    assert_eq!(
        parts,
        vec![
            (format!(r"{w_}\softwaredistribution\download\abc"), 3072),
            (format!(r"{w_}\temp\setup"), 2048),
            (
                format!(r"{}\microsoft\windows\wer\reportarchive", pd.to_lowercase()),
                1536
            ),
        ]
    );
    assert_eq!(w.items[0].alloc, (6656 * MIB) as i64);
    assert!(w.items[0].selected, "safe, so it starts ticked");
    let got: Vec<(String, u64, Option<&str>)> = r
        .areas
        .iter()
        .map(|a| (a.path.to_lowercase(), a.size / GIB, a.hint.as_deref()))
        .collect();
    let w = win.to_lowercase();
    assert_eq!(
        got,
        vec![
            (r"c:\code\covenant-agent".to_string(), 161, None),
            (
                r"c:\users\tester\pictures".to_string(),
                40,
                Some("Add a destination to move them off C:.")
            ),
            (r"c:\vm\disk.vhdx".to_string(), 30, None),
            (r"c:\users\tester\.lmstudio\models".to_string(), 20, None),
            (
                r"c:\pagefile.sys".to_string(),
                16,
                Some("Windows manages its size.")
            ),
            (
                r"c:\hiberfil.sys".to_string(),
                12,
                Some("Windows recreates it at boot; turning hibernation off removes it.")
            ),
            (w.clone(), 10, None),
            (
                format!(r"{w}\winsxs"),
                8,
                Some("Windows can trim this itself.")
            ),
            (r"c:\program files".to_string(), 5, None),
            (r"c:\code\small-proj".to_string(), 2, None),
            (
                format!(r"{}\microsoft\visualstudio\packages", pd.to_lowercase()),
                2,
                Some(ADMIN_HINT)
            ),
        ]
    );
    assert_eq!(ADMIN_HINT, "Needs admin.");

    let agent = &r.areas[0];
    assert!(agent.unknown);
    assert_eq!(
        (agent.label.as_str(), agent.category.as_str(), agent.verdict),
        ("unknown", "unknown", None)
    );
    assert!(agent.actions.is_empty());
    assert!(
        agent.why_not.starts_with("Not identified"),
        "{}",
        agent.why_not
    );
    assert!(
        agent
            .why_not
            .ends_with(" Something in it changed in the last 30 days."),
        "{}",
        agent.why_not
    );
    let pics = &r.areas[1];
    assert_eq!(
        (
            pics.category.as_str(),
            pics.verdict,
            pics.actions.as_slice()
        ),
        ("photos", Some(Verdict::Keep), [Action::Archive].as_slice())
    );
    assert!(!pics.unknown);
    assert_eq!(r.areas[2].category, "vm-disk");
    assert!(r.areas[3].why_not.starts_with("Model files: "));
    assert!(r.areas[4].why_not.starts_with("Page file: "));
    assert_eq!(r.areas[4].category, "system");
    assert!(
        r.areas[6]
            .why_not
            .contains("Hoardini never deletes anything here")
    );
    assert!(r.areas[7].why_not.starts_with("WinSxS component store: "));
    assert!(r.areas[9].unknown, "a pack file is shown as its project");
    assert!(r.areas.iter().all(|a| !a.only_copy));

    let stored = review::get(&db.conn().unwrap(), &r.id).unwrap().unwrap();
    assert_eq!(stored.areas, r.areas);
}

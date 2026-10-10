use std::path::Path;

use hoardini_core::index::{FakeFs, RefreshOptions, refresh};
use hoardini_core::memory::knowledge::{
    MODEL_WEIGHTS, filetype, filetypes, remember_filetype, source_rank, upsert_with,
    weight_extensions,
};
use hoardini_core::memory::location::{
    Answer, answer, large_folders, location_at, location_covering, refresh_volume, tag,
};
use hoardini_core::memory::{
    Connection, Detail, Kind, Knowledge, Memory, NewMemory, Scope, list, note_covering_with,
    protect_covering_with, recall_path_with, remember_with, set_limit,
};
use hoardini_core::policy::{Action, PolicyEnv, Verdict};
use hoardini_core::store::Db;
use serde_json::Value;

use super::P;

const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;
const DAY: i64 = 86_400_000;
const T0: i64 = 1_800_000_000_000;

struct Drive {
    _dir: tempfile::TempDir,
    db: Db,
    env: PolicyEnv,
    fs: FakeFs,
}

impl Drive {
    fn new() -> Drive {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("loc.db")).unwrap();
        let fs = FakeFs::new(r"C:\");
        fs.set_volume(2000 * GIB, 1000 * GIB);
        Drive {
            _dir: dir,
            db,
            env: PolicyEnv::with_paths(Path::new(P), Path::new(r"C:\ProgramData\hoardini-test")),
            fs,
        }
    }

    fn c(&self) -> Connection {
        self.db.conn().unwrap()
    }

    fn sweep(&self, now: i64) {
        let out = refresh(&self.c(), &self.fs, r"C:\", &RefreshOptions::default()).unwrap();
        assert!(out.completed);
        refresh_volume(&self.c(), &self.env, r"C:\", now).unwrap();
    }

    fn row(&self, path: &str) -> Option<Memory> {
        location_at(&self.c(), P, path).unwrap()
    }

    fn two_files(&self, dir: &str, gib_each: u64) {
        self.fs.file(&format!(r"{dir}\a.dat"), gib_each * GIB);
        self.fs.file(&format!(r"{dir}\b.dat"), gib_each * GIB);
    }
}

fn detail(m: &Memory) -> &Detail {
    m.detail.as_ref().unwrap()
}

// Traces: MEM-LOCATION
#[test]
fn large_folders_get_one_classified_row_each() {
    let d = Drive::new();
    let win = d.env.system_root.clone();
    d.fs.file(r"C:\Users\tester\Pictures\2023\a.jpg", 20 * GIB);
    d.fs.file(r"C:\Users\tester\Pictures\2023\b.mp4", 20 * GIB);
    d.fs.file(r"C:\Users\tester\Pictures\2024\c.heic", 25 * GIB);
    d.fs.file(r"C:\Users\tester\Pictures\2024\d.dat", 15 * GIB);
    d.fs.file(r"C:\code\covenant-agent\models\m.dat", 100 * GIB);
    d.fs.file(r"C:\code\covenant-agent\data\x.dat", 61 * GIB);
    d.fs.file(r"C:\code\other\o.dat", 6 * GIB);
    d.fs.file(
        r"C:\Users\tester\.lmstudio\models\lmstudio-community\q\q.gguf",
        20 * GIB,
    );
    d.fs.file(
        r"C:\Users\tester\.lmstudio\models\bartowski\l\l.gguf",
        10 * GIB,
    );
    d.fs.file(&format!(r"{win}\System32\big.dll"), 10 * GIB);
    d.fs.file(&format!(r"{win}\WinSxS\pack.dat"), 10 * GIB);
    d.two_files(r"C:\small", 1);
    d.sweep(T0);

    let pics = d.row(r"C:\Users\tester\Pictures").unwrap();
    assert_eq!(
        (pics.scope, pics.kind, pics.source.as_str()),
        (Scope::Path, Kind::Verdict, "mix")
    );
    assert_eq!(pics.key, r"~\Pictures");
    let pd = detail(&pics);
    assert_eq!(pd.category, "photos");
    assert_eq!(pd.verdict, Some(Verdict::Keep));
    assert_eq!(pd.actions, vec![Action::Archive]);
    assert_eq!(pd.size_at, Some(80 * GIB));
    assert_eq!(pd.assessed_ms, Some(T0));
    assert_eq!(pd.by.as_deref(), Some("mix"));
    assert_eq!(tag(&pics).as_deref(), Some("your photos and videos, keep"));

    let agent = d.row(r"C:\code\covenant-agent").unwrap();
    assert_eq!(agent.source, "index");
    assert_eq!(detail(&agent).category, "unknown");
    assert_eq!(detail(&agent).verdict, None);
    assert_eq!(tag(&agent).as_deref(), Some("unknown"));
    assert!(d.row(r"C:\code").is_none(), "96% sits in one child");

    let lm = d.row(r"C:\Users\tester\.lmstudio\models").unwrap();
    assert_eq!(lm.source, "catalog");
    assert_eq!(detail(&lm).by.as_deref(), Some("catalog:lmstudio-models"));
    assert_eq!(detail(&lm).category, "model-store");
    assert_eq!(detail(&lm).actions.len(), 4);
    assert_eq!(tag(&lm).as_deref(), Some("LM Studio models, redownload"));

    let w = d.row(&win).unwrap();
    assert_eq!(
        (w.source.as_str(), detail(&w).category.as_str()),
        ("deny", "system")
    );
    assert_eq!(detail(&w).verdict, Some(Verdict::Keep));
    assert!(detail(&w).actions.is_empty());

    let profile = d.row(P).unwrap();
    assert_eq!(detail(&profile).category, "container");
    assert_eq!(tag(&profile).as_deref(), Some("your profile folder"));

    assert!(d.row(r"C:\small").is_none());
    assert!(d.row(r"C:\").is_none());
    let inside = location_covering(&d.c(), P, r"C:\Users\tester\Pictures\notes\x.txt")
        .unwrap()
        .unwrap();
    assert_eq!(inside.id, pics.id);
}

// Traces: MEM-LOCATION
#[test]
fn at_most_forty_biggest_folders_per_drive() {
    let d = Drive::new();
    for i in 0..45u64 {
        d.fs.file(&format!(r"C:\many\f{i:02}\a.dat"), 3 * GIB);
        d.fs.file(&format!(r"C:\many\f{i:02}\b.dat"), 3 * GIB + i * MIB);
    }
    let out = refresh(&d.c(), &d.fs, r"C:\", &RefreshOptions::default()).unwrap();
    assert!(out.completed);
    let large = large_folders(&d.c(), &d.env, r"C:\").unwrap();
    assert_eq!(large.len(), 40);
    assert_eq!(large[0].path, r"C:\many");
    let names: Vec<&str> = large.iter().map(|l| l.path.as_str()).collect();
    assert!(names.contains(&r"C:\many\f44") && names.contains(&r"C:\many\f06"));
    assert!(!names.contains(&r"C:\many\f05") && !names.contains(&r"C:\many\f00"));
}

// Traces: MEM-LOCATION
#[test]
fn folders_inside_a_catalog_unit_take_no_slot_and_the_unit_counts_whole() {
    let d = Drive::new();
    let temp = r"C:\Users\tester\AppData\Local\Temp";
    for i in 0..10u64 {
        d.two_files(&format!(r"{temp}\claude\c--code-p{i}"), 3);
    }
    d.two_files(r"C:\Users\tester\.lmstudio\models\org\big", 10);
    d.two_files(r"C:\data\set", 3);
    d.sweep(T0);
    let large = large_folders(&d.c(), &d.env, r"C:\").unwrap();
    let names: Vec<&str> = large.iter().map(|l| l.path.as_str()).collect();
    assert!(names.contains(&temp), "{names:?}");
    assert!(
        !names.iter().any(|n| n.starts_with(&format!(r"{temp}\"))),
        "{names:?}"
    );
    assert!(names.contains(&r"C:\Users\tester\.lmstudio\models"));
    assert!(!names.contains(&r"C:\Users\tester\.lmstudio\models\org\big"));
    assert!(names.contains(&r"C:\data\set"));
    let row = d.row(temp).unwrap();
    assert_eq!(detail(&row).by.as_deref(), Some("catalog:temp-user"));
}

// Traces: MEM-LOCATION
#[test]
fn rows_are_reassessed_on_a_quarter_size_move_or_after_ninety_days() {
    let d = Drive::new();
    d.two_files(r"C:\data\set", 4);
    d.two_files(r"C:\data\mine", 4);
    d.sweep(T0);
    let user = answer(&d.c(), &d.env, r"C:\data\mine", Answer::Keep, T0).unwrap();

    d.fs.resize(
        r"C:\data\set\a.dat",
        4 * GIB + 800 * MIB,
        4 * GIB + 800 * MIB,
    );
    d.fs.resize(r"C:\data\mine\a.dat", 8 * GIB, 8 * GIB);
    d.sweep(T0 + DAY);
    assert_eq!(
        detail(&d.row(r"C:\data\set").unwrap()).assessed_ms,
        Some(T0)
    );

    d.fs.resize(r"C:\data\set\a.dat", 7 * GIB, 7 * GIB);
    d.sweep(T0 + 2 * DAY);
    let set = d.row(r"C:\data\set").unwrap();
    assert_eq!(detail(&set).assessed_ms, Some(T0 + 2 * DAY));
    assert_eq!(detail(&set).size_at, Some(11 * GIB));

    d.sweep(T0 + 2 * DAY + 90 * DAY);
    assert_eq!(
        detail(&d.row(r"C:\data\set").unwrap()).assessed_ms,
        Some(T0 + 92 * DAY)
    );
    assert_eq!(d.row(r"C:\data\mine").unwrap(), user);
}

// Traces: MEM-LOCATION
#[test]
fn rows_go_when_the_folder_is_no_longer_large_unless_someone_wrote_to_them() {
    let d = Drive::new();
    for dir in ["shrinks", "job", "mine"] {
        d.two_files(&format!(r"C:\data\{dir}"), 3);
    }
    d.sweep(T0);
    let job = d.row(r"C:\data\job").unwrap();
    d.c()
        .execute(
            "UPDATE memory SET detail_json = json_set(detail_json, '$.kept', json('[{\"path\":\"C:\\\\data\\\\job\\\\a.dat\"}]')) WHERE id = ?1",
            [job.id],
        )
        .unwrap();
    answer(&d.c(), &d.env, r"C:\data\mine", Answer::Disposable, T0).unwrap();
    for dir in ["shrinks", "job", "mine"] {
        for f in ["a", "b"] {
            d.fs.resize(&format!(r"C:\data\{dir}\{f}.dat"), GIB, GIB);
        }
    }
    d.sweep(T0 + DAY);
    assert!(d.row(r"C:\data\shrinks").is_none());
    assert_eq!(detail(&d.row(r"C:\data\job").unwrap()).kept.len(), 1);
    let mine = d.row(r"C:\data\mine").unwrap();
    assert_eq!(detail(&mine).verdict, Some(Verdict::Safe));
}

// Traces: MEM-LOCATION
#[test]
fn what_is_this_answers_are_user_rows_and_never_touch_protects() {
    let d = Drive::new();
    d.two_files(r"C:\code\covenant-agent", 80);
    d.sweep(T0);
    let c = d.c();
    let cases = [
        (
            Answer::Keep,
            Verdict::Keep,
            vec![Action::Archive, Action::Compress],
        ),
        (Answer::Disposable, Verdict::Safe, Action::ALL.to_vec()),
        (Answer::MoveElsewhere, Verdict::Keep, vec![Action::Archive]),
        (Answer::NeverTouch, Verdict::Keep, vec![]),
    ];
    for (a, verdict, actions) in cases {
        let m = answer(&c, &d.env, r"C:\code\covenant-agent", a, T0 + DAY).unwrap();
        assert_eq!((m.source.as_str(), m.pinned), ("user", true));
        assert_eq!(detail(&m).verdict, Some(verdict), "{a:?}");
        assert_eq!(detail(&m).actions, actions, "{a:?}");
        assert_eq!(detail(&m).size_at, Some(160 * GIB));
    }
    let p = protect_covering_with(&c, r"C:\code\covenant-agent\x.bin", P)
        .unwrap()
        .unwrap();
    assert_eq!(p.source, "user");
    assert_eq!(
        list(&c)
            .unwrap()
            .iter()
            .filter(|m| m.detail.is_some())
            .count(),
        1,
        "one row per location"
    );
}

fn knowledge(key: &str, source: &str, text: &str, detail: Detail) -> Knowledge {
    Knowledge {
        scope: Scope::Path,
        key: key.into(),
        kind: Kind::Verdict,
        text: text.into(),
        source: source.into(),
        detail,
    }
}

fn verdict(v: Verdict) -> Detail {
    Detail {
        category: "test".into(),
        verdict: Some(v),
        actions: vec![Action::Delete],
        ..Detail::default()
    }
}

// Traces: MEM-SOURCE-RANK
#[test]
fn a_lower_source_never_overwrites_a_higher_one() {
    let (_d, c) = super::conn();
    let order = [
        "index",
        "agent",
        "web:https://x",
        "mix",
        "catalog",
        "job:7",
        "user",
    ];
    for w in order.windows(2) {
        assert!(source_rank(w[0]) < source_rank(w[1]), "{w:?}");
    }
    assert_eq!(source_rank("deny"), source_rank("catalog"));

    let put = |src: &str, text: &str| {
        upsert_with(
            &c,
            knowledge(r"D:\x", src, text, verdict(Verdict::Review)),
            1_000,
            P,
        )
        .unwrap()
    };
    let mut last = 0;
    for src in ["agent", "web:https://a", "mix", "catalog", "job:3", "user"] {
        let (m, written) = put(src, src);
        assert!(written, "{src}");
        assert_eq!(m.text, src);
        if last != 0 {
            assert_eq!(m.id, last, "one row per key");
        }
        last = m.id;
    }
    for src in [
        "job:4",
        "catalog",
        "deny",
        "mix",
        "web:https://b",
        "agent",
        "index",
    ] {
        let (m, written) = put(src, "lower");
        assert!(!written, "{src}");
        assert_eq!((m.text.as_str(), m.source.as_str()), ("user", "user"));
    }

    upsert_with(
        &c,
        knowledge(
            r"D:\y",
            "web:https://a",
            "hipfire's cache",
            Detail::default(),
        ),
        1_000,
        P,
    )
    .unwrap();
    let (m, written) = upsert_with(
        &c,
        knowledge(r"D:\y", "index", "unknown", Detail::default()),
        2_000,
        P,
    )
    .unwrap();
    assert!(!written);
    assert_eq!(m.text, "hipfire's cache");
    let (m, written) = upsert_with(
        &c,
        knowledge(r"D:\z", "catalog", "a cache", verdict(Verdict::Safe)),
        1_000,
        P,
    )
    .unwrap();
    assert!(written);
    let (n, written) = upsert_with(
        &c,
        knowledge(r"D:\z", "mix", "now photos", verdict(Verdict::Keep)),
        2_000,
        P,
    )
    .unwrap();
    assert!(written, "the classifier re-decides its own rows");
    assert_eq!((n.id, n.source.as_str()), (m.id, "mix"));
}

// Traces: MEM-WEB-IDENTITY
#[test]
fn web_rows_keep_identity_and_drop_any_verdict_or_actions() {
    let (_d, c) = super::conn();
    let (m, _) = upsert_with(
        &c,
        knowledge(
            r"~\.hipfire",
            "web:https://example.com/hipfire",
            "~\\.hipfire: hipfire's model cache",
            Detail {
                category: "model-store".into(),
                app: Some("hipfire".into()),
                use_: Some("model cache".into()),
                verdict: Some(Verdict::Safe),
                actions: vec![Action::Delete],
                ..Detail::default()
            },
        ),
        1_000,
        P,
    )
    .unwrap();
    assert_eq!(m.kind, Kind::Fact);
    let d = detail(&m);
    assert_eq!((d.verdict, d.actions.len()), (None, 0));
    assert_eq!(d.app.as_deref(), Some("hipfire"));
    let raw: String = c
        .query_row(
            "SELECT detail_json FROM memory WHERE id = ?1",
            [m.id],
            |r| r.get(0),
        )
        .unwrap();
    let json: Value = serde_json::from_str(&raw).unwrap();
    assert!(
        json.get("verdict").is_none() && json.get("actions").is_none(),
        "{raw}"
    );
    assert_eq!(json["use"], "model cache");

    let (t, _) = remember_filetype(
        &c,
        "q4nx",
        ".q4nx: FastFlowLM NPU model weights.",
        "web:https://huggingface.co/FastFlowLM",
        Detail {
            category: MODEL_WEIGHTS.into(),
            verdict: Some(Verdict::Redownload),
            ..Detail::default()
        },
        1_000,
    )
    .unwrap();
    assert_eq!(detail(&t).verdict, None);
}

// Traces: MEM-FILETYPE
#[test]
fn file_type_rows_are_one_glob_per_extension_and_reach_item_notes() {
    let (_d, c) = super::conn();
    let (m, _) = remember_filetype(
        &c,
        ".Q4NX",
        ".q4nx: FastFlowLM NPU model weights.",
        "web:https://huggingface.co/FastFlowLM",
        Detail {
            category: MODEL_WEIGHTS.into(),
            app: Some("FastFlowLM".into()),
            use_: Some("NPU model weights".into()),
            copy_hint: Some("model-hub".into()),
            ..Detail::default()
        },
        1_000,
    )
    .unwrap();
    assert_eq!(
        (m.scope, m.kind, m.key.as_str()),
        (Scope::Glob, Kind::Fact, r"**\*.q4nx")
    );
    assert_eq!(detail(&m).copy_hint.as_deref(), Some("model-hub"));
    assert_eq!(filetype(&c, "q4nx").unwrap().unwrap().id, m.id);

    let (u, written) = remember_filetype(
        &c,
        "q4nx",
        "My NPU models.",
        "user",
        Detail {
            category: MODEL_WEIGHTS.into(),
            ..Detail::default()
        },
        2_000,
    )
    .unwrap();
    assert!(written);
    assert_eq!(u.id, m.id);
    assert_eq!(filetypes(&c).unwrap().len(), 1);
    assert!(weight_extensions(&c).unwrap().contains(&"q4nx".to_string()));

    let file = r"D:\npu\Qwen3.8-27B-NPU2\model.q4nx";
    let recalled = recall_path_with(&c, file, P).unwrap();
    assert_eq!(
        recalled.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![m.id]
    );
    assert_eq!(
        note_covering_with(&c, file, P).unwrap().unwrap().text,
        "My NPU models."
    );
    assert!(remember_filetype(&c, "a/b", "x", "user", Detail::default(), 1).is_err());
    assert!(remember_filetype(&c, " ", "x", "user", Detail::default(), 1).is_err());
}

// Traces: MEM-LIMIT
#[test]
fn location_and_type_rows_are_outside_the_note_limit() {
    let (_d, c) = super::conn();
    set_limit(&c, 10).unwrap();
    for (i, key) in [r"D:\a", r"D:\b", r"D:\c"].iter().enumerate() {
        upsert_with(
            &c,
            knowledge(key, "mix", "loc", verdict(Verdict::Review)),
            i as i64,
            P,
        )
        .unwrap();
    }
    remember_filetype(&c, "q4nx", "weights", "agent", Detail::default(), 0).unwrap();
    for i in 1..=12 {
        remember_with(
            &c,
            NewMemory::new(
                Scope::Path,
                &format!(r"D:\n{i}"),
                Kind::Fact,
                "note",
                "agent",
            ),
            100 + i,
            P,
        )
        .unwrap();
    }
    let rows = list(&c).unwrap();
    let live_notes = rows
        .iter()
        .filter(|m| m.detail.is_none() && m.forgotten_ms.is_none())
        .count();
    assert_eq!(live_notes, 10);
    assert!(
        rows.iter()
            .filter(|m| m.detail.is_some())
            .all(|m| m.forgotten_ms.is_none())
    );
    assert_eq!(rows.iter().filter(|m| m.detail.is_some()).count(), 4);
}

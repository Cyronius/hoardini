use hoardini_core::memory::{
    Connection, Kind, Memory, MemoryPatch, NewMemory, Scope, get, globals, globals_for, limit,
    list, protect_covering_with, recall_path_with, recall_text, remember_with, restore, search,
    set_limit, update_with,
};
use hoardini_core::store::{Db, now_ms};

const P: &str = r"C:\Users\tester";

fn conn() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("mem.db")).unwrap();
    let c = db.conn().unwrap();
    (dir, c)
}

fn add(c: &Connection, scope: Scope, key: &str, kind: Kind, text: &str) -> Memory {
    remember_with(c, NewMemory::new(scope, key, kind, text, "user"), 1_000, P).unwrap()
}

fn texts(ms: &[Memory]) -> Vec<&str> {
    ms.iter().map(|m| m.text.as_str()).collect()
}

// Traces: MEM-RECALL-ANCESTRY
#[test]
fn recall_returns_exact_ancestor_and_glob_rows_most_specific_first() {
    let (_d, c) = conn();
    add(
        &c,
        Scope::Path,
        r"C:\Users\tester\.lmstudio",
        Kind::Fact,
        "ancestor",
    );
    add(
        &c,
        Scope::Path,
        r"C:\Users\tester\.lmstudio\models\big.gguf",
        Kind::Fact,
        "exact",
    );
    add(
        &c,
        Scope::Path,
        r"C:\Users\tester\.lmstudio\models",
        Kind::Preference,
        "parent",
    );
    add(
        &c,
        Scope::Glob,
        r"~\.lmstudio\models\**",
        Kind::Verdict,
        "glob",
    );
    add(
        &c,
        Scope::Path,
        r"C:\Users\tester\.lmstudio\models\other.gguf",
        Kind::Fact,
        "sibling",
    );
    add(
        &c,
        Scope::Path,
        r"C:\Users\tester\.lmstudio\models\big.gguf.bak",
        Kind::Fact,
        "prefix-lookalike",
    );

    let got = recall_path_with(&c, r"C:\Users\tester\.lmstudio\models\big.gguf", P).unwrap();
    assert_eq!(texts(&got), ["exact", "parent", "ancestor", "glob"]);
}

// Traces: MEM-RECALL-ANCESTRY
#[test]
fn recall_ignores_case_and_slash_style_and_profile_prefix() {
    let (_d, c) = conn();
    add(&c, Scope::Path, "~/Projects/Keep", Kind::Protect, "p");
    let got = recall_path_with(&c, r"c:\USERS\Tester\projects\keep\src\main.rs", P).unwrap();
    assert_eq!(texts(&got), ["p"]);
    assert_eq!(got[0].key, r"~\Projects\Keep");
}

// Traces: MEM-RECALL-ANCESTRY
#[test]
fn recall_matches_globs_for_any_dir_named_x() {
    let (_d, c) = conn();
    add(&c, Scope::Glob, r"**\node_modules", Kind::Fact, "nm");
    add(&c, Scope::Glob, r"D:\data\*.parquet", Kind::Fact, "pq");
    assert_eq!(
        texts(&recall_path_with(&c, r"E:\code\web\node_modules\left-pad\index.js", P).unwrap()),
        ["nm"]
    );
    assert_eq!(
        texts(&recall_path_with(&c, r"D:\data\a.parquet", P).unwrap()),
        ["pq"]
    );
    assert!(
        recall_path_with(&c, r"D:\data\sub\a.parquet", P)
            .unwrap()
            .is_empty()
    );
    assert!(recall_path_with(&c, r"D:\other", P).unwrap().is_empty());
}

// Traces: MEM-RECALL-ANCESTRY
#[test]
fn protect_covering_finds_path_ancestor_and_glob_protects_only() {
    let (_d, c) = conn();
    add(&c, Scope::Path, r"D:\keep", Kind::Protect, "dir protect");
    add(&c, Scope::Path, r"D:\notes", Kind::Fact, "just a fact");
    add(
        &c,
        Scope::Glob,
        r"**\*.kdbx",
        Kind::Protect,
        "password vaults",
    );

    let under = protect_covering_with(&c, r"D:\keep\a\b.bin", P)
        .unwrap()
        .unwrap();
    assert_eq!(under.text, "dir protect");
    let glob = protect_covering_with(&c, r"E:\x\vault.kdbx", P)
        .unwrap()
        .unwrap();
    assert_eq!(glob.text, "password vaults");
    assert!(
        protect_covering_with(&c, r"D:\notes\a.txt", P)
            .unwrap()
            .is_none()
    );
    assert!(
        protect_covering_with(&c, r"D:\keeper\a.txt", P)
            .unwrap()
            .is_none()
    );
}

const DAY: i64 = 24 * 60 * 60 * 1000;

fn base() -> i64 {
    now_ms() - 1_000_000
}

fn add_at(
    c: &Connection,
    scope: Scope,
    key: &str,
    kind: Kind,
    text: &str,
    src: &str,
    at: i64,
) -> Memory {
    remember_with(c, NewMemory::new(scope, key, kind, text, src), at, P).unwrap()
}

fn used(c: &Connection, id: i64) -> i64 {
    get(c, id).unwrap().unwrap().last_used_ms.unwrap()
}

fn counted(c: &Connection) -> usize {
    list(c)
        .unwrap()
        .into_iter()
        .filter(|m| m.forgotten_ms.is_none() && !m.pinned && m.kind != Kind::Protect)
        .count()
}

// Traces: MEM-USAGE
#[test]
fn reads_mark_only_the_notes_they_return_as_used() {
    let (_d, c) = conn();
    let t = base();
    let hit = add_at(
        &c,
        Scope::Path,
        r"D:\keep",
        Kind::Protect,
        "keep this",
        "agent",
        t,
    );
    let fact = add_at(
        &c,
        Scope::Path,
        r"D:\keep",
        Kind::Fact,
        "a fact",
        "agent",
        t,
    );
    let other = add_at(
        &c,
        Scope::Path,
        r"D:\other",
        Kind::Fact,
        "elsewhere",
        "agent",
        t,
    );

    protect_covering_with(&c, r"D:\keep\a.bin", P)
        .unwrap()
        .unwrap();
    assert!(used(&c, hit.id) > t);
    assert_eq!(used(&c, fact.id), t);

    recall_path_with(&c, r"D:\keep\a.bin", P).unwrap();
    assert!(used(&c, fact.id) > t);
    assert_eq!(used(&c, other.id), t);
}

// Traces: MEM-USAGE
#[test]
fn text_recall_counts_as_use_but_drawer_search_does_not() {
    let (_d, c) = conn();
    let t = base();
    let n = add_at(
        &c,
        Scope::Path,
        r"D:\scratch",
        Kind::Fact,
        "disposable checkpoints",
        "agent",
        t,
    );
    assert_eq!(search(&c, "checkpoints").unwrap().len(), 1);
    assert_eq!(used(&c, n.id), t);
    assert_eq!(recall_text(&c, "checkpoints").unwrap().len(), 1);
    assert!(used(&c, n.id) > t);
}

// Traces: MEM-USAGE
#[test]
fn globals_matching_the_users_message_are_marked_used() {
    let (_d, c) = conn();
    let t = base();
    let steam = add_at(
        &c,
        Scope::Global,
        "steam",
        Kind::Preference,
        "never touch the steam library",
        "agent",
        t,
    );
    let terse = add_at(
        &c,
        Scope::Global,
        "tone",
        Kind::Preference,
        "be terse",
        "agent",
        t,
    );
    globals_for(&c, 30, "what is in my steam folder?").unwrap();
    assert!(used(&c, steam.id) > t);
    assert_eq!(used(&c, terse.id), t);
}

// Traces: MEM-USAGE
#[test]
fn editing_a_note_marks_it_used() {
    let (_d, c) = conn();
    let n = add_at(&c, Scope::Global, "k", Kind::Fact, "old", "agent", 1_000);
    let patch = MemoryPatch {
        text: Some("new".into()),
        ..MemoryPatch::default()
    };
    update_with(&c, n.id, patch, 5_000, P).unwrap();
    assert_eq!(used(&c, n.id), 5_000);
}

// Traces: MEM-LIMIT
#[test]
fn going_over_the_limit_forgets_the_oldest_used_unpinned_notes() {
    let (_d, c) = conn();
    let t = base();
    let old_pinned = add_at(
        &c,
        Scope::Global,
        "p",
        Kind::Preference,
        "pinned old",
        "user",
        t - 100,
    );
    let old_protect = add_at(
        &c,
        Scope::Path,
        r"D:\p",
        Kind::Protect,
        "protect old",
        "agent",
        t - 99,
    );
    let ids: Vec<i64> = (1..=12)
        .map(|i| {
            add_at(
                &c,
                Scope::Global,
                &format!("n{i}"),
                Kind::Fact,
                &format!("note {i}"),
                "agent",
                t + i,
            )
            .id
        })
        .collect();
    assert_eq!(counted(&c), 12);
    assert_eq!(limit(&c), 200);
    set_limit(&c, 10).unwrap();
    assert_eq!(counted(&c), 10);
    for (i, id) in ids.iter().enumerate() {
        let gone = get(&c, *id).unwrap().unwrap().forgotten_ms.is_some();
        assert_eq!(gone, i < 2, "note {}", i + 1);
    }
    add_at(
        &c,
        Scope::Global,
        "n13",
        Kind::Fact,
        "note 13",
        "agent",
        t + 13,
    );
    assert_eq!(counted(&c), 10);
    assert!(get(&c, ids[2]).unwrap().unwrap().forgotten_ms.is_some());
    assert!(
        get(&c, old_pinned.id)
            .unwrap()
            .unwrap()
            .forgotten_ms
            .is_none()
    );
    assert!(
        get(&c, old_protect.id)
            .unwrap()
            .unwrap()
            .forgotten_ms
            .is_none()
    );
    assert_eq!(set_limit(&c, 1).unwrap(), 10);
    assert_eq!(limit(&c), 10);
}

// Traces: MEM-LIMIT
#[test]
fn forgotten_notes_never_reach_the_model_and_restore_brings_them_back() {
    let (_d, c) = conn();
    let t = base();
    let path = add_at(
        &c,
        Scope::Path,
        r"D:\x",
        Kind::Fact,
        "path note",
        "agent",
        t,
    );
    let glob = add_at(
        &c,
        Scope::Global,
        "g",
        Kind::Fact,
        "global note",
        "agent",
        t,
    );
    for i in 0..10 {
        add_at(
            &c,
            Scope::Global,
            &format!("f{i}"),
            Kind::Fact,
            &format!("filler {i}"),
            "agent",
            t + 10 + i,
        );
    }
    set_limit(&c, 10).unwrap();
    assert!(get(&c, path.id).unwrap().unwrap().forgotten_ms.is_some());
    assert!(get(&c, glob.id).unwrap().unwrap().forgotten_ms.is_some());
    assert!(recall_path_with(&c, r"D:\x\y", P).unwrap().is_empty());
    assert!(!globals(&c, 30).unwrap().iter().any(|m| m.id == glob.id));
    assert!(recall_text(&c, "path").unwrap().is_empty());
    assert_eq!(search(&c, "path").unwrap().len(), 1);
    assert!(list(&c).unwrap().last().unwrap().forgotten_ms.is_some());

    let back = restore(&c, path.id).unwrap().unwrap();
    assert_eq!(back.forgotten_ms, None);
    assert!(back.last_used_ms.unwrap() > t);
    assert_eq!(
        texts(&recall_path_with(&c, r"D:\x\y", P).unwrap()),
        ["path note"]
    );
}

// Traces: MEM-LIMIT
#[test]
fn forgotten_notes_are_purged_after_ninety_days() {
    let (_d, c) = conn();
    let a = add_at(&c, Scope::Global, "a", Kind::Fact, "stale", "agent", 1_000);
    let b = add_at(&c, Scope::Global, "b", Kind::Fact, "recent", "agent", 1_000);
    let now = now_ms();
    c.execute(
        "UPDATE memory SET forgotten_ms = ?1 WHERE id = ?2",
        [now - 91 * DAY, a.id],
    )
    .unwrap();
    c.execute(
        "UPDATE memory SET forgotten_ms = ?1 WHERE id = ?2",
        [now - 89 * DAY, b.id],
    )
    .unwrap();
    let ids: Vec<i64> = list(&c).unwrap().iter().map(|m| m.id).collect();
    assert_eq!(ids, [b.id]);
}

// Traces: MEM-LIMIT
#[test]
fn globals_rank_protect_then_pinned_then_last_used() {
    let (_d, c) = conn();
    let t = base();
    add_at(
        &c,
        Scope::Global,
        "recent",
        Kind::Preference,
        "recent unpinned",
        "agent",
        t + 50,
    );
    add_at(
        &c,
        Scope::Global,
        "old-pin",
        Kind::Preference,
        "old pinned",
        "user",
        t,
    );
    add_at(
        &c,
        Scope::Global,
        "older",
        Kind::Preference,
        "older unpinned",
        "agent",
        t + 10,
    );
    add_at(
        &c,
        Scope::Global,
        "prot",
        Kind::Protect,
        "oldest protect",
        "agent",
        t - 5,
    );
    assert_eq!(
        texts(&globals(&c, 30).unwrap()),
        [
            "oldest protect",
            "old pinned",
            "recent unpinned",
            "older unpinned"
        ]
    );
}

// Traces: MEM-PIN-USER
#[test]
fn user_notes_and_user_asked_notes_start_pinned() {
    let (_d, c) = conn();
    let user = add_at(&c, Scope::Global, "u", Kind::Fact, "typed", "user", 1_000);
    assert!(user.pinned);
    let agent = add_at(
        &c,
        Scope::Global,
        "a",
        Kind::Fact,
        "guessed",
        "agent",
        1_000,
    );
    assert!(!agent.pinned);

    let mut asked = NewMemory::new(Scope::Global, "b", Kind::Fact, "asked", "agent");
    asked.pinned = true;
    assert!(remember_with(&c, asked, 1_000, P).unwrap().pinned);

    let mut again = NewMemory::new(Scope::Global, "a", Kind::Fact, "guessed", "agent");
    again.pinned = true;
    assert!(remember_with(&c, again, 2_000, P).unwrap().pinned);
    let plain = NewMemory::new(Scope::Global, "a", Kind::Fact, "guessed", "agent");
    assert!(remember_with(&c, plain, 3_000, P).unwrap().pinned);

    let off = MemoryPatch {
        pinned: Some(false),
        ..MemoryPatch::default()
    };
    assert!(
        !update_with(&c, user.id, off, 4_000, P)
            .unwrap()
            .unwrap()
            .pinned
    );
}

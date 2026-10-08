use hoardini_core::memory::{
    Connection, Kind, Memory, NewMemory, Scope, protect_covering_with, recall_path_with,
    remember_with,
};
use hoardini_core::store::Db;

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

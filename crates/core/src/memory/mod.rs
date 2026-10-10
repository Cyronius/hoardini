pub mod knowledge;
pub mod learn;
pub mod location;

use globset::{GlobBuilder, GlobMatcher};
pub use rusqlite::Connection;
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::paths;
use crate::store::{get_meta, now_ms, set_meta};

pub use knowledge::{Detail, Knowledge};

pub const USAGE_REVIEW_MS: i64 = 90 * 24 * 60 * 60 * 1000;
pub const FORGOTTEN_PURGE_MS: i64 = 90 * 24 * 60 * 60 * 1000;
pub const DEFAULT_LIMIT: usize = 200;
pub const MIN_LIMIT: usize = 10;
const LIMIT_KEY: &str = "memory_limit";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Path,
    Glob,
    Global,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Scope::Path => "path",
            Scope::Glob => "glob",
            Scope::Global => "global",
        }
    }

    fn parse(s: &str) -> Option<Scope> {
        match s {
            "path" => Some(Scope::Path),
            "glob" => Some(Scope::Glob),
            "global" => Some(Scope::Global),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Fact,
    Preference,
    Protect,
    Verdict,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Fact => "fact",
            Kind::Preference => "preference",
            Kind::Protect => "protect",
            Kind::Verdict => "verdict",
        }
    }

    fn parse(s: &str) -> Option<Kind> {
        match s {
            "fact" => Some(Kind::Fact),
            "preference" => Some(Kind::Preference),
            "protect" => Some(Kind::Protect),
            "verdict" => Some(Kind::Verdict),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub scope: Scope,
    pub key: String,
    pub kind: Kind,
    pub text: String,
    pub source: String,
    pub confidence: Option<f64>,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub review_after_ms: Option<i64>,
    pub pinned: bool,
    pub last_used_ms: Option<i64>,
    pub forgotten_ms: Option<i64>,
    pub detail: Option<Detail>,
}

impl Memory {
    pub fn is_stale(&self, now_ms: i64) -> bool {
        self.review_after_ms.is_some_and(|t| t <= now_ms)
    }
}

#[derive(Clone, Debug)]
pub struct NewMemory {
    pub scope: Scope,
    pub key: String,
    pub kind: Kind,
    pub text: String,
    pub source: String,
    pub confidence: Option<f64>,
    pub review_after_ms: Option<i64>,
    pub pinned: bool,
}

impl NewMemory {
    pub fn new(scope: Scope, key: &str, kind: Kind, text: &str, source: &str) -> NewMemory {
        NewMemory {
            scope,
            key: key.to_string(),
            kind,
            text: text.to_string(),
            source: source.to_string(),
            confidence: None,
            review_after_ms: None,
            pinned: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MemoryPatch {
    pub text: Option<String>,
    pub kind: Option<Kind>,
    pub key: Option<String>,
    pub scope: Option<Scope>,
    pub source: Option<String>,
    pub confidence: Option<Option<f64>>,
    pub review_after_ms: Option<Option<i64>>,
    pub pinned: Option<bool>,
}

const COLS: &str = "id, scope, key, kind, text, source, confidence, created_ms, updated_ms, review_after_ms, pinned, last_used_ms, forgotten_ms, detail_json";
const M_COLS: &str = "m.id, m.scope, m.key, m.kind, m.text, m.source, m.confidence, m.created_ms, m.updated_ms, m.review_after_ms, m.pinned, m.last_used_ms, m.forgotten_ms, m.detail_json";

fn from_row(r: &Row) -> rusqlite::Result<Memory> {
    let bad = |i: usize| {
        rusqlite::Error::InvalidColumnType(i, "enum".into(), rusqlite::types::Type::Text)
    };
    let scope: String = r.get(1)?;
    let kind: String = r.get(3)?;
    Ok(Memory {
        id: r.get(0)?,
        scope: Scope::parse(&scope).ok_or_else(|| bad(1))?,
        key: r.get(2)?,
        kind: Kind::parse(&kind).ok_or_else(|| bad(3))?,
        text: r.get(4)?,
        source: r.get(5)?,
        confidence: r.get(6)?,
        created_ms: r.get(7)?,
        updated_ms: r.get(8)?,
        review_after_ms: r.get(9)?,
        pinned: r.get(10)?,
        last_used_ms: r.get(11)?,
        forgotten_ms: r.get(12)?,
        detail: r
            .get::<_, Option<String>>(13)?
            .and_then(|j| serde_json::from_str(&j).ok()),
    })
}

// Stored keys use `\`, no trailing slash, and `~` for the profile so they survive a user rename.
pub fn normalize_key_with(scope: Scope, key: &str, profile: &str) -> String {
    let key = key.trim();
    if scope == Scope::Global {
        return key.to_string();
    }
    let slashed = key.replace('/', "\\");
    let rel = paths::profile_relative_with(&slashed, profile);
    let trimmed = rel.trim_end_matches('\\');
    if trimmed.is_empty() {
        rel
    } else {
        trimmed.to_string()
    }
}

pub fn normalize_key(scope: Scope, key: &str) -> String {
    normalize_key_with(scope, key, &paths::profile_dir().to_string_lossy())
}

pub fn remember(conn: &Connection, new: NewMemory) -> anyhow::Result<Memory> {
    remember_with(conn, new, now_ms(), &paths::profile_dir().to_string_lossy())
}

pub fn remember_with(
    conn: &Connection,
    new: NewMemory,
    now: i64,
    profile: &str,
) -> anyhow::Result<Memory> {
    let key = normalize_key_with(new.scope, &new.key, profile);
    if key.is_empty() || new.text.trim().is_empty() {
        anyhow::bail!("memory needs a key and text");
    }
    if new.scope == Scope::Glob {
        compile_glob(&key)?;
    }
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM memory WHERE scope = ?1 AND key = ?2 COLLATE NOCASE AND kind = ?3 AND text = ?4",
            params![new.scope.as_str(), key, new.kind.as_str(), new.text],
            |r| r.get(0),
        )
        .optional()?;
    let pinned = new.pinned || new.source == "user";
    if let Some(id) = existing {
        conn.execute(
            "UPDATE memory SET updated_ms = ?1, last_used_ms = ?1, forgotten_ms = NULL, pinned = pinned OR ?2 WHERE id = ?3",
            params![now, pinned, id],
        )?;
        enforce_limit(conn, now)?;
        return get(conn, id)?.ok_or_else(|| anyhow::anyhow!("memory vanished"));
    }
    let review_after = new.review_after_ms.or_else(|| {
        (new.kind == Kind::Fact && new.source != "catalog").then_some(now + USAGE_REVIEW_MS)
    });
    conn.execute(
        "INSERT INTO memory(scope, key, kind, text, source, confidence, created_ms, updated_ms, review_after_ms, pinned, last_used_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8, ?9, ?7)",
        params![
            new.scope.as_str(),
            key,
            new.kind.as_str(),
            new.text,
            new.source,
            new.confidence,
            now,
            review_after,
            pinned
        ],
    )?;
    let id = conn.last_insert_rowid();
    enforce_limit(conn, now)?;
    get(conn, id)?.ok_or_else(|| anyhow::anyhow!("memory vanished"))
}

pub fn get(conn: &Connection, id: i64) -> anyhow::Result<Option<Memory>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM memory WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?)
}

pub fn list(conn: &Connection) -> anyhow::Result<Vec<Memory>> {
    purge_forgotten(conn, now_ms())?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM memory
         ORDER BY forgotten_ms IS NOT NULL, forgotten_ms DESC, last_used_ms DESC, id DESC"
    ))?;
    let rows = stmt
        .query_map([], from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn update(conn: &Connection, id: i64, patch: MemoryPatch) -> anyhow::Result<Option<Memory>> {
    update_with(
        conn,
        id,
        patch,
        now_ms(),
        &paths::profile_dir().to_string_lossy(),
    )
}

pub fn update_with(
    conn: &Connection,
    id: i64,
    patch: MemoryPatch,
    now: i64,
    profile: &str,
) -> anyhow::Result<Option<Memory>> {
    let Some(mut m) = get(conn, id)? else {
        return Ok(None);
    };
    if let Some(t) = patch.text {
        m.text = t;
    }
    if let Some(k) = patch.kind {
        m.kind = k;
    }
    if let Some(s) = patch.scope {
        m.scope = s;
    }
    if let Some(k) = patch.key {
        m.key = k;
    }
    if let Some(s) = patch.source {
        m.source = s;
    }
    if let Some(c) = patch.confidence {
        m.confidence = c;
    }
    if let Some(r) = patch.review_after_ms {
        m.review_after_ms = r;
    }
    if let Some(p) = patch.pinned {
        m.pinned = p;
    }
    m.key = normalize_key_with(m.scope, &m.key, profile);
    if m.key.is_empty() || m.text.trim().is_empty() {
        anyhow::bail!("memory needs a key and text");
    }
    if m.scope == Scope::Glob {
        compile_glob(&m.key)?;
    }
    conn.execute(
        "UPDATE memory SET scope=?1, key=?2, kind=?3, text=?4, source=?5, confidence=?6, review_after_ms=?7, updated_ms=?8, last_used_ms=?8, pinned=?9 WHERE id=?10",
        params![
            m.scope.as_str(),
            m.key,
            m.kind.as_str(),
            m.text,
            m.source,
            m.confidence,
            m.review_after_ms,
            now,
            m.pinned,
            id
        ],
    )?;
    enforce_limit(conn, now)?;
    get(conn, id)
}

pub fn delete(conn: &Connection, id: i64) -> anyhow::Result<bool> {
    Ok(conn.execute("DELETE FROM memory WHERE id = ?1", [id])? > 0)
}

pub fn compile_glob(key: &str) -> anyhow::Result<GlobMatcher> {
    Ok(GlobBuilder::new(&key.replace('\\', "/"))
        .case_insensitive(true)
        .literal_separator(true)
        .backslash_escape(false)
        .build()?
        .compile_matcher())
}

// The path first, then each ancestor up to (and including) `~` or the drive.
fn path_and_ancestors(norm: &str) -> Vec<String> {
    let mut out = vec![norm.to_string()];
    let mut cur = norm.to_string();
    while let Some(i) = cur.rfind('\\') {
        cur.truncate(i);
        if cur.is_empty() {
            break;
        }
        out.push(cur.clone());
    }
    out
}

pub fn recall_path(conn: &Connection, abs_path: &str) -> anyhow::Result<Vec<Memory>> {
    recall_path_with(conn, abs_path, &paths::profile_dir().to_string_lossy())
}

pub fn recall_path_with(
    conn: &Connection,
    abs_path: &str,
    profile: &str,
) -> anyhow::Result<Vec<Memory>> {
    let rows = covering(conn, abs_path, profile)?;
    touch(conn, rows.iter().map(|m| m.id), now_ms())?;
    Ok(rows)
}

fn covering(conn: &Connection, abs_path: &str, profile: &str) -> anyhow::Result<Vec<Memory>> {
    let norm = normalize_key_with(
        Scope::Path,
        abs_path.strip_prefix(r"\\?\").unwrap_or(abs_path),
        profile,
    );
    let chain = path_and_ancestors(&norm);
    let mut found: Vec<(usize, Memory)> = Vec::new();

    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM memory WHERE scope = 'path' AND key = ?1 COLLATE NOCASE AND forgotten_ms IS NULL ORDER BY id"
    ))?;
    for (rank, k) in chain.iter().enumerate() {
        for m in stmt.query_map([k], from_row)? {
            found.push((rank, m?));
        }
    }

    let mut gstmt = conn.prepare(&format!(
        "SELECT {COLS} FROM memory WHERE scope = 'glob' AND forgotten_ms IS NULL ORDER BY id"
    ))?;
    let globs = gstmt
        .query_map([], from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    for m in globs {
        let Ok(matcher) = compile_glob(&m.key) else {
            continue;
        };
        let hit = chain
            .iter()
            .position(|c| matcher.is_match(c.replace('\\', "/")));
        if let Some(rank) = hit {
            found.push((chain.len() + rank, m));
        }
    }
    found.sort_by_key(|(rank, m)| (*rank, m.id));
    Ok(found.into_iter().map(|(_, m)| m).collect())
}

pub fn protect_covering(conn: &Connection, abs_path: &str) -> anyhow::Result<Option<Memory>> {
    protect_covering_with(conn, abs_path, &paths::profile_dir().to_string_lossy())
}

pub fn protect_covering_with(
    conn: &Connection,
    abs_path: &str,
    profile: &str,
) -> anyhow::Result<Option<Memory>> {
    let found = covering(conn, abs_path, profile)?
        .into_iter()
        .find(|m| m.kind == Kind::Protect);
    if let Some(m) = &found {
        touch(conn, [m.id], now_ms())?;
    }
    Ok(found)
}

// Not a use: unit resolution only asks whether to stop walking (REVIEW-UNITS).
pub(crate) fn protect_covering_quiet(
    conn: &Connection,
    abs_path: &str,
    profile: &str,
) -> anyhow::Result<Option<Memory>> {
    Ok(covering(conn, abs_path, profile)?
        .into_iter()
        .find(|m| m.kind == Kind::Protect))
}

// Unlike recall_path, only the note a card shows counts as used; classifier location rows only restate the catalog and index.
pub fn note_covering_with(
    conn: &Connection,
    abs_path: &str,
    profile: &str,
) -> anyhow::Result<Option<Memory>> {
    let mut found = None;
    for m in covering(conn, abs_path, profile)? {
        if m.kind == Kind::Protect || (m.detail.is_some() && knowledge::is_classifier(&m.source)) {
            continue;
        }
        // A job row speaks through what it cleaned; once the sweep has reclassified it, its text is the classifier's.
        if m.detail.is_some() && location::is_job(&m.source) {
            if let Some(c) = location::cleaned_for_with(conn, profile, abs_path)? {
                let text = format!("{}.", location::capitalized(&c.reason()));
                found = Some(Memory { text, ..m });
                break;
            }
            if m.detail.as_ref().and_then(|d| d.by.as_deref()) != Some("job") {
                continue;
            }
        }
        found = Some(m);
        break;
    }
    if let Some(m) = &found {
        touch(conn, [m.id], now_ms())?;
    }
    Ok(found)
}

// Free text becomes quoted OR-ed prefix terms so FTS5 operators in user input cannot raise syntax errors.
pub fn fts_query(q: &str) -> Option<String> {
    let terms: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
        .take(12)
        .map(|t| format!("\"{t}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

fn fts_live(conn: &Connection, q: &str, with_forgotten: bool) -> anyhow::Result<Vec<Memory>> {
    let Some(query) = fts_query(q) else {
        return Ok(Vec::new());
    };
    let live = if with_forgotten {
        ""
    } else {
        "AND m.forgotten_ms IS NULL"
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT {M_COLS} FROM memory_fts f JOIN memory m ON m.id = f.rowid
         WHERE memory_fts MATCH ?1 {live} ORDER BY bm25(memory_fts), m.id LIMIT 50"
    ))?;
    let rows = stmt
        .query_map([query], from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn recall_text(conn: &Connection, q: &str) -> anyhow::Result<Vec<Memory>> {
    let rows = fts_live(conn, q, false)?;
    touch(conn, rows.iter().map(|m| m.id), now_ms())?;
    Ok(rows)
}

// The drawer's search: sees forgotten notes and is not a use.
pub fn search(conn: &Connection, q: &str) -> anyhow::Result<Vec<Memory>> {
    fts_live(conn, q, true)
}

pub fn globals(conn: &Connection, cap: usize) -> anyhow::Result<Vec<Memory>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM memory WHERE scope = 'global' AND forgotten_ms IS NULL
         ORDER BY CASE kind WHEN 'protect' THEN 0 ELSE 1 END, pinned DESC, last_used_ms DESC, id DESC
         LIMIT ?1"
    ))?;
    let rows = stmt
        .query_map([cap as i64], from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// A global note counts as used when the user's message matches it; matches are marked before ranking.
pub fn globals_for(conn: &Connection, cap: usize, user_text: &str) -> anyhow::Result<Vec<Memory>> {
    let hits = fts_live(conn, user_text, false)?;
    touch(
        conn,
        hits.iter()
            .filter(|m| m.scope == Scope::Global)
            .map(|m| m.id),
        now_ms(),
    )?;
    globals(conn, cap)
}

pub fn default_globals(conn: &Connection) -> anyhow::Result<Vec<Memory>> {
    globals(conn, 30)
}

fn touch(conn: &Connection, ids: impl IntoIterator<Item = i64>, now: i64) -> anyhow::Result<()> {
    let mut stmt = conn.prepare_cached("UPDATE memory SET last_used_ms = ?1 WHERE id = ?2")?;
    for id in ids {
        stmt.execute(params![now, id])?;
    }
    Ok(())
}

pub fn limit(conn: &Connection) -> usize {
    get_meta(conn, LIMIT_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<usize>().ok())
        .map_or(DEFAULT_LIMIT, |n| n.max(MIN_LIMIT))
}

pub fn set_limit(conn: &Connection, n: usize) -> anyhow::Result<usize> {
    let n = n.max(MIN_LIMIT);
    set_meta(conn, LIMIT_KEY, &n.to_string())?;
    enforce_limit(conn, now_ms())?;
    Ok(n)
}

pub fn purge_forgotten(conn: &Connection, now: i64) -> anyhow::Result<usize> {
    Ok(conn.execute(
        "DELETE FROM memory WHERE forgotten_ms IS NOT NULL AND forgotten_ms <= ?1",
        [now - FORGOTTEN_PURGE_MS],
    )?)
}

// Pinned, protect, location and file-type notes are exempt and not counted; the oldest-used of the rest are forgotten, never deleted.
pub fn enforce_limit(conn: &Connection, now: i64) -> anyhow::Result<usize> {
    purge_forgotten(conn, now)?;
    let n = limit(conn) as i64;
    Ok(conn.execute(
        "UPDATE memory SET forgotten_ms = ?1 WHERE id IN (
           SELECT id FROM memory WHERE forgotten_ms IS NULL AND pinned = 0 AND kind <> 'protect' AND detail_json IS NULL
           ORDER BY last_used_ms DESC, id DESC LIMIT -1 OFFSET ?2)",
        params![now, n],
    )?)
}

pub fn restore(conn: &Connection, id: i64) -> anyhow::Result<Option<Memory>> {
    let now = now_ms();
    conn.execute(
        "UPDATE memory SET forgotten_ms = NULL, last_used_ms = ?1 WHERE id = ?2",
        params![now, id],
    )?;
    enforce_limit(conn, now)?;
    get(conn, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Db;

    const P: &str = r"C:\Users\tester";

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("t.db")).unwrap();
        let conn = db.conn().unwrap();
        (dir, conn)
    }

    #[test]
    fn keys_normalize_to_profile_relative() {
        assert_eq!(
            normalize_key_with(Scope::Path, r"c:\users\TESTER/.cache/hf/", P),
            r"~\.cache\hf"
        );
        assert_eq!(normalize_key_with(Scope::Global, " tone ", P), "tone");
    }

    #[test]
    fn fts_query_neutralizes_operators() {
        assert_eq!(
            fts_query(r#"foo" OR (bar*) NOT -x"#).unwrap(),
            r#""foo"* OR "OR"* OR "bar"* OR "NOT"*"#
        );
        assert!(fts_query("!!  ?").is_none());
    }

    #[test]
    fn identical_remember_is_idempotent() {
        let (_d, c) = db();
        let n = || NewMemory::new(Scope::Path, r"D:\x", Kind::Fact, "a", "user");
        let a = remember_with(&c, n(), 1000, P).unwrap();
        let b = remember_with(&c, n(), 2000, P).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(list(&c).unwrap().len(), 1);
    }

    #[test]
    fn facts_get_review_date_and_go_stale() {
        let (_d, c) = db();
        let f = remember_with(
            &c,
            NewMemory::new(Scope::Path, r"D:\x", Kind::Fact, "weekly", "user"),
            0,
            P,
        )
        .unwrap();
        assert_eq!(f.review_after_ms, Some(USAGE_REVIEW_MS));
        assert!(!f.is_stale(USAGE_REVIEW_MS - 1));
        assert!(f.is_stale(USAGE_REVIEW_MS));
        let p = remember_with(
            &c,
            NewMemory::new(Scope::Path, r"D:\y", Kind::Protect, "keep", "user"),
            0,
            P,
        )
        .unwrap();
        assert_eq!(p.review_after_ms, None);
    }

    #[test]
    fn text_recall_survives_hostile_input_and_globals_are_capped() {
        let (_d, c) = db();
        remember_with(
            &c,
            NewMemory::new(
                Scope::Path,
                r"D:\scratch",
                Kind::Fact,
                "disposable training checkpoints",
                "user",
            ),
            0,
            P,
        )
        .unwrap();
        for i in 0..5 {
            remember_with(
                &c,
                NewMemory::new(
                    Scope::Global,
                    &format!("g{i}"),
                    Kind::Preference,
                    &format!("pref {i}"),
                    "user",
                ),
                i,
                P,
            )
            .unwrap();
        }
        let hits = recall_text(&c, r#"checkpoint" OR NEAR( AND *"#).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].key, r"D:\scratch");
        assert!(recall_text(&c, "???").unwrap().is_empty());
        let g = globals(&c, 3).unwrap();
        assert_eq!(
            g.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            ["pref 4", "pref 3", "pref 2"]
        );
    }

    #[test]
    fn bad_glob_is_rejected() {
        let (_d, c) = db();
        assert!(
            remember_with(
                &c,
                NewMemory::new(Scope::Glob, "~\\[", Kind::Fact, "x", "user"),
                0,
                P
            )
            .is_err()
        );
    }
}

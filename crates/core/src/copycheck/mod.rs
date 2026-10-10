mod archive;
mod claimed;
pub mod consent;
mod dup;
mod git;
pub mod hash;
pub mod hub;
mod sync;
pub mod winget;

use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cloud::Cloud;
use crate::index::{self, FLAG_DENIED, FLAG_HARDLINK_DUP, FLAG_PLACEHOLDER, is_ancestor_path};
use crate::policy::Verdict;
use crate::store::{Db, now_ms, set_meta};

pub use archive::ArchiveCheck;
pub use claimed::{CatalogCheck, OllamaCheck};
pub use consent::{OnlineConsent, Service, Services};
pub use dup::DupCheck;
pub use git::GitCheck;
pub use hub::HubCheck;
pub use sync::SyncCheck;
pub use winget::{WingetCheck, WingetCli};

pub const FILE_MIN: u64 = 100 << 20;
pub const DIR_MIN: u64 = 1 << 30;
const MAX_CANDIDATES: usize = 2000;
pub const WAITING_KEY: &str = "copy_online_waiting";

// Ordered weakest to strongest, so `max` picks the best answer among checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    None,
    Claimed,
    SameNameAndSize,
    Identical,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::None => "none",
            Confidence::Claimed => "claimed",
            Confidence::SameNameAndSize => "same-name-and-size",
            Confidence::Identical => "identical",
        }
    }

    pub fn parse(s: &str) -> Option<Confidence> {
        match s {
            "none" => Some(Confidence::None),
            "claimed" => Some(Confidence::Claimed),
            "same-name-and-size" => Some(Confidence::SameNameAndSize),
            "identical" => Some(Confidence::Identical),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Effect {
    // May become `redownload` (or `synced` for a sync result), so it can start ticked.
    MayStartTicked,
    Review,
    FollowCatalog,
    OnlyCopy,
}

pub fn effect(c: Confidence) -> Effect {
    match c {
        Confidence::Identical => Effect::MayStartTicked,
        Confidence::SameNameAndSize => Effect::Review,
        Confidence::Claimed => Effect::FollowCatalog,
        Confidence::None => Effect::OnlyCopy,
    }
}

pub fn verdict_for(r: &CopyResult) -> Option<Verdict> {
    match effect(r.confidence) {
        Effect::MayStartTicked if r.check == SyncCheck::NAME => Some(Verdict::Synced),
        Effect::MayStartTicked => Some(Verdict::Redownload),
        Effect::Review => Some(Verdict::Review),
        Effect::FollowCatalog | Effect::OnlyCopy => None,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CopyResult {
    pub check: String,
    pub locator: Option<String>,
    pub confidence: Confidence,
    pub checked_ms: i64,
    pub detail: Value,
}

impl CopyResult {
    pub fn new(
        check: &str,
        locator: Option<String>,
        confidence: Confidence,
        detail: Value,
    ) -> Self {
        CopyResult {
            check: check.to_string(),
            locator,
            confidence,
            checked_ms: now_ms(),
            detail,
        }
    }

    pub fn note(&self) -> String {
        if let Some(n) = self.detail.get("note").and_then(Value::as_str) {
            return n.to_string();
        }
        let at = self.locator.as_deref().unwrap_or("elsewhere");
        match self.confidence {
            Confidence::Identical => format!("identical copy at {at}"),
            Confidence::SameNameAndSize => format!("same name and size as {at}"),
            Confidence::Claimed => format!("{at} can reinstall it"),
            Confidence::None => format!("no copy found by {}", self.check),
        }
    }

    // "hash": waits for the background SHA-256; "sync": waits for the sync engine; "files": waits on its files' results.
    pub fn pending(&self) -> Option<&str> {
        self.detail.get("pending").and_then(Value::as_str)
    }
}

pub(crate) fn with_note(mut detail: Value, note: impl Into<String>) -> Value {
    if !detail.is_object() {
        detail = json!({});
    }
    detail["note"] = Value::String(note.into());
    detail
}

pub(crate) fn pending(mut detail: Value, what: &str) -> Value {
    if !detail.is_object() {
        detail = json!({});
    }
    detail["pending"] = Value::String(what.into());
    detail
}

// Knowledge rows may name a check to try; the hint says where to look, never what the answer is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub check: String,
    pub place: Option<String>,
}

impl Hint {
    pub fn parse(v: &Value) -> Option<Hint> {
        let (check, place) = match v {
            Value::String(s) => match s.split_once(':') {
                Some((c, p)) => (c.trim().to_string(), Some(p.trim().to_string())),
                None => (s.trim().to_string(), None),
            },
            Value::Object(o) => (
                o.get("check")?.as_str()?.trim().to_string(),
                o.get("where")
                    .or_else(|| o.get("place"))
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string()),
            ),
            _ => return None,
        };
        let check = match check.to_ascii_lowercase().as_str() {
            "huggingface" | "hf" | "modelhub" | "model-hub" => HubCheck::NAME.to_string(),
            "cloud" | "onedrive" | "dropbox" => SyncCheck::NAME.to_string(),
            "" => return None,
            other => other.to_string(),
        };
        Some(Hint {
            check,
            place: place.filter(|p| !p.is_empty()),
        })
    }

    pub fn names(&self, check: &str) -> bool {
        self.check == check
    }
}

pub fn hint_for(conn: &Connection, path: &str, profile: &str) -> Option<Hint> {
    let mut st = conn
        .prepare_cached(
            "SELECT scope, key, detail_json FROM memory
             WHERE forgotten_ms IS NULL AND detail_json LIKE '%copy_hint%' ORDER BY id",
        )
        .ok()?;
    let rows: Vec<(String, String, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .ok()?
        .filter_map(Result::ok)
        .collect();
    let rel = crate::memory::normalize_key_with(crate::memory::Scope::Path, path, profile);
    let mut chain = vec![rel.clone()];
    let mut cur = rel;
    while let Some(i) = cur.rfind('\\') {
        cur.truncate(i);
        if cur.is_empty() {
            break;
        }
        chain.push(cur.clone());
    }
    let mut best: Option<(usize, Hint)> = None;
    for (scope, key, detail) in rows {
        let Some(hint) = serde_json::from_str::<Value>(&detail)
            .ok()
            .and_then(|d| d.get("copy_hint").and_then(Hint::parse))
        else {
            continue;
        };
        let rank = match scope.as_str() {
            "path" => chain.iter().position(|c| c.eq_ignore_ascii_case(&key)),
            "glob" => crate::memory::compile_glob(&key)
                .ok()
                .and_then(|m| chain.iter().position(|c| m.is_match(c.replace('\\', "/"))))
                .map(|r| r + chain.len()),
            _ => None,
        };
        if let Some(rank) = rank
            && best.as_ref().is_none_or(|(b, _)| rank < *b)
        {
            best = Some((rank, hint));
        }
    }
    best.map(|(_, h)| h)
}

// Answers are stored per volume + path + check and hold while id, size and mtime do.
#[derive(Clone, Debug)]
pub struct Subject {
    pub volume_id: i64,
    pub path: String,
    pub dir: bool,
    pub size: u64,
    pub mtime: Option<i64>,
    pub file_id: Option<[u8; 16]>,
    pub attrs: u32,
    pub hint: Option<Hint>,
}

impl Subject {
    pub fn load(conn: &Connection, path: &str) -> Option<Subject> {
        let path = index::normalize_path(path);
        let md = std::fs::symlink_metadata(&path).ok()?;
        if md.file_type().is_symlink() {
            return None;
        }
        let dir = md.is_dir();
        let size = if dir {
            index::node_by_path(conn, &path)
                .ok()
                .flatten()
                .map_or(0, |n| n.logical)
        } else {
            md.len()
        };
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64);
        Some(Subject {
            volume_id: volume_id_of(conn, &path),
            file_id: hoardini_win::file_id_of(Path::new(&path)).ok(),
            attrs: md.file_attributes(),
            path,
            dir,
            size,
            mtime,
            hint: None,
        })
    }

    pub fn name(&self) -> &str {
        self.path.rsplit('\\').next().unwrap_or(&self.path)
    }

    pub fn is_placeholder(&self) -> bool {
        hoardini_win::is_cloud_placeholder(self.attrs)
    }
}

pub fn volume_id_of(conn: &Connection, path: &str) -> i64 {
    let Ok(mut st) = conn.prepare_cached("SELECT id, root FROM volume") else {
        return 0;
    };
    let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))) else {
        return 0;
    };
    rows.filter_map(Result::ok)
        .filter(|(_, root)| {
            let root = index::normalize_path(root);
            root.eq_ignore_ascii_case(path) || is_ancestor_path(&root, path)
        })
        .max_by_key(|(_, root)| root.len())
        .map_or(0, |(id, _)| id)
}

pub enum Find {
    Copy(Found),
    NoCopy(Value),
    Skip,
}

pub struct Found {
    pub locator: String,
    pub detail: Value,
}

pub struct CheckEnv {
    pub profile: String,
    pub cloud: Arc<dyn Cloud>,
    pub hub_base: String,
    pub winget: Arc<dyn WingetCli>,
    pub git: String,
    pub consent: OnlineConsent,
    pub catalog: Option<Arc<crate::catalog::Catalog>>,
    pub ollama_models: Option<String>,
}

impl CheckEnv {
    pub fn system(conn: &Connection) -> CheckEnv {
        CheckEnv {
            profile: crate::paths::profile_dir().to_string_lossy().into_owned(),
            cloud: crate::cloud::system(),
            hub_base: hub::HF_BASE.to_string(),
            winget: Arc::new(winget::SystemWinget),
            git: "git".into(),
            consent: consent::load(conn),
            catalog: None,
            ollama_models: std::env::var("OLLAMA_MODELS")
                .ok()
                .filter(|v| !v.trim().is_empty()),
        }
    }

    // No consent: only offline checks can run with this.
    pub fn offline(profile: &str, cloud: Arc<dyn Cloud>) -> CheckEnv {
        CheckEnv {
            profile: profile.to_string(),
            cloud,
            hub_base: hub::HF_BASE.to_string(),
            winget: Arc::new(winget::SystemWinget),
            git: "git".into(),
            consent: OnlineConsent::default(),
            catalog: None,
            ollama_models: None,
        }
    }

    pub fn catalog(&self) -> &crate::catalog::Catalog {
        match &self.catalog {
            Some(c) => c,
            None => crate::catalog::Catalog::embedded(),
        }
    }

    pub fn allows(&self, service: Service) -> bool {
        self.consent.allows(service)
    }
}

pub struct Cx<'a> {
    pub conn: &'a Connection,
    pub env: &'a CheckEnv,
}

pub trait CopyCheck: Send + Sync {
    fn name(&self) -> &'static str;
    // Online checks name the service the user consents to; offline checks always run.
    fn service(&self) -> Option<Service> {
        None
    }
    fn applies(&self, cx: &Cx<'_>, s: &Subject) -> bool;
    // Folder items worth checking in the background even when nobody asked yet.
    fn covers_dir(&self, _cx: &Cx<'_>, _path: &str) -> bool {
        false
    }
    // A stored answer can go stale through something other than the subject (a twin, a push, a moved copy).
    fn still_valid(&self, _cx: &Cx<'_>, _s: &Subject, _r: &CopyResult) -> bool {
        true
    }
    fn find(&self, cx: &Cx<'_>, s: &Subject) -> anyhow::Result<Find>;
    fn verify(&self, cx: &Cx<'_>, s: &Subject, found: Found) -> anyhow::Result<CopyResult>;
}

pub struct Registry {
    checks: Vec<Box<dyn CopyCheck>>,
}

impl Registry {
    pub fn standard() -> Registry {
        Registry {
            checks: vec![
                Box::new(DupCheck),
                Box::new(ArchiveCheck),
                Box::new(SyncCheck),
                Box::new(GitCheck),
                Box::new(HubCheck::default()),
                Box::new(OllamaCheck),
                Box::new(WingetCheck),
                Box::new(CatalogCheck),
            ],
        }
    }

    pub fn with(checks: Vec<Box<dyn CopyCheck>>) -> Registry {
        Registry { checks }
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.checks.iter().map(|c| c.name()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&dyn CopyCheck> {
        self.checks
            .iter()
            .find(|c| c.name() == name)
            .map(|c| c.as_ref())
    }
}

fn fid_matches(stored: &Option<Vec<u8>>, now: &Option<[u8; 16]>) -> bool {
    match (stored, now) {
        (Some(a), Some(b)) => a.as_slice() == b.as_slice(),
        (None, None) => true,
        _ => false,
    }
}

fn row_to_result(
    check: String,
    confidence: String,
    locator: Option<String>,
    detail: Option<String>,
    checked_ms: i64,
) -> Option<CopyResult> {
    Some(CopyResult {
        check,
        locator,
        confidence: Confidence::parse(&confidence)?,
        checked_ms,
        detail: detail
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_else(|| json!({})),
    })
}

type StoredRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    Option<Vec<u8>>,
    i64,
    Option<i64>,
);

fn stored_rows(conn: &Connection, s: &Subject) -> Vec<StoredRow> {
    let Ok(mut st) = conn.prepare_cached(
        "SELECT check_name, confidence, locator, detail_json, checked_ms, file_id, size, mtime
         FROM copy_check WHERE volume_id = ?1 AND path = ?2 COLLATE NOCASE ORDER BY check_name",
    ) else {
        return Vec::new();
    };
    st.query_map(params![s.volume_id, s.path], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
        ))
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

pub fn current(conn: &Connection, s: &Subject) -> Vec<CopyResult> {
    stored_rows(conn, s)
        .into_iter()
        .filter(|(_, _, _, _, _, fid, size, mtime)| {
            fid_matches(fid, &s.file_id) && *size == index::to_i64(s.size) && *mtime == s.mtime
        })
        .filter_map(|(c, conf, loc, det, at, ..)| row_to_result(c, conf, loc, det, at))
        .collect()
}

pub fn store(conn: &Connection, s: &Subject, r: &CopyResult) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO copy_check(volume_id, path, file_id, size, mtime, check_name, confidence, locator, detail_json, checked_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(volume_id, path, check_name) DO UPDATE SET
           path = excluded.path, file_id = excluded.file_id, size = excluded.size, mtime = excluded.mtime,
           confidence = excluded.confidence, locator = excluded.locator, detail_json = excluded.detail_json,
           checked_ms = excluded.checked_ms",
        params![
            s.volume_id,
            s.path,
            s.file_id.map(|f| f.to_vec()),
            index::to_i64(s.size),
            s.mtime,
            r.check,
            r.confidence.as_str(),
            r.locator,
            r.detail.to_string(),
            r.checked_ms
        ],
    )?;
    Ok(())
}

fn forget(conn: &Connection, s: &Subject, check: &str) {
    let _ = conn.execute(
        "DELETE FROM copy_check WHERE volume_id = ?1 AND path = ?2 COLLATE NOCASE AND check_name = ?3",
        params![s.volume_id, s.path, check],
    );
}

fn settled(cx: &Cx<'_>, s: &Subject, r: &CopyResult) -> bool {
    match r.pending() {
        None => true,
        Some("hash") => hash::current(cx.conn, s).is_none(),
        Some("files") => hub::files_still_pending(cx, s),
        Some(_) => false,
    }
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub results: Vec<CopyResult>,
    pub waiting: Vec<Service>,
    pub needs_hash: bool,
}

pub fn check_subject(cx: &Cx<'_>, reg: &Registry, s: &Subject) -> Outcome {
    let mut out = Outcome::default();
    let stored = current(cx.conn, s);
    let mut s = s.clone();
    if s.hint.is_none() {
        s.hint = hint_for(cx.conn, &s.path, &cx.env.profile);
    }
    for c in &reg.checks {
        if !c.applies(cx, &s) {
            continue;
        }
        if let Some(svc) = c.service()
            && !cx.env.allows(svc)
        {
            if !cx.env.consent.asked {
                out.waiting.push(svc);
            }
            continue;
        }
        if let Some(r) = stored.iter().find(|r| r.check == c.name())
            && settled(cx, &s, r)
            && c.still_valid(cx, &s, r)
        {
            out.needs_hash |= r.pending() == Some("hash");
            out.results.push(r.clone());
            continue;
        }
        let r = match c.find(cx, &s) {
            Ok(Find::Copy(f)) => c.verify(cx, &s, f),
            Ok(Find::NoCopy(detail)) => {
                Ok(CopyResult::new(c.name(), None, Confidence::None, detail))
            }
            Ok(Find::Skip) => {
                forget(cx.conn, &s, c.name());
                continue;
            }
            Err(e) => Err(e),
        };
        match r {
            Ok(r) => {
                if let Err(e) = store(cx.conn, &s, &r) {
                    tracing::debug!(path = %s.path, error = %e, "copy check not stored");
                }
                out.needs_hash |= r.pending() == Some("hash");
                out.results.push(r);
            }
            Err(e) => {
                tracing::debug!(path = %s.path, check = c.name(), error = %e, "copy check failed")
            }
        }
    }
    out
}

pub fn check_path(cx: &Cx<'_>, reg: &Registry, path: &str) -> Outcome {
    match Subject::load(cx.conn, path) {
        Some(s) => check_subject(cx, reg, &s),
        None => Outcome::default(),
    }
}

pub fn results_for(conn: &Connection, path: &str) -> Vec<CopyResult> {
    Subject::load(conn, path)
        .map(|s| current(conn, &s))
        .unwrap_or_default()
}

pub fn best(results: &[CopyResult]) -> Option<&CopyResult> {
    results
        .iter()
        .max_by_key(|r| (r.confidence, std::cmp::Reverse(r.check.clone())))
}

fn waiting_on_consent(conn: &Connection, s: &Subject, env: &CheckEnv) -> bool {
    if env.consent.asked {
        return false;
    }
    let reg = Registry::standard();
    let cx = Cx { conn, env };
    reg.checks
        .iter()
        .any(|c| c.service().is_some_and(|svc| !env.allows(svc)) && c.applies(&cx, s))
}

// "Only copy we know of": something checked it, every answer is `none`, and nothing is still pending or waiting on consent.
pub fn only_copy(conn: &Connection, path: &str) -> bool {
    let Some(s) = Subject::load(conn, path) else {
        return false;
    };
    let rs = current(conn, &s);
    if rs.is_empty()
        || rs
            .iter()
            .any(|r| r.confidence != Confidence::None || r.pending().is_some())
    {
        return false;
    }
    !waiting_on_consent(conn, &s, &CheckEnv::system(conn))
}

pub fn note_for(conn: &Connection, path: &str) -> Option<String> {
    let rs = results_for(conn, path);
    let b = best(&rs)?;
    if b.confidence == Confidence::None {
        return only_copy(conn, path).then(|| "only copy we know of".to_string());
    }
    Some(b.note())
}

// An `identical` answer that lets the policy cap relax; sync answers only count for online-only.
pub fn identical_for(conn: &Connection, path: &str, sync: bool) -> Option<CopyResult> {
    results_for(conn, path).into_iter().find(|r| {
        r.confidence == Confidence::Identical
            && r.pending().is_none()
            && (r.check == SyncCheck::NAME) == sync
    })
}

// The sync check alone, run now; online-only cards take their verdict from it.
pub fn sync_result(
    conn: &Connection,
    cloud: Arc<dyn Cloud>,
    profile: &str,
    path: &str,
) -> Option<CopyResult> {
    let env = CheckEnv::offline(profile, cloud);
    let reg = Registry::with(vec![Box::new(SyncCheck)]);
    let cx = Cx { conn, env: &env };
    check_path(&cx, &reg, path).results.into_iter().next()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub path: String,
    pub dir: bool,
    pub size: u64,
}

pub fn candidates(
    cx: &Cx<'_>,
    reg: &Registry,
    volume_root: &str,
) -> anyhow::Result<Vec<Candidate>> {
    let root = index::normalize_path(volume_root);
    let vid: Option<i64> = cx
        .conn
        .query_row(
            "SELECT id FROM volume WHERE root = ?1 COLLATE NOCASE",
            [&root],
            |r| r.get(0),
        )
        .optional()?;
    let Some(vid) = vid else {
        return Ok(Vec::new());
    };
    let mut cache = index::PathCache::default();
    let mut out = Vec::new();
    let mut st = cx.conn.prepare(
        "SELECT id, logical FROM node WHERE volume_id = ?1 AND kind = ?2 AND logical >= ?3 AND (flags & ?4) = 0
         ORDER BY logical DESC LIMIT ?5",
    )?;
    let files: Vec<(i64, i64)> = st
        .query_map(
            params![
                vid,
                index::KIND_FILE,
                index::to_i64(FILE_MIN),
                FLAG_DENIED | FLAG_PLACEHOLDER | FLAG_HARDLINK_DUP,
                MAX_CANDIDATES as i64
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .collect::<rusqlite::Result<_>>()?;
    for (id, size) in files {
        if let Some(path) = cache.get(cx.conn, id)? {
            out.push(Candidate {
                path,
                dir: false,
                size: index::to_u64(size),
            });
        }
    }
    let mut st = cx.conn.prepare(
        "SELECT id, sub_logical FROM node WHERE volume_id = ?1 AND kind = ?2 AND sub_logical >= ?3 AND (flags & ?4) = 0
         ORDER BY sub_logical DESC LIMIT 5000",
    )?;
    let dirs: Vec<(i64, i64)> = st
        .query_map(
            params![vid, index::KIND_DIR, index::to_i64(DIR_MIN), FLAG_DENIED],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .collect::<rusqlite::Result<_>>()?;
    for (id, size) in dirs {
        let Some(path) = cache.get(cx.conn, id)? else {
            continue;
        };
        if reg.checks.iter().any(|c| c.covers_dir(cx, &path)) {
            out.push(Candidate {
                path,
                dir: true,
                size: index::to_u64(size),
            });
        }
    }
    out.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    out.truncate(MAX_CANDIDATES);
    Ok(out)
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RunReport {
    pub candidates: usize,
    pub checked: usize,
    pub hashed: usize,
    pub hashed_bytes: u64,
    pub online_waiting: usize,
    pub stopped: bool,
}

// Checks before hashes, so a pass stopped early has still answered everything that needs no hash.
pub fn run_volume(
    db: &Db,
    env: &CheckEnv,
    reg: &Registry,
    volume_root: &str,
    go: &dyn Fn() -> bool,
) -> anyhow::Result<RunReport> {
    let conn = db.conn()?;
    let cx = Cx { conn: &conn, env };
    let list = candidates(&cx, reg, volume_root)?;
    let mut report = RunReport {
        candidates: list.len(),
        ..RunReport::default()
    };
    let mut to_hash: Vec<Subject> = Vec::new();
    let mut waiting: Vec<Subject> = Vec::new();
    for c in &list {
        if !go() {
            report.stopped = true;
            return Ok(report);
        }
        let Some(s) = Subject::load(&conn, &c.path) else {
            continue;
        };
        let o = check_subject(&cx, reg, &s);
        report.checked += 1;
        report.online_waiting += usize::from(!o.waiting.is_empty());
        if o.needs_hash && !s.dir && hash::current(&conn, &s).is_none() {
            to_hash.push(s.clone());
        }
        if o.needs_hash {
            waiting.push(s);
        }
    }
    to_hash.sort_by(|a, b| b.size.cmp(&a.size));
    for s in to_hash {
        match hash::hash_subject(&conn, &s, go) {
            Ok(Some(_)) => {
                report.hashed += 1;
                report.hashed_bytes += s.size;
            }
            Ok(None) => {
                report.stopped = true;
                return Ok(report);
            }
            Err(e) => {
                tracing::debug!(path = %s.path, error = %e, "not hashed");
                continue;
            }
        }
    }
    // A duplicate's answer needs both hashes, so everything that waited is asked again once all are in.
    for s in waiting {
        check_subject(&cx, reg, &s);
    }
    Ok(report)
}

// The server stores the total over every watched volume after a full pass.
pub fn set_online_waiting(conn: &Connection, n: usize) -> anyhow::Result<()> {
    set_meta(conn, WAITING_KEY, &n.to_string())
}

pub fn online_waiting(conn: &Connection) -> usize {
    crate::store::get_meta(conn, WAITING_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_orders_and_round_trips() {
        assert!(Confidence::Identical > Confidence::SameNameAndSize);
        assert!(Confidence::SameNameAndSize > Confidence::Claimed);
        assert!(Confidence::Claimed > Confidence::None);
        for c in [
            Confidence::None,
            Confidence::Claimed,
            Confidence::SameNameAndSize,
            Confidence::Identical,
        ] {
            assert_eq!(Confidence::parse(c.as_str()), Some(c));
            assert_eq!(
                serde_json::to_value(c).unwrap(),
                Value::String(c.as_str().into())
            );
        }
    }

    #[test]
    fn hints_parse_from_strings_and_objects() {
        assert_eq!(
            Hint::parse(&json!("huggingface:FastFlowLM")),
            Some(Hint {
                check: HubCheck::NAME.into(),
                place: Some("FastFlowLM".into())
            })
        );
        assert_eq!(
            Hint::parse(&json!({"check": "winget"})).map(|h| h.check),
            Some("winget".into())
        );
        assert_eq!(Hint::parse(&json!(3)), None);
    }
}

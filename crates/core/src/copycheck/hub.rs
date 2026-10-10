use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    Confidence, CopyCheck, CopyResult, Cx, FILE_MIN, Find, Found, Service, Subject, current, hash,
    pending, same_contents, with_fingerprint, with_note,
};

pub const HF_BASE: &str = "https://huggingface.co";
const WEIGHT_EXT: &[&str] = &["gguf", "safetensors", "onnx", "pt", "pth", "ckpt"];
const TREE_TTL: Duration = Duration::from_secs(3600);
const MAX_SEARCH_REPOS: usize = 8;
const MAX_TREE_PAGES: usize = 20;
const DIR_FILE_CAP: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: String,
    pub size: u64,
    pub oid: Option<String>,
    pub lfs_oid: Option<String>,
}

#[derive(Deserialize)]
struct RawLfs {
    oid: Option<String>,
    size: Option<u64>,
}

#[derive(Deserialize)]
struct RawEntry {
    #[serde(rename = "type")]
    kind: Option<String>,
    path: String,
    #[serde(default)]
    size: u64,
    oid: Option<String>,
    lfs: Option<RawLfs>,
}

// `GET /api/models/{repo}/tree/{rev}?recursive=1`: files only, LFS size wins over the pointer size.
pub fn parse_tree(body: &str) -> anyhow::Result<Vec<TreeEntry>> {
    let raw: Vec<RawEntry> = serde_json::from_str(body)?;
    Ok(raw
        .into_iter()
        .filter(|e| e.kind.as_deref() != Some("directory"))
        .map(|e| TreeEntry {
            size: e.lfs.as_ref().and_then(|l| l.size).unwrap_or(e.size),
            lfs_oid: e.lfs.and_then(|l| l.oid).map(|o| o.to_ascii_lowercase()),
            oid: e.oid,
            path: e.path,
        })
        .collect())
}

pub fn parse_search(body: &str) -> anyhow::Result<Vec<String>> {
    let raw: Vec<Value> = serde_json::from_str(body)?;
    Ok(raw
        .iter()
        .filter_map(|m| {
            m.get("id")
                .or_else(|| m.get("modelId"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub repo: String,
    pub rev: String,
    // The file's path inside the repo; None for the repo folder itself or a blob.
    pub rel: Option<String>,
    pub cache_root: Option<String>,
}

fn segs(path: &str) -> Vec<&str> {
    path.split('\\').filter(|s| !s.is_empty()).collect()
}

// `...\hub\models--org--name\snapshots\<rev>\a\b.gguf`, `...\models--org--name\blobs\<sha>`, or the repo folder.
pub fn hf_cache_layout(path: &str) -> Option<Layout> {
    let s = segs(path);
    let i = s.iter().position(|p| p.starts_with("models--"))?;
    let (org, repo_name) = s[i].strip_prefix("models--")?.split_once("--")?;
    let repo = format!("{org}/{repo_name}");
    let cache_root = s[..=i].join("\\");
    let refs_main = || {
        std::fs::read_to_string(Path::new(&cache_root).join("refs").join("main"))
            .ok()
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| "main".into())
    };
    let (rev, rel) = match s.get(i + 1).copied() {
        None => (refs_main(), None),
        Some(k) if k.eq_ignore_ascii_case("snapshots") => {
            let rev = (*s.get(i + 2)?).to_string();
            let rel = s
                .get(i + 3..)
                .filter(|r| !r.is_empty())
                .map(|r| r.join("/"));
            (rev, rel)
        }
        Some(k) if k.eq_ignore_ascii_case("blobs") => (refs_main(), None),
        Some(_) => return None,
    };
    Some(Layout {
        repo,
        rev,
        rel,
        cache_root: Some(cache_root),
    })
}

// LM Studio keeps `models\<publisher>\<repo>\<file>`, publisher and repo named as on Hugging Face.
pub fn lmstudio_layout(path: &str) -> Option<Layout> {
    let s = segs(path);
    let i = s.windows(2).position(|w| {
        (w[0].eq_ignore_ascii_case(".lmstudio") || w[0].eq_ignore_ascii_case("lm-studio"))
            && w[1].eq_ignore_ascii_case("models")
    })? + 2;
    let publisher = s.get(i)?;
    let repo = s.get(i + 1)?;
    let rel = s
        .get(i + 2..)
        .filter(|r| !r.is_empty())
        .map(|r| r.join("/"));
    Some(Layout {
        repo: format!("{publisher}/{repo}"),
        rev: "main".into(),
        rel,
        cache_root: None,
    })
}

fn ext_of(path: &str) -> String {
    path.rsplit_once('.')
        .filter(|(_, e)| !e.contains('\\'))
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_weight_file(path: &str) -> bool {
    let ext = ext_of(path);
    if WEIGHT_EXT.contains(&ext.as_str()) {
        return true;
    }
    ext == "bin"
        && Path::new(path)
            .parent()
            .is_some_and(|d| d.join("config.json").exists())
}

fn is_model_folder(path: &str) -> bool {
    let p = Path::new(path);
    p.join("config.json").exists()
        && std::fs::read_dir(p).is_ok_and(|rd| {
            rd.filter_map(Result::ok)
                .any(|e| is_weight_file(&e.path().to_string_lossy()))
        })
}

// The stem minus a trailing quant tag: `Qwen3-8B-Q4_K_M` searches as `Qwen3-8B`.
pub fn search_term(file_name: &str) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(s, _)| s)
        .trim_end_matches('_');
    let quant = |t: &str| {
        let l = t.to_ascii_lowercase();
        let rest = l
            .strip_prefix("iq")
            .or_else(|| l.strip_prefix("q"))
            .or_else(|| l.strip_prefix("tq"))
            .or_else(|| l.strip_prefix("mxfp"));
        rest.is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
            || matches!(l.as_str(), "f16" | "f32" | "bf16" | "fp16" | "fp32")
    };
    for (i, c) in stem.char_indices() {
        if (c == '-' || c == '.') && i > 0 && quant(&stem[i + 1..]) {
            return stem[..i].to_string();
        }
    }
    stem.to_string()
}

fn client() -> &'static reqwest::blocking::Client {
    static C: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("hoardini/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client")
    })
}

fn next_link(h: &reqwest::header::HeaderMap) -> Option<String> {
    let v = h.get(reqwest::header::LINK)?.to_str().ok()?;
    v.split(',').find_map(|part| {
        let (url, rel) = part.split_once(';')?;
        rel.contains("rel=\"next\"").then(|| {
            url.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string()
        })
    })
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn repo_path(repo: &str) -> String {
    repo.split('/').map(enc).collect::<Vec<_>>().join("/")
}

type TreeKey = (String, String, String);
type Cached = (Instant, Arc<Vec<TreeEntry>>);

pub struct HubCheck;

// Process-wide, so passes a few minutes apart reuse a repo's tree instead of asking again.
fn trees() -> &'static Mutex<HashMap<TreeKey, Cached>> {
    static T: OnceLock<Mutex<HashMap<TreeKey, Cached>>> = OnceLock::new();
    T.get_or_init(Mutex::default)
}

impl HubCheck {
    pub const NAME: &'static str = "huggingface";

    // None when the repo or revision does not exist (or is private); errors are network trouble.
    fn tree(
        &self,
        base: &str,
        repo: &str,
        rev: &str,
    ) -> anyhow::Result<Option<Arc<Vec<TreeEntry>>>> {
        let key = (base.to_string(), repo.to_ascii_lowercase(), rev.to_string());
        if let Some((at, t)) = trees().lock().get(&key)
            && at.elapsed() < TREE_TTL
        {
            return Ok(Some(t.clone()));
        }
        let mut url = Some(format!(
            "{base}/api/models/{}/tree/{}?recursive=1",
            repo_path(repo),
            enc(rev)
        ));
        let mut all = Vec::new();
        let mut pages = 0;
        while let Some(u) = url.take() {
            pages += 1;
            let resp = client().get(&u).send()?;
            let status = resp.status();
            if status == reqwest::StatusCode::NOT_FOUND
                || status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Ok(None);
            }
            if !status.is_success() {
                anyhow::bail!("Hugging Face answered {status} for {repo}");
            }
            if pages < MAX_TREE_PAGES {
                url = next_link(resp.headers());
            }
            all.extend(parse_tree(&resp.text()?)?);
        }
        let t = Arc::new(all);
        trees().lock().insert(key, (Instant::now(), t.clone()));
        Ok(Some(t))
    }

    fn tree_or_main(
        &self,
        base: &str,
        repo: &str,
        rev: &str,
    ) -> anyhow::Result<Option<Arc<Vec<TreeEntry>>>> {
        match self.tree(base, repo, rev)? {
            Some(t) => Ok(Some(t)),
            None if rev != "main" => self.tree(base, repo, "main"),
            None => Ok(None),
        }
    }

    // `author` narrows to one org, which is how a knowledge hint naming a publisher is used.
    fn search(
        &self,
        base: &str,
        term: Option<&str>,
        author: Option<&str>,
        gguf: bool,
    ) -> anyhow::Result<Vec<String>> {
        let mut q = Vec::new();
        if let Some(a) = author {
            q.push(format!("author={}", enc(a)));
        }
        if let Some(t) = term {
            q.push(format!("search={}", enc(t)));
        }
        q.push("limit=10".into());
        if gguf {
            q.push("filter=gguf".into());
        }
        let url = format!("{base}/api/models?{}", q.join("&"));
        let resp = client().get(&url).send()?;
        if !resp.status().is_success() {
            anyhow::bail!("Hugging Face search answered {}", resp.status());
        }
        parse_search(&resp.text()?)
    }
}

struct Candidate {
    repo: String,
    rev: String,
    rel: Option<String>,
    via: &'static str,
}

fn push(list: &mut Vec<Candidate>, c: Candidate) {
    if !list.iter().any(|x| x.repo.eq_ignore_ascii_case(&c.repo)) {
        list.push(c);
    }
}

fn config_repo(dir: &Path) -> Option<String> {
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).ok()?).ok()?;
    let n = v.get("_name_or_path")?.as_str()?.trim();
    (n.split('/').count() == 2 && !n.contains(':') && !n.starts_with('.')).then(|| n.to_string())
}

fn readme_base_model(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("README.md")).ok()?;
    let front = text.strip_prefix("---")?.split("\n---").next()?;
    let mut lines = front.lines();
    while let Some(l) = lines.next() {
        let Some(rest) = l.trim().strip_prefix("base_model:") else {
            continue;
        };
        let v = rest.trim().trim_matches(['"', '\'']);
        if !v.is_empty() {
            return Some(v.to_string()).filter(|r| r.split('/').count() == 2);
        }
        let next = lines
            .next()?
            .trim()
            .strip_prefix('-')?
            .trim()
            .trim_matches(['"', '\'']);
        return Some(next.to_string()).filter(|r| r.split('/').count() == 2);
    }
    None
}

fn gguf_info(path: &str, len: u64) -> Option<crate::evidence::GgufInfo> {
    let f = hoardini_win::open_read_preserving_atime(Path::new(path)).ok()?;
    crate::evidence::gguf::parse(f, len).ok()
}

fn layout_of(path: &str) -> Option<Layout> {
    hf_cache_layout(path).or_else(|| lmstudio_layout(path))
}

fn is_lmstudio_repo_dir(path: &str) -> bool {
    lmstudio_layout(path).is_some_and(|l| l.rel.is_none())
}

fn is_hf_repo_dir(path: &str) -> bool {
    segs(path).last().is_some_and(|n| n.starts_with("models--"))
}

// Repo, then the header and sibling metadata, then the hint, then a name search; each says only where to look.
fn file_candidates(cx: &Cx<'_>, hub: &HubCheck, s: &Subject) -> anyhow::Result<Vec<Candidate>> {
    let mut list = Vec::new();
    if let Some(l) = layout_of(&s.path) {
        push(
            &mut list,
            Candidate {
                repo: l.repo,
                rev: l.rev,
                rel: l.rel,
                via: "folder layout",
            },
        );
    }
    if ext_of(&s.path) == "gguf"
        && !s.is_placeholder()
        && let Some(g) = gguf_info(&s.path, s.size)
    {
        for r in g.hf_repos() {
            push(
                &mut list,
                Candidate {
                    repo: r,
                    rev: "main".into(),
                    rel: None,
                    via: "GGUF metadata",
                },
            );
        }
    }
    if let Some(dir) = Path::new(&s.path).parent() {
        for r in [config_repo(dir), readme_base_model(dir)]
            .into_iter()
            .flatten()
        {
            push(
                &mut list,
                Candidate {
                    repo: r,
                    rev: "main".into(),
                    rel: None,
                    via: "model card",
                },
            );
        }
    }
    if let Some(h) = s.hint.as_ref().filter(|h| h.names(HubCheck::NAME))
        && let Some(place) = &h.place
        && place.contains('/')
    {
        push(
            &mut list,
            Candidate {
                repo: place.clone(),
                rev: "main".into(),
                rel: None,
                via: "knowledge hint",
            },
        );
    }
    // A content-addressed blob's name is its local hash, which never leaves the machine.
    if list.iter().all(|c| c.rel.is_none()) && hash::content_address(&s.path).is_none() {
        let term = Some(search_term(s.name())).filter(|t| t.len() >= 3);
        let author = s
            .hint
            .as_ref()
            .filter(|h| h.names(HubCheck::NAME))
            .and_then(|h| h.place.as_deref())
            .filter(|p| !p.contains('/'));
        if term.is_some() || author.is_some() {
            let gguf = ext_of(&s.path) == "gguf";
            for r in hub
                .search(&cx.env.hub_base, term.as_deref(), author, gguf)?
                .into_iter()
                .take(MAX_SEARCH_REPOS)
            {
                push(
                    &mut list,
                    Candidate {
                        repo: r,
                        rev: "main".into(),
                        rel: None,
                        via: "name search",
                    },
                );
            }
        }
    }
    Ok(list)
}

fn file_name(p: &str) -> &str {
    p.rsplit(['/', '\\']).next().unwrap_or(p)
}

fn find_file(cx: &Cx<'_>, hub: &HubCheck, s: &Subject) -> anyhow::Result<Find> {
    let cands = file_candidates(cx, hub, s)?;
    let mut searched = Vec::new();
    let blob_sha = hash::content_address(&s.path);
    for c in &cands {
        searched.push(c.repo.clone());
        let Some(tree) = hub.tree_or_main(&cx.env.hub_base, &c.repo, &c.rev)? else {
            continue;
        };
        let hit = tree.iter().find(|e| {
            let named = match (&c.rel, &blob_sha) {
                (Some(rel), _) => e.path == *rel,
                (None, Some(sha)) => e.lfs_oid.as_deref() == Some(sha.as_str()),
                (None, None) => file_name(&e.path).eq_ignore_ascii_case(s.name()),
            };
            named && e.size == s.size
        });
        if let Some(e) = hit {
            return Ok(Find::Copy(Found {
                locator: c.repo.clone(),
                detail: json!({
                    "repo": c.repo, "rev": c.rev, "path": e.path, "lfs_oid": e.lfs_oid,
                    "via": c.via,
                }),
            }));
        }
    }
    Ok(Find::NoCopy(with_note(
        json!({"searched": searched}),
        "not found on Hugging Face",
    )))
}

type LocalFile = (String, std::path::PathBuf, u64);

// `skipped` says why the walk did not see the whole folder; a folder only partly seen is never `identical`.
struct Walk {
    files: Vec<LocalFile>,
    skipped: Option<String>,
}

fn local_files(dir: &Path) -> Walk {
    let mut w = Walk {
        files: Vec::new(),
        skipped: None,
    };
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((d, prefix)) = stack.pop() {
        let Ok(entries) = hoardini_win::list_dir(&d) else {
            w.skipped = Some(format!("{} could not be read", d.display()));
            continue;
        };
        for e in entries {
            let rel = if prefix.is_empty() {
                e.name.clone()
            } else {
                format!("{prefix}/{}", e.name)
            };
            // What `huggingface-cli download --local-dir` keeps beside the files it fetched.
            if e.is_dir() && rel.eq_ignore_ascii_case(".cache/huggingface") {
                continue;
            }
            if e.is_link() || (e.is_dir() && e.is_reparse()) {
                w.skipped = Some(format!("{rel} is a link or placeholder"));
            } else if e.is_dir() {
                stack.push((d.join(&e.name), rel));
            } else {
                w.files.push((rel, d.join(&e.name), e.logical));
                if w.files.len() > DIR_FILE_CAP {
                    w.skipped = Some(format!("more than {DIR_FILE_CAP} files"));
                    return w;
                }
            }
        }
    }
    w
}

fn find_hf_cache_dir(cx: &Cx<'_>, hub: &HubCheck, s: &Subject, l: &Layout) -> anyhow::Result<Find> {
    let root = Path::new(&s.path);
    let blobs: Vec<String> = hoardini_win::list_dir(&root.join("blobs"))
        .map(|v| {
            v.into_iter()
                .filter(|e| !e.is_dir() && !e.name.ends_with(".incomplete"))
                .map(|e| e.name.to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    if blobs.is_empty() {
        return Ok(Find::Skip);
    }
    let mut revs: Vec<String> = hoardini_win::list_dir(&root.join("snapshots"))
        .map(|v| {
            v.into_iter()
                .filter(|e| e.is_dir())
                .map(|e| e.name)
                .collect()
        })
        .unwrap_or_default();
    revs.truncate(3);
    if revs.is_empty() {
        revs.push(l.rev.clone());
    }
    let mut oids: Vec<String> = Vec::new();
    for rev in &revs {
        if let Some(t) = hub.tree_or_main(&cx.env.hub_base, &l.repo, rev)? {
            for e in t.iter() {
                oids.extend(e.lfs_oid.clone());
                oids.extend(e.oid.clone().map(|o| o.to_ascii_lowercase()));
            }
        }
    }
    let missing = blobs.iter().filter(|b| !oids.contains(b)).count();
    let detail = json!({"repo": l.repo, "revs": revs, "files": blobs.len(), "missing": missing});
    if missing == 0 {
        return Ok(Find::Copy(Found {
            locator: l.repo.clone(),
            detail,
        }));
    }
    let note = format!("{missing} of {} files not on Hugging Face", blobs.len());
    Ok(Find::NoCopy(with_note(detail, note)))
}

fn find_model_dir(cx: &Cx<'_>, hub: &HubCheck, s: &Subject) -> anyhow::Result<Find> {
    let dir = Path::new(&s.path);
    let repo = lmstudio_layout(&s.path)
        .map(|l| l.repo)
        .or_else(|| config_repo(dir))
        .or_else(|| {
            s.hint
                .as_ref()
                .and_then(|h| h.place.clone())
                .filter(|p| p.contains('/'))
        });
    let Some(repo) = repo else {
        return Ok(Find::NoCopy(with_note(
            json!({}),
            "no Hugging Face repo named for this folder",
        )));
    };
    let Some(tree) = hub.tree_or_main(&cx.env.hub_base, &repo, "main")? else {
        return Ok(Find::NoCopy(with_note(
            json!({"repo": repo}),
            format!("{repo} is not on Hugging Face"),
        )));
    };
    let walk = local_files(dir);
    if let Some(why) = walk.skipped {
        return Ok(Find::NoCopy(with_note(
            json!({"repo": repo}),
            format!("not every file was compared with {repo}: {why}"),
        )));
    }
    let files = walk.files;
    let unmatched: Vec<&String> = files
        .iter()
        .filter(|(rel, _, size)| !tree.iter().any(|e| e.path == *rel && e.size == *size))
        .map(|(rel, ..)| rel)
        .collect();
    let detail = json!({"repo": repo, "files": files.len(), "unmatched": unmatched.len()});
    if files.is_empty() || !unmatched.is_empty() {
        let note = format!("{} of {} files not on {repo}", unmatched.len(), files.len());
        return Ok(Find::NoCopy(with_note(detail, note)));
    }
    Ok(Find::Copy(Found {
        locator: repo,
        detail,
    }))
}

// The smaller files a folder answer waits on the hash phase for.
pub(crate) fn hash_files(s: &Subject, r: &CopyResult) -> Vec<String> {
    r.detail["hash_files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|rel| format!("{}\\{}", s.path, rel.replace('/', "\\")))
        .collect()
}

// Big files carry their own results; a folder waits until each of them has a settled answer.
pub(crate) fn files_still_pending(cx: &Cx<'_>, s: &Subject, r: &CopyResult) -> bool {
    let big = local_files(Path::new(&s.path))
        .files
        .into_iter()
        .filter(|(_, _, size)| *size >= FILE_MIN)
        .any(|(_, p, _)| {
            Subject::load(cx.conn, &p.to_string_lossy()).is_none_or(|f| {
                current(cx.conn, &f)
                    .iter()
                    .find(|r| r.check == HubCheck::NAME)
                    .is_none_or(|r| r.pending().is_some())
            })
        });
    big || hash_files(s, r)
        .iter()
        .any(|p| Subject::load(cx.conn, p).is_some_and(|f| hash::current(cx.conn, &f).is_none()))
}

fn verify_model_dir(
    cx: &Cx<'_>,
    hub: &HubCheck,
    s: &Subject,
    f: Found,
) -> anyhow::Result<CopyResult> {
    let repo = f.locator.clone();
    let tree = hub
        .tree_or_main(&cx.env.hub_base, &repo, "main")?
        .unwrap_or_default();
    let mut waiting = false;
    let mut to_hash: Vec<String> = Vec::new();
    for (rel, p, size) in local_files(Path::new(&s.path)).files {
        let Some(e) = tree.iter().find(|e| e.path == rel) else {
            continue;
        };
        let Some(oid) = &e.lfs_oid else {
            continue;
        };
        let Some(sub) = Subject::load(cx.conn, &p.to_string_lossy()) else {
            waiting = true;
            continue;
        };
        let sha = if size >= FILE_MIN {
            match current(cx.conn, &sub)
                .into_iter()
                .find(|r| r.check == HubCheck::NAME)
            {
                Some(r) if r.pending().is_none() && r.confidence == Confidence::Identical => {
                    continue;
                }
                Some(r) if r.pending().is_none() => {
                    let note = format!("{rel} differs from {repo} on Hugging Face");
                    return Ok(CopyResult::new(
                        HubCheck::NAME,
                        None,
                        Confidence::None,
                        with_note(f.detail, note),
                    ));
                }
                _ => {
                    waiting = true;
                    continue;
                }
            }
        } else if let Some(h) = hash::current(cx.conn, &sub) {
            h
        } else {
            // Hashed in the pass's hash phase, which stops when nobody is looking.
            waiting = true;
            to_hash.push(rel);
            continue;
        };
        if sha != *oid {
            let note = format!("{rel} differs from {repo} on Hugging Face");
            return Ok(CopyResult::new(
                HubCheck::NAME,
                None,
                Confidence::None,
                with_note(f.detail, note),
            ));
        }
    }
    if waiting {
        let note = format!("same files and sizes as {repo} on Hugging Face; checking the bytes");
        let mut detail = pending(with_note(f.detail, note), "files");
        if !to_hash.is_empty() {
            detail["hash_files"] = json!(to_hash);
        }
        return Ok(CopyResult::new(
            HubCheck::NAME,
            Some(repo),
            Confidence::SameNameAndSize,
            detail,
        ));
    }
    let note = format!("identical to {repo} on Hugging Face");
    Ok(CopyResult::new(
        HubCheck::NAME,
        Some(repo),
        Confidence::Identical,
        with_note(f.detail, note),
    ))
}

impl CopyCheck for HubCheck {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn service(&self) -> Option<Service> {
        Some(Service::Huggingface)
    }

    fn applies(&self, _cx: &Cx<'_>, s: &Subject) -> bool {
        if s.hint.as_ref().is_some_and(|h| h.names(Self::NAME)) {
            return true;
        }
        if s.dir {
            return is_hf_repo_dir(&s.path)
                || is_lmstudio_repo_dir(&s.path)
                || is_model_folder(&s.path);
        }
        layout_of(&s.path).is_some() || is_weight_file(&s.path)
    }

    fn covers_dir(&self, _cx: &Cx<'_>, path: &str) -> bool {
        is_hf_repo_dir(path) || is_lmstudio_repo_dir(path)
    }

    fn still_valid(&self, _cx: &Cx<'_>, s: &Subject, r: &CopyResult) -> bool {
        same_contents(s, r)
    }

    fn find(&self, cx: &Cx<'_>, s: &Subject) -> anyhow::Result<Find> {
        if !s.dir {
            return find_file(cx, self, s);
        }
        match hf_cache_layout(&s.path).filter(|_| is_hf_repo_dir(&s.path)) {
            Some(l) => find_hf_cache_dir(cx, self, s, &l),
            None => find_model_dir(cx, self, s),
        }
    }

    fn verify(&self, cx: &Cx<'_>, s: &Subject, f: Found) -> anyhow::Result<CopyResult> {
        let repo = f.locator.clone();
        if s.dir {
            if is_hf_repo_dir(&s.path) {
                let note = format!("identical to {repo} on Hugging Face");
                return Ok(with_fingerprint(
                    CopyResult::new(
                        Self::NAME,
                        Some(repo),
                        Confidence::Identical,
                        with_note(f.detail, note),
                    ),
                    s,
                ));
            }
            return Ok(with_fingerprint(verify_model_dir(cx, self, s, f)?, s));
        }
        let remote = f.detail["lfs_oid"].as_str().map(str::to_string);
        let local = hash::current(cx.conn, s);
        let (conf, note, detail) = match (remote, local) {
            (Some(r), Some(l)) if r == l => (
                Confidence::Identical,
                format!("identical to {repo} on Hugging Face"),
                f.detail,
            ),
            (Some(_), Some(_)) => (
                Confidence::None,
                format!("a file with this name and size on {repo} has different bytes"),
                f.detail,
            ),
            (Some(_), None) => (
                Confidence::SameNameAndSize,
                format!("same name and size as {repo} on Hugging Face; checking the bytes"),
                pending(f.detail, "hash"),
            ),
            (None, _) => (
                Confidence::SameNameAndSize,
                format!("same name and size as {repo} on Hugging Face"),
                f.detail,
            ),
        };
        let locator = (conf != Confidence::None).then_some(repo);
        Ok(CopyResult::new(
            Self::NAME,
            locator,
            conf,
            with_note(detail, note),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_cache_paths_name_their_repo_revision_and_file() {
        let l = hf_cache_layout(r"C:\Users\u\.cache\huggingface\hub\models--Qwen--Qwen3-8B\snapshots\abc123\sub\model.safetensors").unwrap();
        assert_eq!(
            (l.repo.as_str(), l.rev.as_str(), l.rel.as_deref()),
            ("Qwen/Qwen3-8B", "abc123", Some("sub/model.safetensors"))
        );
        let d = hf_cache_layout(r"C:\Users\u\.cache\huggingface\hub\models--a--b-c--d").unwrap();
        assert_eq!((d.repo.as_str(), d.rel), ("a/b-c--d", None));
        assert!(
            hf_cache_layout(r"C:\Users\u\.cache\huggingface\hub\models--x--y\refs\main").is_none()
        );
    }

    #[test]
    fn lmstudio_paths_name_publisher_and_repo() {
        let l =
            lmstudio_layout(r"C:\Users\u\.lmstudio\models\lmstudio-community\SmolLM2-GGUF\x.gguf")
                .unwrap();
        assert_eq!(
            (l.repo.as_str(), l.rel.as_deref()),
            ("lmstudio-community/SmolLM2-GGUF", Some("x.gguf"))
        );
        assert_eq!(
            lmstudio_layout(r"C:\Users\u\.lmstudio\models\pub\repo")
                .unwrap()
                .rel,
            None
        );
        assert!(lmstudio_layout(r"C:\Users\u\.lmstudio\models\pub").is_none());
    }

    #[test]
    fn search_terms_drop_the_quant_tag() {
        assert_eq!(
            search_term("SmolLM2-135M-Instruct-Q4_K_M.gguf"),
            "SmolLM2-135M-Instruct"
        );
        assert_eq!(
            search_term("gemma-4-26B-A4B-it-assistant.Q8_0.gguf"),
            "gemma-4-26B-A4B-it-assistant"
        );
        assert_eq!(
            search_term("lfm2.5-350m-base-dillards-q4_k_m.gguf"),
            "lfm2.5-350m-base-dillards"
        );
        assert_eq!(
            search_term("persona-cyrus-2.6B-Q4_K_M_.gguf"),
            "persona-cyrus-2.6B"
        );
        assert_eq!(search_term("Qwen3-8B-IQ4_XS.gguf"), "Qwen3-8B");
        assert_eq!(search_term("model.safetensors"), "model");
    }

    #[test]
    fn readme_front_matter_names_the_base_model() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "---\nlicense: mit\nbase_model:\n- org/base-7b\n---\n# x",
        )
        .unwrap();
        assert_eq!(
            readme_base_model(dir.path()).as_deref(),
            Some("org/base-7b")
        );
        std::fs::write(
            dir.path().join("README.md"),
            "---\nbase_model: org/other\n---\n",
        )
        .unwrap();
        assert_eq!(readme_base_model(dir.path()).as_deref(), Some("org/other"));
    }
}

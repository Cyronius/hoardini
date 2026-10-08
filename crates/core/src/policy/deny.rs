use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use super::PolicyEnv;
use crate::index::{is_ancestor_path, normalize_path};
use crate::memory;

const SYSTEM_DIRS: &[&str] = &["System Volume Information", "$Recycle.Bin"];
const SYSTEM_FILES: &[&str] = &["pagefile.sys", "hiberfil.sys", "swapfile.sys"];
const WALK_BUDGET: usize = 20_000;
const WALK_DEPTH: usize = 7;
const PROTECT_NODE_BUDGET: usize = 100_000;

pub(crate) fn same_or_under(ancestor: &str, path: &str) -> bool {
    ancestor.eq_ignore_ascii_case(path) || is_ancestor_path(ancestor, path)
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    hay.to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

// Drive paths have one root component ("C:"), UNC paths two (server, share).
fn split_root(p: &str) -> (usize, Vec<&str>) {
    if let Some(rest) = p.strip_prefix(r"\\") {
        (2, rest.split('\\').filter(|c| !c.is_empty()).collect())
    } else {
        (1, p.split('\\').filter(|c| !c.is_empty()).collect())
    }
}

pub fn is_volume_root(path: &str) -> bool {
    let p = normalize_path(path);
    let (n, comps) = split_root(&p);
    comps.len() <= n
}

pub fn deny_reason(
    conn: &Connection,
    env: &PolicyEnv,
    path: &str,
) -> anyhow::Result<Option<String>> {
    let p = normalize_path(path);
    if let Some(r) = fixed_reason(env, &p) {
        return Ok(Some(r));
    }
    if let Some(r) = local_share_reason(&p) {
        return Ok(Some(r));
    }
    if let Some(r) = protect_reason(conn, env, &p)? {
        return Ok(Some(r));
    }
    model_reason(conn, env, &p)
}

fn fixed_reason(env: &PolicyEnv, p: &str) -> Option<String> {
    let (n, comps) = split_root(p);
    if comps.len() <= n {
        return Some(format!("{p} is a volume root"));
    }
    let under = &comps[n..];
    if let Some(c) = under
        .iter()
        .find(|c| SYSTEM_DIRS.iter().any(|s| c.eq_ignore_ascii_case(s)))
    {
        return Some(format!("{p} is in the Windows system area {c}"));
    }
    if under[0].to_ascii_lowercase().starts_with("program files") {
        return Some(format!(
            "{p} is in {}, where installed programs live",
            under[0]
        ));
    }
    if let Some(last) = under.last()
        && SYSTEM_FILES.iter().any(|s| last.eq_ignore_ascii_case(s))
    {
        return Some(format!("{p} is a Windows paging or hibernation file"));
    }
    let mut subtree: Vec<(&str, String)> = vec![
        ("the Windows folder", env.system_root.clone()),
        (
            "ProgramData\\Microsoft",
            format!("{}\\Microsoft", env.program_data),
        ),
        ("Hoardini's own data folder", env.data_dir.clone()),
    ];
    for pf in env.program_files() {
        subtree.push(("Program Files", pf));
    }
    for (label, loc) in &subtree {
        if same_or_under(loc, p) {
            return Some(format!("{p} is inside {label} ({loc})"));
        }
        if is_ancestor_path(p, loc) {
            return Some(format!("{p} contains {label} ({loc})"));
        }
    }
    let profile = &env.profile;
    let exact = [
        ("your profile folder", profile.clone()),
        ("the AppData folder", format!("{profile}\\AppData")),
        ("AppData\\Local", format!("{profile}\\AppData\\Local")),
        ("AppData\\Roaming", format!("{profile}\\AppData\\Roaming")),
        ("AppData\\LocalLow", format!("{profile}\\AppData\\LocalLow")),
    ];
    for (label, loc) in &exact {
        if loc.eq_ignore_ascii_case(p) {
            return Some(format!("{p} is {label}"));
        }
        if is_ancestor_path(p, loc) {
            return Some(format!("{p} contains {label} ({loc})"));
        }
    }
    None
}

// \\localhost\C$\... reaches a local disk under a name no rule above knows; the share reports that volume's serial.
fn local_share_reason(p: &str) -> Option<String> {
    if !p.starts_with(r"\\") {
        return None;
    }
    let serial = hoardini_win::volume_info(Path::new(p)).ok()?.serial;
    let local = hoardini_win::fixed_volumes()
        .ok()?
        .into_iter()
        .filter_map(|v| hoardini_win::volume_info(&v).ok())
        .find(|v| v.serial == serial)?;
    Some(format!(
        "{p} is a network path back to this PC ({}); use the local path",
        local.root
    ))
}

fn glob_literal_prefix(key: &str) -> String {
    key.split('\\')
        .take_while(|c| !c.contains(['*', '?', '[', '{']))
        .collect::<Vec<_>>()
        .join("\\")
}

enum Descendant {
    Hit(String),
    None,
    Unknown,
}

// Index nodes are folders and big files, so a folder match covers everything inside it via a probe child.
fn glob_descendant(conn: &Connection, p: &str, glob: &str) -> Descendant {
    let Ok(md) = std::fs::symlink_metadata(p) else {
        return Descendant::None;
    };
    if !md.is_dir() {
        return Descendant::None;
    }
    let Ok(matcher) = memory::compile_glob(glob) else {
        return Descendant::None;
    };
    let Ok(Some(root)) = crate::index::node_id_by_path(conn, p) else {
        return Descendant::Unknown;
    };
    let Ok(mut st) = conn.prepare_cached("SELECT id, name, kind FROM node WHERE parent_id = ?1")
    else {
        return Descendant::Unknown;
    };
    let hits = |path: &str, is_dir: bool| {
        let slashed = path.replace('\\', "/");
        matcher.is_match(&slashed) || (is_dir && matcher.is_match(format!("{slashed}/_")))
    };
    let mut stack = vec![(root, p.to_string())];
    let mut seen = 0usize;
    while let Some((id, path)) = stack.pop() {
        let Ok(rows) = st.query_map([id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        }) else {
            return Descendant::Unknown;
        };
        for row in rows {
            let Ok((cid, name, kind)) = row else {
                return Descendant::Unknown;
            };
            seen += 1;
            if seen > PROTECT_NODE_BUDGET {
                return Descendant::Unknown;
            }
            let child = crate::index::join_path(&path, &name);
            let is_dir = kind == crate::index::KIND_DIR;
            if hits(&child, is_dir) {
                return Descendant::Hit(child);
            }
            if is_dir {
                stack.push((cid, child));
            }
        }
    }
    Descendant::None
}

fn protect_reason(conn: &Connection, env: &PolicyEnv, p: &str) -> anyhow::Result<Option<String>> {
    if let Some(m) = memory::protect_covering_with(conn, p, &env.profile)? {
        return Ok(Some(format!("{p} is protected by a memory: {}", m.text)));
    }
    // A protected path inside the proposed one would go with it.
    let mut st = conn.prepare_cached(
        "SELECT scope, key, text FROM memory WHERE kind = 'protect' AND scope IN ('path', 'glob') ORDER BY id",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (scope, key, text) in rows {
        let key = env.expand(&key);
        let target = if scope == "glob" {
            glob_literal_prefix(&key)
        } else {
            key.clone()
        };
        let target = if target.is_empty() {
            target
        } else {
            normalize_path(&target)
        };
        if !target.is_empty() && same_or_under(p, &target) {
            return Ok(Some(format!(
                "{p} contains the protected path {target}: {text}"
            )));
        }
        // A wildcard before the first literal folder can still match deeper inside p.
        if scope == "glob" && (target.is_empty() || is_ancestor_path(&target, p)) {
            match glob_descendant(conn, p, &key) {
                Descendant::Hit(hit) => {
                    return Ok(Some(format!(
                        "{p} contains {hit}, which the protect pattern {key} covers: {text}"
                    )));
                }
                Descendant::Unknown => {
                    return Ok(Some(format!(
                        "{p} may contain something the protect pattern {key} covers ({text}); the index cannot confirm it does not, so review it first"
                    )));
                }
                Descendant::None => {}
            }
        }
    }
    Ok(None)
}

struct ActiveModel {
    id: String,
    candidates: Vec<String>,
    blobs: Vec<String>,
}

impl ActiveModel {
    fn matches(&self, rel: &str) -> bool {
        if self.candidates.iter().any(|c| contains_ci(rel, c)) {
            return true;
        }
        let last = rel.rsplit('\\').next().unwrap_or(rel);
        self.blobs.iter().any(|b| last.eq_ignore_ascii_case(b))
    }
}

fn is_local_url(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split('/').next().unwrap_or("");
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1" | "0.0.0.0"
    )
}

fn candidates_for(model: &str) -> Vec<String> {
    let base = model.trim().split('@').next().unwrap_or("").trim();
    let mut out = Vec::new();
    if let Some((name, tag)) = base.split_once(':') {
        // Ollama keeps manifests at library\<name>\<tag>; a bare name would match every tag.
        let name = name.rsplit('/').next().unwrap_or(name);
        out.push(format!("{name}\\{tag}"));
    } else {
        out.push(base.replace('/', "\\"));
        if let Some(last) = base.rsplit('/').next() {
            out.push(last.to_string());
        }
        if base.contains('/') {
            out.push(format!("models--{}", base.replace('/', "--")));
        }
    }
    out.retain(|c| c.len() >= 3);
    out.dedup();
    out
}

fn active_local_model(conn: &Connection, env: &PolicyEnv) -> anyhow::Result<Option<ActiveModel>> {
    let mut json: Option<String> = conn
        .query_row(
            "SELECT config_json FROM provider WHERE is_active = 1 ORDER BY created_ms DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if json.is_none()
        && let Some(id) = crate::store::get_meta(conn, "active_provider")?
    {
        json = conn
            .query_row(
                "SELECT config_json FROM provider WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
    }
    let Some(json) = json else {
        return Ok(None);
    };
    let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&json) else {
        return Ok(None);
    };
    let base_url = cfg.get("base_url").and_then(|v| v.as_str()).unwrap_or("");
    let model = cfg
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if model.is_empty() || !is_local_url(base_url) {
        return Ok(None);
    }
    let candidates = candidates_for(model);
    if candidates.is_empty() {
        return Ok(None);
    }
    let mut m = ActiveModel {
        id: model.to_string(),
        candidates,
        blobs: Vec::new(),
    };
    for store in ollama_stores(env) {
        m.blobs.extend(ollama_blobs(&store, &m));
    }
    Ok(Some(m))
}

fn ollama_stores(env: &PolicyEnv) -> Vec<String> {
    let mut out = vec![format!("{}\\.ollama\\models", env.profile)];
    if let Some(v) = env.var("ollama_models") {
        out.push(v.to_string());
    }
    out
}

fn model_stores(env: &PolicyEnv) -> Vec<String> {
    let p = &env.profile;
    let mut out = vec![
        format!("{p}\\.lmstudio\\models"),
        format!("{p}\\.cache\\lm-studio\\models"),
        format!("{p}\\.cache\\huggingface\\hub"),
    ];
    out.extend(ollama_stores(env));
    if let Some(v) = env.var("hf_home") {
        out.push(format!("{v}\\hub"));
    }
    if let Some(v) = env.var("huggingface_hub_cache") {
        out.push(v.to_string());
    }
    out.into_iter().map(|s| normalize_path(&s)).collect()
}

// Ollama names blobs by digest, so the manifest is the only link from model name to file.
fn ollama_blobs(store: &str, m: &ActiveModel) -> Vec<String> {
    let manifests = format!("{store}\\manifests");
    let mut out = Vec::new();
    walk(
        Path::new(&manifests),
        WALK_DEPTH,
        WALK_BUDGET,
        &mut |path, is_dir| {
            if is_dir {
                return false;
            }
            let rel = path.strip_prefix(&manifests).unwrap_or(path);
            if !m.candidates.iter().any(|c| contains_ci(rel, c)) {
                return false;
            }
            let mut text = String::new();
            let read = hoardini_win::open_read_preserving_atime(Path::new(path))
                .and_then(|mut f| std::io::Read::read_to_string(&mut f, &mut text));
            if read.is_err() {
                return false;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                return false;
            };
            let layers = v.get("layers").and_then(|l| l.as_array()).cloned();
            let digests = layers
                .unwrap_or_default()
                .into_iter()
                .chain(v.get("config").cloned())
                .filter_map(|l| l.get("digest")?.as_str().map(str::to_string));
            out.extend(digests.map(|d| d.replace(':', "-")));
            false
        },
    );
    out
}

fn walk(
    root: &Path,
    max_depth: usize,
    budget: usize,
    f: &mut dyn FnMut(&str, bool) -> bool,
) -> bool {
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            seen += 1;
            if seen > budget {
                return false;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            let s = path.to_string_lossy();
            let is_dir = ft.is_dir() && !ft.is_symlink();
            if f(&s, is_dir) {
                return true;
            }
            if is_dir && depth + 1 < max_depth {
                stack.push((path, depth + 1));
            }
        }
    }
    false
}

fn store_holds_model(dir: &str, store: &str, m: &ActiveModel) -> bool {
    walk(Path::new(dir), WALK_DEPTH, WALK_BUDGET, &mut |path, _| {
        let rel = path.get(store.len() + 1..).unwrap_or("");
        m.matches(rel)
    })
}

fn model_reason(conn: &Connection, env: &PolicyEnv, p: &str) -> anyhow::Result<Option<String>> {
    let Some(m) = active_local_model(conn, env)? else {
        return Ok(None);
    };
    let reason = || {
        Some(format!(
            "{p} holds files of the active local model ({}); switch the model first",
            m.id
        ))
    };
    for store in model_stores(env) {
        if is_ancestor_path(&store, p) {
            let rel = &p[store.len() + 1..];
            if m.matches(rel) || store_holds_model(p, &store, &m) {
                return Ok(reason());
            }
        } else if same_or_under(p, &store) && store_holds_model(&store, &store, &m) {
            return Ok(reason());
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_urls_and_candidates() {
        assert!(is_local_url("http://localhost:1234/v1"));
        assert!(is_local_url("http://127.0.0.1:11434/v1"));
        assert!(is_local_url("http://[::1]:8080/v1"));
        assert!(!is_local_url("https://api.openai.com/v1"));
        assert_eq!(
            candidates_for("qwen/qwen3.8-27b@q8_0"),
            vec![
                r"qwen\qwen3.8-27b".to_string(),
                "qwen3.8-27b".into(),
                "models--qwen--qwen3.8-27b".into()
            ]
        );
        assert_eq!(candidates_for("llama3:8b"), vec![r"llama3\8b".to_string()]);
    }

    #[test]
    fn volume_roots() {
        assert!(is_volume_root(r"C:\"));
        assert!(is_volume_root("d:"));
        assert!(is_volume_root(r"\\nas\share"));
        assert!(!is_volume_root(r"\\nas\share\x"));
        assert!(!is_volume_root(r"C:\x"));
    }
}

use std::collections::HashMap;
use std::sync::OnceLock;

use globset::{GlobBuilder, GlobMatcher};
use serde::{Deserialize, Serialize};

use crate::policy::Verdict;

const SOURCES: &[(&str, &str)] = &[
    (
        "models.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../catalog/models.toml"
        )),
    ),
    (
        "caches.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../catalog/caches.toml"
        )),
    ),
    (
        "windows.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../catalog/windows.toml"
        )),
    ),
    (
        "vm_games.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../catalog/vm_games.toml"
        )),
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    ModelStore,
    Cache,
    PackageCache,
    BrowserCache,
    BuildOutput,
    WindowsLeftover,
    System,
    Downloads,
    VmDisk,
    GameLibrary,
    Temp,
    Logs,
    CrashDumps,
    InstallerCache,
    AppData,
    DevTool,
    UserData,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub app: String,
    pub category: Category,
    pub patterns: Vec<String>,
    pub regenerable: bool,
    pub verdict_hint: Verdict,
    #[serde(default)]
    pub cap: Option<Verdict>,
    #[serde(default)]
    pub shared_blobs: bool,
    #[serde(default)]
    pub safe_unit: Option<String>,
    pub notes: String,
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    entry: Vec<Entry>,
}

// What `Evidence.catalog` carries: the entry without its patterns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogMatch {
    pub id: String,
    pub name: String,
    pub app: String,
    pub category: Category,
    pub regenerable: bool,
    pub verdict_hint: Verdict,
    pub cap: Option<Verdict>,
    pub shared_blobs: bool,
    pub safe_unit: Option<String>,
    pub notes: String,
}

impl From<&Entry> for CatalogMatch {
    fn from(e: &Entry) -> Self {
        CatalogMatch {
            id: e.id.clone(),
            name: e.name.clone(),
            app: e.app.clone(),
            category: e.category,
            regenerable: e.regenerable,
            verdict_hint: e.verdict_hint,
            cap: e.cap,
            shared_blobs: e.shared_blobs,
            safe_unit: e.safe_unit.clone(),
            notes: e.notes.clone(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Env {
    profile: String,
    vars: HashMap<String, String>,
}

impl Env {
    pub fn new(profile: impl Into<String>) -> Env {
        Env {
            profile: profile.into(),
            vars: HashMap::new(),
        }
    }

    pub fn set(mut self, name: &str, value: impl Into<String>) -> Env {
        self.vars.insert(name.to_ascii_lowercase(), value.into());
        self
    }

    pub fn system() -> Env {
        let profile = crate::paths::profile_dir().to_string_lossy().into_owned();
        let mut env = Env::new(profile);
        for (k, v) in std::env::vars() {
            env.vars.insert(k.to_ascii_lowercase(), v);
        }
        env.vars
            .entry("systemdrive".into())
            .or_insert_with(|| "C:".into());
        let drive = env.vars["systemdrive"].clone();
        env.vars
            .entry("systemroot".into())
            .or_insert_with(|| format!("{drive}\\Windows"));
        env.vars
            .entry("programdata".into())
            .or_insert_with(|| format!("{drive}\\ProgramData"));
        env.vars
            .entry("programfiles".into())
            .or_insert_with(|| format!("{drive}\\Program Files"));
        env.vars
            .entry("programfiles(x86)".into())
            .or_insert_with(|| format!("{drive}\\Program Files (x86)"));
        let profile = env.profile.clone();
        env.vars
            .entry("localappdata".into())
            .or_insert_with(|| format!("{profile}\\AppData\\Local"));
        env.vars
            .entry("appdata".into())
            .or_insert_with(|| format!("{profile}\\AppData\\Roaming"));
        let local = env.vars["localappdata"].clone();
        env.vars
            .entry("temp".into())
            .or_insert_with(|| format!("{local}\\Temp"));
        env
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    // None when a referenced variable is unset, so the pattern is dropped instead of matching wrongly.
    fn expand(&self, pattern: &str) -> Option<String> {
        let mut out = String::new();
        let mut rest = pattern;
        if rest == "~" || rest.starts_with("~\\") || rest.starts_with("~/") {
            out.push_str(&escape_glob(self.profile.trim_end_matches(['\\', '/'])));
            rest = &rest[1..];
        }
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let end = after.find('%')?;
            let value = self.vars.get(&after[..end].to_ascii_lowercase())?;
            out.push_str(&escape_glob(value.trim_end_matches(['\\', '/'])));
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Some(out)
    }
}

fn escape_glob(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}') {
            out.push('[');
            out.push(c);
            out.push(']');
        } else {
            out.push(c);
        }
    }
    out
}

pub fn normalize_path(path: &str) -> String {
    let p = path.strip_prefix(r"\\?\").unwrap_or(path);
    let mut s = p.replace('\\', "/");
    while s.len() > 1 && s.ends_with('/') && !s.ends_with(":/") {
        s.pop();
    }
    if s.ends_with(":/") {
        s.pop();
    }
    s
}

struct Compiled {
    entry: usize,
    matcher: GlobMatcher,
    literal_len: usize,
}

pub struct Catalog {
    entries: Vec<Entry>,
    compiled: Vec<Compiled>,
}

impl Catalog {
    pub fn embedded() -> &'static Catalog {
        static CAT: OnceLock<Catalog> = OnceLock::new();
        CAT.get_or_init(|| {
            Catalog::from_sources(SOURCES, &Env::system()).expect("embedded catalog is valid")
        })
    }

    pub fn with_env(env: &Env) -> anyhow::Result<Catalog> {
        Catalog::from_sources(SOURCES, env)
    }

    pub fn from_toml(toml_src: &str, env: &Env) -> anyhow::Result<Catalog> {
        Catalog::from_sources(&[("inline", toml_src)], env)
    }

    fn from_sources(sources: &[(&str, &str)], env: &Env) -> anyhow::Result<Catalog> {
        let mut entries: Vec<Entry> = Vec::new();
        for (name, src) in sources {
            let file: File =
                toml::from_str(src).map_err(|e| anyhow::anyhow!("catalog {name}: {e}"))?;
            entries.extend(file.entry);
        }
        let mut seen = std::collections::HashSet::new();
        for e in &entries {
            if !seen.insert(e.id.clone()) {
                anyhow::bail!("duplicate catalog id {}", e.id);
            }
        }
        let mut compiled = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            for pat in &e.patterns {
                let Some(expanded) = env.expand(pat) else {
                    continue;
                };
                let norm = expanded.replace('\\', "/");
                let glob = GlobBuilder::new(&norm)
                    .case_insensitive(true)
                    .literal_separator(true)
                    .backslash_escape(false)
                    .build()
                    .map_err(|err| anyhow::anyhow!("catalog {} pattern {pat}: {err}", e.id))?;
                compiled.push(Compiled {
                    entry: i,
                    matcher: glob.compile_matcher(),
                    literal_len: norm.chars().filter(|c| !matches!(c, '*' | '?')).count(),
                });
            }
        }
        Ok(Catalog { entries, compiled })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    // Deepest matching ancestor-or-self wins; ties go to the longer literal pattern, then file order.
    pub fn lookup(&self, abs_path: &str) -> Option<&Entry> {
        let path = normalize_path(abs_path);
        let mut best: Option<(usize, usize, usize)> = None;
        let mut prefix = path.as_str();
        let mut depth = path.matches('/').count() + 1;
        loop {
            for c in &self.compiled {
                if c.matcher.is_match(prefix) {
                    let key = (depth, c.literal_len, usize::MAX - c.entry);
                    if best.is_none_or(|b| key > b) {
                        best = Some(key);
                    }
                }
            }
            if best.is_some() {
                break;
            }
            match prefix.rfind('/') {
                Some(i) if i > 0 => {
                    prefix = &prefix[..i];
                    depth -= 1;
                }
                _ => break,
            }
        }
        best.map(|(_, _, inv)| &self.entries[usize::MAX - inv])
    }

    pub fn lookup_match(&self, abs_path: &str) -> Option<CatalogMatch> {
        self.lookup(abs_path).map(CatalogMatch::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_env() -> Env {
        Env::new(r"C:\Users\tester")
            .set("LOCALAPPDATA", r"C:\Users\tester\AppData\Local")
            .set("APPDATA", r"C:\Users\tester\AppData\Roaming")
            .set("TEMP", r"C:\Users\tester\AppData\Local\Temp")
            .set("SystemRoot", r"C:\Windows")
            .set("SystemDrive", "C:")
            .set("ProgramData", r"C:\ProgramData")
            .set("ProgramFiles", r"C:\Program Files")
            .set("ProgramFiles(x86)", r"C:\Program Files (x86)")
    }

    fn cat() -> Catalog {
        Catalog::with_env(&test_env()).unwrap()
    }

    #[test]
    fn embedded_catalog_parses_with_unique_ids_and_notes() {
        let c = cat();
        assert!(c.entries().len() >= 80, "{}", c.entries().len());
        for e in c.entries() {
            assert!(!e.notes.trim().is_empty(), "{}", e.id);
            assert!(!e.patterns.is_empty(), "{}", e.id);
        }
        assert!(Catalog::embedded().entries().len() == c.entries().len());
    }

    #[test]
    fn deepest_match_wins() {
        let c = cat();
        let hit = |p: &str| c.lookup(p).map(|e| e.id.as_str());
        assert_eq!(
            hit(r"C:\Users\tester\.cache\huggingface\hub\models--a--b\blobs\abc"),
            Some("hf-hub-blobs")
        );
        assert_eq!(
            hit(r"c:\users\TESTER\.cache\huggingface\hub\models--a--b\snapshots\1\m.gguf"),
            Some("hf-hub-cache")
        );
        assert_eq!(
            hit(r"C:\Windows\SoftwareDistribution\Download\x"),
            Some("wu-download")
        );
        assert_eq!(hit(r"C:\Windows\System32\drivers"), Some("windows-dir"));
        assert_eq!(hit(r"C:\hiberfil.sys"), Some("hiberfil"));
        assert_eq!(hit(r"D:\nothing\here"), None);
    }

    #[test]
    fn any_dir_named_patterns_match_at_any_depth() {
        let c = cat();
        let e = c
            .lookup(r"C:\code\app\web\node_modules\left-pad\index.js")
            .unwrap();
        assert_eq!(e.id, "node-modules");
        assert_eq!(c.lookup(r"E:\work\x\target").unwrap().id, "rust-target");
        assert_eq!(c.lookup(r"D:\models").unwrap().id, "models-root");
    }

    #[test]
    fn shared_blob_stores_carry_review_cap() {
        let c = cat();
        let e = c
            .lookup(r"C:\Users\tester\.ollama\models\blobs\sha256-1")
            .unwrap();
        assert!(e.shared_blobs);
        assert_eq!(e.cap, Some(Verdict::Review));
        assert!(e.safe_unit.is_some());
    }

    #[test]
    fn set_variable_enables_pattern_and_unset_drops_it() {
        let path = r"G:\hf\hub\models--x\blobs\1";
        assert!(cat().lookup(path).is_none());
        let c = Catalog::with_env(&test_env().set("HF_HOME", r"G:\hf")).unwrap();
        assert_eq!(c.lookup(path).unwrap().id, "hf-hub-blobs");
    }
}

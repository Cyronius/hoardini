use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Confidence, CopyCheck, CopyResult, Cx, Find, Found, Subject, with_note};
use crate::catalog::{CatalogMatch, Category};
use crate::policy::Verdict;

// Launchers and package managers say they can reinstall; nothing compares bytes.
pub struct CatalogCheck;

impl CatalogCheck {
    pub const NAME: &'static str = "catalog";
}

fn claimable(m: &CatalogMatch) -> bool {
    matches!(
        m.category,
        Category::GameLibrary
            | Category::PackageCache
            | Category::DevTool
            | Category::InstallerCache
            | Category::BuildOutput
    ) && (m.regenerable || m.verdict_hint == Verdict::Redownload)
}

impl CopyCheck for CatalogCheck {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn applies(&self, cx: &Cx<'_>, s: &Subject) -> bool {
        cx.env
            .catalog()
            .lookup_match(&s.path)
            .is_some_and(|m| claimable(&m))
    }

    fn covers_dir(&self, cx: &Cx<'_>, path: &str) -> bool {
        let parent = Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned());
        let here = cx.env.catalog().lookup_match(path);
        here.as_ref().is_some_and(claimable)
            && parent.is_some_and(|p| {
                cx.env.catalog().lookup_match(&p).map(|m| m.id) == here.map(|m| m.id)
            })
    }

    fn find(&self, cx: &Cx<'_>, s: &Subject) -> anyhow::Result<Find> {
        let Some(m) = cx.env.catalog().lookup_match(&s.path).filter(claimable) else {
            return Ok(Find::Skip);
        };
        Ok(Find::Copy(Found {
            locator: m.app.clone(),
            detail: json!({"catalog": m.id, "name": m.name}),
        }))
    }

    fn verify(&self, _cx: &Cx<'_>, _s: &Subject, f: Found) -> anyhow::Result<CopyResult> {
        let note = format!("{} can reinstall or rebuild it", f.locator);
        Ok(CopyResult::new(
            Self::NAME,
            Some(f.locator),
            Confidence::Claimed,
            with_note(f.detail, note),
        ))
    }
}

// Ollama blobs are `sha256-<hex>`; a manifest naming that digest means `ollama pull` brings it back.
pub struct OllamaCheck;

impl OllamaCheck {
    pub const NAME: &'static str = "ollama";
}

fn models_dir(cx: &Cx<'_>) -> PathBuf {
    match &cx.env.ollama_models {
        Some(d) => PathBuf::from(d),
        None => Path::new(&cx.env.profile).join(".ollama").join("models"),
    }
}

fn blob_digest(cx: &Cx<'_>, path: &str) -> Option<String> {
    let p = Path::new(path);
    let blobs = models_dir(cx).join("blobs");
    if !p
        .parent()?
        .to_string_lossy()
        .eq_ignore_ascii_case(&blobs.to_string_lossy())
    {
        return None;
    }
    let hex = p.file_name()?.to_str()?.strip_prefix("sha256-")?;
    Some(format!("sha256:{}", hex.to_ascii_lowercase()))
}

// `manifests\<registry>\<namespace>\<model>\<tag>` files whose layers or config name the digest.
pub fn models_using(manifests: &Path, digest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(manifests.to_path_buf(), Vec::<String>::new())];
    while let Some((d, parts)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            let mut here = parts.clone();
            here.push(name);
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if here.len() < 4 {
                    stack.push((e.path(), here));
                }
                continue;
            }
            let Ok(v) = std::fs::read_to_string(e.path())
                .map_err(|_| ())
                .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|_| ()))
            else {
                continue;
            };
            let named = v["layers"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(std::iter::once(&v["config"]))
                .any(|l| l["digest"].as_str() == Some(digest));
            if named && here.len() >= 3 {
                let tag = &here[here.len() - 1];
                let model = &here[here.len() - 2];
                let ns = &here[here.len() - 3];
                out.push(if ns == "library" {
                    format!("{model}:{tag}")
                } else {
                    format!("{ns}/{model}:{tag}")
                });
            }
        }
    }
    out.sort();
    out
}

impl CopyCheck for OllamaCheck {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn applies(&self, cx: &Cx<'_>, s: &Subject) -> bool {
        !s.dir && blob_digest(cx, &s.path).is_some()
    }

    fn find(&self, cx: &Cx<'_>, s: &Subject) -> anyhow::Result<Find> {
        let Some(digest) = blob_digest(cx, &s.path) else {
            return Ok(Find::Skip);
        };
        let models = models_using(&models_dir(cx).join("manifests"), &digest);
        match models.first() {
            Some(m) => Ok(Find::Copy(Found {
                locator: format!("ollama {m}"),
                detail: json!({"models": models}),
            })),
            None => Ok(Find::NoCopy(with_note(
                json!({}),
                "no Ollama model uses this blob",
            ))),
        }
    }

    fn verify(&self, _cx: &Cx<'_>, _s: &Subject, f: Found) -> anyhow::Result<CopyResult> {
        let model = f.locator.trim_start_matches("ollama ").to_string();
        let note = format!("in Ollama as {model} (ollama pull brings it back)");
        Ok(CopyResult::new(
            Self::NAME,
            Some(f.locator),
            Confidence::Claimed,
            with_note(f.detail, note),
        ))
    }
}

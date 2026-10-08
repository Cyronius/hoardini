use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::catalog::{Catalog, Env};
use crate::index::normalize_path;
use crate::paths;

// Every location the gate treats as special, so tests can point the profile and data dir at temp folders.
#[derive(Clone)]
pub struct PolicyEnv {
    pub profile: String,
    pub data_dir: String,
    pub system_root: String,
    pub program_data: String,
    pub vars: HashMap<String, String>,
    catalog: Option<Arc<Catalog>>,
}

impl std::fmt::Debug for PolicyEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolicyEnv")
            .field("profile", &self.profile)
            .field("data_dir", &self.data_dir)
            .field("system_root", &self.system_root)
            .finish_non_exhaustive()
    }
}

const PROGRAM_FILES_VARS: &[&str] = &["programfiles", "programfiles(x86)", "programw6432"];
const MODEL_VARS: &[&str] = &["ollama_models", "hf_home", "huggingface_hub_cache"];

fn norm(p: &str) -> String {
    normalize_path(&p.replace('/', "\\"))
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

impl PolicyEnv {
    pub fn system() -> PolicyEnv {
        let drive = env_var("SystemDrive").unwrap_or_else(|| "C:".into());
        let mut vars = HashMap::new();
        for name in PROGRAM_FILES_VARS.iter().chain(MODEL_VARS) {
            if let Some(v) = env_var(name) {
                vars.insert((*name).to_string(), norm(&v));
            }
        }
        PolicyEnv {
            profile: norm(&paths::profile_dir().to_string_lossy()),
            data_dir: norm(&paths::data_dir().to_string_lossy()),
            system_root: norm(
                &env_var("SystemRoot").unwrap_or_else(|| format!("{drive}\\Windows")),
            ),
            program_data: norm(
                &env_var("ProgramData").unwrap_or_else(|| format!("{drive}\\ProgramData")),
            ),
            vars,
            catalog: None,
        }
    }

    // Model-store variables are dropped so a test never looks at the real model folders.
    pub fn with_paths(profile: &Path, data_dir: &Path) -> PolicyEnv {
        let mut env = PolicyEnv::system();
        env.profile = norm(&profile.to_string_lossy());
        env.data_dir = norm(&data_dir.to_string_lossy());
        for name in MODEL_VARS {
            env.vars.remove(*name);
        }
        let p = env.profile.clone();
        let mut cat_env = Env::new(p.clone())
            .set("LOCALAPPDATA", format!("{p}\\AppData\\Local"))
            .set("APPDATA", format!("{p}\\AppData\\Roaming"))
            .set("TEMP", format!("{p}\\AppData\\Local\\Temp"))
            .set("SystemRoot", env.system_root.clone())
            .set("ProgramData", env.program_data.clone())
            .set(
                "SystemDrive",
                env.system_root.get(..2).unwrap_or("C:").to_string(),
            );
        for (k, v) in &env.vars {
            cat_env = cat_env.set(k, v.clone());
        }
        env.catalog = Catalog::with_env(&cat_env).ok().map(Arc::new);
        env
    }

    pub fn catalog(&self) -> &Catalog {
        match &self.catalog {
            Some(c) => c,
            None => Catalog::embedded(),
        }
    }

    pub fn var(&self, name: &str) -> Option<&str> {
        self.vars
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub(crate) fn program_files(&self) -> Vec<String> {
        PROGRAM_FILES_VARS
            .iter()
            .filter_map(|n| self.var(n).map(str::to_string))
            .collect()
    }

    // Accepts `~\...` because the model only ever sees redacted paths.
    pub fn expand(&self, raw: &str) -> String {
        paths::expand_with(raw.trim(), &self.profile)
    }
}

mod steam;

use std::sync::{Arc, RwLock};

use hoardini_win::{InstalledProgram, UserAssistEntry};
use rusqlite::Connection;
use serde::Serialize;

use crate::evidence::owner;
use crate::index::{self, NodeKind, TopFilesQuery, normalize_path};

pub use steam::{SteamApp, parse_app_manifest, parse_library_folders};

pub const MIN_APP_BYTES: u64 = 2 << 30;
pub const IDLE_DAYS: i64 = 90;
const DAY_MS: i64 = 86_400_000;

#[derive(Clone, Debug, Default)]
pub struct AppInputs {
    pub programs: Vec<InstalledProgram>,
    pub assist: Vec<UserAssistEntry>,
    pub steam: Vec<SteamApp>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Uninstaller {
    Command {
        file: String,
        params: Option<String>,
    },
    Url(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
    pub publisher: Option<String>,
    pub source: &'static str,
    pub location: String,
    pub uninstaller: Uninstaller,
    pub last_played_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RarelyUsedApp {
    pub id: String,
    pub name: String,
    pub publisher: Option<String>,
    pub source: &'static str,
    pub path: String,
    pub size: u64,
    pub last_run_ms: Option<i64>,
    pub last_run_from: Option<String>,
}

// The program ends at the first ".exe" followed by a space, so unquoted paths with spaces still split right.
pub fn split_command(cmd: &str) -> Option<(String, Option<String>)> {
    let c = cmd.trim();
    let (file, rest) = if let Some(r) = c.strip_prefix('"') {
        let end = r.find('"')?;
        (&r[..end], &r[end + 1..])
    } else {
        let lower = c.to_ascii_lowercase();
        let end = lower
            .match_indices(".exe")
            .map(|(i, _)| i + 4)
            .find(|&e| c[e..].is_empty() || c[e..].starts_with(' '))
            .or_else(|| c.find(char::is_whitespace))
            .unwrap_or(c.len());
        (&c[..end], &c[end..])
    };
    let file = file.trim();
    if file.is_empty() {
        return None;
    }
    let rest = rest.trim();
    Some((
        file.to_string(),
        (!rest.is_empty()).then(|| rest.to_string()),
    ))
}

// The shell would look a bare name up in the working directory and on PATH first, so only the System folder may supply one.
pub fn program_path(file: &str) -> Option<String> {
    let b = file.as_bytes();
    if (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\')
        || file.starts_with(r"\\")
    {
        return Some(file.to_string());
    }
    if file.contains(['\\', '/', ':']) {
        return None;
    }
    let p = hoardini_win::system_tool(file).ok()?;
    p.is_file().then(|| p.to_string_lossy().into_owned())
}

fn registry_id(key: &str) -> String {
    let h = blake3::hash(key.to_lowercase().as_bytes()).to_hex();
    format!("reg-{}", &h[..16])
}

// Ids are derived, so the uninstall route can only ever run a command the registry or Steam holds.
pub fn installed(inputs: &AppInputs) -> Vec<App> {
    let mut out: Vec<App> = inputs
        .steam
        .iter()
        .map(|s| App {
            id: format!("steam-{}", s.appid),
            name: s.name.clone(),
            publisher: None,
            source: "steam",
            location: normalize_path(&s.dir),
            uninstaller: Uninstaller::Url(format!("steam://uninstall/{}", s.appid)),
            last_played_ms: s.last_played_ms,
        })
        .collect();
    for p in inputs.programs.iter().filter(|p| !p.hidden) {
        let (Some(loc), Some((file, params))) = (
            p.install_location.as_deref(),
            p.uninstall_string
                .as_deref()
                .and_then(split_command)
                .and_then(|(f, a)| Some((program_path(&f)?, a))),
        ) else {
            continue;
        };
        let loc = normalize_path(loc.trim_matches('"'));
        if owner::broad(&owner::norm(&loc))
            || out.iter().any(|a| a.location.eq_ignore_ascii_case(&loc))
        {
            continue;
        }
        out.push(App {
            id: registry_id(&p.uninstall_key),
            name: p.name.clone(),
            publisher: p.publisher.clone(),
            source: "registry",
            location: loc,
            uninstaller: Uninstaller::Command { file, params },
            last_played_ms: None,
        });
    }
    out
}

fn under(parent: &str, child: &str) -> bool {
    owner::is_under(&owner::norm(parent), &owner::norm(child))
        && !parent.eq_ignore_ascii_case(child)
}

fn exe_names(conn: &Connection, location: &str) -> Vec<String> {
    let q = TopFilesQuery {
        under: Some(location.to_string()),
        min_size: 0,
        ext: Some("exe".into()),
        not_used_for_days: None,
        limit: 500,
    };
    index::top_files(conn, &q)
        .map(|r| {
            r.items
                .into_iter()
                .filter_map(|f| f.path.rsplit('\\').next().map(str::to_lowercase))
                .collect()
        })
        .unwrap_or_default()
}

fn squash(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase()
}

// Taskbar and Start launches are recorded as a shortcut or an app id, not the exe path, so names count too.
fn run_of(entry: &str, location: &str, name: &str, exes: &[String]) -> bool {
    let n = owner::expand_assist_name(entry);
    if owner::is_under(&owner::norm(location), &n) {
        return true;
    }
    let leaf = n.rsplit('\\').next().unwrap_or(&n);
    let stem = squash(
        leaf.strip_suffix(".lnk")
            .or_else(|| leaf.strip_suffix(".exe"))
            .unwrap_or(leaf),
    );
    let app = squash(name);
    if (stem.len() >= 5 && app.contains(&stem)) || (app.len() >= 5 && stem.contains(&app)) {
        return true;
    }
    !n.contains('\\') && exes.iter().any(|e| n.contains(e.as_str()))
}

fn last_assist(entries: &[UserAssistEntry], a: &App, exes: &[String]) -> Option<(i64, String)> {
    entries
        .iter()
        .filter(|e| run_of(&e.name, &a.location, &a.name, exes))
        .filter_map(|e| {
            e.last_run_ms
                .map(|ms| (ms, owner::expand_assist_name(&e.name)))
        })
        .max_by_key(|(ms, _)| *ms)
}

pub fn rarely_used(
    conn: &Connection,
    inputs: &AppInputs,
    now: i64,
) -> anyhow::Result<Vec<RarelyUsedApp>> {
    let apps = installed(inputs);
    let mut sized: Vec<(&App, String, u64)> = Vec::new();
    for a in &apps {
        if let Some(n) = index::node_by_path(conn, &a.location)?
            && n.kind == NodeKind::Dir
        {
            sized.push((a, n.path, n.size));
        }
    }
    let mut out = Vec::new();
    for (a, path, size) in &sized {
        // An app installed inside another one (games inside a launcher's folder) is that app's, not this one's.
        let nested: u64 = sized
            .iter()
            .filter(|(b, _, _)| under(&a.location, &b.location))
            .filter(|(b, _, _)| {
                !sized.iter().any(|(c, _, _)| {
                    under(&a.location, &c.location) && under(&c.location, &b.location)
                })
            })
            .map(|(_, _, s)| *s)
            .sum();
        let size = size.saturating_sub(nested);
        if size < MIN_APP_BYTES {
            continue;
        }
        let exes = exe_names(conn, &a.location);
        let signals = [
            last_assist(&inputs.assist, a, &exes).map(|(ms, e)| (ms, format!("last run of {e}"))),
            a.last_played_ms
                .map(|p| (p, "Steam LastPlayed".to_string())),
        ];
        let last = signals.into_iter().flatten().max_by_key(|(ms, _)| *ms);
        let (last_run_ms, last_run_from) = (last.as_ref().map(|l| l.0), last.map(|l| l.1));
        if last_run_ms.is_some_and(|t| t >= now - IDLE_DAYS * DAY_MS) {
            continue;
        }
        out.push(RarelyUsedApp {
            id: a.id.clone(),
            name: a.name.clone(),
            publisher: a.publisher.clone(),
            source: a.source,
            path: path.clone(),
            size,
            last_run_ms,
            last_run_from,
        });
    }
    out.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

pub trait AppSource: Send + Sync {
    fn inputs(&self) -> AppInputs;
    fn launch(&self, how: &Uninstaller) -> anyhow::Result<()>;
}

pub struct SystemApps;

impl AppSource for SystemApps {
    fn inputs(&self) -> AppInputs {
        AppInputs {
            programs: hoardini_win::installed_programs().unwrap_or_default(),
            assist: hoardini_win::user_assist().unwrap_or_default(),
            steam: steam::gather(),
        }
    }

    fn launch(&self, how: &Uninstaller) -> anyhow::Result<()> {
        match how {
            Uninstaller::Command { file, params } => {
                hoardini_win::shell_open(file, params.as_deref())?
            }
            Uninstaller::Url(url) => hoardini_win::shell_open(url, None)?,
        }
        Ok(())
    }
}

static SOURCE: RwLock<Option<Arc<dyn AppSource>>> = RwLock::new(None);

#[doc(hidden)]
pub fn set_source(source: Option<Arc<dyn AppSource>>) {
    *SOURCE.write().unwrap_or_else(|e| e.into_inner()) = source;
}

pub fn source() -> Arc<dyn AppSource> {
    SOURCE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| Arc::new(SystemApps))
}

pub fn list_rarely_used(conn: &Connection, now: i64) -> anyhow::Result<Vec<RarelyUsedApp>> {
    rarely_used(conn, &source().inputs(), now)
}

// None when no installed app has this id; the uninstaller asks for its own confirmation.
pub fn uninstall(id: &str) -> anyhow::Result<Option<App>> {
    let src = source();
    let Some(app) = installed(&src.inputs()).into_iter().find(|a| a.id == id) else {
        return Ok(None);
    };
    src.launch(&app.uninstaller)?;
    Ok(Some(app))
}

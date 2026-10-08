use std::sync::{Arc, Mutex};

use hoardini_win::{InstalledProgram, UserAssistEntry};

use super::OwnerApp;
use crate::catalog::CatalogMatch;

static PROGRAMS: Mutex<Option<Arc<Vec<InstalledProgram>>>> = Mutex::new(None);
static ASSIST: Mutex<Option<Arc<Vec<UserAssistEntry>>>> = Mutex::new(None);

// Install locations this broad would claim half the disk.
const TOO_BROAD: &[&str] = &[
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "users",
    "system32",
];

pub fn refresh_caches() {
    *PROGRAMS.lock().unwrap() = Some(Arc::new(
        hoardini_win::installed_programs().unwrap_or_default(),
    ));
    *ASSIST.lock().unwrap() = Some(Arc::new(hoardini_win::user_assist().unwrap_or_default()));
}

pub fn programs() -> Arc<Vec<InstalledProgram>> {
    let mut g = PROGRAMS.lock().unwrap();
    g.get_or_insert_with(|| Arc::new(hoardini_win::installed_programs().unwrap_or_default()))
        .clone()
}

pub fn assist() -> Arc<Vec<UserAssistEntry>> {
    let mut g = ASSIST.lock().unwrap();
    g.get_or_insert_with(|| Arc::new(hoardini_win::user_assist().unwrap_or_default()))
        .clone()
}

pub fn norm(p: &str) -> String {
    let mut s = p.trim().trim_matches('"').replace('/', "\\").to_lowercase();
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    s
}

pub fn is_under(prefix: &str, path: &str) -> bool {
    path == prefix
        || (path.len() > prefix.len()
            && path.starts_with(prefix)
            && (prefix.ends_with('\\') || path.as_bytes()[prefix.len()] == b'\\'))
}

fn broad(loc: &str) -> bool {
    let rest = loc
        .split_once(':')
        .map_or(loc, |(_, r)| r)
        .trim_matches('\\');
    rest.is_empty() || TOO_BROAD.contains(&rest.rsplit('\\').next().unwrap_or(rest))
}

fn from_programs(programs: &[InstalledProgram], path: &str) -> Option<OwnerApp> {
    programs
        .iter()
        .filter_map(|p| {
            let loc = norm(p.install_location.as_deref()?);
            (!broad(&loc) && is_under(&loc, path)).then_some((loc.len(), p))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, p)| OwnerApp {
            name: p.name.clone(),
            publisher: p.publisher.clone(),
            install_location: p.install_location.clone(),
            source: "uninstall registry (InstallLocation prefix)".into(),
        })
}

fn from_appx(path: &str) -> Option<OwnerApp> {
    const MARK: &str = "\\windowsapps\\";
    let at = path.find(MARK)? + MARK.len();
    let pkg = path[at..].split('\\').next().filter(|s| !s.is_empty())?;
    if pkg.starts_with('.') {
        return None;
    }
    let name = pkg.split('_').next().unwrap_or(pkg);
    Some(OwnerApp {
        name: name.to_string(),
        publisher: None,
        install_location: Some(format!("{}{}", &path[..at], pkg)),
        source: "AppX package folder".into(),
    })
}

pub fn resolve(path: &str, catalog: Option<&CatalogMatch>) -> Option<OwnerApp> {
    let p = norm(path);
    from_programs(&programs(), &p)
        .or_else(|| from_appx(&p))
        .or_else(|| {
            catalog.map(|c| OwnerApp {
                name: c.app.clone(),
                publisher: None,
                install_location: None,
                source: "catalog (heuristic)".into(),
            })
        })
}

fn known_folder(guid: &str) -> Option<String> {
    let env = |k: &str| std::env::var(k).ok();
    let sys = || env("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    Some(match guid.to_ascii_uppercase().as_str() {
        "6D809377-6AF0-444B-8957-A3773F02200E" => env("ProgramW6432").or(env("ProgramFiles"))?,
        "905E63B6-C1BF-494E-B29C-65B732D3D21A" => env("ProgramFiles")?,
        "7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E" => env("ProgramFiles(x86)")?,
        "1AC14E77-02E7-4E5D-B744-2EB1AE5198B7" => format!(r"{}\System32", sys()),
        "D65231B0-B2F1-4857-A4CE-A8E7C6EA7D27" => format!(r"{}\SysWOW64", sys()),
        "F38BF404-1D43-42F2-9305-67DE0B28FC23" => sys(),
        "F1B32785-6FBA-4FCF-9D55-7B8E7F157091" => env("LOCALAPPDATA")?,
        "3EB685DB-65F9-4CF6-A03A-E3EF65729F3D" => env("APPDATA")?,
        "62AB5D82-FDC1-4DC3-A9DD-070D1D495D97" => env("ProgramData")?,
        _ => return None,
    })
}

pub fn expand_assist_name(name: &str) -> String {
    if let Some(rest) = name.strip_prefix('{')
        && let Some((guid, tail)) = rest.split_once("}\\")
        && let Some(base) = known_folder(guid)
    {
        return norm(&format!("{base}\\{tail}"));
    }
    norm(name)
}

// Latest UserAssist run of any exe equal to or under `prefix`.
pub fn last_run_under(entries: &[UserAssistEntry], prefix: &str) -> Option<(i64, String)> {
    let prefix = norm(prefix);
    entries
        .iter()
        .filter_map(|e| {
            let n = expand_assist_name(&e.name);
            let ms = e.last_run_ms?;
            is_under(&prefix, &n).then_some((ms, n))
        })
        .max_by_key(|(ms, _)| *ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog(name: &str, loc: &str) -> InstalledProgram {
        InstalledProgram {
            name: name.into(),
            install_location: Some(loc.into()),
            ..Default::default()
        }
    }

    #[test]
    fn longest_install_location_wins_and_broad_roots_are_ignored() {
        let ps = vec![
            prog("Outer", r"C:\Apps\Suite"),
            prog("Inner", r"C:\Apps\Suite\Tool\"),
            prog("Everything", r"C:\Program Files"),
        ];
        let o = from_programs(&ps, &norm(r"C:\Apps\Suite\Tool\bin\x.dll")).unwrap();
        assert_eq!(o.name, "Inner");
        assert!(from_programs(&ps, &norm(r"C:\Program Files\Other\a.exe")).is_none());
        assert!(from_programs(&ps, &norm(r"C:\Apps\SuiteX\a.dll")).is_none());
    }

    #[test]
    fn appx_owner_comes_from_the_package_folder() {
        let o = from_appx(&norm(
            r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal_1.2.3_x64__8wekyb3d8bbwe\a.dll",
        ))
        .unwrap();
        assert_eq!(o.name, "microsoft.windowsterminal");
    }

    #[test]
    fn known_folder_guids_expand() {
        let n = expand_assist_name(r"{F38BF404-1D43-42F2-9305-67DE0B28FC23}\notepad.exe");
        assert!(n.ends_with(r"\notepad.exe") && !n.contains('{'));
    }
}

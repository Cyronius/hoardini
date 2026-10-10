use std::path::Path;

use crate::index::normalize_path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SteamApp {
    pub appid: u32,
    pub name: String,
    pub dir: String,
    pub last_played_ms: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
enum Tok {
    Str(String),
    Open,
    Close,
}

fn tokens(text: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' => out.push(Tok::Open),
            '}' => out.push(Tok::Close),
            '"' => {
                let mut s = String::new();
                while let Some(c) = it.next() {
                    match c {
                        '"' => break,
                        '\\' => match it.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(other) => s.push(other),
                            None => {}
                        },
                        _ => s.push(c),
                    }
                }
                out.push(Tok::Str(s));
            }
            '/' if it.peek() == Some(&'/') => {
                for c in it.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            c => {
                let mut s = String::from(c);
                while let Some(&n) = it.peek() {
                    if n.is_whitespace() || n == '{' || n == '}' || n == '"' {
                        break;
                    }
                    s.push(n);
                    it.next();
                }
                out.push(Tok::Str(s));
            }
        }
    }
    out
}

fn pairs(text: &str) -> Vec<(usize, String, String)> {
    let toks = tokens(text);
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < toks.len() {
        match (&toks[i], toks.get(i + 1)) {
            (Tok::Str(k), Some(Tok::Str(v))) => {
                out.push((depth, k.clone(), v.clone()));
                i += 2;
            }
            (Tok::Str(_), Some(Tok::Open)) => {
                depth += 1;
                i += 2;
            }
            (Tok::Open, _) => {
                depth += 1;
                i += 1;
            }
            (Tok::Close, _) => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

fn steam_dir(p: &str) -> String {
    normalize_path(&p.replace('/', "\\"))
}

// Current files nest each library under a number with a "path" key; old ones map the number to the path.
pub fn parse_library_folders(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (depth, k, v) in pairs(text) {
        let legacy = depth == 1 && k.chars().all(|c| c.is_ascii_digit()) && v.contains(":\\");
        if (k.eq_ignore_ascii_case("path") || legacy)
            && !out.iter().any(|p| p.eq_ignore_ascii_case(&steam_dir(&v)))
        {
            out.push(steam_dir(&v));
        }
    }
    out
}

pub fn parse_app_manifest(text: &str, library: &str) -> Option<SteamApp> {
    let mut appid = None;
    let mut name = None;
    let mut installdir = None;
    let mut last_played = None;
    for (depth, k, v) in pairs(text) {
        if depth != 1 {
            continue;
        }
        match k.to_ascii_lowercase().as_str() {
            "appid" => appid = v.trim().parse::<u32>().ok(),
            "name" => name = Some(v),
            "installdir" => installdir = Some(v).filter(|d| !d.trim().is_empty()),
            "lastplayed" => last_played = v.trim().parse::<i64>().ok(),
            _ => {}
        }
    }
    let installdir = installdir?;
    let appid = appid?;
    Some(SteamApp {
        appid,
        name: name.unwrap_or_else(|| format!("Steam app {appid}")),
        dir: format!(r"{}\steamapps\common\{installdir}", steam_dir(library)),
        last_played_ms: last_played.filter(|s| *s > 0).map(|s| s * 1000),
    })
}

pub fn gather() -> Vec<SteamApp> {
    let Some(steam) = hoardini_win::steam_path().map(|p| steam_dir(&p)) else {
        return Vec::new();
    };
    let mut libraries = vec![steam.clone()];
    for vdf in [
        r"steamapps\libraryfolders.vdf",
        r"config\libraryfolders.vdf",
    ] {
        if let Ok(text) = std::fs::read_to_string(Path::new(&steam).join(vdf)) {
            for lib in parse_library_folders(&text) {
                if !libraries.iter().any(|l| l.eq_ignore_ascii_case(&lib)) {
                    libraries.push(lib);
                }
            }
        }
    }
    let mut out: Vec<SteamApp> = Vec::new();
    for lib in libraries {
        let Ok(entries) = std::fs::read_dir(Path::new(&lib).join("steamapps")) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            if let Some(app) = std::fs::read_to_string(e.path())
                .ok()
                .and_then(|t| parse_app_manifest(&t, &lib))
                && !out.iter().any(|a| a.appid == app.appid)
            {
                out.push(app);
            }
        }
    }
    out
}

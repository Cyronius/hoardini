use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;

use super::{
    Confidence, CopyCheck, CopyResult, Cx, Find, Found, Service, Subject, hash, pending, with_note,
};

const EXTS: &[&str] = &["exe", "msi", "msix", "msixbundle", "appx", "appxbundle"];
const TIMEOUT: Duration = Duration::from_secs(90);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub trait WingetCli: Send + Sync {
    fn run(&self, args: &[&str]) -> std::io::Result<String>;
}

pub struct SystemWinget;

impl WingetCli for SystemWinget {
    fn run(&self, args: &[&str]) -> std::io::Result<String> {
        let mut child = Command::new("winget")
            .args(args)
            .args(["--accept-source-agreements", "--disable-interactivity"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        let mut out = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("no stdout"))?;
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            buf
        });
        let started = Instant::now();
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if started.elapsed() > TIMEOUT {
                let _ = child.kill();
                return Err(std::io::Error::other("winget timed out"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let buf = reader.join().unwrap_or_default();
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchRow {
    pub name: String,
    pub id: String,
    pub version: String,
}

fn char_col(line: &str, col: usize) -> &str {
    let start = line.char_indices().nth(col).map_or(line.len(), |(i, _)| i);
    &line[start..]
}

fn slice(line: &str, from: usize, to: Option<usize>) -> String {
    let rest = char_col(line, from);
    let s: String = match to {
        Some(t) => rest.chars().take(t.saturating_sub(from)).collect(),
        None => rest.to_string(),
    };
    s.trim().to_string()
}

// `winget search` prints a fixed-width table; columns start where the header's words do.
pub fn parse_search(out: &str) -> Vec<SearchRow> {
    let lines: Vec<&str> = out
        .lines()
        .map(|l| l.rsplit('\r').next().unwrap_or(l))
        .collect();
    let Some(h) = lines.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with("Name") && l.contains(" Id ") && l.contains("Version")
    }) else {
        return Vec::new();
    };
    let header = lines[h];
    let col = |word: &str| header.find(word).map(|b| header[..b].chars().count());
    let (Some(id_c), Some(ver_c)) = (col(" Id ").map(|c| c + 1), col("Version")) else {
        return Vec::new();
    };
    let name_c = col("Name").unwrap_or(0);
    let after_ver = ["Match", "Source"]
        .iter()
        .filter_map(|w| col(w))
        .filter(|c| *c > ver_c)
        .min();
    lines[h + 1..]
        .iter()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('-'))
        .filter_map(|l| {
            let row = SearchRow {
                name: slice(l, name_c, Some(id_c)),
                id: slice(l, id_c, Some(ver_c)),
                version: slice(l, ver_c, after_ver),
            };
            (!row.id.is_empty()).then_some(row)
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShowInfo {
    pub name: String,
    pub id: String,
    pub version: Option<String>,
    pub publisher: Option<String>,
    pub installer_type: Option<String>,
    pub installer_url: Option<String>,
    pub installer_sha256: Option<String>,
}

// `winget show`: "Found <name> [<id>]", then "Key: value" lines; installer fields are indented under "Installer:".
pub fn parse_show(out: &str) -> Option<ShowInfo> {
    let mut info = ShowInfo::default();
    let mut found = false;
    for raw in out.lines() {
        let l = raw.rsplit('\r').next().unwrap_or(raw);
        let t = l.trim();
        if let Some(rest) = t.strip_prefix("Found ") {
            let (name, id) = rest.rsplit_once(" [")?;
            info.name = name.trim().to_string();
            info.id = id.trim_end_matches(']').trim().to_string();
            found = true;
            continue;
        }
        let Some((k, v)) = t.split_once(": ") else {
            continue;
        };
        let v = Some(v.trim().to_string()).filter(|v| !v.is_empty());
        match k {
            "Version" if info.version.is_none() => info.version = v,
            "Publisher" if info.publisher.is_none() => info.publisher = v,
            "Installer Type" => info.installer_type = v,
            "Installer Url" => info.installer_url = v,
            "Installer SHA256" => info.installer_sha256 = v.map(|s| s.to_ascii_lowercase()),
            _ => {}
        }
    }
    found.then_some(info)
}

fn ext_of(path: &str) -> String {
    path.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_version(t: &str) -> bool {
    let t = t.trim_start_matches(['v', 'V']);
    t.contains('.')
        && t.split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

pub fn name_and_version(file_name: &str) -> (String, Option<String>) {
    let stem = file_name.rsplit_once('.').map_or(file_name, |(s, _)| s);
    let noise = [
        "setup",
        "install",
        "installer",
        "x64",
        "x86",
        "arm64",
        "win",
        "win64",
        "64",
        "32",
        "web",
        "full",
        "online",
        "offline",
    ];
    let mut words = Vec::new();
    let mut version = None;
    let mut rest = stem;
    while !rest.is_empty() {
        let (tok, tail) = match rest.find(['-', '_', ' ', '(', ')']) {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, ""),
        };
        rest = tail;
        if tok.is_empty() {
            continue;
        }
        let digits_lead = tok.starts_with(|c: char| c.is_ascii_digit() || c == 'v');
        let (head, ver) = match tok.find(|c: char| c.is_ascii_digit()) {
            Some(i) if digits_lead || i == 0 => ("", Some(tok)),
            _ => (tok, None),
        };
        if let Some(v) = ver {
            let v = v.trim_end_matches(|c: char| !c.is_ascii_digit());
            let core: String = v
                .trim_start_matches(['v', 'V'])
                .split('.')
                .take_while(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
                .collect::<Vec<_>>()
                .join(".");
            if version.is_none() && is_version(&core) {
                version = Some(core);
            }
            continue;
        }
        let word = head.split('.').next().unwrap_or(head);
        if !word.is_empty() && !noise.contains(&word.to_ascii_lowercase().as_str()) {
            words.push(word.to_string());
        }
    }
    (words.join(" "), version)
}

fn query_terms(file_name: &str, product: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(p) = product
        .map(str::trim)
        .filter(|p| !p.is_empty() && p.len() <= 40 && p.split_whitespace().count() <= 4)
    {
        out.push(p.to_string());
    }
    let (name, _) = name_and_version(file_name);
    if name.len() >= 3 && !out.iter().any(|o| o.eq_ignore_ascii_case(&name)) {
        out.push(name);
    }
    out
}

fn pick<'a>(rows: &'a [SearchRow], term: &str, version: Option<&str>) -> Option<&'a SearchRow> {
    let t = term.to_ascii_lowercase();
    let named = |r: &&SearchRow| {
        r.name.to_ascii_lowercase().contains(&t)
            || r.id.to_ascii_lowercase().contains(&t.replace(' ', ""))
    };
    rows.iter()
        .filter(named)
        .find(|r| version.is_some_and(|v| r.version == v))
        .or_else(|| rows.iter().find(|r| r.name.eq_ignore_ascii_case(term)))
        .or_else(|| rows.iter().find(named))
}

pub struct WingetCheck;

impl WingetCheck {
    pub const NAME: &'static str = "winget";
}

impl CopyCheck for WingetCheck {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn service(&self) -> Option<Service> {
        Some(Service::Winget)
    }

    fn applies(&self, _cx: &Cx<'_>, s: &Subject) -> bool {
        !s.dir
            && !s.is_placeholder()
            && (EXTS.contains(&ext_of(&s.path).as_str())
                || s.hint.as_ref().is_some_and(|h| h.names(Self::NAME)))
    }

    fn find(&self, cx: &Cx<'_>, s: &Subject) -> anyhow::Result<Find> {
        let path = Path::new(&s.path);
        let pe = hoardini_win::open_read_preserving_atime(path)
            .ok()
            .and_then(|f| crate::evidence::pe::parse(f, s.size));
        if pe.as_ref().is_some_and(|p| p.is_dll) {
            return Ok(Find::Skip);
        }
        let (_, name_version) = name_and_version(s.name());
        let version = pe
            .as_ref()
            .and_then(|p| p.product_version.clone())
            .map(|v| v.trim().to_string())
            .filter(|v| is_version(v))
            .or(name_version);
        let mut terms = query_terms(
            s.name(),
            pe.as_ref().and_then(|p| p.product_name.as_deref()),
        );
        if let Some(place) = s
            .hint
            .as_ref()
            .filter(|h| h.names(Self::NAME))
            .and_then(|h| h.place.clone())
        {
            terms.insert(0, place);
        }
        for term in &terms {
            let rows = parse_search(
                &cx.env
                    .winget
                    .run(&["search", "--name", term, "--source", "winget"])?,
            );
            let Some(row) = pick(&rows, term, version.as_deref()) else {
                continue;
            };
            let exact = version.as_deref() == Some(row.version.as_str());
            let mut args = vec![
                "show",
                "--id",
                row.id.as_str(),
                "--exact",
                "--source",
                "winget",
            ];
            if let Some(v) = version.as_deref().filter(|_| exact) {
                args.extend(["--version", v]);
            }
            let Some(show) = parse_show(&cx.env.winget.run(&args)?) else {
                continue;
            };
            let shown = show.version.clone().unwrap_or_else(|| row.version.clone());
            return Ok(Find::Copy(Found {
                locator: format!("{} {shown}", show.id),
                detail: json!({
                    "id": show.id, "name": show.name, "version": shown, "publisher": show.publisher,
                    "installer_sha256": show.installer_sha256, "installer_url": show.installer_url,
                    "file_version": version, "same_version": exact, "query": term,
                    "signer": crate::evidence::pe::signer(path),
                }),
            }));
        }
        Ok(Find::NoCopy(with_note(
            json!({"queries": terms}),
            "not found in winget",
        )))
    }

    fn verify(&self, cx: &Cx<'_>, s: &Subject, f: Found) -> anyhow::Result<CopyResult> {
        let at = f.locator.clone();
        let same_version = f.detail["same_version"].as_bool().unwrap_or(false);
        let remote = f.detail["installer_sha256"].as_str().map(str::to_string);
        let local = hash::current(cx.conn, s);
        let (conf, note, detail) = match (same_version, remote, local) {
            (true, Some(r), Some(l)) if r == l => (
                Confidence::Identical,
                format!("in winget as {at}"),
                f.detail,
            ),
            (true, Some(_), None) => (
                Confidence::SameNameAndSize,
                format!("in winget as {at}; checking the installer's bytes"),
                pending(f.detail, "hash"),
            ),
            (true, _, _) => (
                Confidence::Claimed,
                format!("winget has {at}, but not this exact installer"),
                f.detail,
            ),
            (false, _, _) => (
                Confidence::Claimed,
                format!("winget can install {at}"),
                f.detail,
            ),
        };
        Ok(CopyResult::new(
            Self::NAME,
            Some(at),
            conf,
            with_note(detail, note),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_names_split_into_name_and_version() {
        assert_eq!(
            name_and_version("balenaEtcher-2.1.7.Setup.exe"),
            ("balenaEtcher".into(), Some("2.1.7".into()))
        );
        assert_eq!(
            name_and_version("SumatraPDF-3.6.1-64-install.exe"),
            ("SumatraPDF".into(), Some("3.6.1".into()))
        );
        assert_eq!(
            name_and_version("ChromeSetup.exe"),
            ("ChromeSetup".into(), None)
        );
        assert_eq!(
            name_and_version("Firefox Installer.exe"),
            ("Firefox".into(), None)
        );
        assert_eq!(
            name_and_version("ImageMagick-7.1.2-13-Q16-HDRI-x64-dll.exe")
                .1
                .as_deref(),
            Some("7.1.2")
        );
    }

    #[test]
    fn long_product_names_are_not_used_as_queries() {
        let q = query_terms(
            "balenaEtcher-2.1.7.Setup.exe",
            Some("Flash OS images to SD cards and USB drives, safely and easily."),
        );
        assert_eq!(q, vec!["balenaEtcher".to_string()]);
        assert_eq!(
            query_terms("SumatraPDF-3.6.1-64-install.exe", Some("SumatraPDF")),
            vec!["SumatraPDF".to_string()]
        );
    }
}

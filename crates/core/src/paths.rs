use std::path::{Path, PathBuf};

pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("HOARDINI_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| profile_dir().join("AppData").join("Local"));
    base.join("hoardini")
}

pub fn db_path() -> PathBuf {
    data_dir().join("hoardini.db")
}

pub fn profile_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\Default"))
}

// Windows paths compare case-insensitively; keep the caller's casing for the tail.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() < prefix.len() || !s.is_char_boundary(prefix.len()) {
        return None;
    }
    let (head, tail) = s.split_at(prefix.len());
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    if tail.is_empty() || tail.starts_with('\\') || tail.starts_with('/') {
        Some(tail)
    } else {
        None
    }
}

pub fn redact_with(s: &str, profile: &str) -> String {
    let profile = profile.trim_end_matches(['\\', '/']);
    if profile.is_empty() {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = find_ci(rest, profile) {
        let after = &rest[pos + profile.len()..];
        let boundary = after.is_empty()
            || after.starts_with([
                '\\', '/', '"', '\'', ' ', ',', ')', ']', '}', '\n', ':', ';',
            ]);
        out.push_str(&rest[..pos]);
        if boundary {
            out.push('~');
        } else {
            out.push_str(&rest[pos..pos + profile.len()]);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len())
        .find(|&i| hay.is_char_boundary(i) && h[i..i + n.len()].eq_ignore_ascii_case(n))
}

pub fn redact(s: &str) -> String {
    redact_with(s, &profile_dir().to_string_lossy())
}

pub fn expand_with(s: &str, profile: &str) -> String {
    if s == "~" {
        return profile.to_string();
    }
    match s.strip_prefix("~\\").or_else(|| s.strip_prefix("~/")) {
        Some(rest) => format!("{}\\{}", profile.trim_end_matches(['\\', '/']), rest),
        None => s.to_string(),
    }
}

pub fn expand(s: &str) -> String {
    expand_with(s, &profile_dir().to_string_lossy())
}

pub fn profile_relative_with(path: &str, profile: &str) -> String {
    match strip_prefix_ci(path, profile.trim_end_matches(['\\', '/'])) {
        Some(tail) => format!("~{tail}"),
        None => path.to_string(),
    }
}

pub fn profile_relative(path: &Path) -> String {
    profile_relative_with(&path.to_string_lossy(), &profile_dir().to_string_lossy())
}

// Binary units with "GB" labels, matching what Explorer shows for the same file.
pub fn humanize_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if v >= 100.0 {
        format!("{v:.0} {}", UNITS[unit])
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_profile_case_insensitively_on_boundaries() {
        let p = r"C:\Users\josha";
        assert_eq!(
            redact_with(r"c:\users\JOSHA\.lmstudio\x.gguf", p),
            r"~\.lmstudio\x.gguf"
        );
        assert_eq!(redact_with(r"C:\Users\joshany\x", p), r"C:\Users\joshany\x");
        assert_eq!(redact_with(r#"{"p":"C:\Users\josha"}"#, p), r#"{"p":"~"}"#);
    }

    #[test]
    fn profile_relative_round_trips() {
        let p = r"C:\Users\josha";
        let k = profile_relative_with(r"C:\Users\josha\.cache\hf", p);
        assert_eq!(k, r"~\.cache\hf");
        assert_eq!(expand_with(&k, p), r"C:\Users\josha\.cache\hf");
        assert_eq!(profile_relative_with(r"D:\models", p), r"D:\models");
    }

    #[test]
    fn humanizes_binary_units() {
        assert_eq!(humanize_bytes(1023), "1023 B");
        assert_eq!(humanize_bytes(45_634_027_520), "42.5 GB");
        assert_eq!(humanize_bytes(150 * 1024 * 1024 * 1024), "150 GB");
    }
}

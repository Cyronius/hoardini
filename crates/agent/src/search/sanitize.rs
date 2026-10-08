const MAX_QUERY_CHARS: usize = 200;

fn is_path_start(tok: &str) -> bool {
    let t = tok.trim_matches(['"', '\'']);
    let b = t.as_bytes();
    t.starts_with("\\\\")
        || t.starts_with("~\\")
        || t.starts_with("~/")
        || (b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && (b[2] == b'\\' || b[2] == b'/'))
}

fn has_sep(tok: &str) -> bool {
    tok.contains(['\\', '/'])
}

fn components(s: &str) -> Vec<&str> {
    s.split(['\\', '/']).filter(|p| !p.is_empty()).collect()
}

fn reduce_path(group: &str, profile: &str) -> String {
    let all = components(group.trim_matches(['"', '\'']));
    let prof = components(profile);
    let same = !prof.is_empty()
        && all.len() >= prof.len()
        && all
            .iter()
            .zip(&prof)
            .all(|(a, b)| a.eq_ignore_ascii_case(b));
    let rest = if same { &all[prof.len()..] } else { &all[..] };
    let parts: Vec<&str> = rest
        .iter()
        .copied()
        .filter(|p| *p != "~" && !p.ends_with(':'))
        .collect();
    let tail = &parts[parts.len().saturating_sub(2)..];
    let quote = if group.starts_with('"') { "\"" } else { "" };
    format!("{quote}{}{quote}", tail.join("/"))
}

fn user_name(profile: &str) -> &str {
    profile
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
}

fn remove_word_ci(s: &str, word: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let lower = s.to_lowercase();
    let w = word.to_lowercase();
    // to_lowercase can change byte lengths; fall back to the untouched text then.
    if lower.len() != s.len() {
        return s.to_string();
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut i = 0;
    while i < s.len() {
        if lower[i..].starts_with(&w) {
            let before = s[..i].chars().next_back();
            let after = s[i + w.len()..].chars().next();
            if !before.is_some_and(is_word) && !after.is_some_and(is_word) {
                i += w.len();
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub fn sanitize_query(q: &str, profile: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut toks = q.split_whitespace().peekable();
    while let Some(tok) = toks.next() {
        if is_path_start(tok) {
            let mut group = tok.to_string();
            // Paths with spaces ("Program Files") arrive as several tokens.
            while let Some(next) = toks.peek() {
                if has_sep(next) && !is_path_start(next) {
                    group.push(' ');
                    group.push_str(next);
                    toks.next();
                } else {
                    break;
                }
            }
            let reduced = reduce_path(&group, profile);
            if !reduced.is_empty() && reduced != "\"\"" {
                words.push(reduced);
            }
        } else {
            words.push(tok.to_string());
        }
    }
    let mut out = words.join(" ");
    let name = user_name(profile);
    if name.chars().count() >= 2 {
        out = remove_word_ci(&out, name);
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    out.chars()
        .take(MAX_QUERY_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

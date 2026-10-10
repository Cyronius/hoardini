use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::knowledge::{self, Detail, Knowledge, knowledge_at, set_detail};
use super::location::{Cleaned, Kept, cleaned_entries, is_job, kept_entries, short_date};
use super::{
    COLS, Kind, Memory, Scope, compile_glob, delete, from_row, get, normalize_key_with,
    path_and_ancestors,
};
use crate::catalog::UnitKind;
use crate::index::{is_ancestor_path, normalize_path};
use crate::paths::humanize_bytes;
use crate::policy::{PolicyEnv, area_of, is_volume_root};
use crate::proposals::{self, ProposalItem, ProposalKind};

pub const MAX_ENTRIES: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Learned {
    pub memory_id: i64,
    pub text: String,
    // The user confirmed the row (its source is `user`), so the chip offers no Confirm.
    #[serde(default)]
    pub confirmed: bool,
}

fn same_or_under(a: &str, p: &str) -> bool {
    a.eq_ignore_ascii_case(p) || is_ancestor_path(a, p)
}

fn escape(s: &str) -> String {
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

fn unescape(s: &str) -> String {
    let mut out = s.to_string();
    for c in ['*', '?', '[', ']', '{', '}'] {
        out = out.replace(&format!("[{c}]"), &c.to_string());
    }
    out
}

// Containers (the profile, AppData) hold everything, so they say nothing about what was cleaned.
fn covering_row(conn: &Connection, env: &PolicyEnv, path: &str) -> anyhow::Result<Option<Memory>> {
    let key = normalize_key_with(Scope::Path, path, &env.profile);
    for k in path_and_ancestors(&key) {
        if let Some(m) = knowledge_at(conn, Scope::Path, &k)?
            && m.detail.as_ref().is_some_and(|d| d.category != "container")
        {
            return Ok(Some(m));
        }
    }
    Ok(None)
}

fn parent(path: &str) -> Option<&str> {
    path.rsplit_once('\\')
        .map(|(p, _)| p)
        .filter(|p| !p.is_empty())
}

fn common_parent(paths: &[&str]) -> Option<String> {
    let parents: Vec<Vec<&str>> = paths
        .iter()
        .map(|p| {
            let mut c: Vec<&str> = p.split('\\').collect();
            c.pop();
            c
        })
        .collect();
    let first = parents.first()?;
    let mut n = first.len();
    for other in &parents[1..] {
        n = (0..n.min(other.len()))
            .take_while(|&i| first[i].eq_ignore_ascii_case(other[i]))
            .count();
    }
    (n > 0).then(|| first[..n].join("\\"))
}

fn acceptable(env: &PolicyEnv, anchor: &str) -> bool {
    !is_volume_root(anchor) && area_of(env, anchor).is_none()
}

fn pick_anchor(
    env: &PolicyEnv,
    card: Option<&str>,
    common: Option<&str>,
    path: &str,
) -> Option<String> {
    [
        card.filter(|a| same_or_under(a, path)),
        common,
        parent(path),
    ]
    .into_iter()
    .flatten()
    .find(|a| acceptable(env, a))
    .map(str::to_string)
}

fn catalog_rule(env: &PolicyEnv, loc: &str, path: &str) -> Option<String> {
    let cat = env.catalog();
    let e = cat.lookup(path)?;
    let matched = std::iter::successors(Some(path), |p| parent(p))
        .find(|p| cat.lookup_exact(p).is_some_and(|x| x.id == e.id))?;
    let inside = !matched.eq_ignore_ascii_case(path);
    // Inside a Downloads-like entry each child is its own thing, so the entry's pattern says nothing about it.
    if inside && matches!(e.unit, UnitKind::Children | UnitKind::Container) {
        return None;
    }
    if is_ancestor_path(loc, matched) {
        let slashed = matched.replace('\\', "/");
        if let Some(p) = e
            .patterns
            .iter()
            .find(|p| p.starts_with("**\\") && compile_glob(p).is_ok_and(|g| g.is_match(&slashed)))
        {
            return Some(format!("{}\\{p}", escape(loc)));
        }
    }
    same_or_under(loc, matched).then(|| escape(matched))
}

fn file_parts(path: &str) -> Option<(&str, String)> {
    let (dir, name) = path.rsplit_once('\\')?;
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then(|| (dir, ext.to_ascii_lowercase()))
}

fn entry_path(v: &Value) -> Option<&str> {
    v.get("path").and_then(Value::as_str)
}

fn push_capped(list: &mut Vec<Value>, v: Value) {
    list.push(v);
    if list.len() > MAX_ENTRIES {
        let extra = list.len() - MAX_ENTRIES;
        list.drain(..extra);
    }
}

struct Target {
    row: Option<Memory>,
    key: String,
    detail: Detail,
}

pub fn learn_from_job(
    conn: &Connection,
    env: &PolicyEnv,
    proposal_id: &str,
    now: i64,
) -> anyhow::Result<usize> {
    let Some(p) = proposals::load(conn, proposal_id)? else {
        return Ok(0);
    };
    // Online-only keeps the cloud copy and system cleanups act on Windows' own areas: nothing left to learn.
    if !matches!(
        p.kind,
        ProposalKind::Delete | ProposalKind::Archive | ProposalKind::Compress
    ) {
        return Ok(0);
    }
    let done: Vec<&ProposalItem> = p.items.iter().filter(|i| i.status == "done").collect();
    let kept: Vec<&ProposalItem> = p
        .items
        .iter()
        .filter(|i| !i.selected && i.status == "skipped")
        .collect();
    if done.is_empty() && kept.is_empty() {
        return Ok(0);
    }
    let card: Option<String> = conn
        .query_row(
            "SELECT anchor FROM proposal WHERE id = ?1",
            [proposal_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        .map(|a| normalize_path(&env.expand(&a)));

    let tx = conn.unchecked_transaction()?;
    let all: Vec<&ProposalItem> = done.iter().chain(&kept).copied().collect();
    let mut rows: Vec<Option<Memory>> = Vec::with_capacity(all.len());
    for it in &all {
        rows.push(covering_row(&tx, env, &it.path)?);
    }
    let loose: Vec<&str> = all
        .iter()
        .zip(&rows)
        .filter(|(_, r)| r.is_none())
        .map(|(it, _)| it.path.as_str())
        .collect();
    let common = common_parent(&loose);

    let mut targets: Vec<Target> = Vec::new();
    let mut home: Vec<Option<usize>> = Vec::with_capacity(all.len());
    for (it, row) in all.iter().zip(rows) {
        let found = match row {
            Some(m) => Some((normalize_path(&env.expand(&m.key)), Some(m))),
            None => {
                pick_anchor(env, card.as_deref(), common.as_deref(), &it.path).map(|a| (a, None))
            }
        };
        let Some((abs, row)) = found else {
            home.push(None);
            continue;
        };
        let at = match targets
            .iter()
            .position(|t| t.key.eq_ignore_ascii_case(&abs))
        {
            Some(i) => i,
            None => {
                let detail = row
                    .as_ref()
                    .and_then(|m| m.detail.clone())
                    .unwrap_or_default();
                targets.push(Target {
                    row,
                    key: abs,
                    detail,
                });
                targets.len() - 1
            }
        };
        home.push(Some(at));
    }

    let rules: Vec<Option<String>> = done
        .iter()
        .zip(&home)
        .map(|(it, h)| h.and_then(|i| catalog_rule(env, &targets[i].key, &it.path)))
        .collect();
    let mut groups: HashMap<(String, String), usize> = HashMap::new();
    for (it, rule) in done.iter().zip(&rules) {
        if rule.is_none()
            && it.kind == "file"
            && let Some((dir, ext)) = file_parts(&it.path)
        {
            *groups.entry((dir.to_lowercase(), ext)).or_default() += 1;
        }
    }
    let key = |p: &str| normalize_key_with(Scope::Path, p, &env.profile);
    let kind = p.kind.as_str().to_string();
    let mut fresh: Vec<Vec<Cleaned>> = vec![Vec::new(); targets.len()];
    let mut fresh_kept: Vec<usize> = vec![0; targets.len()];
    for ((it, rule), h) in done.iter().zip(rules).zip(&home) {
        let Some(i) = *h else { continue };
        let rule = rule.unwrap_or_else(|| match file_parts(&it.path) {
            Some((dir, ext))
                if it.kind == "file"
                    && groups.get(&(dir.to_lowercase(), ext.clone())).copied() >= Some(2) =>
            {
                format!("{}\\*.{}", escape(dir), escape(&ext))
            }
            _ => escape(&it.path),
        });
        let freed: Option<i64> = tx
            .query_row(
                "SELECT freed_bytes FROM action WHERE item_id = ?1 AND status = 'done' ORDER BY id DESC LIMIT 1",
                [it.id],
                |r| r.get(0),
            )
            .optional()?;
        let c = Cleaned {
            ms: now,
            bytes: u64::try_from(freed.unwrap_or(it.alloc)).unwrap_or(0),
            path: key(&it.path),
            rule: key(&rule),
            job: proposal_id.to_string(),
            kind: kind.clone(),
        };
        let d = &mut targets[i].detail;
        d.cleaned
            .retain(|v| !entry_path(v).is_some_and(|x| x.eq_ignore_ascii_case(&c.path)));
        d.kept
            .retain(|v| !entry_path(v).is_some_and(|x| x.eq_ignore_ascii_case(&c.path)));
        push_capped(&mut d.cleaned, serde_json::to_value(&c)?);
        fresh[i].push(c);
    }
    for (it, h) in kept.iter().zip(&home[done.len()..]) {
        let Some(i) = *h else { continue };
        let k = Kept {
            ms: now,
            path: key(&it.path),
            size: u64::try_from(it.alloc).unwrap_or(0),
            job: proposal_id.to_string(),
        };
        let d = &mut targets[i].detail;
        d.kept
            .retain(|v| !entry_path(v).is_some_and(|x| x.eq_ignore_ascii_case(&k.path)));
        push_capped(&mut d.kept, serde_json::to_value(&k)?);
        fresh_kept[i] += 1;
    }

    let mut touched = 0;
    for (i, t) in targets.into_iter().enumerate() {
        if fresh[i].is_empty() && fresh_kept[i] == 0 {
            continue;
        }
        touched += 1;
        match &t.row {
            Some(m) => set_detail(&tx, m.id, &t.detail)?,
            None => {
                let text = if fresh[i].is_empty() {
                    format!("You kept {} here.", items(fresh_kept[i]))
                } else {
                    format!("You {} {} here.", fresh[i][0].verb(), describe(&fresh[i]))
                };
                let detail = Detail {
                    category: "unknown".into(),
                    by: Some("job".into()),
                    ..t.detail
                };
                knowledge::upsert_with(
                    &tx,
                    Knowledge {
                        scope: Scope::Path,
                        key: t.key,
                        kind: Kind::Verdict,
                        text,
                        source: format!("job:{proposal_id}"),
                        detail,
                    },
                    now,
                    &env.profile,
                )?;
            }
        }
    }
    tx.commit()?;
    Ok(touched)
}

fn items(n: usize) -> String {
    format!("{n} item{}", if n == 1 { "" } else { "s" })
}

fn describe(cleaned: &[Cleaned]) -> String {
    let rule = &cleaned[0].rule;
    let tail = rule.rsplit('\\').next().unwrap_or(rule);
    let shared = cleaned.iter().all(|c| c.rule == *rule);
    if shared && let Some(ext) = tail.strip_prefix("*.") {
        format!(".{} files", unescape(ext))
    } else if shared && rule.contains("**") {
        format!("{} folders", unescape(tail))
    } else if let [one] = cleaned {
        one.path
            .rsplit('\\')
            .next()
            .unwrap_or(&one.path)
            .to_string()
    } else {
        items(cleaned.len())
    }
}

fn learned_text(key: &str, cleaned: &[Cleaned], kept: &[Kept]) -> String {
    let n = kept.len();
    let verb = if n == 1 { "is" } else { "are" };
    let Some(first) = cleaned.first() else {
        return format!("{} you left unticked in {key} {verb} kept", items(n));
    };
    let bytes: u64 = cleaned.iter().map(|c| c.bytes).sum();
    let ms = cleaned.iter().map(|c| c.ms).max().unwrap_or(first.ms);
    let mut text = format!(
        "{} in {key} — {} {} on {}",
        describe(cleaned),
        first.verb(),
        humanize_bytes(bytes),
        short_date(ms)
    );
    if n > 0 {
        text.push_str(&format!("; {} you left unticked {verb} kept", items(n)));
    }
    text
}

pub fn learned_for(conn: &Connection, proposal_id: &str) -> anyhow::Result<Vec<Learned>> {
    let needle = format!("\"job\":{}", serde_json::to_string(proposal_id)?);
    let mut st = conn.prepare_cached(&format!(
        "SELECT {COLS} FROM memory WHERE scope = 'path' AND detail_json IS NOT NULL AND instr(detail_json, ?1) > 0 ORDER BY id"
    ))?;
    let rows = st
        .query_map([&needle], from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|m| {
            let d = m.detail.as_ref()?;
            let mut c = cleaned_entries(d);
            c.retain(|e| e.job == proposal_id);
            let mut k = kept_entries(d);
            k.retain(|e| e.job == proposal_id);
            (!c.is_empty() || !k.is_empty()).then(|| Learned {
                memory_id: m.id,
                text: learned_text(&m.key, &c, &k),
                confirmed: m.source == "user",
            })
        })
        .collect())
}

pub fn undo(conn: &Connection, proposal_id: &str, memory_id: i64) -> anyhow::Result<bool> {
    let Some(m) = get(conn, memory_id)? else {
        return Ok(false);
    };
    let Some(mut d) = m.detail.clone() else {
        return Ok(false);
    };
    let mine = |v: &Value| v.get("job").and_then(Value::as_str) == Some(proposal_id);
    let before = d.cleaned.len() + d.kept.len();
    d.cleaned.retain(|v| !mine(v));
    d.kept.retain(|v| !mine(v));
    if d.cleaned.len() + d.kept.len() == before {
        return Ok(false);
    }
    if is_job(&m.source) && !d.has_job_content() && !m.pinned {
        delete(conn, memory_id)?;
    } else {
        set_detail(conn, memory_id, &d)?;
    }
    Ok(true)
}

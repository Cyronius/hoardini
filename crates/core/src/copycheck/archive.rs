use std::path::Path;

use rusqlite::OptionalExtension;
use serde_json::json;

use super::{
    Confidence, CopyCheck, CopyResult, Cx, Find, Found, Subject, fingerprint, same_contents,
    with_fingerprint, with_note,
};

pub struct ArchiveCheck;

impl ArchiveCheck {
    pub const NAME: &'static str = "archive";
}

// File systems store mtimes coarser than milliseconds; a restored file keeps its original mtime.
const MTIME_SLACK_MS: i64 = 2_000;

struct Entry {
    id: i64,
    original: String,
    dest_uri: String,
    size: i64,
    archived_ms: i64,
    restored_ms: Option<i64>,
}

fn chain(path: &str) -> Vec<String> {
    let mut out = vec![path.to_string()];
    let mut cur = path.to_string();
    while let Some(i) = cur.rfind('\\') {
        cur.truncate(i);
        if cur.len() < 3 {
            break;
        }
        out.push(cur.clone());
    }
    out
}

fn entry_for(cx: &Cx<'_>, path: &str) -> Option<Entry> {
    let mut st = cx
        .conn
        .prepare_cached(
            "SELECT id, original_path, dest_uri, size, archived_ms, restored_ms FROM archive_entry
             WHERE original_path = ?1 COLLATE NOCASE ORDER BY archived_ms DESC, id DESC LIMIT 1",
        )
        .ok()?;
    for p in chain(path) {
        let e = st
            .query_row([&p], |r| {
                Ok(Entry {
                    id: r.get(0)?,
                    original: r.get(1)?,
                    dest_uri: r.get(2)?,
                    size: r.get(3)?,
                    archived_ms: r.get(4)?,
                    restored_ms: r.get(5)?,
                })
            })
            .optional()
            .ok()
            .flatten();
        if e.is_some() {
            return e;
        }
    }
    None
}

fn is_remote(uri: &str) -> bool {
    uri.starts_with(crate::destinations::rclone::URI_PREFIX)
}

impl CopyCheck for ArchiveCheck {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn applies(&self, cx: &Cx<'_>, s: &Subject) -> bool {
        entry_for(cx, &s.path).is_some()
    }

    fn covers_dir(&self, cx: &Cx<'_>, path: &str) -> bool {
        entry_for(cx, path).is_some_and(|e| e.original.eq_ignore_ascii_case(path))
    }

    fn still_valid(&self, _cx: &Cx<'_>, s: &Subject, r: &CopyResult) -> bool {
        r.locator
            .as_deref()
            .is_none_or(|l| is_remote(l) || Path::new(l).exists())
            && same_contents(s, r)
    }

    fn find(&self, cx: &Cx<'_>, s: &Subject) -> anyhow::Result<Find> {
        let Some(e) = entry_for(cx, &s.path) else {
            return Ok(Find::Skip);
        };
        let exact = e.original.eq_ignore_ascii_case(&s.path);
        let copy = if exact {
            e.dest_uri.clone()
        } else {
            format!(
                "{}{}",
                e.dest_uri.trim_end_matches(['\\', '/']),
                &s.path[e.original.len()..]
            )
        };
        let detail = json!({
            "archive_entry_id": e.id,
            "dest_uri": copy,
            "archived_ms": e.archived_ms,
            "restored_ms": e.restored_ms,
        });
        // Not restored: the archive removed the original, so whatever sits here now is a newer file.
        if e.restored_ms.is_none() {
            if exact && !s.dir && i64::try_from(s.size).ok() == Some(e.size) {
                return Ok(Find::Copy(Found {
                    locator: copy,
                    detail: json!({"archive_entry_id": e.id, "dest_uri": e.dest_uri, "newer": true}),
                }));
            }
            return Ok(Find::NoCopy(with_note(
                detail,
                "a newer file than the one Hoardini archived",
            )));
        }
        if !is_remote(&copy) && !Path::new(&copy).exists() {
            return Ok(Find::NoCopy(with_note(
                detail,
                "the archived copy is no longer reachable",
            )));
        }
        Ok(Find::Copy(Found {
            locator: copy,
            detail,
        }))
    }

    fn verify(&self, _cx: &Cx<'_>, s: &Subject, f: Found) -> anyhow::Result<CopyResult> {
        let d = &f.detail;
        let archived = d["archived_ms"].as_i64().unwrap_or(0);
        if d.get("newer").is_some() {
            let note = format!("same name and size as the archived copy at {}", f.locator);
            return Ok(CopyResult::new(
                Self::NAME,
                Some(f.locator),
                Confidence::SameNameAndSize,
                with_note(f.detail, note),
            ));
        }
        let (size, newest) = if s.dir {
            let fp = fingerprint(Path::new(&s.path));
            (fp.bytes, Some(fp.newest_ms))
        } else {
            (s.size, s.mtime)
        };
        let copy_size = if is_remote(&f.locator) {
            None
        } else if s.dir {
            Some(fingerprint(Path::new(&f.locator)).bytes)
        } else {
            std::fs::metadata(&f.locator).ok().map(|m| m.len())
        };
        let same_size = copy_size.is_none_or(|c| c == size);
        let unchanged = newest.is_some_and(|m| m <= archived + MTIME_SLACK_MS);
        let (conf, note) = match (same_size, unchanged) {
            (true, true) => (
                Confidence::Identical,
                format!("archived copy at {} (verified when archived)", f.locator),
            ),
            (true, false) => (
                Confidence::SameNameAndSize,
                format!(
                    "archived copy at {}; changed since it was archived",
                    f.locator
                ),
            ),
            (false, _) => (
                Confidence::None,
                "changed since it was archived".to_string(),
            ),
        };
        let locator = (conf != Confidence::None).then_some(f.locator);
        Ok(with_fingerprint(
            CopyResult::new(Self::NAME, locator, conf, with_note(f.detail, note)),
            s,
        ))
    }
}

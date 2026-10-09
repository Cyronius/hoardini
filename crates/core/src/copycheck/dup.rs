use rusqlite::params;
use serde_json::{Value, json};

use super::{
    Confidence, CopyCheck, CopyResult, Cx, FILE_MIN, Find, Found, Subject, hash, pending, with_note,
};
use crate::index::{self, FLAG_DENIED, FLAG_HARDLINK_DUP, FLAG_PLACEHOLDER, FileId};

pub struct DupCheck;

impl DupCheck {
    pub const NAME: &'static str = "dup";
}

struct Twin {
    path: String,
    subject: Subject,
}

// Other indexed files with the same logical size and a different file id (hard links are one file).
fn same_size(cx: &Cx<'_>, s: &Subject) -> anyhow::Result<Vec<Twin>> {
    let mut st = cx.conn.prepare_cached(
        "SELECT id, file_id FROM node WHERE kind = ?1 AND logical = ?2 AND (flags & ?3) = 0",
    )?;
    let rows: Vec<(i64, Option<FileId>)> = st
        .query_map(
            params![
                index::KIND_FILE,
                index::to_i64(s.size),
                FLAG_DENIED | FLAG_PLACEHOLDER | FLAG_HARDLINK_DUP
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::new();
    for (id, fid) in rows {
        if fid.is_some_and(|f| Some(f.0) == s.file_id) {
            continue;
        }
        let Some(path) = index::path_of(cx.conn, id)? else {
            continue;
        };
        if path.eq_ignore_ascii_case(&s.path) {
            continue;
        }
        let Some(subject) = Subject::load(cx.conn, &path) else {
            continue;
        };
        if subject.dir || subject.size != s.size || subject.file_id == s.file_id {
            continue;
        }
        out.push(Twin { path, subject });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

// A persisted duplicate group counts while both members still have the mtime and file id they were hashed with.
fn grouped_twin(cx: &Cx<'_>, s: &Subject, twins: &[Twin]) -> Option<(String, i64)> {
    let mut st = cx
        .conn
        .prepare_cached(
            "SELECT m.group_id, m.mtime, m.file_id FROM dup_member m WHERE m.path = ?1 COLLATE NOCASE",
        )
        .ok()?;
    let mine: Vec<(i64, Option<i64>, Option<Vec<u8>>)> = st
        .query_map([&s.path], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .ok()?
        .filter_map(Result::ok)
        .collect();
    let same = |m: &Option<i64>, f: &Option<Vec<u8>>, sub: &Subject| {
        *m == sub.mtime && f.as_deref() == sub.file_id.as_ref().map(|x| x.as_slice())
    };
    for (gid, mtime, fid) in mine {
        if !same(&mtime, &fid, s) {
            continue;
        }
        let mut st = cx
            .conn
            .prepare_cached(
                "SELECT path, mtime, file_id FROM dup_member WHERE group_id = ?1 AND path <> ?2 COLLATE NOCASE ORDER BY path",
            )
            .ok()?;
        let others: Vec<(String, Option<i64>, Option<Vec<u8>>)> = st
            .query_map(params![gid, s.path], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .ok()?
            .filter_map(Result::ok)
            .collect();
        for (p, m, f) in others {
            if let Some(t) = twins.iter().find(|t| t.path.eq_ignore_ascii_case(&p))
                && same(&m, &f, &t.subject)
            {
                return Some((t.path.clone(), gid));
            }
        }
    }
    None
}

fn state_of(s: &Subject) -> Value {
    json!({"mtime": s.mtime, "size": s.size, "file_id": s.file_id.map(|f| hash::hex(&f))})
}

fn twin_state(mut detail: Value, twin: Option<&Subject>) -> Value {
    if let Some(t) = twin {
        detail["twin"] = state_of(t);
    }
    detail
}

impl CopyCheck for DupCheck {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn applies(&self, _cx: &Cx<'_>, s: &Subject) -> bool {
        !s.dir && s.size >= FILE_MIN && !s.is_placeholder()
    }

    // The answer is only as good as the twin: one that changed since makes it stale.
    fn still_valid(&self, cx: &Cx<'_>, _s: &Subject, r: &CopyResult) -> bool {
        let Some(loc) = r.locator.as_deref() else {
            return true;
        };
        Subject::load(cx.conn, loc).is_some_and(|t| r.detail.get("twin") == Some(&state_of(&t)))
    }

    fn find(&self, cx: &Cx<'_>, s: &Subject) -> anyhow::Result<Find> {
        let twins = same_size(cx, s)?;
        if twins.is_empty() {
            return Ok(Find::NoCopy(with_note(
                json!({}),
                "no other file of this size on the indexed drives",
            )));
        }
        if let Some((path, gid)) = grouped_twin(cx, s, &twins) {
            let t = twins.iter().find(|t| t.path == path).map(|t| &t.subject);
            return Ok(Find::Copy(Found {
                locator: path,
                detail: twin_state(json!({"via": "duplicate group", "group_id": gid}), t),
            }));
        }
        let mine = hash::current(cx.conn, s);
        let mut unhashed = 0usize;
        for t in &twins {
            match hash::current(cx.conn, &t.subject) {
                Some(h) if mine.as_deref() == Some(h.as_str()) => {
                    return Ok(Find::Copy(Found {
                        locator: t.path.clone(),
                        detail: twin_state(json!({"via": "sha256"}), Some(&t.subject)),
                    }));
                }
                Some(_) => {}
                None => unhashed += 1,
            }
        }
        let detail = json!({"same_size": twins.len()});
        if mine.is_none() || unhashed > 0 {
            let note = format!("{} file(s) of the same size, not compared yet", twins.len());
            return Ok(Find::NoCopy(pending(with_note(detail, note), "hash")));
        }
        Ok(Find::NoCopy(with_note(
            detail,
            "files of the same size differ in content",
        )))
    }

    // Both paths were read when found; a group or SHA-256 match is a full-content match.
    fn verify(&self, _cx: &Cx<'_>, _s: &Subject, f: Found) -> anyhow::Result<CopyResult> {
        let note = format!("identical copy at {}", f.locator);
        Ok(CopyResult::new(
            Self::NAME,
            Some(f.locator),
            Confidence::Identical,
            with_note(f.detail, note),
        ))
    }
}

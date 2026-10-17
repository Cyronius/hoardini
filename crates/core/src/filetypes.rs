use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use crate::index;
use crate::memory::knowledge::{self, Detail, MODEL_WEIGHTS, WEIGHT_EXTS};
use crate::memory::location::{DOCUMENT_EXTS, PHOTO_VIDEO_EXTS};

pub const MAX_TYPES: usize = 200;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TypeRow {
    pub ext: Option<String>,
    pub bytes: u64,
    pub files: u64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub small: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub use_: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

fn fixed_category(ext: &str) -> Option<&'static str> {
    let has = |list: &[&str]| list.iter().any(|e| e.eq_ignore_ascii_case(ext));
    if has(PHOTO_VIDEO_EXTS) {
        Some("photos")
    } else if has(DOCUMENT_EXTS) {
        Some("documents")
    } else if has(WEIGHT_EXTS) {
        Some(MODEL_WEIGHTS)
    } else {
        None
    }
}

pub fn types(conn: &Connection, node_id: i64) -> Result<Vec<TypeRow>> {
    let mix = index::ext_mix(conn, node_id)?;
    let mut known: HashMap<String, (String, Detail)> = HashMap::new();
    for m in knowledge::filetypes(conn)? {
        if m.forgotten_ms.is_some() {
            continue;
        }
        if let Some(ext) = m.key.rsplit("*.").next() {
            known.insert(
                ext.to_ascii_lowercase(),
                (m.text.clone(), m.detail.unwrap_or_default()),
            );
        }
    }
    let mut out: Vec<TypeRow> = Vec::new();
    let mut rest = (0u64, 0u64, 0u64);
    for t in mix.types {
        if out.len() >= MAX_TYPES {
            rest.0 += 1;
            rest.1 += t.bytes;
            rest.2 += t.files;
            continue;
        }
        let row = known.get(&t.ext);
        let category = row
            .map(|(_, d)| d.category.clone())
            .filter(|c| !c.is_empty())
            .or_else(|| fixed_category(&t.ext).map(str::to_string));
        out.push(TypeRow {
            label: row.map(|(text, _)| text.clone()),
            app: row.and_then(|(_, d)| d.app.clone()),
            use_: row.and_then(|(_, d)| d.use_.clone()),
            category,
            ext: Some(t.ext),
            bytes: t.bytes,
            files: t.files,
            small: false,
            other: None,
        });
    }
    if mix.small_files > 0 || mix.small_bytes > 0 {
        let at = out
            .iter()
            .position(|r| r.bytes < mix.small_bytes)
            .unwrap_or(out.len());
        out.insert(
            at,
            TypeRow {
                ext: None,
                bytes: mix.small_bytes,
                files: mix.small_files,
                small: true,
                other: None,
                label: None,
                app: None,
                use_: None,
                category: None,
            },
        );
    }
    if rest.0 > 0 {
        out.push(TypeRow {
            ext: None,
            bytes: rest.1,
            files: rest.2,
            small: false,
            other: Some(rest.0),
            label: None,
            app: None,
            use_: None,
            category: None,
        });
    }
    Ok(out)
}

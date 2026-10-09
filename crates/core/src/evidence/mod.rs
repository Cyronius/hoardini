pub mod cache;
pub mod gguf;
pub mod identify;
pub mod inspect;
pub mod inuse;
pub mod owner;
pub mod pe;
pub mod safetensors;
pub mod summary;
pub mod usage;

use serde::{Deserialize, Serialize};

pub use gguf::GgufInfo;
pub use identify::identify;
pub use inspect::inspect;
pub use pe::PeInfo;
pub use safetensors::SafetensorsInfo;

use crate::catalog::CatalogMatch;
use crate::memory::Memory;
use crate::paths::humanize_bytes;
use crate::policy::Verdict;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathKind {
    File,
    Dir,
    Link,
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Family {
    Model,
    Executable,
    DiskImage,
    Archive,
    Video,
    Image,
    Audio,
    Document,
    Database,
    Other,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    pub kind: String,
    pub family: Family,
    pub mime: Option<String>,
    // Compression heuristics read this; recompressing such data gains nothing.
    pub already_compressed: bool,
    pub gguf: Option<GgufInfo>,
    pub safetensors: Option<SafetensorsInfo>,
    pub pe: Option<PeInfo>,
    pub note: Option<String>,
}

impl Identity {
    pub fn unknown_no_access() -> Identity {
        Identity {
            kind: "unknown".into(),
            family: Family::Other,
            mime: None,
            already_compressed: false,
            gguf: None,
            safetensors: None,
            pe: None,
            note: Some("unknown (no access)".into()),
        }
    }

    pub fn describe(&self) -> String {
        if let Some(g) = &self.gguf {
            let mut parts = vec!["GGUF model".to_string()];
            if let Some(n) = &g.name {
                parts.push(n.clone());
            }
            if let Some(a) = &g.architecture {
                parts.push(a.clone());
            }
            if let Some(q) = &g.quant {
                parts.push(q.clone());
            }
            if let Some(p) = g.parameters {
                parts.push(format!("{} params", short_count(p)));
            }
            return parts.join(", ");
        }
        if let Some(s) = &self.safetensors {
            let dt = s.dominant_dtype().unwrap_or("?");
            return format!("safetensors, {} params, {dt}", short_count(s.parameters));
        }
        if let Some(pe) = &self.pe {
            let mut label = pe
                .product_name
                .clone()
                .or_else(|| pe.file_description.clone())
                .unwrap_or_else(|| self.kind.clone());
            if let Some(v) = &pe.product_version {
                label.push(' ');
                label.push_str(v);
            }
            if let Some(c) = &pe.company_name {
                label.push_str(&format!(" ({c})"));
            }
            return label;
        }
        self.kind.clone()
    }
}

pub fn short_count(n: u64) -> String {
    let f = n as f64;
    if f >= 1e9 {
        format!("{:.1}B", f / 1e9)
    } else if f >= 1e6 {
        format!("{:.1}M", f / 1e6)
    } else if f >= 1e3 {
        format!("{:.1}K", f / 1e3)
    } else {
        n.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LastUsedSignal {
    pub source: String,
    pub value_ms: Option<i64>,
    pub confidence: Confidence,
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OwnerApp {
    pub name: String,
    pub publisher: Option<String>,
    pub install_location: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcessRef {
    pub pid: u32,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DuplicateRef {
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence {
    pub path: String,
    pub kind: PathKind,
    pub alloc_bytes: Option<u64>,
    pub logical_bytes: Option<u64>,
    pub file_count: Option<u64>,
    pub catalog: Option<CatalogMatch>,
    pub identity: Option<Identity>,
    pub owner: Option<OwnerApp>,
    pub in_use: Vec<ProcessRef>,
    pub last_used: Vec<LastUsedSignal>,
    pub duplicates: Vec<DuplicateRef>,
    #[serde(default)]
    pub copies: Vec<crate::copycheck::CopyResult>,
    #[serde(default)]
    pub only_copy: bool,
    pub memories: Vec<Memory>,
    pub policy_cap: Option<Verdict>,
    pub summary: String,
}

impl Evidence {
    pub fn new(path: impl Into<String>, kind: PathKind) -> Evidence {
        Evidence {
            path: path.into(),
            kind,
            alloc_bytes: None,
            logical_bytes: None,
            file_count: None,
            catalog: None,
            identity: None,
            owner: None,
            in_use: Vec::new(),
            last_used: Vec::new(),
            duplicates: Vec::new(),
            copies: Vec::new(),
            only_copy: false,
            memories: Vec::new(),
            policy_cap: None,
            summary: String::new(),
        }
    }

    pub fn compute_summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let size = self
            .alloc_bytes
            .map(|b| format!(", {}", humanize_bytes(b)))
            .unwrap_or_default();
        parts.push(match self.kind {
            PathKind::File => format!("file{size}"),
            PathKind::Dir => format!("folder{size}"),
            PathKind::Link => format!("link{size}"),
            PathKind::Missing => "missing".to_string(),
        });
        if let Some(c) = &self.catalog {
            let mut s = format!("{} ({}); {}", c.name, c.app, c.notes);
            if c.shared_blobs {
                s.push_str(" Shared blobs: delete only whole units.");
            }
            parts.push(s);
        }
        if let Some(i) = &self.identity {
            parts.push(i.describe());
        }
        if let Some(o) = &self.owner {
            parts.push(format!("owner {}", o.name));
        }
        if !self.in_use.is_empty() {
            let names: Vec<&str> = self
                .in_use
                .iter()
                .take(3)
                .map(|p| p.name.as_str())
                .collect();
            parts.push(format!("IN USE by {}", names.join(", ")));
        }
        if !self.duplicates.is_empty() {
            parts.push(format!("{} duplicate(s)", self.duplicates.len()));
        }
        if let Some(c) = self.copy_phrase() {
            parts.push(c);
        }
        if let Some(m) = self
            .memories
            .iter()
            .find(|m| m.kind == crate::memory::Kind::Protect)
        {
            parts.push(format!("PROTECTED by user note: {}", m.text));
        } else if !self.memories.is_empty() {
            parts.push(format!("{} saved note(s)", self.memories.len()));
        }
        if let Some(cap) = self.policy_cap {
            parts.push(format!("verdict capped at {}", cap.as_str()));
        }
        parts.join("; ")
    }

    // The strongest copy-check answer, or "only copy we know of" when every check came up empty.
    pub fn copy_phrase(&self) -> Option<String> {
        let best = crate::copycheck::best(&self.copies)?;
        if best.confidence == crate::copycheck::Confidence::None {
            return self.only_copy.then(|| "only copy we know of".to_string());
        }
        Some(best.note())
    }

    pub fn with_summary(mut self) -> Evidence {
        self.summary = self.compute_summary();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_leads_with_the_decisive_facts() {
        let mut e = Evidence::new(r"C:\x\a.bin", PathKind::File);
        e.alloc_bytes = Some(45_634_027_520);
        e.in_use.push(ProcessRef {
            pid: 4,
            name: "llama-server.exe".into(),
        });
        e.policy_cap = Some(Verdict::Keep);
        let s = e.with_summary().summary;
        assert_eq!(
            s,
            "file, 42.5 GB; IN USE by llama-server.exe; verdict capped at keep"
        );
    }

    #[test]
    fn evidence_round_trips_json() {
        let e = Evidence::new(r"C:\x", PathKind::Dir).with_summary();
        let back: Evidence = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back.summary, "folder");
    }
}

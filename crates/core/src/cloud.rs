use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::index::{is_ancestor_path, normalize_path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncRoot {
    pub path: String,
    pub provider: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudState {
    pub in_sync: bool,
    pub pinned: bool,
    pub local_bytes: u64,
}

// The Win32 side (registry roots, placeholder state, dehydrate) behind a trait so the decisions are testable.
pub trait Cloud: Send + Sync {
    fn roots(&self) -> Vec<SyncRoot>;
    // None: an ordinary file, not a cloud placeholder.
    fn state(&self, path: &Path) -> std::io::Result<Option<CloudState>>;
    // Unpins and drops the local data of one in-sync file; returns the bytes freed.
    fn free(&self, path: &Path) -> std::io::Result<u64>;
}

pub fn root_for<'a>(roots: &'a [SyncRoot], path: &str) -> Option<&'a SyncRoot> {
    let p = normalize_path(path);
    roots
        .iter()
        .filter(|r| {
            let root = normalize_path(&r.path);
            root.eq_ignore_ascii_case(&p) || is_ancestor_path(&root, &p)
        })
        .max_by_key(|r| r.path.len())
}

#[derive(Default)]
pub struct SystemCloud {
    cache: Mutex<Option<(Instant, Vec<SyncRoot>)>>,
}

const ROOTS_TTL: Duration = Duration::from_secs(60);

pub fn system() -> Arc<dyn Cloud> {
    static ONE: std::sync::OnceLock<Arc<SystemCloud>> = std::sync::OnceLock::new();
    ONE.get_or_init(|| Arc::new(SystemCloud::default())).clone()
}

fn registered_roots() -> Vec<SyncRoot> {
    let regs = hoardini_win::sync_root_registrations().unwrap_or_else(|e| {
        tracing::debug!(error = %e, "sync roots unreadable");
        Vec::new()
    });
    let mut out = Vec::new();
    for r in regs {
        let provider = hoardini_win::provider_display_name(&r.id);
        for p in r.paths {
            out.push(SyncRoot {
                path: normalize_path(&p),
                provider: provider.clone(),
            });
        }
    }
    out
}

impl Cloud for SystemCloud {
    fn roots(&self) -> Vec<SyncRoot> {
        let mut g = self.cache.lock();
        if let Some((at, roots)) = g.as_ref()
            && at.elapsed() < ROOTS_TTL
        {
            return roots.clone();
        }
        let roots = registered_roots();
        *g = Some((Instant::now(), roots.clone()));
        roots
    }

    fn state(&self, path: &Path) -> std::io::Result<Option<CloudState>> {
        Ok(hoardini_win::placeholder_info(path)?.map(|i| CloudState {
            in_sync: i.in_sync,
            pinned: i.pinned,
            local_bytes: i.on_disk_bytes,
        }))
    }

    fn free(&self, path: &Path) -> std::io::Result<u64> {
        hoardini_win::free_up_space(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deepest_root_holding_the_path_wins() {
        let roots = vec![
            SyncRoot {
                path: r"C:\Users\a\OneDrive".into(),
                provider: "OneDrive".into(),
            },
            SyncRoot {
                path: r"C:\Users\a\OneDrive\Work".into(),
                provider: "OneDrive - Work".into(),
            },
        ];
        let hit = |p: &str| root_for(&roots, p).map(|r| r.provider.as_str());
        assert_eq!(
            hit(r"c:\users\A\onedrive\Work\x.mp4"),
            Some("OneDrive - Work")
        );
        assert_eq!(hit(r"C:\Users\a\OneDrive\Pictures"), Some("OneDrive"));
        assert_eq!(hit(r"C:\Users\a\OneDrive"), Some("OneDrive"));
        assert_eq!(hit(r"C:\Users\a\OneDriveX\y"), None);
    }
}

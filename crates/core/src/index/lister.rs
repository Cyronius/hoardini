use std::io;
use std::path::Path;

use hoardini_win::{DirEntry, FILE_ATTRIBUTE_SPARSE_FILE, VolumeInfo};

use super::join_path;

pub trait DirLister: Send + Sync {
    fn list_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>>;
    fn alloc_size_by_open(&self, path: &Path) -> io::Result<u64>;
    fn volume_info(&self, _root: &Path) -> io::Result<VolumeInfo> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WinLister;

impl DirLister for WinLister {
    fn list_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        hoardini_win::list_dir(path)
    }

    fn alloc_size_by_open(&self, path: &Path) -> io::Result<u64> {
        hoardini_win::alloc_size_by_open(path)
    }

    fn volume_info(&self, root: &Path) -> io::Result<VolumeInfo> {
        hoardini_win::volume_info(root)
    }
}

// WOF files list as alloc 0 with no reparse tag (measured); sparse and cloud zeros are already true.
pub(crate) fn needs_alloc_by_open(e: &DirEntry) -> bool {
    !e.is_dir()
        && e.alloc == 0
        && e.logical > 0
        && e.attrs & FILE_ATTRIBUTE_SPARSE_FILE == 0
        && !e.is_placeholder()
}

pub(crate) fn list_fixed(lister: &dyn DirLister, dir: &str) -> io::Result<Vec<DirEntry>> {
    let mut entries = lister.list_dir(Path::new(dir))?;
    for e in entries.iter_mut().filter(|e| needs_alloc_by_open(e)) {
        let path = join_path(dir, &e.name);
        e.alloc = match lister.alloc_size_by_open(Path::new(&path)) {
            Ok(a) => a,
            Err(err) => {
                tracing::debug!(%path, error = %err, "WOF size unavailable, using logical size");
                e.logical
            }
        };
    }
    Ok(entries)
}

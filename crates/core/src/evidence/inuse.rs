use std::path::PathBuf;

use super::ProcessRef;

pub const DIR_FILE_CAP: usize = 500;

// Restart Manager takes files only, so a folder is checked through its largest indexed files.
pub fn holders(files: &[PathBuf]) -> Vec<ProcessRef> {
    let mut out: Vec<ProcessRef> = hoardini_win::processes_using(files)
        .unwrap_or_default()
        .into_iter()
        .map(|p| ProcessRef {
            pid: p.pid,
            name: p
                .exe
                .as_deref()
                .and_then(|e| e.rsplit('\\').next())
                .map_or(p.app_name, str::to_string),
        })
        .collect();
    out.sort_by_key(|p| p.pid);
    out.dedup_by_key(|p| p.pid);
    out
}

// INTERFACE CONTRACT (wave 0). Signatures are shared by every crate; bodies below are portable
// fallbacks so dependents can build and test before the real Win32 implementations land.
#![allow(clippy::missing_safety_doc)]

use std::io;
use std::path::{Path, PathBuf};

pub const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
pub const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
pub const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;
pub const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x800;
pub const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
pub const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x40000;
pub const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x400000;

pub const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
pub const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
pub const IO_REPARSE_TAG_WOF: u32 = 0x8000_0017;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub attrs: u32,
    pub reparse_tag: u32,
    pub alloc: u64,
    pub logical: u64,
    pub ctime_ms: i64,
    pub atime_ms: i64,
    pub mtime_ms: i64,
    pub file_id: [u8; 16],
}

impl DirEntry {
    pub fn is_dir(&self) -> bool {
        self.attrs & FILE_ATTRIBUTE_DIRECTORY != 0
    }
    pub fn is_reparse(&self) -> bool {
        self.attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    pub fn is_link(&self) -> bool {
        self.is_reparse()
            && matches!(
                self.reparse_tag,
                IO_REPARSE_TAG_MOUNT_POINT | IO_REPARSE_TAG_SYMLINK
            )
    }
    pub fn is_placeholder(&self) -> bool {
        is_cloud_placeholder(self.attrs)
    }
}

pub fn is_cloud_placeholder(attrs: u32) -> bool {
    attrs
        & (FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
            | FILE_ATTRIBUTE_RECALL_ON_OPEN
            | FILE_ATTRIBUTE_OFFLINE)
        != 0
}

// One bulk listing of a directory: no per-file open. Excludes "." and "..".
pub fn list_dir(path: &Path) -> io::Result<Vec<DirEntry>> {
    use std::os::windows::fs::MetadataExt;
    let mut out = Vec::new();
    for e in std::fs::read_dir(path)? {
        let e = e?;
        let md = std::fs::symlink_metadata(e.path())?;
        let attrs = md.file_attributes();
        let to_ms = |ft: u64| filetime_to_unix_ms(ft);
        out.push(DirEntry {
            name: e.file_name().to_string_lossy().into_owned(),
            attrs,
            reparse_tag: 0,
            alloc: if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
                0
            } else {
                md.file_size()
            },
            logical: if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
                0
            } else {
                md.file_size()
            },
            ctime_ms: to_ms(md.creation_time()),
            atime_ms: to_ms(md.last_access_time()),
            mtime_ms: to_ms(md.last_write_time()),
            file_id: [0; 16],
        });
    }
    Ok(out)
}

// Allocated size via FILE_STANDARD_INFO for entries whose listing reports 0 (WOF-compressed).
pub fn alloc_size_by_open(_path: &Path) -> io::Result<u64> {
    Err(unsupported())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VolumeInfo {
    pub root: String,
    pub serial: u64,
    pub fs_name: String,
    pub label: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

pub fn volume_info(_root: &Path) -> io::Result<VolumeInfo> {
    Err(unsupported())
}

pub fn volume_root_of(_path: &Path) -> io::Result<PathBuf> {
    Err(unsupported())
}

pub fn fixed_volumes() -> io::Result<Vec<PathBuf>> {
    Err(unsupported())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LastAccessSetting {
    pub raw: u32,
    pub enabled: bool,
    pub system_managed: bool,
}

pub fn last_access_setting() -> io::Result<LastAccessSetting> {
    Err(unsupported())
}

// PROCESS_MODE_BACKGROUND_BEGIN + EcoQoS for the current process.
pub fn enter_background_mode() -> io::Result<()> {
    Err(unsupported())
}

// Kept alive for the life of the process; dropping it closes the job handle.
pub struct JobGuard {
    _private: (),
}

pub fn cap_process_memory(_bytes: u64) -> io::Result<JobGuard> {
    Err(unsupported())
}

pub fn current_working_set() -> io::Result<u64> {
    Err(unsupported())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessUse {
    pub pid: u32,
    pub app_name: String,
    pub exe: Option<String>,
}

// Restart Manager: processes holding any of these files open. Files only, not directories.
pub fn processes_using(_files: &[PathBuf]) -> io::Result<Vec<ProcessUse>> {
    Err(unsupported())
}

pub fn dpapi_protect(_data: &[u8], _entropy: &[u8]) -> io::Result<Vec<u8>> {
    Err(unsupported())
}

pub fn dpapi_unprotect(_data: &[u8], _entropy: &[u8]) -> io::Result<Vec<u8>> {
    Err(unsupported())
}

// Opens for reading with last-access updates suspended on this handle (SetFileTime 0xFFFFFFFF).
pub fn open_read_preserving_atime(_path: &Path) -> io::Result<std::fs::File> {
    Err(unsupported())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstalledProgram {
    pub name: String,
    pub publisher: Option<String>,
    pub version: Option<String>,
    pub install_location: Option<String>,
    pub uninstall_key: String,
}

pub fn installed_programs() -> io::Result<Vec<InstalledProgram>> {
    Err(unsupported())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserAssistEntry {
    pub name: String,
    pub run_count: u32,
    pub last_run_ms: Option<i64>,
}

pub fn user_assist() -> io::Result<Vec<UserAssistEntry>> {
    Err(unsupported())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsnJournal {
    pub journal_id: u64,
    pub first_usn: i64,
    pub next_usn: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsnRecord {
    pub usn: i64,
    pub file_id: [u8; 16],
    pub parent_id: [u8; 16],
    pub reason: u32,
    pub attrs: u32,
}

pub fn usn_query(_volume_root: &Path) -> io::Result<UsnJournal> {
    Err(unsupported())
}

// FSCTL_READ_UNPRIVILEGED_USN_JOURNAL from start_usn; returns records and the next USN to resume from.
pub fn usn_read(
    _volume_root: &Path,
    _journal_id: u64,
    _start_usn: i64,
) -> io::Result<(Vec<UsnRecord>, i64)> {
    Err(unsupported())
}

pub fn path_by_file_id(_volume_root: &Path, _file_id: [u8; 16]) -> io::Result<PathBuf> {
    Err(unsupported())
}

// WNetAddConnection2W; None credentials use the current Windows session.
pub fn smb_connect(_unc: &str, _user: Option<&str>, _password: Option<&str>) -> io::Result<()> {
    Err(unsupported())
}

pub fn filetime_to_unix_ms(ft: u64) -> i64 {
    const EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;
    if ft == 0 {
        return 0;
    }
    (ft as i64 - EPOCH_DIFF_100NS) / 10_000
}

fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "not implemented yet")
}

#![allow(clippy::missing_safety_doc)]

mod atime;
mod dpapi;
mod enumerate;
mod fsutil;
mod net;
mod process;
mod registry;
mod rm;
mod usn;
mod util;
mod volume;

pub use atime::open_read_preserving_atime;
pub use dpapi::{dpapi_protect, dpapi_unprotect};
pub use enumerate::{alloc_size_by_open, list_dir, list_dir_each};
pub use fsutil::{create_junction, file_id_of, set_file_times, set_sparse_and_zero_range};
pub use net::smb_connect;
pub use process::{
    JobGuard, cap_process_memory, current_working_set, enter_background_mode, leave_background_mode,
};
pub use registry::{installed_programs, last_access_setting, user_assist};
pub use rm::processes_using;
pub use usn::{path_by_file_id, usn_query, usn_read};
pub use volume::{fixed_volumes, volume_info, volume_root_of};

pub const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
pub const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
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
    // WOF hides its reparse tag from listings and reports alloc 0, so this shape is the only signal.
    pub fn needs_alloc_by_open(&self) -> bool {
        !self.is_dir()
            && self.alloc == 0
            && self.logical > 0
            && self.attrs & FILE_ATTRIBUTE_SPARSE_FILE == 0
            && !self.is_placeholder()
            && (!self.is_reparse() || self.reparse_tag == IO_REPARSE_TAG_WOF)
    }
}

pub fn is_cloud_placeholder(attrs: u32) -> bool {
    attrs
        & (FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
            | FILE_ATTRIBUTE_RECALL_ON_OPEN
            | FILE_ATTRIBUTE_OFFLINE)
        != 0
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LastAccessSetting {
    pub raw: u32,
    pub enabled: bool,
    pub system_managed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessUse {
    pub pid: u32,
    pub app_name: String,
    pub exe: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstalledProgram {
    pub name: String,
    pub publisher: Option<String>,
    pub version: Option<String>,
    pub install_location: Option<String>,
    pub uninstall_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserAssistEntry {
    pub name: String,
    pub run_count: u32,
    pub last_run_ms: Option<i64>,
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

pub fn filetime_to_unix_ms(ft: u64) -> i64 {
    const EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;
    if ft == 0 {
        return 0;
    }
    (ft as i64 - EPOCH_DIFF_100NS) / 10_000
}

pub fn unix_ms_to_filetime(ms: i64) -> u64 {
    const EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;
    (ms * 10_000 + EPOCH_DIFF_100NS).max(0) as u64
}

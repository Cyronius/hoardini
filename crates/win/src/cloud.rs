use std::io;
use std::mem::size_of;
use std::os::windows::io::OwnedHandle;
use std::path::Path;

use windows::Win32::Foundation::{ERROR_NOT_A_CLOUD_FILE, WIN32_ERROR};
use windows::Win32::Storage::CloudFilters::{
    CF_DEHYDRATE_FLAG_NONE, CF_IN_SYNC_STATE_IN_SYNC, CF_PIN_STATE, CF_PIN_STATE_PINNED,
    CF_PIN_STATE_UNPINNED, CF_PLACEHOLDER_INFO_STANDARD, CF_PLACEHOLDER_STANDARD_INFO,
    CF_SET_PIN_FLAG_NONE, CfDehydratePlaceholder, CfGetPlaceholderInfo, CfSetPinState,
};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_READ_ATTRIBUTES, FILE_STANDARD_INFO,
    FILE_WRITE_ATTRIBUTES, FileStandardInfo, GetFileInformationByHandleEx, WRITE_DAC,
};

use crate::util::{open, raw, werr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaceholderInfo {
    pub in_sync: bool,
    pub pinned: bool,
    pub unpinned: bool,
    pub on_disk_bytes: u64,
}

// NO_RECALL keeps a metadata open from ever pulling the file's data down.
fn open_meta(path: &Path, write: bool) -> io::Result<OwnedHandle> {
    let mut access = FILE_READ_ATTRIBUTES.0;
    if write {
        access |= FILE_WRITE_ATTRIBUTES.0 | WRITE_DAC.0;
    }
    open(
        path,
        access,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_NO_RECALL,
    )
}

fn is_not_cloud(e: &windows::core::Error) -> bool {
    WIN32_ERROR::from_error(e) == Some(ERROR_NOT_A_CLOUD_FILE)
}

fn info_of(h: &OwnedHandle) -> io::Result<Option<PlaceholderInfo>> {
    Ok(raw_info(h)?.map(|(i, _)| i))
}

fn raw_info(h: &OwnedHandle) -> io::Result<Option<(PlaceholderInfo, CF_PIN_STATE)>> {
    // The identity blob trails the fixed part and is at most 4 KiB.
    let mut buf = vec![0u64; (size_of::<CF_PLACEHOLDER_STANDARD_INFO>() + 4096) / 8 + 1];
    let r = unsafe {
        CfGetPlaceholderInfo(
            raw(h),
            CF_PLACEHOLDER_INFO_STANDARD,
            buf.as_mut_ptr().cast(),
            (buf.len() * 8) as u32,
            None,
        )
    };
    match r {
        Ok(()) => {}
        Err(e) if is_not_cloud(&e) => return Ok(None),
        Err(e) => return Err(werr(e)),
    }
    let info = unsafe { &*(buf.as_ptr() as *const CF_PLACEHOLDER_STANDARD_INFO) };
    Ok(Some((
        PlaceholderInfo {
            in_sync: info.InSyncState == CF_IN_SYNC_STATE_IN_SYNC,
            pinned: info.PinState == CF_PIN_STATE_PINNED,
            unpinned: info.PinState == CF_PIN_STATE_UNPINNED,
            on_disk_bytes: u64::try_from(info.OnDiskDataSize).unwrap_or(0),
        },
        info.PinState,
    )))
}

pub fn placeholder_info(path: &Path) -> io::Result<Option<PlaceholderInfo>> {
    info_of(&open_meta(path, false)?)
}

fn alloc_of(h: &OwnedHandle) -> io::Result<u64> {
    let mut info = FILE_STANDARD_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            raw(h),
            FileStandardInfo,
            (&mut info as *mut FILE_STANDARD_INFO).cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    }
    .map_err(werr)?;
    Ok(info.AllocationSize as u64)
}

// Explorer's "Free up space": unpin, then drop the local data. The cloud filter refuses unless the file is in sync.
pub fn free_up_space(path: &Path) -> io::Result<u64> {
    let h = open_meta(path, true)?;
    let Some((info, prior)) = raw_info(&h)? else {
        return Err(io::Error::other("not a cloud file"));
    };
    if !info.in_sync {
        return Err(io::Error::other("not in sync with the cloud"));
    }
    let before = alloc_of(&h)?;
    unsafe { CfSetPinState(raw(&h), CF_PIN_STATE_UNPINNED, CF_SET_PIN_FLAG_NONE, None) }
        .map_err(werr)?;
    if let Err(e) = unsafe { CfDehydratePlaceholder(raw(&h), 0, -1, CF_DEHYDRATE_FLAG_NONE, None) }
    {
        // A failed free must not leave the file unpinned, or the provider frees it later anyway.
        let _ = unsafe { CfSetPinState(raw(&h), prior, CF_SET_PIN_FLAG_NONE, None) };
        return Err(werr(e));
    }
    let after = alloc_of(&h)?;
    Ok(before.saturating_sub(after))
}

pub fn provider_display_name(registration: &str) -> String {
    let head = registration.split('!').next().unwrap_or(registration);
    let low = head.to_ascii_lowercase();
    if low.starts_with("onedrive") {
        "OneDrive".into()
    } else if low.starts_with("dropbox") {
        "Dropbox".into()
    } else if low.starts_with("googledrive") || low.starts_with("drivefs") {
        "Google Drive".into()
    } else if low.starts_with("icloud") {
        "iCloud Drive".into()
    } else if low.starts_with("box") {
        "Box".into()
    } else {
        head.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_names_come_from_the_registration_id() {
        assert_eq!(
            provider_display_name("OneDrive!S-1-5-21-1!Personal|ABC"),
            "OneDrive"
        );
        assert_eq!(
            provider_display_name("Dropbox!S-1-5-21-1!dbid:AA"),
            "Dropbox"
        );
        assert_eq!(provider_display_name("iCloudDrive!S-1!x"), "iCloud Drive");
        assert_eq!(provider_display_name("GoogleDrive!S-1!x"), "Google Drive");
        assert_eq!(provider_display_name("Nextcloud!S-1!x"), "Nextcloud");
    }

    #[test]
    fn an_ordinary_file_is_not_a_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("plain.bin");
        std::fs::write(&f, b"data").unwrap();
        assert_eq!(placeholder_info(&f).unwrap(), None);
        assert!(free_up_space(&f).is_err());
        assert_eq!(std::fs::read(&f).unwrap(), b"data");
    }
}

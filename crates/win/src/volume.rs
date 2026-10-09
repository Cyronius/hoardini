use std::io;
use std::path::{Path, PathBuf};

use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW, GetVolumeInformationW,
    GetVolumePathNameW, QueryDosDeviceW,
};
use windows::core::PCWSTR;

use crate::VolumeInfo;
use crate::util::{from_wide, werr, wide, wide_str};

const DRIVE_FIXED: u32 = 3;

fn with_trailing_slash(root: &Path) -> PathBuf {
    let s = root.as_os_str().to_string_lossy();
    if s.ends_with('\\') || s.ends_with('/') {
        root.to_path_buf()
    } else {
        PathBuf::from(format!("{s}\\"))
    }
}

pub fn volume_info(path: &Path) -> io::Result<VolumeInfo> {
    // GetVolumeInformationW accepts only a volume root; a folder reports the volume that holds it.
    let root = volume_root_of(path).unwrap_or_else(|_| with_trailing_slash(path));
    let w = wide(root.as_os_str())?;
    let mut label = [0u16; 261];
    let mut fs_name = [0u16; 261];
    let mut serial = 0u32;
    unsafe {
        GetVolumeInformationW(
            PCWSTR(w.as_ptr()),
            Some(&mut label),
            Some(&mut serial),
            None,
            None,
            Some(&mut fs_name),
        )
    }
    .map_err(werr)?;
    let (mut total, mut free) = (0u64, 0u64);
    unsafe { GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), None, Some(&mut total), Some(&mut free)) }
        .map_err(werr)?;
    Ok(VolumeInfo {
        root: root.to_string_lossy().into_owned(),
        serial: serial as u64,
        fs_name: from_wide(&fs_name),
        label: from_wide(&label),
        total_bytes: total,
        free_bytes: free,
    })
}

pub fn volume_root_of(path: &Path) -> io::Result<PathBuf> {
    let w = wide(path.as_os_str())?;
    let mut buf = vec![0u16; 1024];
    unsafe { GetVolumePathNameW(PCWSTR(w.as_ptr()), &mut buf) }.map_err(werr)?;
    Ok(PathBuf::from(from_wide(&buf)))
}

pub fn fixed_volumes() -> io::Result<Vec<PathBuf>> {
    let needed = unsafe { GetLogicalDriveStringsW(None) };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buf = vec![0u16; needed as usize + 1];
    let n = unsafe { GetLogicalDriveStringsW(Some(&mut buf)) } as usize;
    if n == 0 || n > buf.len() {
        return Err(io::Error::last_os_error());
    }
    let mut out = Vec::new();
    for drive in buf[..n].split(|&c| c == 0).filter(|s| !s.is_empty()) {
        let mut z = drive.to_vec();
        z.push(0);
        if unsafe { GetDriveTypeW(PCWSTR(z.as_ptr())) } == DRIVE_FIXED {
            out.push(PathBuf::from(String::from_utf16_lossy(drive)));
        }
    }
    Ok(out)
}

// A `subst` drive is a folder on another volume; its DOS device maps to `\??\<folder>`.
pub fn is_subst(root: &Path) -> bool {
    let s = root.as_os_str().to_string_lossy();
    let Ok(w) = wide_str(s.trim_end_matches(['\\', '/'])) else {
        return false;
    };
    let mut buf = vec![0u16; 1024];
    let n = unsafe { QueryDosDeviceW(PCWSTR(w.as_ptr()), Some(&mut buf)) };
    n > 0 && from_wide(&buf).starts_with(r"\??\")
}

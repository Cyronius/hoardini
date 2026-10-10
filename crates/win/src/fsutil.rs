use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Foundation::{FILETIME, GENERIC_WRITE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    CreateDirectoryW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
    FILE_READ_ATTRIBUTES, FILE_WRITE_ATTRIBUTES, FileIdInfo, FileTimeToLocalFileTime,
    GetFileInformationByHandleEx, SetFileTime,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    FILE_ZERO_DATA_INFORMATION, FSCTL_SET_REPARSE_POINT, FSCTL_SET_SPARSE, FSCTL_SET_ZERO_DATA,
};
use windows::core::PCWSTR;

use crate::util::{open, raw, werr, wide_path, wide_str};
use crate::{IO_REPARSE_TAG_MOUNT_POINT, unix_ms_to_filetime};

fn filetime(ms: i64) -> FILETIME {
    let ft = unix_ms_to_filetime(ms);
    FILETIME {
        dwLowDateTime: ft as u32,
        dwHighDateTime: (ft >> 32) as u32,
    }
}

pub fn local_offset_ms(utc_ms: i64) -> i64 {
    let ft = filetime(utc_ms);
    let mut local = FILETIME::default();
    // SAFETY: both pointers are valid for the call.
    if unsafe { FileTimeToLocalFileTime(&ft, &mut local) }.is_err() {
        return 0;
    }
    let l = u64::from(local.dwHighDateTime) << 32 | u64::from(local.dwLowDateTime);
    (l as i64 - unix_ms_to_filetime(utc_ms) as i64) / 10_000
}

pub fn set_file_times(path: &Path, atime_ms: Option<i64>, mtime_ms: Option<i64>) -> io::Result<()> {
    let h = open(
        path,
        FILE_WRITE_ATTRIBUTES.0,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
    )?;
    let a = atime_ms.map(filetime);
    let m = mtime_ms.map(filetime);
    unsafe {
        SetFileTime(
            raw(&h),
            None,
            a.as_ref().map(|t| t as *const FILETIME),
            m.as_ref().map(|t| t as *const FILETIME),
        )
    }
    .map_err(werr)
}

pub fn file_id_of(path: &Path) -> io::Result<[u8; 16]> {
    let h = open(
        path,
        FILE_READ_ATTRIBUTES.0,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
    )?;
    let mut info = FILE_ID_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            raw(&h),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    }
    .map_err(werr)?;
    Ok(info.FileId.Identifier)
}

// Mount-point reparse data written directly; junctions need no admin, unlike symlinks.
pub fn create_junction(link: &Path, target: &Path) -> io::Result<()> {
    let target = std::path::absolute(target)?;
    let mut print: Vec<u16> = target.as_os_str().encode_wide().collect();
    let verbatim: Vec<u16> = r"\\?\".encode_utf16().collect();
    if print.starts_with(&verbatim) {
        print.drain(..verbatim.len());
    }
    let mut subst: Vec<u16> = r"\??\".encode_utf16().collect();
    subst.extend_from_slice(&print);

    let subst_bytes = subst.len() * 2;
    let print_bytes = print.len() * 2;
    let path_buf_bytes = subst_bytes + 2 + print_bytes + 2;
    let data_len = 8 + path_buf_bytes;
    if data_len > u16::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "junction target too long",
        ));
    }
    let mut buf: Vec<u8> = Vec::with_capacity(8 + data_len);
    buf.extend_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    buf.extend_from_slice(&(data_len as u16).to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&(subst_bytes as u16).to_le_bytes());
    buf.extend_from_slice(&((subst_bytes + 2) as u16).to_le_bytes());
    buf.extend_from_slice(&(print_bytes as u16).to_le_bytes());
    for u in subst
        .iter()
        .chain([0u16].iter())
        .chain(print.iter())
        .chain([0u16].iter())
    {
        buf.extend_from_slice(&u.to_le_bytes());
    }

    std::fs::create_dir(link)?;
    let result = (|| {
        let h = open(
            link,
            GENERIC_WRITE.0,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        )?;
        unsafe {
            DeviceIoControl(
                raw(&h),
                FSCTL_SET_REPARSE_POINT,
                Some(buf.as_ptr().cast()),
                buf.len() as u32,
                None,
                0,
                None,
                None,
            )
        }
        .map_err(werr)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir(link);
    }
    result
}

pub fn set_sparse_and_zero_range(path: &Path, offset: u64, len: u64) -> io::Result<()> {
    let h = open(path, GENERIC_WRITE.0, Default::default())?;
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            raw(&h),
            FSCTL_SET_SPARSE,
            None,
            0,
            None,
            0,
            Some(&mut returned),
            None,
        )
    }
    .map_err(werr)?;
    let zero = FILE_ZERO_DATA_INFORMATION {
        FileOffset: offset as i64,
        BeyondFinalZero: (offset + len) as i64,
    };
    unsafe {
        DeviceIoControl(
            raw(&h),
            FSCTL_SET_ZERO_DATA,
            Some((&zero as *const FILE_ZERO_DATA_INFORMATION).cast()),
            size_of::<FILE_ZERO_DATA_INFORMATION>() as u32,
            None,
            0,
            Some(&mut returned),
            None,
        )
    }
    .map_err(werr)
}

// Protected DACL: SYSTEM and Administrators only, and OWNER RIGHTS cut to reading so an unelevated owner cannot rewrite it.
const PRIVATE_DIR_SDDL: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FR;;;OW)";

// Fails when anything already has the name, so a folder or junction put there first is never reused.
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    let sddl = wide_str(PRIVATE_DIR_SDDL)?;
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: `sddl` is NUL-terminated; the descriptor is freed below with LocalFree.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
    }
    .map_err(werr)?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    let created = wide_path(path).and_then(|w| {
        // SAFETY: `w` is NUL-terminated and `sa` outlives the call.
        unsafe { CreateDirectoryW(PCWSTR(w.as_ptr()), Some(&sa)) }.map_err(werr)
    });
    // SAFETY: `sd` came from ConvertStringSecurityDescriptorToSecurityDescriptorW.
    unsafe { LocalFree(Some(HLOCAL(sd.0))) };
    created
}

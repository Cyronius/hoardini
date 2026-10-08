use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;

use windows::Win32::Foundation::{HANDLE, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::core::PCWSTR;

// Below this length a plain path works everywhere; above it CreateFileW needs the \\?\ form.
const LEGACY_MAX: usize = 248;

pub(crate) fn werr(e: windows::core::Error) -> io::Error {
    match WIN32_ERROR::from_error(&e) {
        Some(code) => io::Error::from_raw_os_error(code.0 as i32),
        None => io::Error::other(e),
    }
}

pub(crate) fn win32(code: WIN32_ERROR) -> io::Result<()> {
    if code.is_ok() {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code.0 as i32))
    }
}

pub(crate) fn wide(s: &OsStr) -> io::Result<Vec<u16>> {
    let mut v: Vec<u16> = s.encode_wide().collect();
    if v.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "string contains a NUL",
        ));
    }
    v.push(0);
    Ok(v)
}

pub(crate) fn wide_str(s: &str) -> io::Result<Vec<u16>> {
    wide(OsStr::new(s))
}

fn has_prefix(v: &[u16], prefix: &str) -> bool {
    let p: Vec<u16> = prefix.encode_utf16().collect();
    v.len() >= p.len() && v[..p.len()] == p[..]
}

// NUL-terminated path for CreateFileW-family calls, switched to the \\?\ form when long.
pub(crate) fn wide_path(p: &Path) -> io::Result<Vec<u16>> {
    let raw = wide(p.as_os_str())?;
    if raw.len() - 1 < LEGACY_MAX || has_prefix(&raw, r"\\?\") || has_prefix(&raw, r"\\.\") {
        return Ok(raw);
    }
    let abs = wide(std::path::absolute(p)?.as_os_str())?;
    if has_prefix(&abs, r"\\?\") || has_prefix(&abs, r"\\.\") {
        return Ok(abs);
    }
    let mut out: Vec<u16>;
    if has_prefix(&abs, r"\\") {
        out = r"\\?\UNC\".encode_utf16().collect();
        out.extend_from_slice(&abs[2..]);
    } else {
        out = r"\\?\".encode_utf16().collect();
        out.extend_from_slice(&abs);
    }
    Ok(out)
}

pub(crate) fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub(crate) fn raw(h: &OwnedHandle) -> HANDLE {
    HANDLE(h.as_raw_handle())
}

pub(crate) fn owned(h: HANDLE) -> OwnedHandle {
    unsafe { OwnedHandle::from_raw_handle(h.0) }
}

pub(crate) fn open(
    path: &Path,
    access: u32,
    flags: FILE_FLAGS_AND_ATTRIBUTES,
) -> io::Result<OwnedHandle> {
    let w = wide_path(path)?;
    let h = unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
    }
    .map_err(werr)?;
    Ok(owned(h))
}

pub(crate) fn read_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

pub(crate) fn read_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap_or([0; 4]))
}

pub(crate) fn read_u64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap_or([0; 8]))
}

pub(crate) fn read_id(b: &[u8], off: usize) -> [u8; 16] {
    b[off..off + 16].try_into().unwrap_or([0; 16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_paths_pass_through() {
        let w = wide_path(Path::new(r"C:\x\y")).unwrap();
        assert_eq!(from_wide(&w), r"C:\x\y");
    }

    #[test]
    fn long_paths_get_verbatim_prefix() {
        let long = format!(r"C:\{}\f.txt", "a".repeat(300));
        assert_eq!(
            from_wide(&wide_path(Path::new(&long)).unwrap()),
            format!(r"\\?\{long}")
        );
        let unc = format!(r"\\srv\share\{}", "b".repeat(300));
        assert_eq!(
            from_wide(&wide_path(Path::new(&unc)).unwrap()),
            format!(r"\\?\UNC\srv\share\{}", "b".repeat(300))
        );
    }

    #[test]
    fn interior_nul_is_rejected() {
        assert_eq!(
            wide_str("a\0b").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}

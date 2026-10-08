use std::io;
use std::path::Path;

use windows::Win32::Foundation::{FILETIME, GENERIC_READ};
use windows::Win32::Storage::FileSystem::{
    FILE_WRITE_ATTRIBUTES, GetFileAttributesW, INVALID_FILE_ATTRIBUTES, SetFileTime,
};
use windows::core::PCWSTR;

use crate::is_cloud_placeholder;
use crate::util::{open, raw, werr, wide_path};

pub(crate) fn refuse_placeholder(attrs: u32) -> io::Result<()> {
    if is_cloud_placeholder(attrs) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing to read a cloud placeholder (it would download)",
        ));
    }
    Ok(())
}

// Opens for reading with last-access updates suspended on this handle (SetFileTime 0xFFFFFFFF).
pub fn open_read_preserving_atime(path: &Path) -> io::Result<std::fs::File> {
    let w = wide_path(path)?;
    let attrs = unsafe { GetFileAttributesW(PCWSTR(w.as_ptr())) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return Err(io::Error::last_os_error());
    }
    refuse_placeholder(attrs)?;
    let h = open(
        path,
        GENERIC_READ.0 | FILE_WRITE_ATTRIBUTES.0,
        Default::default(),
    )?;
    let keep = FILETIME {
        dwLowDateTime: u32::MAX,
        dwHighDateTime: u32::MAX,
    };
    unsafe { SetFileTime(raw(&h), None, Some(&keep), None) }.map_err(werr)?;
    Ok(std::fs::File::from(h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
        FILE_ATTRIBUTE_RECALL_ON_OPEN,
    };

    #[test]
    fn placeholder_attrs_are_refused() {
        for a in [
            FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
            FILE_ATTRIBUTE_RECALL_ON_OPEN,
            FILE_ATTRIBUTE_OFFLINE,
        ] {
            let e = refuse_placeholder(a | FILE_ATTRIBUTE_ARCHIVE).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
            assert!(e.to_string().contains("cloud placeholder"));
        }
        assert!(refuse_placeholder(FILE_ATTRIBUTE_ARCHIVE).is_ok());
    }
}

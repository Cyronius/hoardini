use std::io;
use std::mem::size_of;
use std::os::windows::io::OwnedHandle;
use std::path::Path;

use windows::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows::Wdk::Storage::FileSystem::{
    FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_FOR_BACKUP_INTENT,
    FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, NTCREATEFILE_CREATE_OPTIONS,
    NtCreateFile,
};
use windows::Win32::Foundation::{
    ERROR_SHARING_VIOLATION, HANDLE, OBJ_CASE_INSENSITIVE, RtlNtStatusToDosError, UNICODE_STRING,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, DELETE, FILE_ACCESS_RIGHTS, FILE_ATTRIBUTE_TAG_INFO, FILE_BASIC_INFO,
    FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO_EX,
    FILE_DISPOSITION_INFO_EX_FLAGS, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_FLAGS_AND_ATTRIBUTES, FILE_LIST_DIRECTORY, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_MODE, FILE_SHARE_NONE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_STANDARD_INFO, FILE_WRITE_ATTRIBUTES, FileAttributeTagInfo, FileBasicInfo,
    FileDispositionInfoEx, FileStandardInfo, GetFileInformationByHandleEx,
    GetFinalPathNameByHandleW, OPEN_EXISTING, SYNCHRONIZE, SetFileInformationByHandle,
};
use windows::Win32::System::IO::IO_STATUS_BLOCK;
use windows::core::{PCWSTR, PWSTR};

use crate::util::{open, owned, raw, werr, wide_path};
use crate::{DirEntry, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT};
use crate::{filetime_to_unix_ms, unix_ms_to_filetime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    pub attrs: u32,
    pub reparse_tag: u32,
    pub created_ms: i64,
    pub written_ms: i64,
    pub changed_ms: i64,
    pub size: u64,
}

impl Stat {
    pub fn is_dir(&self) -> bool {
        self.attrs & FILE_ATTRIBUTE_DIRECTORY != 0
    }
    pub fn is_reparse(&self) -> bool {
        self.attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    // Unpacked files keep an old last write; creation and change times show when they arrived.
    pub fn newest_ms(&self) -> i64 {
        self.created_ms.max(self.written_ms).max(self.changed_ms)
    }
}

// A file or folder opened as itself: a final link or junction is held, never followed.
pub struct NoFollow {
    h: OwnedHandle,
}

const DIR_ACCESS: u32 = FILE_LIST_DIRECTORY.0 | FILE_READ_ATTRIBUTES.0 | SYNCHRONIZE.0;
const DELETE_ACCESS: u32 = DELETE.0 | FILE_READ_ATTRIBUTES.0 | SYNCHRONIZE.0;

fn share_all() -> FILE_SHARE_MODE {
    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
}

fn by_path(path: &Path, access: u32, share: FILE_SHARE_MODE) -> io::Result<OwnedHandle> {
    let w = wide_path(path)?;
    // SAFETY: `w` is NUL-terminated and outlives the call.
    let h = unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            access,
            share,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(werr)?;
    Ok(owned(h))
}

fn sharing_violation(e: &io::Error) -> bool {
    e.raw_os_error() == Some(ERROR_SHARING_VIOLATION.0 as i32)
}

impl NoFollow {
    pub fn open_dir(path: &Path) -> io::Result<NoFollow> {
        Ok(NoFollow {
            h: by_path(path, DIR_ACCESS, share_all())?,
        })
    }

    // No sharing, so a file anything else holds open is reported as `None` and left alone.
    pub fn open_for_delete(path: &Path) -> io::Result<Option<NoFollow>> {
        match by_path(path, DELETE_ACCESS, FILE_SHARE_NONE) {
            Ok(h) => Ok(Some(NoFollow { h })),
            Err(e) if sharing_violation(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    // Relative to this folder's handle, so a parent swapped for a junction after the listing is never walked through.
    fn child(
        &self,
        name: &str,
        access: u32,
        share: FILE_SHARE_MODE,
        options: NTCREATEFILE_CREATE_OPTIONS,
    ) -> io::Result<NoFollow> {
        let units: Vec<u16> = name.encode_utf16().collect();
        if units.is_empty() || units.len() > 32_767 || units.contains(&(b'\\' as u16)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a single file name",
            ));
        }
        let name_us = UNICODE_STRING {
            Length: (units.len() * 2) as u16,
            MaximumLength: (units.len() * 2) as u16,
            Buffer: PWSTR(units.as_ptr() as *mut u16),
        };
        let attrs = OBJECT_ATTRIBUTES {
            Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: raw(&self.h),
            ObjectName: &name_us,
            Attributes: OBJ_CASE_INSENSITIVE,
            ..Default::default()
        };
        let mut h = HANDLE::default();
        let mut iosb = IO_STATUS_BLOCK::default();
        // SAFETY: every pointer refers to a local that outlives the call; `units` backs the name.
        let status = unsafe {
            NtCreateFile(
                &mut h,
                FILE_ACCESS_RIGHTS(access),
                &attrs,
                &mut iosb,
                None,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                share,
                FILE_OPEN,
                options
                    | FILE_OPEN_REPARSE_POINT
                    | FILE_OPEN_FOR_BACKUP_INTENT
                    | FILE_SYNCHRONOUS_IO_NONALERT,
                None,
                0,
            )
        };
        if status.is_err() {
            // SAFETY: a plain status-code conversion.
            let code = unsafe { RtlNtStatusToDosError(status) };
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        Ok(NoFollow { h: owned(h) })
    }

    // Asks for delete access so an emptied folder can go through this same handle; a folder in use is still listed.
    pub fn child_dir(&self, name: &str) -> io::Result<NoFollow> {
        match self.child(
            name,
            DIR_ACCESS | DELETE.0,
            share_all(),
            FILE_DIRECTORY_FILE,
        ) {
            Err(e) if sharing_violation(&e) => {
                self.child(name, DIR_ACCESS, share_all(), FILE_DIRECTORY_FILE)
            }
            r => r,
        }
    }

    pub fn child_for_delete(&self, name: &str) -> io::Result<Option<NoFollow>> {
        match self.child(
            name,
            DELETE_ACCESS,
            FILE_SHARE_NONE,
            FILE_NON_DIRECTORY_FILE,
        ) {
            Ok(f) => Ok(Some(f)),
            Err(e) if sharing_violation(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn stat(&self) -> io::Result<Stat> {
        let mut basic = FILE_BASIC_INFO::default();
        let mut tag = FILE_ATTRIBUTE_TAG_INFO::default();
        let mut std_info = FILE_STANDARD_INFO::default();
        // SAFETY: each buffer is the struct its class fills, sized to match.
        unsafe {
            GetFileInformationByHandleEx(
                raw(&self.h),
                FileBasicInfo,
                (&mut basic as *mut FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
            .map_err(werr)?;
            GetFileInformationByHandleEx(
                raw(&self.h),
                FileAttributeTagInfo,
                (&mut tag as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
            .map_err(werr)?;
            GetFileInformationByHandleEx(
                raw(&self.h),
                FileStandardInfo,
                (&mut std_info as *mut FILE_STANDARD_INFO).cast(),
                size_of::<FILE_STANDARD_INFO>() as u32,
            )
            .map_err(werr)?;
        }
        let ms = |t: i64| filetime_to_unix_ms(t.max(0) as u64);
        Ok(Stat {
            attrs: basic.FileAttributes | tag.FileAttributes,
            reparse_tag: if tag.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                tag.ReparseTag
            } else {
                0
            },
            created_ms: ms(basic.CreationTime),
            written_ms: ms(basic.LastWriteTime),
            changed_ms: ms(basic.ChangeTime),
            size: std_info.EndOfFile.max(0) as u64,
        })
    }

    pub fn list(&self, f: impl FnMut(DirEntry)) -> io::Result<()> {
        crate::enumerate::list_handle_each(&self.h, f)
    }

    pub fn final_path(&self) -> io::Result<String> {
        let mut buf = vec![0u16; 512];
        loop {
            // SAFETY: the buffer is valid for its whole length.
            let n =
                unsafe { GetFinalPathNameByHandleW(raw(&self.h), &mut buf, FILE_NAME_NORMALIZED) }
                    as usize;
            if n == 0 {
                return Err(io::Error::last_os_error());
            }
            if n < buf.len() {
                return Ok(String::from_utf16_lossy(&buf[..n]));
            }
            buf.resize(n + 1, 0);
        }
    }

    pub fn is_inside(&self, root: &str) -> bool {
        self.final_path().is_ok_and(|p| path_inside(root, &p))
    }

    // POSIX semantics: the name goes at once, and a folder that is not empty is refused.
    pub fn delete(&self) -> io::Result<()> {
        let info = FILE_DISPOSITION_INFO_EX {
            Flags: FILE_DISPOSITION_INFO_EX_FLAGS(
                FILE_DISPOSITION_FLAG_DELETE.0
                    | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS.0
                    | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE.0,
            ),
        };
        // SAFETY: `info` is the struct FileDispositionInfoEx expects.
        unsafe {
            SetFileInformationByHandle(
                raw(&self.h),
                FileDispositionInfoEx,
                (&info as *const FILE_DISPOSITION_INFO_EX).cast(),
                size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
            )
        }
        .map_err(werr)
    }
}

pub fn path_inside(root: &str, path: &str) -> bool {
    let root = format!("{}\\", root.trim_end_matches('\\').to_lowercase());
    let path = path.to_lowercase();
    path.len() > root.len() && path.starts_with(&root)
}

// Sets all four times, change time included, which SetFileTime cannot reach.
pub fn set_all_times(path: &Path, ms: i64) -> io::Result<()> {
    let h = open(
        path,
        FILE_WRITE_ATTRIBUTES.0 | FILE_READ_ATTRIBUTES.0,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
    )?;
    let mut basic = FILE_BASIC_INFO::default();
    // SAFETY: `basic` is the struct FileBasicInfo reads and writes.
    unsafe {
        GetFileInformationByHandleEx(
            raw(&h),
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
        .map_err(werr)?;
        let t = unix_ms_to_filetime(ms) as i64;
        basic.CreationTime = t;
        basic.LastAccessTime = t;
        basic.LastWriteTime = t;
        basic.ChangeTime = t;
        SetFileInformationByHandle(
            raw(&h),
            FileBasicInfo,
            (&basic as *const FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
        .map_err(werr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_means_strictly_below_on_a_component_boundary() {
        let root = r"\\?\C:\Windows\Temp";
        assert!(path_inside(root, r"\\?\C:\Windows\Temp\a.tmp"));
        assert!(path_inside(
            r"\\?\C:\Windows\Temp\",
            r"\\?\c:\windows\temp\Sub\b"
        ));
        assert!(!path_inside(root, root));
        assert!(!path_inside(root, r"\\?\C:\Windows\Temp2\a.tmp"));
        assert!(!path_inside(root, r"\\?\C:\Users\me\a.tmp"));
        assert!(!path_inside(root, r"\\?\C:\Windows"));
    }

    #[test]
    fn a_handle_reports_every_time_and_the_final_path_it_holds() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.bin");
        std::fs::write(&f, vec![1u8; 5000]).unwrap();
        let long_ago = 1_700_000_000_000;
        set_all_times(&f, long_ago).unwrap();
        let root = NoFollow::open_dir(dir.path()).unwrap();
        let held = root.child_for_delete("a.bin").unwrap().unwrap();
        let st = held.stat().unwrap();
        assert_eq!(
            (st.created_ms, st.written_ms, st.changed_ms, st.size),
            (long_ago, long_ago, long_ago, 5000)
        );
        assert_eq!(st.newest_ms(), long_ago);
        assert!(!st.is_dir() && !st.is_reparse());
        let root_final = root.final_path().unwrap();
        assert!(root_final.starts_with(r"\\?\"), "{root_final}");
        assert!(held.is_inside(&root_final));
        assert!(!root.is_inside(&root_final));
        assert!(
            root.child_for_delete("a.bin").unwrap().is_none(),
            "a file already held is reported in use"
        );
        drop(held);

        std::fs::write(dir.path().join("fresh.txt"), b"x").unwrap();
        let fresh = root.child_for_delete("fresh.txt").unwrap().unwrap();
        assert!(fresh.stat().unwrap().newest_ms() > long_ago);
        assert!(root.child_for_delete(r"..\a.bin").is_err());
        assert_eq!(
            root.child_dir("missing").err().map(|e| e.kind()),
            Some(io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn a_junction_child_is_held_as_itself_never_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), b"keep").unwrap();
        let inner = dir.path().join("inner");
        std::fs::create_dir(&inner).unwrap();
        crate::create_junction(&inner.join("link"), &outside).unwrap();
        let root = NoFollow::open_dir(&inner).unwrap();
        let link = root.child_dir("link").unwrap();
        let st = link.stat().unwrap();
        assert!(st.is_reparse() && st.is_dir());
        assert_eq!(st.reparse_tag, crate::IO_REPARSE_TAG_MOUNT_POINT);
        let mut names = Vec::new();
        let _ = link.list(|e| names.push(e.name));
        assert!(
            !names.iter().any(|n| n == "keep.txt"),
            "the junction's own handle never lists the target's files"
        );
        let outside_final = NoFollow::open_dir(&outside).unwrap().final_path().unwrap();
        assert!(!link.is_inside(&outside_final));
    }
}

use std::io;
use std::mem::{offset_of, size_of};
use std::path::Path;

use windows::Win32::Foundation::{
    ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, ERROR_NOT_SUPPORTED,
    WIN32_ERROR,
};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_BOTH_DIR_INFO,
    FILE_ID_EXTD_DIR_INFO, FILE_INFO_BY_HANDLE_CLASS, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
    FILE_STANDARD_INFO, FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo,
    FileIdExtdDirectoryInfo, FileIdExtdDirectoryRestartInfo, FileStandardInfo,
    GetFileInformationByHandleEx, SYNCHRONIZE,
};

use crate::util::{open, raw, werr};
use crate::{DirEntry, FILE_ATTRIBUTE_REPARSE_POINT, filetime_to_unix_ms};

const LIST_BUF_BYTES: usize = 64 * 1024;
// Lets the last entry's fixed header be copied whole even when its short name ends inside the padding.
const SLACK_U64S: usize = 2;

// u64 backing keeps the LARGE_INTEGER fields 8-byte aligned.
fn new_buf() -> Vec<u64> {
    vec![0u64; LIST_BUF_BYTES / 8 + SLACK_U64S]
}

pub fn list_dir(path: &Path) -> io::Result<Vec<DirEntry>> {
    let mut out = Vec::new();
    list_dir_each(path, |e| out.push(e))?;
    Ok(out)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    Extd,
    // FAT/exFAT predate FileIdExtdDirectoryInfo; this class carries a 64-bit id and the tag in EaSize.
    IdBoth,
}

impl Class {
    fn restart(self) -> FILE_INFO_BY_HANDLE_CLASS {
        match self {
            Class::Extd => FileIdExtdDirectoryRestartInfo,
            Class::IdBoth => FileIdBothDirectoryRestartInfo,
        }
    }

    fn next(self) -> FILE_INFO_BY_HANDLE_CLASS {
        match self {
            Class::Extd => FileIdExtdDirectoryInfo,
            Class::IdBoth => FileIdBothDirectoryInfo,
        }
    }
}

// Same listing as list_dir, handed out one entry at a time so callers need not hold the whole dir.
pub fn list_dir_each(path: &Path, mut f: impl FnMut(DirEntry)) -> io::Result<()> {
    let h = open(
        path,
        FILE_LIST_DIRECTORY.0 | SYNCHRONIZE.0,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
    )?;
    let mut buf = new_buf();
    let mut class = Class::Extd;
    let mut info_class = class.restart();
    loop {
        let r = unsafe {
            GetFileInformationByHandleEx(
                raw(&h),
                info_class,
                buf.as_mut_ptr().cast(),
                LIST_BUF_BYTES as u32,
            )
        };
        if let Err(e) = r {
            let code = WIN32_ERROR::from_error(&e);
            if code == Some(ERROR_NO_MORE_FILES) {
                return Ok(());
            }
            let unsupported = matches!(
                code,
                Some(ERROR_INVALID_PARAMETER | ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED)
            );
            if class == Class::Extd && info_class == class.restart() && unsupported {
                class = Class::IdBoth;
                info_class = class.restart();
                continue;
            }
            return Err(werr(e));
        }
        info_class = class.next();
        parse_dir_info(&buf, class, &mut f);
    }
}

struct Fields {
    next: usize,
    name_off: usize,
    name_len: usize,
    attrs: u32,
    tag: u32,
    alloc: i64,
    logical: i64,
    ctime: i64,
    atime: i64,
    mtime: i64,
    id: [u8; 16],
}

fn read_fields(b: &[u8], limit: usize, off: usize, class: Class) -> Option<Fields> {
    let f = match class {
        Class::Extd => {
            if off + offset_of!(FILE_ID_EXTD_DIR_INFO, FileName) > limit
                || off + size_of::<FILE_ID_EXTD_DIR_INFO>() > b.len()
            {
                return None;
            }
            let i: FILE_ID_EXTD_DIR_INFO =
                unsafe { std::ptr::read_unaligned(b.as_ptr().add(off).cast()) };
            Fields {
                next: i.NextEntryOffset as usize,
                name_off: offset_of!(FILE_ID_EXTD_DIR_INFO, FileName),
                name_len: i.FileNameLength as usize,
                attrs: i.FileAttributes,
                tag: i.ReparsePointTag,
                alloc: i.AllocationSize,
                logical: i.EndOfFile,
                ctime: i.CreationTime,
                atime: i.LastAccessTime,
                mtime: i.LastWriteTime,
                id: i.FileId.Identifier,
            }
        }
        Class::IdBoth => {
            if off + offset_of!(FILE_ID_BOTH_DIR_INFO, FileName) > limit
                || off + size_of::<FILE_ID_BOTH_DIR_INFO>() > b.len()
            {
                return None;
            }
            let i: FILE_ID_BOTH_DIR_INFO =
                unsafe { std::ptr::read_unaligned(b.as_ptr().add(off).cast()) };
            let mut id = [0u8; 16];
            id[..8].copy_from_slice(&i.FileId.to_le_bytes());
            Fields {
                next: i.NextEntryOffset as usize,
                name_off: offset_of!(FILE_ID_BOTH_DIR_INFO, FileName),
                name_len: i.FileNameLength as usize,
                attrs: i.FileAttributes,
                tag: i.EaSize,
                alloc: i.AllocationSize,
                logical: i.EndOfFile,
                ctime: i.CreationTime,
                atime: i.LastAccessTime,
                mtime: i.LastWriteTime,
                id,
            }
        }
    };
    Some(f)
}

fn parse_dir_info(buf: &[u64], class: Class, f: &mut impl FnMut(DirEntry)) {
    let b = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), buf.len() * 8) };
    let limit = b.len().saturating_sub(SLACK_U64S * 8);
    let mut off = 0usize;
    while let Some(e) = read_fields(b, limit, off, class) {
        let name_start = off + e.name_off;
        if name_start + e.name_len > limit || !name_start.is_multiple_of(2) {
            return;
        }
        // Entries start 8-aligned in a u64-backed buffer, so the name is u16-aligned.
        let units: &[u16] = unsafe {
            std::slice::from_raw_parts(b.as_ptr().add(name_start).cast(), e.name_len / 2)
        };
        if units != [0x2E] && units != [0x2E, 0x2E] {
            f(DirEntry {
                name: String::from_utf16_lossy(units),
                attrs: e.attrs,
                reparse_tag: if e.attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    e.tag
                } else {
                    0
                },
                alloc: e.alloc as u64,
                logical: e.logical as u64,
                ctime_ms: filetime_to_unix_ms(e.ctime as u64),
                atime_ms: filetime_to_unix_ms(e.atime as u64),
                mtime_ms: filetime_to_unix_ms(e.mtime as u64),
                file_id: e.id,
            });
        }
        if e.next == 0 {
            return;
        }
        off += e.next;
    }
}

// Listing reports alloc 0 for WOF-compressed files; FILE_STANDARD_INFO on a metadata-only open has the real figure.
pub fn alloc_size_by_open(path: &Path) -> io::Result<u64> {
    alloc_size_with_flags(path, true)
}

pub(crate) fn alloc_size_with_flags(path: &Path, open_reparse: bool) -> io::Result<u64> {
    let flags = if open_reparse {
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT
    } else {
        FILE_FLAG_BACKUP_SEMANTICS
    };
    let h = open(path, FILE_READ_ATTRIBUTES.0, flags)?;
    let mut info = FILE_STANDARD_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            raw(&h),
            FileStandardInfo,
            (&mut info as *mut FILE_STANDARD_INFO).cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    }
    .map_err(werr)?;
    Ok(info.AllocationSize as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FILE_ATTRIBUTE_COMPRESSED, IO_REPARSE_TAG_MOUNT_POINT};

    fn compressible(path: &Path) -> Vec<u8> {
        let line = b"hoardini compressible line of text 0123456789\n";
        let mut data = Vec::new();
        while data.len() < 4 * 1024 * 1024 {
            data.extend_from_slice(line);
        }
        std::fs::write(path, &data).unwrap();
        data
    }

    fn compact(args: &[&str], path: &Path) {
        let out = std::process::Command::new(crate::system_tool("compact.exe").unwrap())
            .args(args)
            .arg(path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }

    fn entry(dir: &Path, name: &str) -> DirEntry {
        list_dir(dir)
            .unwrap()
            .into_iter()
            .find(|e| e.name == name)
            .unwrap()
    }

    // Measured: WOF hides its reparse point from listing and handle queries, and the listing reports alloc 0.
    #[test]
    fn wof_file_lists_alloc_zero_without_reparse_tag() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("wof.bin");
        let data = compressible(&f);
        assert!(!entry(dir.path(), "wof.bin").needs_alloc_by_open());

        compact(&["/c", "/exe:lzx"], &f);
        let e = entry(dir.path(), "wof.bin");
        assert_eq!(e.alloc, 0);
        assert_eq!(e.logical, data.len() as u64);
        assert_eq!(e.attrs & FILE_ATTRIBUTE_REPARSE_POINT, 0);
        assert_eq!(e.reparse_tag, 0);
        assert!(e.needs_alloc_by_open());

        let by_open = alloc_size_by_open(&f).unwrap();
        assert!(by_open > 0 && by_open < e.logical / 10, "by_open {by_open}");
        assert_eq!(alloc_size_with_flags(&f, false).unwrap(), by_open);
        assert_eq!(std::fs::read(&f).unwrap(), data);
    }

    #[test]
    fn lznt1_file_lists_real_alloc() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("lznt1.bin");
        compressible(&f);
        compact(&["/c"], &f);
        let e = entry(dir.path(), "lznt1.bin");
        assert_ne!(e.attrs & FILE_ATTRIBUTE_COMPRESSED, 0);
        assert!(e.alloc > 0 && e.alloc < e.logical);
        assert!(!e.needs_alloc_by_open());
        assert_eq!(alloc_size_by_open(&f).unwrap(), e.alloc);
    }

    #[test]
    fn id_both_fallback_parses_64_bit_ids_and_ea_size_tag() {
        fn entry(name: &str, attrs: u32, ea: u32, id: i64, last: bool) -> Vec<u8> {
            let name_off = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
            let len = (name_off + name.len() * 2).next_multiple_of(8);
            let info = FILE_ID_BOTH_DIR_INFO {
                NextEntryOffset: if last { 0 } else { len as u32 },
                CreationTime: 133_485_408_000_000_000,
                LastAccessTime: 133_485_408_000_000_000,
                LastWriteTime: 133_485_408_000_000_000,
                EndOfFile: 5000,
                AllocationSize: 8192,
                FileAttributes: attrs,
                FileNameLength: (name.len() * 2) as u32,
                EaSize: ea,
                FileId: id,
                ..Default::default()
            };
            let mut v = vec![0u8; len];
            unsafe {
                std::ptr::write_unaligned(v.as_mut_ptr().cast::<FILE_ID_BOTH_DIR_INFO>(), info)
            };
            for (i, u) in name.encode_utf16().enumerate() {
                v[name_off + i * 2..name_off + i * 2 + 2].copy_from_slice(&u.to_le_bytes());
            }
            v
        }
        let mut bytes = entry(".", 0x10, 0, 1, false);
        bytes.extend(entry("plain.txt", 0x20, 0x55, 0x0102, false));
        bytes.extend(entry("link", 0x410, IO_REPARSE_TAG_MOUNT_POINT, 7, true));
        let mut buf = new_buf();
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.as_mut_ptr().cast(), bytes.len())
        };
        let mut out = Vec::new();
        parse_dir_info(&buf, Class::IdBoth, &mut |e| out.push(e));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "plain.txt");
        assert_eq!(out[0].reparse_tag, 0);
        assert_eq!(&out[0].file_id[..2], &[0x02, 0x01]);
        assert_eq!(&out[0].file_id[8..], &[0u8; 8]);
        assert_eq!((out[0].alloc, out[0].logical), (8192, 5000));
        assert_eq!(out[0].mtime_ms, 1_704_067_200_000);
        assert!(out[1].is_link());
        assert_eq!(out[1].reparse_tag, IO_REPARSE_TAG_MOUNT_POINT);
    }

    #[test]
    fn zero_alloc_sparse_and_empty_files_need_no_open() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("empty"), b"").unwrap();
        std::fs::write(dir.path().join("tiny"), b"hello").unwrap();
        let sp = dir.path().join("sparse");
        std::fs::write(&sp, vec![1u8; 1 << 20]).unwrap();
        crate::set_sparse_and_zero_range(&sp, 0, 1 << 20).unwrap();
        let sparse = entry(dir.path(), "sparse");
        assert_eq!((sparse.alloc, sparse.logical), (0, 1 << 20));
        assert!(!sparse.needs_alloc_by_open());
        assert!(!entry(dir.path(), "empty").needs_alloc_by_open());
        // Resident data still reports an 8-byte-rounded allocation.
        assert_eq!(entry(dir.path(), "tiny").alloc, 8);
    }
}

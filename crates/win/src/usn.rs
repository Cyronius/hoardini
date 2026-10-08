use std::io;
use std::mem::size_of;
use std::os::windows::io::OwnedHandle;
use std::path::{Path, PathBuf};

use windows::Win32::Storage::FileSystem::{
    ExtendedFileIdType, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_128,
    FILE_ID_DESCRIPTOR, FILE_ID_DESCRIPTOR_0, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFinalPathNameByHandleW, OpenFileById,
    SYNCHRONIZE, VOLUME_NAME_DOS,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, READ_USN_JOURNAL_DATA_V1,
    USN_JOURNAL_DATA_V2,
};

use crate::util::{open, owned, raw, read_id, read_u16, read_u32, read_u64, werr};
use crate::{UsnJournal, UsnRecord};

const READ_BUF_BYTES: usize = 64 * 1024;

// The root directory handle is enough for the unprivileged FSCTLs; \\.\C: needs admin.
fn open_root(volume_root: &Path) -> io::Result<OwnedHandle> {
    open(
        volume_root,
        FILE_LIST_DIRECTORY.0 | FILE_READ_ATTRIBUTES.0 | SYNCHRONIZE.0,
        FILE_FLAG_BACKUP_SEMANTICS,
    )
}

pub fn usn_query(volume_root: &Path) -> io::Result<UsnJournal> {
    let h = open_root(volume_root)?;
    let mut data = USN_JOURNAL_DATA_V2::default();
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            raw(&h),
            FSCTL_QUERY_USN_JOURNAL,
            None,
            0,
            Some((&mut data as *mut USN_JOURNAL_DATA_V2).cast()),
            size_of::<USN_JOURNAL_DATA_V2>() as u32,
            Some(&mut returned),
            None,
        )
    }
    .map_err(werr)?;
    Ok(UsnJournal {
        journal_id: data.UsnJournalID,
        first_usn: data.FirstUsn,
        next_usn: data.NextUsn,
    })
}

// FSCTL_READ_UNPRIVILEGED_USN_JOURNAL from start_usn; returns records and the next USN to resume from.
pub fn usn_read(
    volume_root: &Path,
    journal_id: u64,
    start_usn: i64,
) -> io::Result<(Vec<UsnRecord>, i64)> {
    let h = open_root(volume_root)?;
    let req = READ_USN_JOURNAL_DATA_V1 {
        StartUsn: start_usn,
        ReasonMask: u32::MAX,
        ReturnOnlyOnClose: 0,
        Timeout: 0,
        BytesToWaitFor: 0,
        UsnJournalID: journal_id,
        MinMajorVersion: 2,
        MaxMajorVersion: 3,
    };
    let mut buf = vec![0u64; READ_BUF_BYTES / 8];
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            raw(&h),
            FSCTL_READ_UNPRIVILEGED_USN_JOURNAL,
            Some((&req as *const READ_USN_JOURNAL_DATA_V1).cast()),
            size_of::<READ_USN_JOURNAL_DATA_V1>() as u32,
            Some(buf.as_mut_ptr().cast()),
            READ_BUF_BYTES as u32,
            Some(&mut returned),
            None,
        )
    }
    .map_err(werr)?;
    let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), READ_BUF_BYTES) };
    parse_usn_page(&bytes[..(returned as usize).min(READ_BUF_BYTES)])
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "short USN read"))
}

pub(crate) fn parse_usn_page(b: &[u8]) -> Option<(Vec<UsnRecord>, i64)> {
    if b.len() < 8 {
        return None;
    }
    let next = read_u64(b, 0) as i64;
    let mut out = Vec::new();
    let mut off = 8usize;
    while off + 8 <= b.len() {
        let len = read_u32(b, off) as usize;
        if len < 8 || off + len > b.len() {
            break;
        }
        let rec = &b[off..off + len];
        match read_u16(rec, 4) {
            2 if len >= 60 => {
                let mut file_id = [0u8; 16];
                file_id[..8].copy_from_slice(&rec[8..16]);
                let mut parent_id = [0u8; 16];
                parent_id[..8].copy_from_slice(&rec[16..24]);
                out.push(UsnRecord {
                    usn: read_u64(rec, 24) as i64,
                    file_id,
                    parent_id,
                    reason: read_u32(rec, 40),
                    attrs: read_u32(rec, 52),
                });
            }
            3 if len >= 76 => out.push(UsnRecord {
                usn: read_u64(rec, 40) as i64,
                file_id: read_id(rec, 8),
                parent_id: read_id(rec, 24),
                reason: read_u32(rec, 56),
                attrs: read_u32(rec, 68),
            }),
            _ => {}
        }
        off += len;
    }
    Some((out, next))
}

pub fn path_by_file_id(volume_root: &Path, file_id: [u8; 16]) -> io::Result<PathBuf> {
    let root = open_root(volume_root)?;
    let desc = FILE_ID_DESCRIPTOR {
        dwSize: size_of::<FILE_ID_DESCRIPTOR>() as u32,
        Type: ExtendedFileIdType,
        Anonymous: FILE_ID_DESCRIPTOR_0 {
            ExtendedFileId: FILE_ID_128 {
                Identifier: file_id,
            },
        },
    };
    let h = owned(
        unsafe {
            OpenFileById(
                raw(&root),
                &desc,
                FILE_READ_ATTRIBUTES.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            )
        }
        .map_err(werr)?,
    );
    let mut buf = vec![0u16; 512];
    loop {
        let n = unsafe { GetFinalPathNameByHandleW(raw(&h), &mut buf, VOLUME_NAME_DOS) } as usize;
        if n == 0 {
            return Err(io::Error::last_os_error());
        }
        if n >= buf.len() {
            buf = vec![0u16; n + 1];
            continue;
        }
        return Ok(PathBuf::from(strip_verbatim(&String::from_utf16_lossy(
            &buf[..n],
        ))));
    }
}

pub(crate) fn strip_verbatim(p: &str) -> String {
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = p.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        p.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v3(usn: i64, id: u8, parent: u8, reason: u32, attrs: u32) -> Vec<u8> {
        let mut r = vec![0u8; 80];
        r[0..4].copy_from_slice(&80u32.to_le_bytes());
        r[4..6].copy_from_slice(&3u16.to_le_bytes());
        r[8] = id;
        r[24] = parent;
        r[40..48].copy_from_slice(&usn.to_le_bytes());
        r[56..60].copy_from_slice(&reason.to_le_bytes());
        r[68..72].copy_from_slice(&attrs.to_le_bytes());
        r
    }

    fn v2(usn: i64, id: u64, parent: u64, reason: u32, attrs: u32) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[0..4].copy_from_slice(&64u32.to_le_bytes());
        r[4..6].copy_from_slice(&2u16.to_le_bytes());
        r[8..16].copy_from_slice(&id.to_le_bytes());
        r[16..24].copy_from_slice(&parent.to_le_bytes());
        r[24..32].copy_from_slice(&usn.to_le_bytes());
        r[40..44].copy_from_slice(&reason.to_le_bytes());
        r[52..56].copy_from_slice(&attrs.to_le_bytes());
        r
    }

    #[test]
    fn parses_v2_and_v3_records() {
        let mut page = 999i64.to_le_bytes().to_vec();
        page.extend(v3(100, 7, 5, 0x100, 0x20));
        page.extend(v2(200, 0x1122, 0x33, 0x200, 0x10));
        let (recs, next) = parse_usn_page(&page).unwrap();
        assert_eq!(next, 999);
        assert_eq!(recs.len(), 2);
        let mut id = [0u8; 16];
        id[0] = 7;
        assert_eq!(recs[0].file_id, id);
        assert_eq!(recs[0].parent_id[0], 5);
        assert_eq!(
            (recs[0].usn, recs[0].reason, recs[0].attrs),
            (100, 0x100, 0x20)
        );
        assert_eq!(&recs[1].file_id[..2], &[0x22, 0x11]);
        assert_eq!(&recs[1].file_id[8..], &[0u8; 8]);
        assert_eq!(recs[1].parent_id[0], 0x33);
        assert_eq!(
            (recs[1].usn, recs[1].reason, recs[1].attrs),
            (200, 0x200, 0x10)
        );
    }

    #[test]
    fn truncated_record_stops_parsing() {
        let mut page = 5i64.to_le_bytes().to_vec();
        page.extend(&v3(1, 1, 1, 1, 1)[..40]);
        assert_eq!(parse_usn_page(&page).unwrap(), (vec![], 5));
        assert!(parse_usn_page(&[0u8; 4]).is_none());
    }

    #[test]
    fn verbatim_prefix_is_stripped() {
        assert_eq!(strip_verbatim(r"\\?\C:\a\b"), r"C:\a\b");
        assert_eq!(strip_verbatim(r"\\?\UNC\srv\s\x"), r"\\srv\s\x");
        assert_eq!(strip_verbatim(r"C:\a"), r"C:\a");
    }
}

use std::io::{Read, Seek, SeekFrom};

use serde::{Deserialize, Serialize};

const MAX_RSRC: u64 = 16 * 1024 * 1024;
const RT_VERSION: u32 = 16;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PeInfo {
    pub is_dll: bool,
    pub is_64bit: bool,
    pub machine: u16,
    pub product_name: Option<String>,
    pub company_name: Option<String>,
    pub file_description: Option<String>,
    pub product_version: Option<String>,
    pub file_version: Option<String>,
    pub original_filename: Option<String>,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

fn read_at<R: Read + Seek>(r: &mut R, off: u64, len: usize) -> std::io::Result<Vec<u8>> {
    r.seek(SeekFrom::Start(off))?;
    let mut buf = vec![0u8; len];
    let mut got = 0;
    while got < len {
        let n = r.read(&mut buf[got..])?;
        if n == 0 {
            break;
        }
        got += n;
    }
    buf.truncate(got);
    Ok(buf)
}

pub fn parse<R: Read + Seek>(mut r: R, file_len: u64) -> Option<PeInfo> {
    let dos = read_at(&mut r, 0, 0x40).ok()?;
    if dos.get(..2)? != b"MZ" {
        return None;
    }
    let lfanew = u32_at(&dos, 0x3c)? as u64;
    if lfanew + 24 > file_len || lfanew > 4 * 1024 * 1024 {
        return None;
    }
    let nt = read_at(&mut r, lfanew, 24).ok()?;
    if nt.get(..4)? != b"PE\0\0" {
        return None;
    }
    let machine = u16_at(&nt, 4)?;
    let nsec = u16_at(&nt, 6)? as usize;
    let opt_size = u16_at(&nt, 20)? as usize;
    let chars = u16_at(&nt, 22)?;
    if nsec > 96 || opt_size < 2 {
        return None;
    }
    let rest = read_at(&mut r, lfanew + 24, opt_size + nsec * 40).ok()?;
    let opt = rest.get(..opt_size)?;
    let magic = u16_at(opt, 0)?;
    let is_64bit = magic == 0x20b;
    let dd_base = if is_64bit { 112 } else { 96 };

    let mut info = PeInfo {
        is_dll: chars & 0x2000 != 0,
        is_64bit,
        machine,
        ..Default::default()
    };
    let rsrc_rva = u32_at(opt, dd_base + 16).unwrap_or(0) as u64;
    let rsrc_size = u32_at(opt, dd_base + 20).unwrap_or(0) as u64;
    if rsrc_rva == 0 || rsrc_size == 0 {
        return Some(info);
    }

    let sections = rest.get(opt_size..)?;
    let mut rsrc_off = None;
    for s in sections.chunks_exact(40) {
        let vsize = u32_at(s, 8)? as u64;
        let va = u32_at(s, 12)? as u64;
        let raw_size = u32_at(s, 16)? as u64;
        let raw_ptr = u32_at(s, 20)? as u64;
        if rsrc_rva >= va && rsrc_rva < va + vsize.max(raw_size) {
            rsrc_off = Some(raw_ptr + (rsrc_rva - va));
            break;
        }
    }
    let Some(off) = rsrc_off.filter(|o| *o < file_len) else {
        return Some(info);
    };
    let len = rsrc_size.min(MAX_RSRC).min(file_len - off) as usize;
    let Ok(rsrc) = read_at(&mut r, off, len) else {
        return Some(info);
    };
    if let Some(block) = find_version_blob(&rsrc, rsrc_rva) {
        fill_strings(block, &mut info);
    }
    Some(info)
}

const MAX_SIGNATURE: u64 = 1024 * 1024;

// The embedded Authenticode PKCS#7 blob (data directory 4 holds a file offset, not an RVA).
pub fn signature_blob<R: Read + Seek>(mut r: R, file_len: u64) -> Option<Vec<u8>> {
    let dos = read_at(&mut r, 0, 0x40).ok()?;
    if dos.get(..2)? != b"MZ" {
        return None;
    }
    let lfanew = u32_at(&dos, 0x3c)? as u64;
    let nt = read_at(&mut r, lfanew, 24).ok()?;
    if nt.get(..4)? != b"PE\0\0" {
        return None;
    }
    let opt_size = u16_at(&nt, 20)? as usize;
    let opt = read_at(&mut r, lfanew + 24, opt_size).ok()?;
    let dd_base = if u16_at(&opt, 0)? == 0x20b { 112 } else { 96 };
    let off = u32_at(&opt, dd_base + 32)? as u64;
    let size = u32_at(&opt, dd_base + 36)? as u64;
    if off == 0 || !(8..=MAX_SIGNATURE).contains(&size) || off + size > file_len {
        return None;
    }
    let cert = read_at(&mut r, off, size as usize).ok()?;
    let len = (u32_at(&cert, 0)? as usize).min(cert.len());
    // WIN_CERT_TYPE_PKCS_SIGNED_DATA
    if u16_at(&cert, 6)? != 2 || len <= 8 {
        return None;
    }
    Some(cert[8..len].to_vec())
}

// The signer's display name, read through the atime-preserving handle.
pub fn signer(path: &std::path::Path) -> Option<String> {
    let f = hoardini_win::open_read_preserving_atime(path).ok()?;
    let len = f.metadata().ok()?.len();
    let blob = signature_blob(f, len)?;
    hoardini_win::pkcs7_signer(&blob).ok().flatten()
}

fn find_version_blob(rsrc: &[u8], rsrc_rva: u64) -> Option<&[u8]> {
    let first_entry = |dir: u32, want_id: Option<u32>| -> Option<u32> {
        let d = dir as usize;
        let named = u16_at(rsrc, d + 12)? as usize;
        let ids = u16_at(rsrc, d + 14)? as usize;
        let base = d + 16 + named * 8;
        for i in 0..ids {
            let e = base + i * 8;
            let id = u32_at(rsrc, e)?;
            if want_id.is_none_or(|w| w == id) {
                return u32_at(rsrc, e + 4);
            }
        }
        None
    };
    let type_dir = first_entry(0, Some(RT_VERSION))?;
    let name_dir = first_entry(type_dir & 0x7fff_ffff, None)?;
    let lang_entry = first_entry(name_dir & 0x7fff_ffff, None)?;
    let data = (lang_entry & 0x7fff_ffff) as usize;
    let rva = u32_at(rsrc, data)? as u64;
    let size = u32_at(rsrc, data + 4)? as usize;
    let start = usize::try_from(rva.checked_sub(rsrc_rva)?).ok()?;
    rsrc.get(start..start.checked_add(size)?)
}

struct Block<'a> {
    value_len: usize,
    key: String,
    value_off: usize,
    buf: &'a [u8],
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

fn block_at(buf: &[u8], off: usize) -> Option<Block<'_>> {
    let len = u16_at(buf, off)? as usize;
    let value_len = u16_at(buf, off + 2)? as usize;
    if len < 6 || off + len > buf.len() {
        return None;
    }
    let mut p = off + 6;
    let mut units = Vec::new();
    loop {
        let u = u16_at(buf, p)?;
        p += 2;
        if u == 0 {
            break;
        }
        units.push(u);
    }
    Some(Block {
        value_len,
        key: String::from_utf16_lossy(&units),
        value_off: off + align4(p - off),
        buf: &buf[..off + len],
    })
}

fn children(b: &Block<'_>, value_bytes: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut p = align4(b.value_off + value_bytes);
    let end = b.buf.len();
    while p + 6 <= end {
        let Some(len) = u16_at(b.buf, p).map(|l| l as usize) else {
            break;
        };
        if len < 6 {
            break;
        }
        out.push(p);
        p = align4(p + len);
    }
    out
}

fn fill_strings(blob: &[u8], info: &mut PeInfo) {
    let Some(root) = block_at(blob, 0) else {
        return;
    };
    if root.key != "VS_VERSION_INFO" {
        return;
    }
    for sfi_off in children(&root, root.value_len) {
        let Some(sfi) = block_at(blob, sfi_off) else {
            continue;
        };
        if sfi.key != "StringFileInfo" {
            continue;
        }
        for table_off in children(&sfi, 0) {
            let Some(table) = block_at(blob, table_off) else {
                continue;
            };
            for s_off in children(&table, 0) {
                let Some(s) = block_at(blob, s_off) else {
                    continue;
                };
                let vbytes = s.value_len * 2;
                let Some(raw) = s
                    .buf
                    .get(s.value_off..(s.value_off + vbytes).min(s.buf.len()))
                else {
                    continue;
                };
                let units: Vec<u16> = raw
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .take_while(|u| *u != 0)
                    .collect();
                let val = String::from_utf16_lossy(&units).trim().to_string();
                if val.is_empty() {
                    continue;
                }
                let slot = match s.key.as_str() {
                    "ProductName" => &mut info.product_name,
                    "CompanyName" => &mut info.company_name,
                    "FileDescription" => &mut info.file_description,
                    "ProductVersion" => &mut info.product_version,
                    "FileVersion" => &mut info.file_version,
                    "OriginalFilename" => &mut info.original_filename,
                    _ => continue,
                };
                slot.get_or_insert(val);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_version_info_from_a_system_binary() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let path = std::path::Path::new(&root)
            .join("System32")
            .join("kernel32.dll");
        let Ok(f) = std::fs::File::open(&path) else {
            return;
        };
        let len = f.metadata().unwrap().len();
        let info = parse(f, len).unwrap();
        assert!(info.is_dll && info.is_64bit);
        assert_eq!(info.company_name.as_deref(), Some("Microsoft Corporation"));
        assert!(info.product_version.is_some());
    }

    #[test]
    fn rejects_non_pe() {
        assert!(parse(std::io::Cursor::new(b"MZ\0\0".to_vec()), 4).is_none());
    }
}

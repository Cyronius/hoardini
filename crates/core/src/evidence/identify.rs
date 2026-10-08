use std::io::{Read, Seek, SeekFrom};

use super::{Family, Identity, gguf, pe, safetensors};

const HEAD: usize = 512;

fn base(kind: &str, family: Family, compressed: bool) -> Identity {
    Identity {
        kind: kind.to_string(),
        family,
        mime: None,
        already_compressed: compressed,
        gguf: None,
        safetensors: None,
        pe: None,
        note: None,
    }
}

fn read_range<R: Read + Seek>(r: &mut R, off: u64, len: usize) -> Vec<u8> {
    if r.seek(SeekFrom::Start(off)).is_err() {
        return Vec::new();
    }
    let mut buf = vec![0u8; len];
    let mut got = 0;
    while got < len {
        match r.read(&mut buf[got..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => got += n,
        }
    }
    buf.truncate(got);
    buf
}

fn ext_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

// Reads only the head (plus a few fixed probes and the format headers); never tensor or payload data.
pub fn identify<R: Read + Seek>(mut reader: R, len: u64, name: &str) -> Identity {
    let head = read_range(&mut reader, 0, HEAD);
    let ext = ext_of(name);

    if head.starts_with(b"GGUF") {
        return match gguf::parse(&mut reader, len) {
            Ok(g) => {
                let compressed = g.quant.as_deref().is_some_and(|q| !gguf::is_float_quant(q));
                let mut id = base("gguf", Family::Model, compressed);
                id.gguf = Some(g);
                id
            }
            Err(_) => {
                let mut id = base("gguf", Family::Model, true);
                id.note = Some("GGUF header unreadable".into());
                id
            }
        };
    }

    if safetensors::header_len_if_plausible(&head, len).is_some()
        && let Ok(s) = safetensors::parse(&mut reader, len)
    {
        let quantized = s
            .dominant_dtype()
            .is_some_and(|d| matches!(d, "I8" | "U8" | "I4" | "U4" | "F8_E4M3" | "F8_E5M2"));
        let mut id = base("safetensors", Family::Model, quantized);
        id.safetensors = Some(s);
        return id;
    }

    if head.starts_with(b"MZ") {
        let info = pe::parse(&mut reader, len);
        let dll = info.as_ref().is_some_and(|p| p.is_dll) || matches!(ext.as_str(), "dll" | "ocx");
        let kind = if info.is_none() {
            "dos-exe"
        } else if dll {
            "pe-dll"
        } else {
            "pe-exe"
        };
        let mut id = base(kind, Family::Executable, false);
        id.pe = info;
        return id;
    }

    if head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        let kind = match ext.as_str() {
            "msi" | "msp" | "msm" => "msi",
            "doc" | "xls" | "ppt" => "ole-office",
            _ => "ole-cfb",
        };
        let family = if kind == "msi" {
            Family::Executable
        } else {
            Family::Document
        };
        return base(kind, family, false);
    }

    if head.starts_with(b"vhdxfile") {
        return base("vhdx", Family::DiskImage, false);
    }
    if head.starts_with(b"conectix") {
        return base("vhd", Family::DiskImage, false);
    }
    if len >= 0x8006 && read_range(&mut reader, 0x8001, 5) == b"CD001" {
        return base("iso", Family::DiskImage, false);
    }
    if len >= 512 && ext == "vhd" && read_range(&mut reader, len - 512, 8) == b"conectix" {
        return base("vhd", Family::DiskImage, false);
    }

    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return base("zip", Family::Archive, true);
    }
    if head.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        return base("7z", Family::Archive, true);
    }
    if head.starts_with(b"MSCF") {
        return base("cab", Family::Archive, true);
    }
    if head.starts_with(&[0x1F, 0x8B]) {
        return base("gzip", Family::Archive, true);
    }
    if head.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        return base("zstd", Family::Archive, true);
    }
    if head.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0]) {
        return base("xz", Family::Archive, true);
    }
    if head.starts_with(b"BZh") && head.len() > 3 && head[3].is_ascii_digit() {
        return base("bzip2", Family::Archive, true);
    }
    if head.starts_with(b"Rar!") {
        return base("rar", Family::Archive, true);
    }
    if head.len() >= 262 && &head[257..262] == b"ustar" {
        return base("tar", Family::Archive, false);
    }
    if head.starts_with(b"SQLite format 3\0") {
        return base("sqlite", Family::Database, false);
    }

    if let Some(t) = infer::get(&head) {
        use infer::MatcherType as M;
        let (family, compressed) = match t.matcher_type() {
            M::Video => (Family::Video, true),
            M::Image => (
                Family::Image,
                !matches!(t.extension(), "bmp" | "tif" | "psd" | "tiff"),
            ),
            M::Audio => (Family::Audio, !matches!(t.extension(), "wav" | "aiff")),
            M::Archive => (Family::Archive, true),
            M::Doc | M::Book => (
                Family::Document,
                matches!(t.extension(), "docx" | "xlsx" | "pptx" | "epub"),
            ),
            M::App => (Family::Executable, false),
            _ => (Family::Other, false),
        };
        let mut id = base(t.extension(), family, compressed);
        id.mime = Some(t.mime_type().to_string());
        return id;
    }

    match ext.as_str() {
        "gguf" => {
            let mut id = base("gguf", Family::Model, true);
            id.note = Some("extension says GGUF but the header is missing".into());
            id
        }
        "mp4" | "mkv" | "webm" | "avi" | "mov" => base(&ext, Family::Video, true),
        "mp3" | "aac" | "ogg" | "opus" | "flac" | "m4a" => base(&ext, Family::Audio, true),
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "heic" | "avif" => {
            base(&ext, Family::Image, true)
        }
        _ => base("unknown", Family::Other, false),
    }
}

use std::io::Read;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use super::Subject;
use crate::index::to_i64;
use crate::store::now_ms;

const CHUNK: usize = 1 << 20;

fn is_hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit())
}

// Hugging Face blobs and Ollama `sha256-*` files are named by their SHA-256, so they never need hashing.
pub fn content_address(path: &str) -> Option<String> {
    let mut parts = path.rsplit('\\');
    let name = parts.next()?;
    let parent = parts.next()?;
    if let Some(hex) = name.strip_prefix("sha256-")
        && parent.eq_ignore_ascii_case("blobs")
        && is_hex(hex, 64)
    {
        return Some(hex.to_ascii_lowercase());
    }
    let repo = parts.next()?;
    if parent.eq_ignore_ascii_case("blobs")
        && is_hex(name, 64)
        && (repo.starts_with("models--") || repo.starts_with("datasets--"))
    {
        return Some(name.to_ascii_lowercase());
    }
    None
}

// A stored hash counts only while the file id, size and mtime are the ones it was taken for.
pub fn current(conn: &Connection, s: &Subject) -> Option<String> {
    if s.dir {
        return None;
    }
    if let Some(h) = content_address(&s.path) {
        return Some(h);
    }
    let fid = s.file_id?;
    let (size, mtime, sha): (i64, Option<i64>, String) = conn
        .query_row(
            "SELECT size, mtime, sha256 FROM file_hash WHERE volume_id = ?1 AND file_id = ?2",
            params![s.volume_id, fid.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .ok()??;
    (size == to_i64(s.size) && mtime == s.mtime).then_some(sha)
}

pub fn store(conn: &Connection, s: &Subject, sha: &str) -> anyhow::Result<()> {
    let Some(fid) = s.file_id else {
        anyhow::bail!("no file id for {}", s.path);
    };
    conn.execute(
        "INSERT OR REPLACE INTO file_hash(volume_id, file_id, size, mtime, sha256, hashed_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            s.volume_id,
            fid.to_vec(),
            to_i64(s.size),
            s.mtime,
            sha,
            now_ms()
        ],
    )?;
    Ok(())
}

// None when `go` said stop part-way. Reads through the atime-preserving handle, which also refuses placeholders.
pub fn sha256_file(path: &Path, go: &dyn Fn() -> bool) -> std::io::Result<Option<String>> {
    let mut f = hoardini_win::open_read_preserving_atime(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        if !go() {
            return Ok(None);
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(Some(hex(&h.finalize())))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// Hashes the subject unless a current hash exists; the file must still be what the subject describes.
pub fn hash_subject(
    conn: &Connection,
    s: &Subject,
    go: &dyn Fn() -> bool,
) -> anyhow::Result<Option<String>> {
    if let Some(h) = current(conn, s) {
        return Ok(Some(h));
    }
    if s.dir || s.is_placeholder() {
        anyhow::bail!("{} cannot be hashed here", s.path);
    }
    let Some(sha) = sha256_file(Path::new(&s.path), go)? else {
        return Ok(None);
    };
    let Some(now) = Subject::load(conn, &s.path) else {
        anyhow::bail!("{} vanished while hashing", s.path);
    };
    if now.size != s.size || now.mtime != s.mtime || now.file_id != s.file_id {
        anyhow::bail!("{} changed while hashing", s.path);
    }
    store(conn, s, &sha)?;
    Ok(Some(sha))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_addressed_names_are_recognized() {
        let sha = "a".repeat(64);
        assert_eq!(
            content_address(&format!(
                r"C:\Users\u\.cache\huggingface\hub\models--org--m\blobs\{sha}"
            )),
            Some(sha.clone())
        );
        assert_eq!(
            content_address(&format!(r"C:\Users\u\.ollama\models\blobs\sha256-{sha}")),
            Some(sha.clone())
        );
        assert_eq!(
            content_address(&format!(r"C:\x\blobs\{sha}")),
            None,
            "a blobs folder outside a hub repo is not content-addressed"
        );
        assert_eq!(
            content_address(r"C:\Users\u\.cache\huggingface\hub\models--o--m\blobs\abc"),
            None
        );
    }
}

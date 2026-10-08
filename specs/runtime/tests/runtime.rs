// Traces: RUNTIME-PINNED-HASH (canonical spec: specs/runtime/spec.md)
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use hoardini_core::runtime::{self, Kind, Pinned};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn serve(body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let mut seen = Vec::new();
                while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    seen.extend_from_slice(&buf[..n]);
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                for chunk in body.chunks(1000) {
                    let _ = sock.write_all(chunk).await;
                }
                let _ = sock.shutdown().await;
            });
        }
    });
    format!("http://{addr}/blob/file.bin")
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn pin(url: String, hash: String, size: usize, kind: Kind) -> Pinned {
    Pinned {
        id: "tool",
        version: "1.0",
        url: leak(url),
        sha256: leak(hash),
        size: size as u64,
        kind,
        license_url: "https://example.com/license",
    }
}

fn zip_bytes() -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (name, data) in [
        ("pkg/tool.exe", &b"MZ-tool"[..]),
        ("pkg/readme.txt", b"read me"),
        ("pkg/extra.dll", b"dll"),
    ] {
        w.start_file(name, opts).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn no_progress(_: u64, _: u64) {}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[tokio::test]
async fn correct_hash_installs_and_verifies() {
    let data = vec![7u8; 5000];
    let url = serve(data.clone()).await;
    let p = pin(url, sha(&data), data.len(), Kind::File);
    let root = tempfile::tempdir().unwrap();
    let last = AtomicU64::new(0);
    let cb = |done: u64, total: u64| {
        assert_eq!(total, 5000);
        last.store(done, Ordering::Relaxed);
    };
    let path = runtime::ensure(&p, root.path(), &cb, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert_eq!(last.load(Ordering::Relaxed), 5000);
    runtime::verify_before_run(&path, &p).unwrap();
    assert!(runtime::is_installed(&p, root.path()));
    runtime::remove(&p, root.path()).unwrap();
    assert!(!path.exists());
    assert!(names(&root.path().join("models")).is_empty());
}

#[tokio::test]
async fn wrong_hash_deletes_and_errors() {
    let data = vec![1u8; 3000];
    let url = serve(data.clone()).await;
    let p = pin(url, sha(b"something else"), data.len(), Kind::File);
    let root = tempfile::tempdir().unwrap();
    let err = runtime::ensure(&p, root.path(), &no_progress, &AtomicBool::new(false))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("SHA-256"), "{err}");
    assert!(names(&root.path().join("models")).is_empty());
    assert!(names(root.path()).iter().all(|n| !n.ends_with(".part")));
}

#[tokio::test]
async fn tampering_after_install_is_caught() {
    let data = vec![9u8; 4096];
    let url = serve(data.clone()).await;
    let p = pin(url, sha(&data), data.len(), Kind::File);
    let root = tempfile::tempdir().unwrap();
    let path = runtime::ensure(&p, root.path(), &no_progress, &AtomicBool::new(false))
        .await
        .unwrap();
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    f.write_all(b"x").unwrap();
    drop(f);
    assert!(runtime::verify_before_run(&path, &p).is_err());
}

#[tokio::test]
async fn missing_marker_is_rehashed() {
    let data = vec![3u8; 2048];
    let url = serve(data.clone()).await;
    let p = pin(url, sha(&data), data.len(), Kind::File);
    let root = tempfile::tempdir().unwrap();
    let path = runtime::ensure(&p, root.path(), &no_progress, &AtomicBool::new(false))
        .await
        .unwrap();
    let marker = path.with_file_name("file.bin.verified");
    std::fs::remove_file(&marker).unwrap();
    runtime::verify_before_run(&path, &p).unwrap();
    assert!(marker.exists());
}

#[tokio::test]
async fn zip_extracts_only_listed_files() {
    let data = zip_bytes();
    let url = serve(data.clone()).await;
    let p = pin(
        url,
        sha(&data),
        data.len(),
        Kind::Zip {
            extract: &["pkg/tool.exe"],
        },
    );
    let root = tempfile::tempdir().unwrap();
    let path = runtime::ensure(&p, root.path(), &no_progress, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"MZ-tool");
    let dir = path.parent().unwrap();
    assert_eq!(names(dir), vec![".verified", "tool.exe"]);
    runtime::verify_before_run(&path, &p).unwrap();
    assert_eq!(names(&root.path().join("runtime")), vec!["tool-1.0"]);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"!")
        .unwrap();
    assert!(runtime::verify_before_run(&path, &p).is_err());
}

#[tokio::test]
async fn cancel_leaves_nothing() {
    let data = vec![5u8; 3000];
    let url = serve(data.clone()).await;
    let p = pin(url, sha(&data), data.len(), Kind::File);
    let root = tempfile::tempdir().unwrap();
    let err = runtime::ensure(&p, root.path(), &no_progress, &AtomicBool::new(true))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("cancelled"), "{err}");
    assert!(names(&root.path().join("models")).is_empty());
}

#[test]
fn static_pins_are_well_formed() {
    assert!(runtime::PINS.len() >= 6);
    for p in runtime::PINS {
        assert_eq!(p.sha256.len(), 64, "{}", p.id);
        assert!(
            p.sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "{}",
            p.id
        );
        assert!(p.url.starts_with("https://"), "{}", p.id);
        assert!(p.license_url.starts_with("https://"), "{}", p.id);
        assert!(p.size > 0, "{}", p.id);
        if let Kind::Zip { extract } = p.kind {
            assert!(!extract.is_empty(), "{}", p.id);
        }
    }
    assert!(runtime::find("rclone").is_some());
    assert!(runtime::find("llama.cpp").is_some());
}

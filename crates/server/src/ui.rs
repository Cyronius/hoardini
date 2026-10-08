use std::path::{Component, Path, PathBuf};

use axum::body::Body;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::Response;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_ui.rs"));
}

const PLACEHOLDER: &str = "<!doctype html><meta charset=\"utf-8\"><title>Hoardini</title>\
<body style=\"font-family:system-ui;margin:3rem\"><h1>Hoardini</h1>\
<p>The UI bundle is not built. Run <code>bun run build</code> in <code>ui/</code>, \
or start with <code>--ui-dir</code>.</p></body>";

fn mime(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" | "webmanifest" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn clean(path: &str) -> Option<String> {
    let p = path.trim_start_matches('/');
    let mut parts = Vec::new();
    for c in Path::new(p).components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?.to_string()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

fn respond(path: &str, bytes: Vec<u8>, head: bool) -> Response {
    let mut r = Response::new(if head {
        Body::empty()
    } else {
        Body::from(bytes)
    });
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime(path)));
    if path.ends_with(".html") {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    r
}

async fn lookup(ui_dir: Option<&PathBuf>, rel: &str) -> Option<Vec<u8>> {
    match ui_dir {
        Some(dir) => {
            let full = dir.join(rel);
            if full.is_file() {
                tokio::fs::read(full).await.ok()
            } else {
                None
            }
        }
        None => embedded::FILES
            .iter()
            .find(|(p, _)| *p == rel)
            .map(|(_, b)| b.to_vec()),
    }
}

pub async fn serve(ui_dir: Option<&PathBuf>, method: &Method, path: &str) -> Response {
    let head = *method == Method::HEAD;
    if !head && *method != Method::GET {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
        return r;
    }
    let Some(rel) = clean(path) else {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::BAD_REQUEST;
        return r;
    };
    let wanted = if rel.is_empty() { "index.html" } else { &rel };
    if let Some(b) = lookup(ui_dir, wanted).await {
        return respond(wanted, b, head);
    }
    // Paths with an extension are asset requests; only extensionless ones are SPA routes.
    let is_asset = Path::new(wanted).extension().is_some();
    if !is_asset && let Some(b) = lookup(ui_dir, "index.html").await {
        return respond("index.html", b, head);
    }
    if !is_asset && ui_dir.is_none() && embedded::FILES.is_empty() {
        return respond("index.html", PLACEHOLDER.as_bytes().to_vec(), head);
    }
    let mut r = Response::new(Body::from("not found"));
    *r.status_mut() = StatusCode::NOT_FOUND;
    r
}

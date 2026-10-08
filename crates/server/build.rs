use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, root, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push((rel.to_string_lossy().replace('\\', "/"), p));
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let dist = manifest.join("../../ui/dist");
    println!("cargo:rerun-if-changed=../../ui/dist");
    let mut files = Vec::new();
    walk(&dist, &dist, &mut files);
    let mut src = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for (rel, abs) in &files {
        let abs = abs.canonicalize().unwrap_or_else(|_| abs.clone());
        let abs = abs.to_string_lossy();
        let abs = abs.strip_prefix(r"\\?\").unwrap_or(&abs);
        let _ = writeln!(src, "    ({rel:?}, include_bytes!({abs:?})),");
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("embedded_ui.rs");
    if let Err(e) = std::fs::write(&out, src) {
        panic!("writing {}: {e}", out.display());
    }
}

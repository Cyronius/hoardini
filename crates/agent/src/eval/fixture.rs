use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use hoardini_core::destinations::{self, NewDestination};
use hoardini_core::index::{self, FakeFs, Mode, RefreshOptions, refresh};
use hoardini_core::memory::{self, Kind, NewMemory, Scope};
use hoardini_core::store::{Db, now_ms};
use rusqlite::params;

const MB: f64 = 1_048_576.0;
const GB: f64 = 1_073_741_824.0;
const DAY: i64 = 86_400_000;
const FREE: u64 = 62 * (1 << 30);
// Folded into parent totals by the 1 MB row threshold, so padding never shows up as a file row.
const SMALL: u64 = 996_000;

pub const PROFILE_REL: &str = r"Users\ana";
pub const QWEN_LMSTUDIO: &str =
    r"~\.lmstudio\models\lmstudio-community\Qwen3.8-27B-GGUF\Qwen3.8-27B-Q8_0.gguf";
pub const QWEN_COPY: &str = r"~\models-src\Qwen3.8-27B-Q8_0.gguf";
pub const PROTECTED: &str = r"~\Downloads\tax-2025";
pub const DESTINATION: &str = "NAS";

#[derive(Clone, Copy, PartialEq)]
enum When {
    Both,
    New,
    Grew(u64),
}

struct File {
    path: String,
    size: u64,
    mdays: i64,
    adays: i64,
    when: When,
}

struct Pad {
    dir: String,
    count: u64,
}

#[derive(Default)]
struct Spec {
    files: Vec<File>,
    pads: Vec<Pad>,
}

fn gb(x: f64) -> u64 {
    (x * GB) as u64
}

fn mb(x: f64) -> u64 {
    (x * MB) as u64
}

impl Spec {
    fn file(&mut self, path: &str, size: u64, mdays: i64, adays: i64) -> &mut Self {
        self.files.push(File {
            path: path.into(),
            size,
            mdays,
            adays,
            when: When::Both,
        });
        self
    }

    fn new_file(&mut self, path: &str, size: u64, days: i64) -> &mut Self {
        self.files.push(File {
            path: path.into(),
            size,
            mdays: days,
            adays: days,
            when: When::New,
        });
        self
    }

    fn grew(&mut self, path: &str, before: u64, after: u64, days: i64) -> &mut Self {
        self.files.push(File {
            path: path.into(),
            size: after,
            mdays: days,
            adays: days,
            when: When::Grew(before),
        });
        self
    }

    fn pad(&mut self, dir: &str, bytes: u64) -> &mut Self {
        self.pads.push(Pad {
            dir: dir.into(),
            count: bytes.div_ceil(SMALL),
        });
        self
    }
}

// Shaped after the design's survey of the author's PC (design section 1), scaled to stay quick to index.
fn spec() -> Spec {
    let mut s = Spec::default();
    let lm = r"~\.lmstudio\models";
    s.file(QWEN_LMSTUDIO, gb(27.1), 160, 1)
        .file(
            &format!(r"{lm}\lmstudio-community\Llama-3.3-70B-Instruct-GGUF\Llama-3.3-70B-Instruct-Q4_K_M.gguf"),
            gb(42.5),
            210,
            75,
        )
        .file(
            &format!(r"{lm}\bartowski\Mistral-Small-24B-Instruct-2501-GGUF\Mistral-Small-24B-Instruct-2501-Q3_K_M.gguf"),
            gb(9.9),
            180,
            64,
        )
        .file(
            &format!(r"{lm}\lmstudio-community\gemma-3-12b-it-GGUF\gemma-3-12b-it-Q4_K_M.gguf"),
            gb(7.3),
            150,
            20,
        )
        .file(
            &format!(r"{lm}\openbmb\MiniCPM5-2B-GGUF\MiniCPM5-2B-Q4_K_M.gguf"),
            gb(1.6),
            40,
            0,
        )
        .new_file(
            &format!(r"{lm}\unsloth\Qwen3-Coder-30B-A3B-Instruct-GGUF\Qwen3-Coder-30B-A3B-Instruct-Q4_K_M.gguf"),
            gb(17.3),
            3,
        )
        .file(
            r"~\.lmstudio\extensions\backends\llama.cpp-win-x86_64-nvidia-cuda12-avx2-1.52.0\ggml-cuda.dll",
            gb(1.4),
            30,
            0,
        )
        .pad(r"~\.lmstudio\extensions\backends\llama.cpp-win-x86_64-avx2-1.52.0", gb(1.6))
        .pad(r"~\.lmstudio\conversations", mb(14.0));

    let hub = r"~\.cache\huggingface\hub";
    for (i, size) in [4.98, 5.0, 4.92, 1.17].iter().enumerate() {
        s.file(
            &format!(
                r"{hub}\models--meta-llama--Llama-3.1-8B-Instruct\blobs\{}",
                hex64(&format!("llama31-{i}"))
            ),
            gb(*size),
            230,
            95,
        );
    }
    s.file(
        &format!(
            r"{hub}\models--openai--whisper-large-v3\blobs\{}",
            hex64("whisper")
        ),
        gb(3.09),
        300,
        300,
    )
    .file(
        &format!(r"{hub}\models--BAAI--bge-m3\blobs\{}", hex64("bge")),
        gb(2.27),
        120,
        30,
    )
    .pad(r"~\.cache\pip\http-v2", gb(6.2))
    .file(
        r"~\.cache\uv\archive-v0\Zq3Hk1\torch\lib\torch_cuda.dll",
        gb(2.4),
        44,
        12,
    )
    .pad(r"~\.cache\uv\archive-v0\Zq3Hk1\torch\lib\site", gb(2.2))
    .file(
        r"~\.cache\torch\hub\checkpoints\sam_vit_h_4b8939.pth",
        gb(2.4),
        400,
        400,
    );

    s.file(QWEN_COPY, gb(27.1), 160, 40);

    let dl = r"~\Downloads";
    s.file(
        &format!(r"{dl}\Win11_25H2_English_x64.iso"),
        gb(7.0),
        200,
        200,
    )
    .file(
        &format!(r"{dl}\ubuntu-24.04.3-desktop-amd64.iso"),
        gb(5.9),
        150,
        148,
    )
    .new_file(&format!(r"{dl}\proxmox-ve_9.0-1.iso"), gb(1.4), 4)
    .file(
        &format!(r"{dl}\LM-Studio-0.3.30-x64.exe"),
        mb(530.0),
        60,
        60,
    )
    .file(&format!(r"{dl}\OllamaSetup.exe"), mb(690.0), 45, 45)
    .file(
        &format!(r"{dl}\VSCodeUserSetup-x64-1.104.2.exe"),
        mb(110.0),
        30,
        30,
    )
    .file(&format!(r"{dl}\rustup-init.exe"), mb(12.0), 90, 90)
    .file(
        &format!(r"{dl}\tax-2025\receipts-2025.zip"),
        mb(180.0),
        20,
        20,
    )
    .file(&format!(r"{dl}\tax-2025\2025-return.pdf"), mb(2.4), 20, 20)
    .pad(dl, mb(120.0));

    let nuget = r"~\.nuget\packages";
    let pkgs = [
        ("microsoft.windowsappsdk", "1.8.250916003", 1200.0),
        ("microsoft.windowsappsdk", "1.7.250606001", 1150.0),
        ("microsoft.netcore.app.runtime.win-x64", "9.0.9", 820.0),
        ("microsoft.netcore.app.runtime.win-x64", "8.0.20", 790.0),
        ("microsoft.aspnetcore.app.runtime.win-x64", "9.0.9", 610.0),
        ("microsoft.windows.sdk.net.ref", "10.0.26100.57", 930.0),
        ("microsoft.windows.sdk.net.ref", "10.0.22621.57", 910.0),
        ("microsoft.ml.onnxruntime.gpu.windows", "1.22.0", 1450.0),
        ("microsoft.ml.onnxruntime", "1.22.0", 380.0),
        ("skiasharp.nativeassets.win32", "3.119.0", 210.0),
        ("microsoft.web.webview2", "1.0.3405.78", 260.0),
        (
            "runtime.win-x64.microsoft.dotnet.ilcompiler",
            "9.0.9",
            1100.0,
        ),
        ("microsoft.dotnet.ilcompiler", "9.0.9", 640.0),
        ("avalonia.native", "11.3.6", 150.0),
        ("microsoft.codeanalysis.csharp", "4.14.0", 190.0),
        ("libtorch-cuda-12.1-win-x64-part1", "2.5.1", 1700.0),
    ];
    for (id, ver, size) in pkgs {
        s.file(
            &format!(r"{nuget}\{id}\{ver}\{id}.{ver}.nupkg"),
            mb(size),
            120,
            40,
        );
    }
    s.pad(nuget, gb(1.3));

    s.file(
        r"~\.venvs\ml\Lib\site-packages\torch\lib\torch_cuda.dll",
        gb(2.1),
        25,
        2,
    )
    .file(
        r"~\.venvs\ml\Lib\site-packages\torch\lib\cudnn_engines_precompiled64_9.dll",
        mb(600.0),
        25,
        2,
    )
    .pad(r"~\.venvs\ml\Lib\site-packages", gb(2.3))
    .file(
        r"~\.venvs\scratch\Lib\site-packages\tensorflow\python\_pywrap_tensorflow_internal.pyd",
        gb(1.1),
        310,
        300,
    )
    .pad(r"~\.venvs\scratch\Lib\site-packages", mb(560.0))
    .file(
        r"~\.venvs\whisperx\Lib\site-packages\ctranslate2\ctranslate2.dll",
        mb(410.0),
        75,
        70,
    )
    .pad(r"~\.venvs\whisperx\Lib\site-packages", mb(380.0));

    s.file(
        r"~\.unsloth\studio\models\unsloth--gemma-3-4b-it\model.safetensors",
        gb(4.3),
        100,
        100,
    )
    .pad(r"~\.unsloth\unsloth_compiled_cache", mb(400.0))
    .file(
        r"~\AppData\Local\Docker\wsl\disk\docker_data.vhdx",
        gb(21.3),
        0,
        0,
    )
    .pad(r"~\AppData\Local\Temp", gb(1.8))
    .file(r"~\Documents\Videos\wedding-2019.mp4", gb(4.2), 900, 400)
    .pad(r"~\Documents", mb(800.0));

    let code = "code";
    s.grew(
        &format!(r"{code}\hoardini\target\debug\deps\libwindows-5d1f0c3a.rlib"),
        gb(0.9),
        gb(2.4),
        1,
    )
    .new_file(
        &format!(r"{code}\hoardini\target\debug\incremental\hoardini_server-3k9x\query-cache.bin"),
        gb(4.2),
        1,
    )
    .file(
        &format!(r"{code}\hoardini\target\debug\hoardini.pdb"),
        gb(1.2),
        1,
        1,
    )
    .pad(&format!(r"{code}\hoardini\target\debug\build"), gb(1.9))
    .pad(&format!(r"{code}\hoardini\ui\node_modules"), mb(640.0))
    .file(
        &format!(r"{code}\based\target\debug\deps\libbased_core-9c1e.rlib"),
        gb(3.0),
        6,
        6,
    )
    .pad(&format!(r"{code}\based\target\debug\build"), gb(3.1))
    .pad(&format!(r"{code}\based\node_modules"), gb(1.35))
    .pad(&format!(r"{code}\based-old\node_modules"), gb(1.3))
    .file(
        &format!(r"{code}\based-old\target\debug\deps\libbased_core-77aa.rlib"),
        gb(2.6),
        140,
        140,
    )
    .file(
        &format!(r"{code}\llama.cpp\build\bin\Release\ggml-cuda.dll"),
        gb(1.1),
        33,
        33,
    )
    .pad(&format!(r"{code}\llama.cpp\build"), gb(1.2))
    .pad(&format!(r"{code}\lm-admin\node_modules"), gb(1.05))
    .pad(&format!(r"{code}\client-site\node_modules"), mb(420.0))
    .pad(&format!(r"{code}\client-site\.next\cache"), mb(310.0));

    s.file(r"models\whisper\ggml-large-v3.bin", gb(3.1), 300, 120)
        .file(r"models\sd\sd_xl_base_1.0.safetensors", gb(6.5), 260, 210)
        .file(r"$WINDOWS.~BT\Sources\install.esd", gb(4.1), 400, 400)
        .pad(r"$WINDOWS.~BT\Sources", gb(1.05))
        .file(r"ESD\Windows\sources\install.esd", gb(4.4), 410, 410)
        .file(
            r"Windows\SoftwareDistribution\Download\9b4c1f\windows11.0-kb5065789-x64.msu",
            gb(3.4),
            12,
            12,
        )
        .pad(r"Windows\System32", gb(6.0))
        .pad(r"Windows\WinSxS", gb(5.0))
        .pad(r"Program Files\Common Files", gb(2.5))
        .pad(r"Program Files\JetBrains", gb(3.5))
        .pad(r"ProgramData\Package Cache", gb(1.2))
        .file("pagefile.sys", gb(16.0), 0, 0)
        .file("hiberfil.sys", gb(12.7), 0, 0);
    s
}

fn hex64(seed: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut out = String::with_capacity(64);
    for round in 0..4u64 {
        for b in seed.bytes().chain(round.to_le_bytes()) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        out.push_str(&format!("{h:016x}"));
    }
    out
}

#[derive(Clone, Debug)]
pub struct Fixture {
    pub root: String,
    pub profile: String,
}

impl Fixture {
    pub fn new(root: &str) -> Fixture {
        let root = format!("{}\\", root.trim_end_matches(['\\', '/']));
        Fixture {
            profile: format!("{root}{PROFILE_REL}"),
            root,
        }
    }

    pub fn abs(&self, rel: &str) -> String {
        match rel.strip_prefix("~\\") {
            Some(tail) => format!(r"{}\{tail}", self.profile),
            None if rel == "~" => self.profile.clone(),
            None => format!("{}{rel}", self.root),
        }
    }

    pub fn size_of(&self, db: &Db, rel: &str) -> Result<u64> {
        let conn = db.conn()?;
        Ok(index::node_by_path(&conn, &self.abs(rel))?
            .map(|n| n.size)
            .unwrap_or(0))
    }
}

fn gguf_header(name: &str, arch: &str, size_label: &str, ftype: u32) -> Vec<u8> {
    let mut b = Vec::new();
    let s = |b: &mut Vec<u8>, v: &str| {
        b.extend_from_slice(&(v.len() as u64).to_le_bytes());
        b.extend_from_slice(v.as_bytes());
    };
    b.extend_from_slice(b"GGUF");
    b.extend_from_slice(&3u32.to_le_bytes());
    b.extend_from_slice(&0u64.to_le_bytes());
    b.extend_from_slice(&5u64.to_le_bytes());
    for (k, v) in [
        ("general.architecture", arch),
        ("general.name", name),
        ("general.size_label", size_label),
    ] {
        s(&mut b, k);
        b.extend_from_slice(&8u32.to_le_bytes());
        s(&mut b, v);
    }
    s(&mut b, "general.file_type");
    b.extend_from_slice(&4u32.to_le_bytes());
    b.extend_from_slice(&ftype.to_le_bytes());
    s(&mut b, &format!("{arch}.context_length"));
    b.extend_from_slice(&4u32.to_le_bytes());
    b.extend_from_slice(&131_072u32.to_le_bytes());
    b
}

// Just enough of each format's header for the evidence engine to recognise the file.
fn content(path: &str) -> Vec<u8> {
    let name = path.rsplit('\\').next().unwrap_or(path);
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".gguf") {
        let stem = name.trim_end_matches(".gguf");
        let (ftype, quant) = [("Q8_0", 7), ("Q4_K_M", 15), ("Q3_K_M", 12), ("Q6_K", 18)]
            .iter()
            .find(|(q, _)| stem.ends_with(q))
            .map(|(q, f)| (*f, *q))
            .unwrap_or((15, "Q4_K_M"));
        let base = stem
            .trim_end_matches(quant)
            .trim_end_matches(['-', '_', '.']);
        let arch = if lower.contains("llama") || lower.contains("mistral") {
            "llama"
        } else if lower.contains("gemma") {
            "gemma3"
        } else if lower.contains("coder") {
            "qwen3moe"
        } else if lower.contains("minicpm") {
            "minicpm"
        } else {
            "qwen35"
        };
        let label = base
            .split(['-', '_'])
            .find(|p| p.ends_with('B') && p[..p.len() - 1].chars().all(|c| c.is_ascii_digit()))
            .unwrap_or("");
        return gguf_header(base, arch, label, ftype);
    }
    if lower.ends_with(".iso") {
        let mut b = vec![0u8; 0x8800];
        b[0x8000] = 1;
        b[0x8001..0x8006].copy_from_slice(b"CD001");
        return b;
    }
    if lower.ends_with(".exe") || lower.ends_with(".dll") || lower.ends_with(".pyd") {
        let mut b = vec![0u8; 64];
        b[..2].copy_from_slice(b"MZ");
        return b;
    }
    if lower.ends_with(".zip") || lower.ends_with(".nupkg") {
        let mut b = b"PK\x05\x06".to_vec();
        b.extend_from_slice(&[0u8; 18]);
        return b;
    }
    if lower.ends_with(".vhdx") {
        return b"vhdxfile".to_vec();
    }
    if lower.ends_with(".pdf") {
        return b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    }
    if lower.ends_with(".mp4") {
        let mut b = vec![0, 0, 0, 0x18];
        b.extend_from_slice(b"ftypmp42\0\0\0\0mp42isom");
        return b;
    }
    Vec::new()
}

fn ancestors(fx: &Fixture, abs: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = abs.to_string();
    while let Some(i) = cur.rfind('\\') {
        cur.truncate(i);
        if cur.len() < fx.root.len() {
            break;
        }
        out.push(cur.clone());
    }
    out
}

// Real tiny files at the indexed paths, so inspect reads headers and times instead of reporting "missing".
fn materialize(fx: &Fixture, s: &Spec, now: i64) -> Result<()> {
    let mut dir_mtime: BTreeMap<String, i64> = BTreeMap::new();
    for p in &s.pads {
        let abs = fx.abs(&p.dir);
        std::fs::create_dir_all(&abs).with_context(|| format!("creating {abs}"))?;
        for a in std::iter::once(abs.clone()).chain(ancestors(fx, &abs)) {
            let e = dir_mtime.entry(a).or_insert(now - 400 * DAY);
            *e = (*e).max(now - 30 * DAY);
        }
    }
    for f in &s.files {
        let abs = fx.abs(&f.path);
        if let Some(parent) = Path::new(&abs).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&abs).with_context(|| format!("creating {abs}"))?;
        out.write_all(&content(&f.path))?;
        drop(out);
        let m = now - f.mdays * DAY;
        hoardini_win::set_file_times(Path::new(&abs), Some(now - f.adays * DAY), Some(m))?;
        for a in ancestors(fx, &abs) {
            let e = dir_mtime.entry(a).or_insert(m);
            *e = (*e).max(m);
        }
    }
    let mut dirs: Vec<(&String, &i64)> = dir_mtime.iter().collect();
    dirs.sort_by_key(|(d, _)| std::cmp::Reverse(d.len()));
    for (d, m) in dirs {
        hoardini_win::set_file_times(Path::new(d), Some(*m), Some(*m))?;
    }
    std::fs::create_dir_all(fx.abs(r"NAS"))?;
    Ok(())
}

fn fake_fs(fx: &Fixture, s: &Spec, gen2: bool, now: i64) -> (FakeFs, u64) {
    let fs = FakeFs::new(&fx.root);
    let mut total = 0u64;
    for f in &s.files {
        let size = match (f.when, gen2) {
            (When::New, false) => continue,
            (When::Grew(before), false) => before,
            _ => f.size,
        };
        let abs = fx.abs(&f.path);
        fs.file(&abs, size);
        fs.set_times(&abs, now - f.mdays * DAY, now - f.adays * DAY);
        total += size;
    }
    for p in &s.pads {
        let dir = fx.abs(&p.dir);
        for i in 0..p.count {
            let path = format!(r"{dir}\f{i:05}.bin");
            fs.file(&path, SMALL);
            fs.set_times(&path, now - 30 * DAY, now - 30 * DAY);
        }
        total += p.count * SMALL;
    }
    fs.dir(&fx.abs("NAS"));
    (fs, total)
}

pub struct Built {
    pub fixture: Fixture,
    pub memories_baseline: i64,
}

pub fn build(root: &str, db: &Db) -> Result<Built> {
    let fx = Fixture::new(root);
    let s = spec();
    let now = now_ms();
    materialize(&fx, &s, now)?;
    let conn = db.conn()?;
    let opts = RefreshOptions {
        mode: Mode::Fast,
        hotspots_first: false,
        ..RefreshOptions::default()
    };
    let (fs1, total1) = fake_fs(&fx, &s, false, now);
    let (fs2, total2) = fake_fs(&fx, &s, true, now);
    let growth = total2 - total1;
    let volume = total2 + FREE + gb(3.0);
    fs1.set_volume(volume, FREE + growth);
    refresh(&conn, &fs1, &fx.root, &opts)?;
    let week_ago = now - 8 * DAY;
    conn.execute(
        "UPDATE scan SET started_ms = ?1 - 20000, finished_ms = ?1",
        params![week_ago],
    )?;
    fs2.set_volume(volume, FREE);
    refresh(&conn, &fs2, &fx.root, &opts)?;

    let qwen = [fx.abs(QWEN_LMSTUDIO), fx.abs(QWEN_COPY)];
    conn.execute(
        "INSERT INTO dup_group(size, hash, created_ms) VALUES (?1, ?2, ?3)",
        params![gb(27.1) as i64, format!("blake3:{}", hex64("qwen")), now],
    )?;
    let group = conn.last_insert_rowid();
    for p in &qwen {
        let node = index::node_by_path(&conn, p)?.map(|n| n.id);
        conn.execute(
            "INSERT INTO dup_member(group_id, path, node_id, mtime) VALUES (?1, ?2, ?3, ?4)",
            params![group, p, node, now - 160 * DAY],
        )?;
    }

    let notes = [
        (
            Scope::Path,
            PROTECTED,
            Kind::Protect,
            "Tax records for 2025. Never delete or move them.",
        ),
        (
            Scope::Path,
            r"~\.lmstudio\models\lmstudio-community\Qwen3.8-27B-GGUF",
            Kind::Preference,
            "Keep Qwen3.8 27B. I use it every day.",
        ),
        (
            Scope::Global,
            "models-archive",
            Kind::Preference,
            "Prefer archiving big models to the NAS over deleting them.",
        ),
    ];
    for (scope, key, kind, text) in notes {
        memory::remember_with(
            &conn,
            NewMemory::new(scope, key, kind, text, "user"),
            now - 26 * DAY,
            &fx.profile,
        )?;
    }
    destinations::create(
        &conn,
        NewDestination {
            name: DESTINATION.into(),
            kind: "local".into(),
            config: serde_json::json!({ "path": fx.abs("NAS") }),
        },
    )?;
    let baseline: i64 =
        conn.query_row("SELECT COALESCE(MAX(id), 0) FROM memory", [], |r| r.get(0))?;
    Ok(Built {
        fixture: fx,
        memories_baseline: baseline,
    })
}

// Notes a case wrote (remember) must not leak into the next case's prompt.
pub fn reset_memories(db: &Db, baseline: i64) -> Result<()> {
    let conn = db.conn()?;
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM memory WHERE id > ?1")?
        .query_map([baseline], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in ids {
        memory::delete(&conn, id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gguf_header_names_model_and_quant() {
        let h = content(r"~\x\Qwen3.8-27B-Q8_0.gguf");
        let g = hoardini_core::evidence::identify::identify(
            std::io::Cursor::new(&h),
            h.len() as u64,
            "Qwen3.8-27B-Q8_0.gguf",
        );
        let info = g.gguf.expect("gguf parsed");
        assert_eq!(info.name.as_deref(), Some("Qwen3.8-27B"));
        assert_eq!(info.quant.as_deref(), Some("Q8_0"));
        assert_eq!(info.size_label.as_deref(), Some("27B"));
    }

    #[test]
    fn fixture_paths_resolve_profile_and_root() {
        let fx = Fixture::new(r"X:\");
        assert_eq!(fx.abs(r"~\Downloads"), r"X:\Users\ana\Downloads");
        assert_eq!(fx.abs(r"code\based"), r"X:\code\based");
        assert_eq!(fx.profile, r"X:\Users\ana");
    }

    #[test]
    fn hashes_are_stable_hex() {
        let a = hex64("whisper");
        assert_eq!(a.len(), 64);
        assert_eq!(a, hex64("whisper"));
        assert_ne!(a, hex64("bge"));
    }
}

use std::io::Cursor;

use hoardini_core::catalog::{Catalog, Category, Env, UnitKind};
use hoardini_core::evidence::{Family, identify};
use hoardini_core::policy::Verdict;

fn env() -> Env {
    Env::new(r"C:\Users\tester")
        .set("LOCALAPPDATA", r"C:\Users\tester\AppData\Local")
        .set("APPDATA", r"C:\Users\tester\AppData\Roaming")
        .set("TEMP", r"C:\Users\tester\AppData\Local\Temp")
        .set("SystemRoot", r"C:\Windows")
        .set("SystemDrive", "C:")
        .set("ProgramData", r"C:\ProgramData")
        .set("ProgramFiles", r"C:\Program Files")
        .set("ProgramFiles(x86)", r"C:\Program Files (x86)")
}

// Traces: EVID-CATALOG
#[test]
fn catalog_match_supplies_app_category_and_notes() {
    let cat = Catalog::with_env(&env()).unwrap();
    let m = cat
        .lookup_match(r"C:\Users\tester\.lmstudio\models\TheBloke\x\x.Q4_K_M.gguf")
        .unwrap();
    assert_eq!(m.app, "LM Studio");
    assert_eq!(m.category, Category::ModelStore);
    assert!(m.notes.contains("whole model folder"));
    assert_eq!(m.verdict_hint, Verdict::Redownload);
}

// Traces: EVID-CATALOG
#[test]
fn catalog_picks_the_most_specific_entry_and_carries_caps() {
    let cat = Catalog::with_env(&env()).unwrap();
    let blob = cat
        .lookup_match(r"C:\Users\tester\.cache\huggingface\hub\models--a--b\blobs\0123")
        .unwrap();
    assert_eq!(blob.id, "hf-hub-blobs");
    assert!(blob.shared_blobs);
    assert_eq!(blob.cap, Some(Verdict::Review));
    assert_eq!(
        blob.safe_unit.as_deref(),
        Some("a whole models--org--name folder")
    );

    let hib = cat.lookup_match(r"C:\hiberfil.sys").unwrap();
    assert_eq!(
        (hib.category, hib.cap),
        (Category::System, Some(Verdict::Keep))
    );

    let pagefile = cat.lookup_match(r"C:\pagefile.sys").unwrap();
    assert_eq!(pagefile.cap, Some(Verdict::Keep));

    let nm = cat
        .lookup_match(r"C:\code\app\node_modules\react\index.js")
        .unwrap();
    assert_eq!((nm.id.as_str(), nm.regenerable), ("node-modules", true));

    assert!(cat.lookup_match(r"D:\photos\2024\a.jpg").is_none());
}

// Traces: EVID-CATALOG
#[test]
fn catalog_seed_covers_the_machine_locations() {
    let cat = Catalog::with_env(&env()).unwrap();
    for (path, id) in [
        (
            r"C:\Users\tester\.ollama\models\blobs\sha256-ab",
            "ollama-blobs",
        ),
        (r"C:\Users\tester\.triton\cache\x", "triton-cache"),
        (r"C:\Users\tester\.miopen\db\x", "miopen-cache"),
        (
            r"C:\Users\tester\.nuget\packages\newtonsoft.json",
            "nuget-global-packages",
        ),
        (r"C:\Users\tester\.cargo\registry\cache", "cargo-registry"),
        (r"C:\Users\tester\.venvs\torch", "venvs-dir"),
        (
            r"C:\code\llama.cpp\build\bin\llama-server.exe",
            "llamacpp-build",
        ),
        (r"C:\code\proj\target\debug\x.exe", "rust-target"),
        (r"C:\$WINDOWS.~BT\Sources", "windows-setup-bt"),
        (r"C:\Windows\SoftwareDistribution\Download\a", "wu-download"),
        (r"C:\Windows\Installer\x.msi", "windows-installer"),
        (r"C:\ESD\install.esd", "windows-esd"),
        (
            r"C:\Users\tester\AppData\Local\Docker\wsl\disk\docker_data.vhdx",
            "docker-vhdx",
        ),
        (
            r"D:\SteamLibrary\steamapps\common\Game\g.exe",
            "steam-common",
        ),
        (
            r"C:\Users\tester\Downloads\setup.exe",
            "downloads-installers",
        ),
        (r"C:\models\qwen.gguf", "models-root"),
        (
            r"C:\Users\tester\unsloth_compiled_cache",
            "unsloth-compiled-cache",
        ),
    ] {
        assert_eq!(cat.lookup(path).map(|e| e.id.as_str()), Some(id), "{path}");
    }
}

// Traces: EVID-CATALOG
#[test]
fn catalog_covers_the_review_first_gaps() {
    let cat = Catalog::with_env(&env()).unwrap();
    for (path, id) in [
        (
            r"C:\Users\tester\.flm\models\Qwen3.8-27B-NPU2",
            "flm-models",
        ),
        (r"C:\Users\tester\.flm\logs", "flm-home"),
        (r"C:\Users\tester\.hipfire\cache", "hipfire-home"),
        (r"C:\NVIDIA\DisplayDriver\581.15", "gpu-driver-unpack"),
        (r"C:\AMD\Chipset_Software", "gpu-driver-unpack"),
        (r"C:\Intel\GFX", "intel-driver-unpack"),
        (
            r"C:\Users\tester\AppData\Roaming\Apple Computer\MobileSync\Backup\00008110-001A",
            "iphone-backup",
        ),
        (
            r"C:\Users\tester\Apple\MobileSync\Backup\x",
            "iphone-backup",
        ),
        (
            r"C:\Users\tester\AppData\Roaming\Adobe\Common\Media Cache Files\a.cfa",
            "adobe-media-cache",
        ),
        (
            r"C:\Users\tester\AppData\Local\Spotify\Storage\ab",
            "spotify-cache",
        ),
        (
            r"C:\Users\tester\AppData\Roaming\discord\Cache\Cache_Data",
            "discord-cache",
        ),
        (
            r"C:\Users\tester\AppData\Roaming\discord\Code Cache",
            "discord-cache",
        ),
        (
            r"C:\Users\tester\AppData\Local\Temp\WinGet\Git.Git.2.47",
            "winget-cache",
        ),
        (
            r"C:\Users\tester\.lmstudio\server-logs\2026-10\2026-10-09.1.log",
            "lmstudio-server-logs",
        ),
    ] {
        assert_eq!(cat.lookup(path).map(|e| e.id.as_str()), Some(id), "{path}");
    }
    let backup = cat.get("iphone-backup").unwrap();
    assert_eq!(
        (backup.verdict(), backup.unit),
        (Verdict::Review, UnitKind::Children)
    );
    let logs = cat
        .lookup_exact(r"C:\Users\tester\.lmstudio\server-logs")
        .unwrap();
    assert_eq!(
        (
            logs.id.as_str(),
            logs.category,
            logs.verdict(),
            logs.regenerable,
            logs.unit
        ),
        (
            "lmstudio-server-logs",
            Category::Logs,
            Verdict::Safe,
            true,
            UnitKind::Contents
        )
    );
}

// Traces: EVID-CATALOG
#[test]
fn catalog_entries_say_what_one_deletable_unit_is() {
    let cat = Catalog::with_env(&env()).unwrap();
    let unit = |id: &str| {
        let e = cat.get(id).unwrap();
        (e.unit, e.unit_depth, e.min_age_hours)
    };
    assert_eq!(unit("lmstudio-models"), (UnitKind::Children, Some(2), None));
    for id in [
        "hf-hub-cache",
        "steam-common",
        "downloads",
        "flm-models",
        "models-root",
    ] {
        assert_eq!(unit(id), (UnitKind::Children, None, None), "{id}");
    }
    assert_eq!(unit("temp-user"), (UnitKind::Contents, None, Some(24)));
    assert_eq!(unit("temp-system"), (UnitKind::Contents, None, Some(24)));
    assert_eq!(unit("crash-dumps"), (UnitKind::Contents, None, None));
    assert_eq!(unit("wu-download"), (UnitKind::Contents, None, None));
    for id in [
        "appdata-local",
        "lmstudio-home",
        "windows-dir",
        "user-documents",
    ] {
        assert_eq!(unit(id).0, UnitKind::Container, "{id}");
    }
    assert_eq!(unit("node-modules"), (UnitKind::Whole, None, None));
    for e in cat.entries() {
        assert!(
            e.unit_depth.is_none() || e.unit == UnitKind::Children,
            "{}",
            e.id
        );
        assert!(
            e.min_age_hours.is_none() || e.unit == UnitKind::Contents,
            "{}",
            e.id
        );
    }
}

// Traces: EVID-CATALOG
#[test]
fn exact_lookup_and_literal_paths_inside_a_folder() {
    let cat = Catalog::with_env(&env()).unwrap();
    let temp = r"C:\Users\tester\AppData\Local\Temp";
    assert_eq!(cat.lookup_exact(temp).unwrap().id, "temp-user");
    let inside = format!(r"{temp}\setup-123");
    assert!(cat.lookup_exact(&inside).is_none());
    assert_eq!(cat.lookup(&inside).unwrap().id, "temp-user");
    assert_eq!(
        cat.lookup_exact(r"C:\code\proj\target").unwrap().id,
        "rust-target"
    );

    let lits = cat.literal_paths_under(r"C:\Windows");
    let ids: Vec<(&str, &str)> = lits
        .iter()
        .map(|(p, e)| (p.as_str(), e.id.as_str()))
        .collect();
    assert!(
        ids.contains(&(r"C:\Windows\Temp", "temp-system")),
        "{ids:?}"
    );
    assert!(
        ids.contains(&(r"C:\Windows\SoftwareDistribution\Download", "wu-download")),
        "{ids:?}"
    );
    assert!(
        lits.iter()
            .all(|(p, _)| !p.contains('*') && p.to_lowercase().starts_with(r"c:\windows\"))
    );
    assert!(!ids.iter().any(|(p, _)| *p == r"C:\Windows"));
}

struct Gguf {
    bytes: Vec<u8>,
}

impl Gguf {
    fn new(tensors: u64, kvs: u64) -> Gguf {
        let mut b = b"GGUF".to_vec();
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&tensors.to_le_bytes());
        b.extend_from_slice(&kvs.to_le_bytes());
        Gguf { bytes: b }
    }

    fn s(&mut self, s: &str) {
        self.bytes
            .extend_from_slice(&(s.len() as u64).to_le_bytes());
        self.bytes.extend_from_slice(s.as_bytes());
    }

    fn kv_str(&mut self, k: &str, v: &str) {
        self.s(k);
        self.bytes.extend_from_slice(&8u32.to_le_bytes());
        self.s(v);
    }

    fn kv_u32(&mut self, k: &str, v: u32) {
        self.s(k);
        self.bytes.extend_from_slice(&4u32.to_le_bytes());
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    fn kv_str_array(&mut self, k: &str, items: &[&str]) {
        self.s(k);
        self.bytes.extend_from_slice(&9u32.to_le_bytes());
        self.bytes.extend_from_slice(&8u32.to_le_bytes());
        self.bytes
            .extend_from_slice(&(items.len() as u64).to_le_bytes());
        for i in items {
            self.s(i);
        }
    }

    fn tensor(&mut self, name: &str, dims: &[u64], ty: u32) {
        self.s(name);
        self.bytes
            .extend_from_slice(&(dims.len() as u32).to_le_bytes());
        for d in dims {
            self.bytes.extend_from_slice(&d.to_le_bytes());
        }
        self.bytes.extend_from_slice(&ty.to_le_bytes());
        self.bytes.extend_from_slice(&0u64.to_le_bytes());
    }
}

// Traces: EVID-GGUF
#[test]
fn gguf_header_yields_name_architecture_quant_and_params() {
    let mut g = Gguf::new(2, 6);
    g.kv_str("general.architecture", "llama");
    g.kv_str("general.name", "Tiny Test 7B");
    g.kv_u32("general.file_type", 15);
    g.kv_str("general.size_label", "7B");
    g.kv_u32("llama.context_length", 32768);
    g.kv_str_array("tokenizer.ggml.tokens", &["<s>", "</s>", "hello"]);
    g.tensor("token_embd.weight", &[4096, 1000], 12);
    g.tensor("blk.0.attn_q.weight", &[4096, 4096], 14);
    let len = g.bytes.len() as u64 + 4096;

    let id = identify(Cursor::new(g.bytes), len, "tiny.Q4_K_M.gguf");
    let info = id.gguf.expect("gguf parsed");
    assert_eq!(id.kind, "gguf");
    assert_eq!(id.family, Family::Model);
    assert!(id.already_compressed);
    assert_eq!(info.name.as_deref(), Some("Tiny Test 7B"));
    assert_eq!(info.architecture.as_deref(), Some("llama"));
    assert_eq!(info.quant.as_deref(), Some("Q4_K_M"));
    assert_eq!(info.size_label.as_deref(), Some("7B"));
    assert_eq!(info.context_length, Some(32768));
    assert_eq!(info.parameters, Some(4096 * 1000 + 4096 * 4096));
    assert!(!info.truncated);
}

// Traces: EVID-GGUF
#[test]
fn gguf_without_file_type_falls_back_to_dominant_tensor_type() {
    let mut g = Gguf::new(2, 1);
    g.kv_str("general.architecture", "qwen2");
    g.tensor("a", &[1000, 1000], 1);
    g.tensor("b", &[10], 0);
    let len = g.bytes.len() as u64;

    let id = identify(Cursor::new(g.bytes), len, "m.gguf");
    let info = id.gguf.unwrap();
    assert_eq!(info.quant.as_deref(), Some("F16"));
    assert!(!id.already_compressed);
    assert_eq!(info.name, None);
}

// Traces: EVID-GGUF
#[test]
fn gguf_reader_stays_within_the_header() {
    struct Counting<R> {
        inner: R,
        max_pos: u64,
        pos: u64,
    }
    impl<R: std::io::Read> std::io::Read for Counting<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.pos += n as u64;
            self.max_pos = self.max_pos.max(self.pos);
            Ok(n)
        }
    }
    impl<R: std::io::Seek> std::io::Seek for Counting<R> {
        fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
            self.pos = self.inner.seek(p)?;
            Ok(self.pos)
        }
    }

    let mut g = Gguf::new(1, 1);
    g.kv_str("general.architecture", "llama");
    g.tensor("w", &[8, 8], 0);
    let header_len = g.bytes.len();
    let mut file = g.bytes;
    file.extend(std::iter::repeat_n(0xAB, 5_000_000));
    let len = file.len() as u64;
    let mut r = Counting {
        inner: Cursor::new(file),
        max_pos: 0,
        pos: 0,
    };
    let id = identify(&mut r, len, "w.gguf");
    assert_eq!(id.gguf.unwrap().parameters, Some(64));
    assert!(
        r.max_pos <= (header_len as u64).max(65536),
        "read {} bytes of a {header_len}-byte header",
        r.max_pos
    );
}

// Traces: EVID-GGUF-SOURCE
#[test]
fn gguf_header_names_its_hugging_face_sources() {
    let mut g = Gguf::new(1, 8);
    g.kv_str("general.architecture", "llama");
    g.kv_str("general.basename", "SmolLM2");
    g.kv_str(
        "general.source.huggingface.repository",
        "HuggingFaceTB/SmolLM2-135M-Instruct",
    );
    g.kv_str(
        "general.source.url",
        "https://huggingface.co/HuggingFaceTB/SmolLM2-135M-Instruct",
    );
    g.kv_str(
        "general.url",
        "https://huggingface.co/lmstudio-community/SmolLM2-135M-Instruct-GGUF",
    );
    g.kv_str(
        "general.base_model.0.repo_url",
        "https://huggingface.co/datasets/org/corpus",
    );
    g.kv_str(
        "general.base_model.1.repo_url",
        "https://huggingface.co/HuggingFaceTB/SmolLM2-135M/tree/main",
    );
    g.kv_u32("general.file_type", 15);
    g.tensor("w", &[8, 8], 12);
    let len = g.bytes.len() as u64;
    let info = identify(Cursor::new(g.bytes), len, "smol.gguf")
        .gguf
        .unwrap();
    assert_eq!(info.basename.as_deref(), Some("SmolLM2"));
    assert_eq!(
        info.source_repo.as_deref(),
        Some("HuggingFaceTB/SmolLM2-135M-Instruct")
    );
    assert_eq!(
        info.source_url.as_deref(),
        Some("https://huggingface.co/HuggingFaceTB/SmolLM2-135M-Instruct")
    );
    assert_eq!(
        info.url.as_deref(),
        Some("https://huggingface.co/lmstudio-community/SmolLM2-135M-Instruct-GGUF")
    );
    assert_eq!(
        info.base_model_urls,
        [
            "https://huggingface.co/datasets/org/corpus",
            "https://huggingface.co/HuggingFaceTB/SmolLM2-135M/tree/main"
        ]
    );
    assert_eq!(info.architecture.as_deref(), Some("llama"));
    assert_eq!(
        info.hf_repos(),
        [
            "HuggingFaceTB/SmolLM2-135M-Instruct",
            "lmstudio-community/SmolLM2-135M-Instruct-GGUF",
            "HuggingFaceTB/SmolLM2-135M"
        ]
    );
    assert_eq!(info.quant.as_deref(), Some("Q4_K_M"));

    let mut plain = Gguf::new(1, 1);
    plain.kv_str("general.architecture", "llama");
    plain.tensor("w", &[8, 8], 0);
    let len = plain.bytes.len() as u64;
    let info = identify(Cursor::new(plain.bytes), len, "p.gguf")
        .gguf
        .unwrap();
    assert!(info.hf_repos().is_empty());
    assert_eq!(
        (info.basename, info.source_repo, info.source_url, info.url),
        (None, None, None, None)
    );
    assert!(info.base_model_urls.is_empty());
    assert_eq!(info.architecture.as_deref(), Some("llama"));
    assert_eq!(info.parameters, Some(64));
}

#[path = "inspect_mod.rs"]
mod inspect_tests;

#[path = "copy.rs"]
mod copy_tests;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use hoardini_core::catalog::{Catalog, Env};
use hoardini_core::cloud::{Cloud, CloudState, SyncRoot};
use hoardini_core::copycheck::{
    self, CheckEnv, Confidence, CopyCheck, CopyResult, Cx, Effect, Find, Found, HubCheck,
    OnlineConsent, Registry, Service, Services, Subject, WingetCli, hash, winget,
};
use hoardini_core::index::{self, RefreshOptions, WinLister, normalize_path};
use hoardini_core::memory::{self, Kind, NewMemory, Scope};
use hoardini_core::policy::Verdict;
use hoardini_core::store::Db;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const MIB: u64 = 1 << 20;
const SMOL: &str = "SmolLM2-135M-Instruct-Q4_K_M.gguf";
const SMOL_SIZE: u64 = 105_454_144;
const SMOL_REPO: &str = "lmstudio-community/SmolLM2-135M-Instruct-GGUF";
const SMOL_OID: &str = "bda484992f9655d22504b14e57985257fa6a86937c61f957cf99c10a3bcae169";
const ETCHER_SHA: &str = "386bb45da6e0edeb20e065427cdef30a876e4d2a59579f8add329d8510ac12f8";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/evid/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn s(p: &Path) -> String {
    normalize_path(&p.to_string_lossy())
}

// Sparse, so a 120 MiB test file costs no disk space and reads as zeros.
fn sparse(path: &Path, len: u64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::File::create(path).unwrap();
    hoardini_win::set_sparse_and_zero_range(path, 0, 0).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(len)
        .unwrap();
}

fn sha_of(path: &Path) -> String {
    let mut f = std::fs::File::open(path).unwrap();
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn set_mtime(p: &Path, ms: i64) {
    hoardini_win::set_file_times(p, None, Some(ms)).unwrap();
}

struct Fx {
    _dir: tempfile::TempDir,
    root: PathBuf,
    db: Db,
}

impl Fx {
    fn new() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let root = PathBuf::from(
            s(&std::fs::canonicalize(dir.path()).unwrap()).trim_start_matches(r"\\?\"),
        );
        let db = Db::open(root.join("h.db")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        Fx {
            _dir: dir,
            root,
            db,
        }
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn profile(&self) -> PathBuf {
        self.root.join("profile")
    }

    fn index(&self) {
        let out = index::refresh(
            &self.db.conn().unwrap(),
            &WinLister,
            &s(&self.data()),
            &RefreshOptions {
                workers: 2,
                ..RefreshOptions::default()
            },
        )
        .unwrap();
        assert!(out.completed);
    }

    fn env(&self) -> CheckEnv {
        let p = s(&self.profile());
        let cat = Env::new(p.clone())
            .set("LOCALAPPDATA", format!(r"{p}\AppData\Local"))
            .set("APPDATA", format!(r"{p}\AppData\Roaming"))
            .set("TEMP", format!(r"{p}\AppData\Local\Temp"))
            .set("SystemRoot", r"C:\Windows")
            .set("SystemDrive", "C:")
            .set("ProgramData", r"C:\ProgramData")
            .set("ProgramFiles", r"C:\Program Files");
        CheckEnv {
            profile: p.clone(),
            cloud: Arc::new(FakeCloud::default()),
            hub_base: "http://127.0.0.1:9".into(),
            winget: Arc::new(FakeWinget::default()),
            git: "git".into(),
            consent: OnlineConsent::default(),
            catalog: Some(Arc::new(Catalog::with_env(&cat).unwrap())),
            ollama_models: Some(format!(r"{p}\.ollama\models")),
        }
    }
}

fn check(fx: &Fx, env: &CheckEnv, reg: &Registry, path: &Path) -> copycheck::Outcome {
    let conn = fx.db.conn().unwrap();
    let cx = Cx { conn: &conn, env };
    copycheck::check_path(&cx, reg, &s(path))
}

fn answer(fx: &Fx, env: &CheckEnv, name: &str, path: &Path) -> Option<CopyResult> {
    check(fx, env, &Registry::standard(), path)
        .results
        .into_iter()
        .find(|r| r.check == name)
}

fn note(r: &CopyResult) -> String {
    r.note()
}

#[derive(Default)]
struct FakeCloud {
    roots: Vec<SyncRoot>,
    states: Mutex<HashMap<String, CloudState>>,
}

impl FakeCloud {
    fn with_root(root: &Path, provider: &str) -> FakeCloud {
        FakeCloud {
            roots: vec![SyncRoot {
                path: s(root),
                provider: provider.into(),
            }],
            states: Mutex::default(),
        }
    }

    fn set(&self, p: &Path, in_sync: bool) {
        self.states.lock().unwrap().insert(
            s(p).to_lowercase(),
            CloudState {
                in_sync,
                pinned: false,
                local_bytes: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
            },
        );
    }
}

impl Cloud for FakeCloud {
    fn roots(&self) -> Vec<SyncRoot> {
        self.roots.clone()
    }
    fn state(&self, path: &Path) -> std::io::Result<Option<CloudState>> {
        Ok(self
            .states
            .lock()
            .unwrap()
            .get(&s(path).to_lowercase())
            .copied())
    }
    fn free(&self, _path: &Path) -> std::io::Result<u64> {
        Err(std::io::Error::other("not used here"))
    }
}

#[derive(Default)]
struct FakeWinget {
    calls: Mutex<Vec<String>>,
}

impl WingetCli for FakeWinget {
    fn run(&self, args: &[&str]) -> std::io::Result<String> {
        let line = args.join(" ");
        self.calls.lock().unwrap().push(line.clone());
        Ok(if line == "search --name balenaEtcher --source winget" {
            fixture("winget-search-etcher.txt")
        } else if line.starts_with("show --id Balena.Etcher --exact --source winget") {
            fixture("winget-show-etcher.txt")
        } else {
            fixture("winget-search-none.txt")
        })
    }
}

// A stand-in for huggingface.co that answers from recorded responses and logs every request line.
struct StandIn {
    base: String,
    log: Arc<Mutex<Vec<String>>>,
}

impl StandIn {
    fn start(routes: Vec<(String, String)>) -> StandIn {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let l2 = log.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { continue };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    let Ok(n) = conn.read(&mut chunk) else { break };
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let head = String::from_utf8_lossy(&buf).to_string();
                let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
                l2.lock().unwrap().push(target.clone());
                let (status, body) = match routes.iter().find(|(p, _)| *p == target) {
                    Some((_, b)) => ("200 OK", b.clone()),
                    None if target.starts_with("/api/models?") => ("200 OK", "[]".to_string()),
                    None => (
                        "404 Not Found",
                        r#"{"error":"Repository not found"}"#.to_string(),
                    ),
                };
                let _ = conn.write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
            }
        });
        StandIn { base, log }
    }

    fn requests(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

fn smol_routes() -> Vec<(String, String)> {
    vec![
        (
            format!("/api/models/{SMOL_REPO}/tree/main?recursive=1"),
            fixture("hf-tree-smollm2-gguf.json"),
        ),
        (
            "/api/models/bartowski/SmolLM2-135M-Instruct-GGUF/tree/main?recursive=1".into(),
            fixture("hf-tree-bartowski-smollm2.json"),
        ),
        (
            "/api/models?search=SmolLM2-135M-Instruct&limit=10&filter=gguf".into(),
            fixture("hf-search-smollm2-gguf.json"),
        ),
    ]
}

fn hf_on() -> OnlineConsent {
    OnlineConsent {
        asked: true,
        services: Services {
            huggingface: true,
            winget: true,
        },
    }
}

fn store_hash(fx: &Fx, path: &Path, sha: &str) {
    let conn = fx.db.conn().unwrap();
    let sub = Subject::load(&conn, &s(path)).unwrap();
    hash::store(&conn, &sub, sha).unwrap();
}

struct Counting(Arc<AtomicUsize>);

impl CopyCheck for Counting {
    fn name(&self) -> &'static str {
        "counting"
    }
    fn applies(&self, _cx: &Cx<'_>, s: &Subject) -> bool {
        !s.dir
    }
    fn find(&self, _cx: &Cx<'_>, _s: &Subject) -> anyhow::Result<Find> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Find::NoCopy(json!({"note": "nothing"})))
    }
    fn verify(&self, _cx: &Cx<'_>, _s: &Subject, _f: Found) -> anyhow::Result<CopyResult> {
        unreachable!()
    }
}

// Traces: EVID-COPY-CHECK
#[test]
fn confidence_decides_the_effect_and_the_verdict() {
    assert_eq!(
        copycheck::effect(Confidence::Identical),
        Effect::MayStartTicked
    );
    assert_eq!(
        copycheck::effect(Confidence::SameNameAndSize),
        Effect::Review
    );
    assert_eq!(
        copycheck::effect(Confidence::Claimed),
        Effect::FollowCatalog
    );
    assert_eq!(copycheck::effect(Confidence::None), Effect::OnlyCopy);
    let r = |check: &str, c| CopyResult::new(check, Some("x".into()), c, json!({}));
    assert_eq!(
        copycheck::verdict_for(&r("sync", Confidence::Identical)),
        Some(Verdict::Synced)
    );
    assert_eq!(
        copycheck::verdict_for(&r("huggingface", Confidence::Identical)),
        Some(Verdict::Redownload)
    );
    assert_eq!(
        copycheck::verdict_for(&r("huggingface", Confidence::SameNameAndSize)),
        Some(Verdict::Review)
    );
    assert_eq!(
        copycheck::verdict_for(&r("catalog", Confidence::Claimed)),
        None
    );
    assert_eq!(copycheck::verdict_for(&r("dup", Confidence::None)), None);
}

// Traces: EVID-COPY-CHECK
#[test]
fn answers_are_reused_until_the_file_changes() {
    let fx = Fx::new();
    let f = fx.data().join("a.bin");
    std::fs::write(&f, b"abc").unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let reg = Registry::with(vec![Box::new(Counting(runs.clone()))]);
    let env = fx.env();
    check(&fx, &env, &reg, &f);
    check(&fx, &env, &reg, &f);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    let conn = fx.db.conn().unwrap();
    let stored = copycheck::results_for(&conn, &s(&f));
    assert_eq!(stored.len(), 1);
    assert_eq!(
        (stored[0].check.as_str(), stored[0].confidence),
        ("counting", Confidence::None)
    );

    set_mtime(&f, 1_600_000_000_000);
    assert!(
        copycheck::results_for(&conn, &s(&f)).is_empty(),
        "stale once the mtime moved"
    );
    check(&fx, &env, &reg, &f);
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

// Traces: EVID-COPY-CHECK
#[test]
fn only_copy_needs_every_answer_empty_and_settled() {
    let fx = Fx::new();
    let f = fx.data().join("own.bin");
    std::fs::write(&f, b"mine").unwrap();
    let conn = fx.db.conn().unwrap();
    let path = s(&f);
    assert!(!copycheck::only_copy(&conn, &path), "nothing answered yet");
    let sub = Subject::load(&conn, &path).unwrap();
    let none = CopyResult::new("dup", None, Confidence::None, json!({"note": "no twin"}));
    copycheck::store(&conn, &sub, &none).unwrap();
    assert!(copycheck::only_copy(&conn, &path));
    assert_eq!(
        copycheck::note_for(&conn, &path).as_deref(),
        Some("only copy we know of")
    );

    let waiting = CopyResult::new("dup", None, Confidence::None, json!({"pending": "hash"}));
    copycheck::store(&conn, &sub, &waiting).unwrap();
    assert!(
        !copycheck::only_copy(&conn, &path),
        "a pending answer is not settled"
    );

    copycheck::store(&conn, &sub, &none).unwrap();
    let found = CopyResult::new(
        "archive",
        Some(r"D:\arch\own.bin".into()),
        Confidence::Identical,
        json!({"note": "archived copy at D:\\arch\\own.bin"}),
    );
    copycheck::store(&conn, &sub, &found).unwrap();
    assert!(!copycheck::only_copy(&conn, &path));
    assert_eq!(
        copycheck::note_for(&conn, &path).as_deref(),
        Some(r"archived copy at D:\arch\own.bin")
    );
}

// Traces: EVID-COPY-CHECK, EVID-COPY-DUP
#[test]
fn candidates_are_listed_biggest_first_and_hashed_in_the_background() {
    let fx = Fx::new();
    let d = fx.data();
    for (name, size) in [
        ("a120.bin", 120 * MIB),
        ("b120.bin", 120 * MIB),
        ("a101.bin", 101 * MIB),
        ("b101.bin", 101 * MIB),
        ("small.bin", 99 * MIB),
    ] {
        sparse(&d.join(name), size);
    }
    for n in 0..25 {
        sparse(&d.join("proj").join(format!("p{n}.bin")), 45 * MIB);
        sparse(&d.join("plain").join(format!("p{n}.bin")), 45 * MIB);
    }
    std::fs::create_dir_all(d.join("proj").join(".git")).unwrap();
    fx.index();

    let env = fx.env();
    let reg = Registry::standard();
    let conn = fx.db.conn().unwrap();
    let cx = Cx {
        conn: &conn,
        env: &env,
    };
    let list = copycheck::candidates(&cx, &reg, &s(&d)).unwrap();
    let names: Vec<String> = list
        .iter()
        .map(|c| c.path.rsplit('\\').next().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        ["proj", "a120.bin", "b120.bin", "a101.bin", "b101.bin"]
    );
    assert!(list[0].dir && !list[1].dir);

    let report = copycheck::run_volume(&fx.db, &env, &reg, &s(&d), &|| true).unwrap();
    assert_eq!(report.hashed, 4);
    let order: Vec<i64> = conn
        .prepare("SELECT size FROM file_hash ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        order,
        [
            120 * MIB as i64,
            120 * MIB as i64,
            101 * MIB as i64,
            101 * MIB as i64
        ]
    );
    for (me, twin) in [("a120.bin", "b120.bin"), ("b101.bin", "a101.bin")] {
        let sub = Subject::load(&conn, &s(&d.join(me))).unwrap();
        assert_eq!(hash::current(&conn, &sub).unwrap(), sha_of(&d.join(me)));
        let dup = copycheck::results_for(&conn, &s(&d.join(me)))
            .into_iter()
            .find(|r| r.check == "dup")
            .unwrap();
        assert_eq!(
            dup.confidence,
            Confidence::Identical,
            "{me}: {}",
            note(&dup)
        );
        assert_eq!(dup.locator.as_deref(), Some(s(&d.join(twin)).as_str()));
        assert_eq!(dup.detail["via"], "sha256");
    }

    let again = copycheck::run_volume(&fx.db, &env, &reg, &s(&d), &|| true).unwrap();
    assert_eq!(
        again.hashed, 0,
        "hashes are reused while size and mtime hold"
    );

    let changed = d.join("a120.bin");
    let mut w = std::fs::OpenOptions::new()
        .write(true)
        .open(&changed)
        .unwrap();
    w.write_all(b"x").unwrap();
    drop(w);
    let third = copycheck::run_volume(&fx.db, &env, &reg, &s(&d), &|| true).unwrap();
    assert_eq!(third.hashed, 1);
    let sub = Subject::load(&conn, &s(&changed)).unwrap();
    assert_eq!(hash::current(&conn, &sub).unwrap(), sha_of(&changed));
    let dup = copycheck::results_for(&conn, &s(&d.join("b120.bin")))
        .into_iter()
        .find(|r| r.check == "dup")
        .unwrap();
    assert_eq!(
        dup.confidence,
        Confidence::None,
        "the twin changed: {}",
        note(&dup)
    );
}

// Traces: EVID-COPY-CHECK, EVID-COPY-BACKGROUND
#[test]
fn a_pass_stops_mid_hash_when_told_and_resumes_later() {
    let fx = Fx::new();
    let d = fx.data();
    sparse(&d.join("x.bin"), 101 * MIB);
    sparse(&d.join("y.bin"), 101 * MIB);
    fx.index();
    let env = fx.env();
    let reg = Registry::standard();
    let polls = AtomicUsize::new(0);
    // Two polls go to the candidate loop, so the stop lands inside the first file's hash.
    let go = || polls.fetch_add(1, Ordering::SeqCst) < 6;
    let r = copycheck::run_volume(&fx.db, &env, &reg, &s(&d), &go).unwrap();
    assert!(r.stopped && r.hashed == 0, "{r:?}");
    let conn = fx.db.conn().unwrap();
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM file_hash", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0, "a stopped hash leaves no row");
    let r = copycheck::run_volume(&fx.db, &env, &reg, &s(&d), &|| true).unwrap();
    assert_eq!((r.stopped, r.hashed), (false, 2));
}

// Traces: EVID-COPY-CHECK
#[test]
fn content_addressed_blobs_are_never_hashed() {
    let fx = Fx::new();
    let name = "ab".repeat(32);
    let blob = fx.data().join(r"hub\models--o--m\blobs").join(&name);
    std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
    std::fs::write(&blob, b"these bytes do not hash to the name").unwrap();
    let conn = fx.db.conn().unwrap();
    let sub = Subject::load(&conn, &s(&blob)).unwrap();
    assert_eq!(
        hash::hash_subject(&conn, &sub, &|| true).unwrap(),
        Some(name.clone())
    );
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM file_hash", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

// Traces: EVID-COPY-CHECK
#[test]
fn a_copy_hint_only_says_where_to_look() {
    let fx = Fx::new();
    let f = fx.data().join("x.q4nx");
    std::fs::write(&f, b"npu weights").unwrap();
    let mut env = fx.env();
    env.consent = hf_on();
    let hub = StandIn::start(vec![]);
    env.hub_base = hub.base.clone();

    let reg = Registry::with(vec![Box::new(HubCheck)]);
    assert!(
        check(&fx, &env, &reg, &f).results.is_empty(),
        "no hint, no Hugging Face check"
    );

    let conn = fx.db.conn().unwrap();
    let m = memory::remember_with(
        &conn,
        NewMemory::new(
            Scope::Glob,
            r"**\*.q4nx",
            Kind::Fact,
            "q4nx: FastFlowLM NPU weights",
            "web:x",
        ),
        1,
        &env.profile,
    )
    .unwrap();
    conn.execute(
        "UPDATE memory SET detail_json = ?1 WHERE id = ?2",
        rusqlite::params![json!({"copy_hint": "huggingface:acme"}).to_string(), m.id],
    )
    .unwrap();
    set_mtime(&f, 1_650_000_000_000);
    let out = check(&fx, &env, &reg, &f);
    let r = out.results.first().expect("the hint makes the check apply");
    assert_eq!(r.confidence, Confidence::None);
    assert_eq!(
        hub.requests(),
        ["/api/models?author=acme&limit=10"],
        "the hint names the org; `x` alone is too short to search for"
    );
}

fn smol_in_lmstudio(fx: &Fx) -> PathBuf {
    let f = fx
        .profile()
        .join(r".lmstudio\models\lmstudio-community\SmolLM2-135M-Instruct-GGUF")
        .join(SMOL);
    sparse(&f, SMOL_SIZE);
    f
}

// Traces: EVID-COPY-CONSENT
#[test]
fn online_checks_wait_for_consent_and_never_send_local_hashes() {
    let fx = Fx::new();
    let f = smol_in_lmstudio(&fx);
    let hub = StandIn::start(smol_routes());
    let mut env = fx.env();
    env.hub_base = hub.base.clone();

    let out = check(&fx, &env, &Registry::standard(), &f);
    assert!(hub.requests().is_empty(), "{:?}", hub.requests());
    assert_eq!(out.waiting, vec![Service::Huggingface]);
    assert!(
        out.results.iter().any(|r| r.check == "dup"),
        "offline checks still run"
    );
    let conn = fx.db.conn().unwrap();
    assert!(
        !copycheck::only_copy(&conn, &s(&f)),
        "waiting on consent is not 'only copy'"
    );

    env.consent = OnlineConsent {
        asked: true,
        services: Services::default(),
    };
    let out = check(&fx, &env, &Registry::standard(), &f);
    assert!(hub.requests().is_empty() && out.waiting.is_empty());

    env.consent = hf_on();
    store_hash(&fx, &f, SMOL_OID);
    check(&fx, &env, &Registry::standard(), &f);
    let reqs = hub.requests();
    assert!(!reqs.is_empty());
    let profile = s(&fx.profile()).to_lowercase();
    for q in &reqs {
        let low = q.to_lowercase();
        assert!(
            !low.contains(SMOL_OID) && !low.contains(&SMOL_SIZE.to_string()),
            "{q}"
        );
        assert!(!low.contains(&profile) && !low.contains("users"), "{q}");
    }
}

// Traces: EVID-COPY-DUP
#[test]
fn a_duplicate_group_counts_while_both_files_are_unchanged() {
    let fx = Fx::new();
    let d = fx.data();
    let (a, b, lone) = (d.join("a.bin"), d.join("b.bin"), d.join("lone.bin"));
    sparse(&a, 101 * MIB);
    sparse(&b, 101 * MIB);
    sparse(&lone, 102 * MIB);
    fx.index();
    let conn = fx.db.conn().unwrap();
    conn.execute(
        "INSERT INTO dup_group(id, size, hash, created_ms) VALUES (7, ?1, 'blake', 0)",
        [101 * MIB as i64],
    )
    .unwrap();
    for p in [&a, &b] {
        let sub = Subject::load(&conn, &s(p)).unwrap();
        conn.execute(
            "INSERT INTO dup_member(group_id, path, file_id, mtime) VALUES (7, ?1, ?2, ?3)",
            rusqlite::params![s(p), sub.file_id.unwrap().to_vec(), sub.mtime],
        )
        .unwrap();
    }
    let env = fx.env();
    for (me, twin) in [(&a, &b), (&b, &a)] {
        let r = answer(&fx, &env, "dup", me).unwrap();
        assert_eq!(r.confidence, Confidence::Identical);
        assert_eq!(r.locator.as_deref(), Some(s(twin).as_str()));
        assert_eq!(r.detail["via"], "duplicate group");
    }
    set_mtime(&b, 1_600_000_000_000);
    let r = answer(&fx, &env, "dup", &a).unwrap();
    assert_eq!(
        (r.confidence, r.pending()),
        (Confidence::None, Some("hash")),
        "{}",
        note(&r)
    );

    let r = answer(&fx, &env, "dup", &lone).unwrap();
    assert_eq!(r.confidence, Confidence::None);
    assert_eq!(note(&r), "no other file of this size on the indexed drives");
}

fn archive_entry(
    fx: &Fx,
    original: &Path,
    copy: &Path,
    size: i64,
    archived_ms: i64,
    restored: bool,
) {
    fx.db
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO archive_entry(original_path, destination_id, dest_uri, kind, size, archived_ms, restored_ms)
             VALUES (?1, 'd1', ?2, 1, ?3, ?4, ?5)",
            rusqlite::params![s(original), s(copy), size, archived_ms, restored.then_some(archived_ms + 1000)],
        )
        .unwrap();
}

// Traces: EVID-COPY-ARCHIVE
#[test]
fn hoardinis_archive_copies_count_while_the_file_is_unchanged() {
    let fx = Fx::new();
    let env = fx.env();
    let f = fx.data().join("report.pdf");
    let copy = fx.root.join(r"archive\2026-10-01\report.pdf");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::write(&f, b"report bytes").unwrap();
    std::fs::write(&copy, b"report bytes").unwrap();
    set_mtime(&f, 1_700_000_000_000);
    archive_entry(&fx, &f, &copy, 12, 1_700_000_500_000, true);

    let r = answer(&fx, &env, "archive", &f).unwrap();
    assert_eq!(r.confidence, Confidence::Identical, "{}", note(&r));
    assert_eq!(r.locator.as_deref(), Some(s(&copy).as_str()));
    assert!(note(&r).contains("verified when archived"));

    set_mtime(&f, 1_700_000_900_000);
    assert_eq!(
        answer(&fx, &env, "archive", &f).unwrap().confidence,
        Confidence::SameNameAndSize
    );
    std::fs::write(&f, b"report bytes!").unwrap();
    assert_eq!(
        answer(&fx, &env, "archive", &f).unwrap().confidence,
        Confidence::None
    );

    let g = fx.data().join("gone.iso");
    std::fs::write(&g, b"0123456789").unwrap();
    archive_entry(
        &fx,
        &g,
        &fx.root.join("nowhere.iso"),
        10,
        1_700_000_000_000,
        false,
    );
    assert_eq!(
        answer(&fx, &env, "archive", &g).unwrap().confidence,
        Confidence::SameNameAndSize
    );
    std::fs::write(&g, b"01234").unwrap();
    assert_eq!(
        answer(&fx, &env, "archive", &g).unwrap().confidence,
        Confidence::None
    );

    let h = fx.data().join("lost.bin");
    std::fs::write(&h, b"abc").unwrap();
    set_mtime(&h, 1_700_000_000_000);
    archive_entry(
        &fx,
        &h,
        &fx.root.join("missing-copy.bin"),
        3,
        1_700_000_500_000,
        true,
    );
    let r = answer(&fx, &env, "archive", &h).unwrap();
    assert_eq!(r.confidence, Confidence::None);
    assert!(note(&r).contains("no longer reachable"));
}

// Traces: EVID-COPY-SYNC
#[test]
fn the_sync_engines_in_sync_state_is_a_cloud_copy() {
    let fx = Fx::new();
    let root = fx.data().join("OneDrive");
    let cloud = Arc::new(FakeCloud::with_root(&root, "OneDrive"));
    let (a, b, plain) = (
        root.join(r"Pictures\a.jpg"),
        root.join(r"Pictures\b.jpg"),
        root.join("plain.txt"),
    );
    let outside = fx.data().join("outside.jpg");
    for p in [&a, &b, &plain, &outside] {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"pixels").unwrap();
    }
    cloud.set(&a, true);
    cloud.set(&b, true);
    cloud.set(&outside, true);
    std::fs::write(root.join(r"Pictures\desktop.ini"), b"[.ShellClassInfo]").unwrap();
    let mut env = fx.env();
    env.cloud = cloud.clone();

    let r = answer(&fx, &env, "sync", &a).unwrap();
    assert_eq!(
        (r.confidence, r.locator.as_deref()),
        (Confidence::Identical, Some("OneDrive"))
    );
    assert_eq!(note(&r), "synced to OneDrive");
    let folder = answer(&fx, &env, "sync", &root.join("Pictures")).unwrap();
    assert_eq!(folder.confidence, Confidence::Identical);

    cloud.set(&b, false);
    let r = answer(&fx, &env, "sync", &b).unwrap();
    assert_eq!(
        (r.confidence, r.pending()),
        (Confidence::None, Some("sync"))
    );
    let r = answer(&fx, &env, "sync", &plain).unwrap();
    assert_eq!(
        (r.confidence, r.pending()),
        (Confidence::None, Some("sync"))
    );
    let folder = answer(&fx, &env, "sync", &root.join("Pictures")).unwrap();
    assert_eq!(folder.confidence, Confidence::None);
    assert!(
        note(&folder).starts_with("1 of 2 files not in sync"),
        "{}",
        note(&folder)
    );
    assert!(answer(&fx, &env, "sync", &outside).is_none());
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=t",
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .output()
        .expect("git must be on PATH for EVID-COPY-GIT");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// Traces: EVID-COPY-GIT
#[test]
fn git_copies_are_tracked_unchanged_and_pushed() {
    let fx = Fx::new();
    let remote = fx.root.join("remote.git");
    let work = fx.data().join("proj");
    std::fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "--bare", "-q"]);
    git(&fx.data(), &["clone", "-q", &s(&remote), "proj"]);
    std::fs::write(work.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(work.join("model.bin"), b"weights").unwrap();
    std::fs::write(work.join("readme.txt"), b"hello").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "first"]);
    git(&work, &["push", "-q", "origin", "HEAD"]);
    git(
        &work,
        &[
            "branch",
            "-q",
            "--set-upstream-to",
            &format!("origin/{}", current_branch(&work)),
        ],
    );
    let env = fx.env();

    let r = answer(&fx, &env, "git", &work.join("model.bin")).unwrap();
    assert_eq!(r.confidence, Confidence::Identical, "{}", note(&r));
    assert!(r.locator.as_deref().unwrap().starts_with("origin/"));
    assert!(
        note(&r).starts_with("tracked and pushed to origin/"),
        "{}",
        note(&r)
    );
    let folder = answer(&fx, &env, "git", &work).unwrap();
    assert_eq!(
        folder.confidence,
        Confidence::Identical,
        "{}",
        note(&folder)
    );
    assert!(note(&folder).starts_with("everything tracked is pushed to"));

    std::fs::create_dir_all(work.join("target")).unwrap();
    std::fs::write(work.join(r"target\out.bin"), b"build").unwrap();
    assert!(
        answer(&fx, &env, "git", &work.join(r"target\out.bin")).is_none(),
        "ignored"
    );
    std::fs::write(work.join("new.bin"), b"new").unwrap();
    let r = answer(&fx, &env, "git", &work.join("new.bin")).unwrap();
    assert_eq!(
        (r.confidence, note(&r).as_str()),
        (Confidence::None, "not tracked by git")
    );
    assert_eq!(
        answer(&fx, &env, "git", &work).unwrap().confidence,
        Confidence::None
    );
    std::fs::remove_file(work.join("new.bin")).unwrap();

    std::fs::write(work.join("model.bin"), b"weights v2").unwrap();
    let r = answer(&fx, &env, "git", &work.join("model.bin")).unwrap();
    assert_eq!(
        (r.confidence, note(&r).as_str()),
        (Confidence::None, "changed since the last commit")
    );
    git(&work, &["commit", "-q", "-am", "second"]);
    let r = answer(&fx, &env, "git", &work.join("model.bin")).unwrap();
    assert_eq!(r.confidence, Confidence::None);
    assert!(
        note(&r).starts_with("changed in 1 commit(s) not pushed"),
        "{}",
        note(&r)
    );
    let r = answer(&fx, &env, "git", &work.join("readme.txt")).unwrap();
    assert_eq!(
        r.confidence,
        Confidence::Identical,
        "untouched by the unpushed commit: {}",
        note(&r)
    );
}

fn current_branch(work: &Path) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(work)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

// Traces: EVID-COPY-MODELHUB
#[test]
fn an_lmstudio_gguf_is_checked_against_its_repo_and_then_its_hash() {
    let fx = Fx::new();
    let f = smol_in_lmstudio(&fx);
    let hub = StandIn::start(smol_routes());
    let mut env = fx.env();
    env.hub_base = hub.base.clone();
    env.consent = hf_on();

    let r = answer(&fx, &env, HubCheck::NAME, &f).unwrap();
    assert_eq!(
        hub.requests(),
        [format!("/api/models/{SMOL_REPO}/tree/main?recursive=1")]
    );
    assert_eq!(
        (r.confidence, r.pending()),
        (Confidence::SameNameAndSize, Some("hash"))
    );
    assert_eq!(r.locator.as_deref(), Some(SMOL_REPO));

    store_hash(&fx, &f, SMOL_OID);
    let r = answer(&fx, &env, HubCheck::NAME, &f).unwrap();
    assert_eq!(r.confidence, Confidence::Identical);
    assert_eq!(
        note(&r),
        format!("identical to {SMOL_REPO} on Hugging Face")
    );

    set_mtime(&f, 1_650_000_000_000);
    store_hash(&fx, &f, &"0".repeat(64));
    let r = answer(&fx, &env, HubCheck::NAME, &f).unwrap();
    assert_eq!(r.confidence, Confidence::None, "{}", note(&r));
}

// Traces: EVID-COPY-MODELHUB
#[test]
fn a_gguf_anywhere_is_found_by_a_name_search() {
    let fx = Fx::new();
    let f = fx.data().join(r"downloads\models").join(SMOL);
    sparse(&f, SMOL_SIZE);
    let other = fx
        .data()
        .join(r"downloads\other")
        .join("SmolLM2-135M-Instruct-Q4_K_M.gguf");
    sparse(&other, SMOL_SIZE + 4096);
    let hub = StandIn::start(smol_routes());
    let mut env = fx.env();
    env.hub_base = hub.base.clone();
    env.consent = hf_on();

    let r = answer(&fx, &env, HubCheck::NAME, &f).unwrap();
    let reqs = hub.requests();
    assert_eq!(
        reqs[0],
        "/api/models?search=SmolLM2-135M-Instruct&limit=10&filter=gguf"
    );
    assert!(reqs.contains(
        &"/api/models/bartowski/SmolLM2-135M-Instruct-GGUF/tree/main?recursive=1".to_string()
    ));
    assert_eq!(
        r.locator.as_deref(),
        Some(SMOL_REPO),
        "bartowski's file has another size"
    );
    assert_eq!(r.detail["via"], "name search");
    assert_eq!(r.confidence, Confidence::SameNameAndSize);

    let r = answer(&fx, &env, HubCheck::NAME, &other).unwrap();
    assert_eq!(
        (r.confidence, note(&r).as_str()),
        (Confidence::None, "not found on Hugging Face")
    );
    let searched: Vec<&str> = r.detail["searched"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        searched.contains(&"bartowski/SmolLM2-135M-Instruct-GGUF") && searched.contains(&SMOL_REPO)
    );
}

// Traces: EVID-COPY-MODELHUB
#[test]
fn a_hub_cache_repo_is_identical_when_its_blobs_are_the_trees_oids() {
    let fx = Fx::new();
    let repo = fx.data().join(r"hub\models--o--m");
    let (big, small) = ("c".repeat(64), "d".repeat(40));
    for b in [&big, &small] {
        std::fs::create_dir_all(repo.join("blobs")).unwrap();
        std::fs::write(repo.join("blobs").join(b), b"blob").unwrap();
    }
    std::fs::create_dir_all(repo.join(r"snapshots\rev1")).unwrap();
    let tree = json!([
        {"type": "file", "oid": small, "size": 4, "path": "config.json"},
        {"type": "file", "oid": "e".repeat(40), "size": 4, "path": "model.safetensors", "lfs": {"oid": big, "size": 4, "pointerSize": 130}}
    ])
    .to_string();
    let hub = StandIn::start(vec![("/api/models/o/m/tree/rev1?recursive=1".into(), tree)]);
    let mut env = fx.env();
    env.hub_base = hub.base.clone();
    env.consent = hf_on();
    let r = answer(&fx, &env, HubCheck::NAME, &repo).unwrap();
    assert_eq!(
        (r.confidence, r.locator.as_deref()),
        (Confidence::Identical, Some("o/m")),
        "{}",
        note(&r)
    );
    let rows: i64 = fx
        .db
        .conn()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM file_hash", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0, "nothing was hashed");

    std::fs::write(repo.join("blobs").join("f".repeat(64)), b"stray").unwrap();
    set_mtime(&repo, 1_600_000_000_000);
    let r = answer(&fx, &env, HubCheck::NAME, &repo).unwrap();
    assert_eq!(r.confidence, Confidence::None, "{}", note(&r));
}

// Traces: EVID-COPY-MODELHUB
#[test]
fn ollama_blobs_are_claimed_by_their_manifest() {
    let fx = Fx::new();
    let env = fx.env();
    let models = PathBuf::from(env.ollama_models.clone().unwrap());
    let digest = "1".repeat(64);
    let blob = models.join("blobs").join(format!("sha256-{digest}"));
    let orphan = models
        .join("blobs")
        .join(format!("sha256-{}", "2".repeat(64)));
    let manifest = models.join(r"manifests\registry.ollama.ai\library\llama3\8b");
    std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
    std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    std::fs::write(&blob, b"gguf").unwrap();
    std::fs::write(&orphan, b"gguf").unwrap();
    std::fs::write(
        &manifest,
        json!({"layers": [{"digest": format!("sha256:{digest}")}], "config": {"digest": "sha256:9"}}).to_string(),
    )
    .unwrap();
    let r = answer(&fx, &env, "ollama", &blob).unwrap();
    assert_eq!(r.confidence, Confidence::Claimed);
    assert!(
        note(&r).starts_with("in Ollama as llama3:8b"),
        "{}",
        note(&r)
    );
    assert_eq!(
        answer(&fx, &env, "ollama", &orphan).unwrap().confidence,
        Confidence::None
    );
}

// Traces: EVID-COPY-WINGET
#[test]
fn recorded_winget_output_parses() {
    let rows = winget::parse_search(&fixture("winget-search-etcher.txt"));
    assert_eq!(
        rows,
        vec![winget::SearchRow {
            name: "balenaEtcher".into(),
            id: "Balena.Etcher".into(),
            version: "2.1.7".into()
        }]
    );
    let ff = winget::parse_search(&fixture("winget-search-firefox.txt"));
    assert!(ff.len() >= 5);
    assert_eq!(
        (
            ff[1].name.as_str(),
            ff[1].id.as_str(),
            ff[1].version.as_str()
        ),
        ("Mozilla Firefox (en-US)", "Mozilla.Firefox", "157.0.1")
    );
    assert!(winget::parse_search(&fixture("winget-search-none.txt")).is_empty());
    let show = winget::parse_show(&fixture("winget-show-etcher.txt")).unwrap();
    assert_eq!(show.id, "Balena.Etcher");
    assert_eq!(show.version.as_deref(), Some("2.1.7"));
    assert_eq!(
        show.publisher.as_deref(),
        Some("Balena Ltd. <hello@balena.io>")
    );
    assert_eq!(
        show.installer_url.as_deref(),
        Some(
            "https://github.com/balena-io/etcher/releases/download/v2.1.7/balenaEtcher-2.1.7.Setup.exe"
        )
    );
    assert_eq!(show.installer_sha256.as_deref(), Some(ETCHER_SHA));
}

// Traces: EVID-COPY-WINGET
#[test]
fn an_installer_is_identical_when_winget_lists_its_hash() {
    let fx = Fx::new();
    let f = fx.data().join("balenaEtcher-2.1.7.Setup.exe");
    std::fs::write(&f, b"not really an installer").unwrap();
    let cli = Arc::new(FakeWinget::default());
    let mut env = fx.env();
    env.winget = cli.clone();
    env.consent = hf_on();

    let r = answer(&fx, &env, "winget", &f).unwrap();
    assert_eq!(
        *cli.calls.lock().unwrap(),
        [
            "search --name balenaEtcher --source winget",
            "show --id Balena.Etcher --exact --source winget --version 2.1.7"
        ]
    );
    assert_eq!(
        (r.confidence, r.pending()),
        (Confidence::SameNameAndSize, Some("hash"))
    );
    store_hash(&fx, &f, ETCHER_SHA);
    let r = answer(&fx, &env, "winget", &f).unwrap();
    assert_eq!(
        (r.confidence, note(&r).as_str()),
        (Confidence::Identical, "in winget as Balena.Etcher 2.1.7")
    );
    set_mtime(&f, 1_650_000_000_000);
    store_hash(&fx, &f, &"9".repeat(64));
    assert_eq!(
        answer(&fx, &env, "winget", &f).unwrap().confidence,
        Confidence::Claimed
    );

    let g = fx.data().join("NoSuchTool-1.0.exe");
    std::fs::write(&g, b"x").unwrap();
    let r = answer(&fx, &env, "winget", &g).unwrap();
    assert_eq!(
        (r.confidence, note(&r).as_str()),
        (Confidence::None, "not found in winget")
    );
}

// Traces: EVID-COPY-CLAIMED
#[test]
fn launchers_and_package_managers_claim_their_files() {
    let fx = Fx::new();
    let env = fx.env();
    let wheel = fx
        .profile()
        .join(r"AppData\Local\pip\cache\wheels\torch.whl");
    std::fs::create_dir_all(wheel.parent().unwrap()).unwrap();
    std::fs::write(&wheel, b"zip").unwrap();
    let r = answer(&fx, &env, "catalog", &wheel).unwrap();
    assert_eq!(
        (r.confidence, r.locator.as_deref()),
        (Confidence::Claimed, Some("pip"))
    );
    let conn = fx.db.conn().unwrap();
    let cx = Cx {
        conn: &conn,
        env: &env,
    };
    let game = Subject {
        volume_id: 0,
        path: r"D:\SteamLibrary\steamapps\common\Game\data.pak".into(),
        dir: false,
        size: 1,
        mtime: None,
        file_id: None,
        attrs: 0,
        hint: None,
    };
    let reg = Registry::with(vec![Box::new(copycheck::CatalogCheck)]);
    let steam = copycheck::check_subject(&cx, &reg, &game).results;
    assert_eq!(steam[0].confidence, Confidence::Claimed);
    assert_eq!(steam[0].locator.as_deref(), Some("Steam"));
    let plain = fx.data().join("notes.bin");
    std::fs::write(&plain, b"n").unwrap();
    assert!(answer(&fx, &env, "catalog", &plain).is_none());
}

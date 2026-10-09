use std::path::Path;

use hoardini_core::catalog::UnitKind;
use hoardini_core::index::{FakeFs, RefreshOptions, refresh};
use hoardini_core::memory::Detail;
use hoardini_core::memory::knowledge::{MODEL_WEIGHTS, remember_filetype};
use hoardini_core::memory::location::{Answer, answer};
use hoardini_core::policy::PolicyEnv;
use hoardini_core::review::units::{Unit, Why, resolve};
use hoardini_core::store::Db;

const KIB: u64 = 1 << 10;
const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;
const NOW: i64 = 1_700_000_000_000 + 30 * DAY;
const PROFILE: &str = r"C:\Users\tester";

struct Fx {
    _dir: tempfile::TempDir,
    db: Db,
    env: PolicyEnv,
    fs: FakeFs,
}

fn fx() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("h.db")).unwrap();
    let env = PolicyEnv::with_paths(
        Path::new(PROFILE),
        Path::new(r"C:\ProgramData\hoardini-test"),
    );
    Fx {
        _dir: dir,
        db,
        env,
        fs: FakeFs::new(r"C:\"),
    }
}

impl Fx {
    fn index(&self) {
        let out = refresh(
            &self.db.conn().unwrap(),
            &self.fs,
            r"C:\",
            &RefreshOptions::default(),
        )
        .unwrap();
        assert!(out.completed);
    }

    fn units(&self, root: &str) -> Vec<Unit> {
        resolve(&self.db.conn().unwrap(), &self.env, &self.fs, root, NOW).unwrap()
    }

    fn q4nx_is_weights(&self) {
        remember_filetype(
            &self.db.conn().unwrap(),
            "q4nx",
            ".q4nx: FastFlowLM NPU model weights.",
            "user",
            Detail {
                category: MODEL_WEIGHTS.into(),
                app: Some("FastFlowLM".into()),
                ..Detail::default()
            },
            NOW,
        )
        .unwrap();
    }
}

fn summary(units: &[Unit]) -> Vec<(&str, UnitKind, Why)> {
    units
        .iter()
        .map(|u| (u.path.as_str(), u.kind, u.why))
        .collect()
}

fn flm_tree(fs: &FakeFs, root: &str) {
    fs.file(
        &format!(r"{root}\models\Qwen3.6-35B-A3B-NPU2\model.q4nx"),
        60 * GIB,
    );
    fs.file(
        &format!(r"{root}\models\Qwen3.6-35B-A3B-NPU2\config.json"),
        4 * KIB,
    );
    fs.file(
        &format!(r"{root}\models\Qwen3.8-27B-NPU2\model.q4nx"),
        40 * GIB,
    );
    fs.file(&format!(r"{root}\logs\flm.log"), 2 * MIB);
}

// Traces: REVIEW-UNITS
#[test]
fn a_model_folder_tree_resolves_to_one_item_per_model() {
    let f = fx();
    flm_tree(&f.fs, r"C:\Users\tester\.flm");
    flm_tree(&f.fs, r"C:\npu");
    f.index();

    let flm = f.units(r"~\.flm");
    assert_eq!(
        summary(&flm),
        vec![
            (
                r"C:\Users\tester\.flm\models\Qwen3.6-35B-A3B-NPU2",
                UnitKind::Children,
                Why::Catalog
            ),
            (
                r"C:\Users\tester\.flm\models\Qwen3.8-27B-NPU2",
                UnitKind::Children,
                Why::Catalog
            ),
        ]
    );
    assert!(
        flm.iter()
            .all(|u| u.catalog_id.as_deref() == Some("flm-models"))
    );
    assert!(
        flm.iter()
            .all(|u| u.container.as_deref() == Some(r"C:\Users\tester\.flm\models"))
    );
    assert_eq!(flm[0].size, 60 * GIB + 4 * KIB);

    let unknown_type = f.units(r"C:\npu");
    assert_eq!(
        summary(&unknown_type),
        vec![
            (
                r"C:\npu\models\Qwen3.6-35B-A3B-NPU2\model.q4nx",
                UnitKind::Whole,
                Why::Leaf
            ),
            (
                r"C:\npu\models\Qwen3.8-27B-NPU2\model.q4nx",
                UnitKind::Whole,
                Why::Leaf
            ),
        ]
    );

    f.q4nx_is_weights();
    let detected = f.units(r"C:\npu");
    assert_eq!(
        summary(&detected),
        vec![
            (
                r"C:\npu\models\Qwen3.6-35B-A3B-NPU2",
                UnitKind::Children,
                Why::ModelStore
            ),
            (
                r"C:\npu\models\Qwen3.8-27B-NPU2",
                UnitKind::Children,
                Why::ModelStore
            ),
        ]
    );
    assert!(
        detected
            .iter()
            .all(|u| u.container.as_deref() == Some(r"C:\npu\models") && u.catalog_id.is_none())
    );
}

// Traces: REVIEW-UNITS
#[test]
fn bin_weights_make_a_model_store_only_next_to_a_config() {
    let f = fx();
    f.fs.file(r"C:\hf\bert\pytorch_model.bin", 2 * GIB);
    f.fs.file(r"C:\hf\bert\config.json", KIB);
    f.fs.file(r"C:\hf\gpt\pytorch_model.bin", 3 * GIB);
    f.fs.file(r"C:\hf\gpt\config.json", KIB);
    f.fs.file(r"C:\blobs\a\x.bin", 2 * GIB);
    f.fs.file(r"C:\blobs\b\y.bin", 3 * GIB);
    f.index();

    let hf = f.units(r"C:\hf");
    assert_eq!(
        summary(&hf),
        vec![
            (r"C:\hf\gpt", UnitKind::Children, Why::ModelStore),
            (r"C:\hf\bert", UnitKind::Children, Why::ModelStore),
        ]
    );
    let blobs = f.units(r"C:\blobs");
    assert_eq!(
        summary(&blobs),
        vec![
            (r"C:\blobs\b\y.bin", UnitKind::Whole, Why::Leaf),
            (r"C:\blobs\a\x.bin", UnitKind::Whole, Why::Leaf),
        ]
    );
}

// Traces: REVIEW-UNITS
#[test]
fn build_output_is_one_item_and_unknown_folders_descend_by_size() {
    let f = fx();
    f.fs.file(r"C:\code\proj\target\debug\app.exe", 3 * GIB);
    f.fs.file(r"C:\code\proj\src\main.rs", 4 * KIB);
    for i in 0..40 {
        f.fs.file(&format!(r"C:\work\big\part{i:02}.dat"), 50 * MIB);
    }
    for i in 0..30 {
        f.fs.file(&format!(r"C:\work\medium\part{i:02}.dat"), 10 * MIB);
    }
    for i in 0..5 {
        f.fs.file(&format!(r"C:\work\tiny\part{i}.dat"), 10 * MIB);
    }
    f.index();

    let proj = f.units(r"C:\code\proj");
    assert_eq!(
        summary(&proj),
        vec![(r"C:\code\proj\target", UnitKind::Whole, Why::Catalog)]
    );
    assert_eq!(proj[0].catalog_id.as_deref(), Some("rust-target"));
    assert_eq!(proj[0].container.as_deref(), Some(r"C:\code\proj"));

    let work = f.units(r"C:\work");
    assert_eq!(
        summary(&work),
        vec![
            (r"C:\work\big", UnitKind::Whole, Why::Leaf),
            (r"C:\work\medium", UnitKind::Whole, Why::Leaf),
        ]
    );
    assert_eq!(work[0].size, 40 * 50 * MIB);
}

// Traces: REVIEW-UNITS
#[test]
fn temp_is_one_contents_item_without_fresh_children() {
    let f = fx();
    let temp = r"C:\Users\tester\AppData\Local\Temp";
    f.fs.file(&format!(r"{temp}\old-session\scratch.bin"), 5 * GIB);
    f.fs.file(&format!(r"{temp}\old.iso"), 2 * GIB);
    f.fs.file(&format!(r"{temp}\live-session\model.gguf"), 8 * GIB);
    f.fs.touch(&format!(r"{temp}\old-session\scratch.bin"), NOW - 3 * DAY);
    f.fs.touch(&format!(r"{temp}\old.iso"), NOW - 3 * DAY);
    f.fs.touch(&format!(r"{temp}\live-session\model.gguf"), NOW - HOUR);
    f.index();

    let units = f.units(temp);
    assert_eq!(
        summary(&units),
        vec![(temp, UnitKind::Contents, Why::Catalog)]
    );
    let u = &units[0];
    assert_eq!(u.catalog_id.as_deref(), Some("temp-user"));
    let mut parts: Vec<&str> = u.parts.iter().map(|p| p.path.as_str()).collect();
    parts.sort();
    assert_eq!(
        parts,
        vec![
            r"C:\Users\tester\AppData\Local\Temp\old-session",
            r"C:\Users\tester\AppData\Local\Temp\old.iso",
        ]
    );
    assert_eq!(u.size, 7 * GIB);

    let whole_drive = f.units(r"C:\");
    assert!(
        whole_drive
            .iter()
            .any(|x| x.path == temp && x.kind == UnitKind::Contents),
        "{whole_drive:#?}"
    );
}

// Traces: REVIEW-UNITS
#[test]
fn steam_games_and_windows_temp_inside_a_kept_windows() {
    let f = fx();
    let win = f.env.system_root.clone();
    f.fs.file(r"C:\SteamLibrary\steamapps\common\GameA\a.pak", 20 * GIB);
    f.fs.file(r"C:\SteamLibrary\steamapps\common\GameB\b.pak", 10 * GIB);
    f.fs.file(&format!(r"{win}\System32\big.dll"), 10 * GIB);
    f.fs.file(&format!(r"{win}\Temp\setup.tmp"), 2 * GIB);
    f.index();

    let steam = f.units(r"C:\SteamLibrary\steamapps\common");
    assert_eq!(
        summary(&steam),
        vec![
            (
                r"C:\SteamLibrary\steamapps\common\GameA",
                UnitKind::Children,
                Why::Catalog
            ),
            (
                r"C:\SteamLibrary\steamapps\common\GameB",
                UnitKind::Children,
                Why::Catalog
            ),
        ]
    );
    assert!(
        steam
            .iter()
            .all(|u| u.catalog_id.as_deref() == Some("steam-common"))
    );

    let windows = f.units(&win);
    let got: Vec<(String, UnitKind, Why)> = windows
        .iter()
        .map(|u| (u.path.to_lowercase(), u.kind, u.why))
        .collect();
    assert_eq!(
        got,
        vec![
            (win.to_lowercase(), UnitKind::Whole, Why::Kept),
            (
                format!(r"{win}\Temp").to_lowercase(),
                UnitKind::Contents,
                Why::Catalog
            ),
        ]
    );
}

// Traces: REVIEW-UNITS
#[test]
fn a_folder_the_user_keeps_is_one_kept_item() {
    let f = fx();
    f.fs.file(r"C:\code\proj\target\debug\app.exe", 3 * GIB);
    f.fs.file(r"C:\code\proj\data\set.dat", 2 * GIB);
    f.index();
    answer(
        &f.db.conn().unwrap(),
        &f.env,
        r"C:\code\proj",
        Answer::Keep,
        NOW,
    )
    .unwrap();
    assert_eq!(
        summary(&f.units(r"C:\code")),
        vec![(r"C:\code\proj", UnitKind::Whole, Why::Kept)]
    );
}

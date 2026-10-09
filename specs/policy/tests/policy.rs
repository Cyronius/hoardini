#[path = "support.rs"]
mod support;

use std::path::{Path, PathBuf};

use hoardini_core::memory::{self, Kind, NewMemory, Scope, remember_with};
use hoardini_core::policy::{Verdict, cap_for_with, deny_reason};
use hoardini_core::proposals::{self, NewItem, ProposalKind, SMALL_BYTES, default_tick, is_small};
use hoardini_win::set_file_times;
use support::*;

fn denied(fx: &Fx, p: &str) -> Option<String> {
    deny_reason(&fx.db.conn().unwrap(), &fx.env, p).unwrap()
}

// Traces: POLICY-DENY
#[test]
fn deny_list_rejects_system_locations_profile_roots_and_their_ancestors() {
    let fx = Fx::new();
    let sysroot = fx.env.system_root.clone();
    let progdata = fx.env.program_data.clone();
    let profile = s(&fx.profile);
    let data = s(&fx.data);
    let must_deny = [
        r"C:\".to_string(),
        r"D:".to_string(),
        r"\\nas\share".to_string(),
        format!(r"{sysroot}\System32\drivers\etc\hosts"),
        sysroot.clone(),
        r"C:\Program Files\Vendor\app.exe".into(),
        r"C:\Program Files (x86)".into(),
        r"E:\Program Files\Steam".into(),
        format!(r"{progdata}\Microsoft\Windows\WER"),
        progdata.clone(),
        r"C:\System Volume Information\tracking.log".into(),
        r"D:\$Recycle.Bin\S-1-5-21-1\$R1.bin".into(),
        r"C:\pagefile.sys".into(),
        r"D:\hiberfil.sys".into(),
        r"C:\swapfile.sys".into(),
        profile.clone(),
        format!(r"{profile}\AppData"),
        format!(r"{profile}\AppData\Local"),
        format!(r"{profile}\AppData\Roaming"),
        format!(r"{profile}\AppData\LocalLow"),
        s(fx.profile.parent().unwrap()),
        data.clone(),
        format!(r"{data}\hoardini.db"),
    ];
    for p in &must_deny {
        assert!(denied(&fx, p).is_some(), "{p} must be denied");
    }

    let downloads = fx.pfile(r"Downloads\big.iso", b"iso");
    let pip = fx.pfile(r"AppData\Local\pip\cache\wheel.whl", b"whl");
    let scratch = fx.file(r"scratch\a.bin", b"a");
    for p in [&downloads, &pip, &scratch] {
        assert_eq!(denied(&fx, &s(p)), None, "{} must be allowed", p.display());
    }

    let all: Vec<PathBuf> = must_deny.iter().map(PathBuf::from).collect();
    let refs: Vec<&Path> = all.iter().map(PathBuf::as_path).collect();
    let err = fx
        .propose(ProposalKind::Delete, &refs, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("refused by policy"), "{err}");
    assert!(
        err.contains("volume root") && err.contains("profile"),
        "{err}"
    );
    assert_eq!(count(&fx.db, "SELECT COUNT(*) FROM proposal"), 0);

    let sneaky = PathBuf::from(format!(r"{}\Downloads\..\..", profile));
    let tilde = PathBuf::from(r"~\AppData\Local");
    let p = fx
        .propose(
            ProposalKind::Delete,
            &[&downloads, &PathBuf::from(&sysroot), &sneaky, &tilde, &pip],
            None,
        )
        .unwrap();
    assert_eq!(p.items.len(), 2);
    item(&p, &downloads);
    item(&p, &pip);
    assert_eq!(p.dropped.len(), 3, "{:#?}", p.dropped);
    assert!(p.dropped.iter().all(|d| !d.reason.is_empty()));
    assert!(downloads.exists() && pip.exists());
}

fn via_admin_share(p: &Path) -> String {
    let local = s(p);
    let (drive, rest) = local.split_at(1);
    format!(
        r"\\localhost\{drive}$\{}",
        rest.trim_start_matches(':').trim_start_matches('\\')
    )
}

// Traces: POLICY-DENY
#[test]
fn a_share_that_leads_back_to_this_pc_cannot_dodge_the_deny_list() {
    let fx = Fx::new();
    let profile_unc = via_admin_share(&fx.profile);
    if std::fs::metadata(&profile_unc).is_err() {
        eprintln!("skipped: the administrative share is not reachable here ({profile_unc})");
        return;
    }
    let file = fx.pfile(r"Documents\taxes.pdf", b"pdf");
    for p in [profile_unc.clone(), via_admin_share(&file)] {
        let why = denied(&fx, &p).unwrap_or_else(|| panic!("{p} must be denied"));
        assert!(why.contains("this PC"), "{why}");
    }
    let err = fx
        .propose(ProposalKind::Delete, &[Path::new(&profile_unc)], None)
        .unwrap_err();
    assert!(err.contains("refused by policy"), "{err}");
    assert!(file.exists());
}

// Traces: POLICY-PROTECT
#[test]
fn a_protected_cloud_file_cannot_be_named_around_its_protect_memory() {
    use std::os::windows::fs::OpenOptionsExt;
    let fx = Fx::new();
    let conn = fx.db.conn().unwrap();
    let f = fx.work.join("Tax Return 2024.pdf");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .attributes(hoardini_win::FILE_ATTRIBUTE_OFFLINE)
        .open(&f)
        .unwrap();
    remember_with(
        &conn,
        NewMemory::new(Scope::Path, &s(&f), Kind::Protect, "tax records", "user"),
        1,
        &s(&fx.profile),
    )
    .unwrap();
    let dotted = PathBuf::from(format!("{}.", s(&f)));
    let err = fx
        .propose(ProposalKind::Delete, &[&dotted], None)
        .unwrap_err();
    assert!(err.contains("tax records"), "{err}");
    assert!(f.exists());
}

// Traces: POLICY-DENY
#[test]
fn files_of_the_active_local_model_are_denied() {
    let fx = Fx::new();
    let conn = fx.db.conn().unwrap();
    conn.execute(
        "INSERT INTO provider(id, name, config_json, is_active, created_ms) VALUES ('p1', 'LM Studio', ?1, 1, 0)",
        [r#"{"name":"LM Studio","base_url":"http://localhost:1234/v1","model":"qwen3.8-27b"}"#],
    )
    .unwrap();
    let models = fx.profile.join(r".lmstudio\models");
    let qwen = fx.pfile(
        r".lmstudio\models\lmstudio-community\Qwen3.8-27B-GGUF\Qwen3.8-27B-Q8_0.gguf",
        b"GGUF",
    );
    let llama = fx.pfile(
        r".lmstudio\models\lmstudio-community\Llama-3.3-70B-GGUF\llama-3.3-70b-Q4_K_M.gguf",
        b"GGUF",
    );
    for p in [
        qwen.clone(),
        qwen.parent().unwrap().to_path_buf(),
        models.join("lmstudio-community"),
        models.clone(),
        fx.profile.join(".lmstudio"),
    ] {
        let why = denied(&fx, &s(&p)).unwrap_or_else(|| panic!("{} must be denied", p.display()));
        assert!(why.contains("qwen3.8-27b"), "{why}");
    }
    assert_eq!(denied(&fx, &s(&llama)), None);
    assert_eq!(denied(&fx, &s(llama.parent().unwrap())), None);

    conn.execute(
        "UPDATE provider SET config_json = ?1 WHERE id = 'p1'",
        [r#"{"name":"OpenAI","base_url":"https://api.openai.com/v1","model":"qwen3.8-27b"}"#],
    )
    .unwrap();
    assert_eq!(
        denied(&fx, &s(&qwen)),
        None,
        "a remote provider's model is not on this disk"
    );
}

// Traces: POLICY-PROTECT
#[test]
fn protect_memories_block_the_path_its_contents_and_its_ancestors() {
    let fx = Fx::new();
    let conn = fx.db.conn().unwrap();
    let photo = fx.file(r"keep\2024\a.jpg", b"jpg");
    let kdbx = fx.file(r"secrets\vault.kdbx", b"kdbx");
    let other = fx.file(r"other\b.bin", b"b");
    let profile = s(&fx.profile);
    let keep = fx.work.join("keep");
    let m = remember_with(
        &conn,
        NewMemory::new(
            Scope::Path,
            &s(&keep),
            Kind::Protect,
            "family photos",
            "user",
        ),
        1,
        &profile,
    )
    .unwrap();
    remember_with(
        &conn,
        NewMemory::new(
            Scope::Glob,
            &format!(r"{}\secrets\*.kdbx", s(&fx.work)),
            Kind::Protect,
            "password database",
            "user",
        ),
        1,
        &profile,
    )
    .unwrap();

    for p in [&photo, &keep, &fx.work] {
        let why = denied(&fx, &s(p)).unwrap_or_else(|| panic!("{} must be denied", p.display()));
        assert!(why.contains("family photos"), "{why}");
    }
    assert!(
        denied(&fx, &s(&kdbx))
            .unwrap()
            .contains("password database")
    );
    assert!(denied(&fx, &s(kdbx.parent().unwrap())).is_some());
    assert_eq!(denied(&fx, &s(&other)), None);

    let err = fx
        .propose(ProposalKind::Delete, &[&photo], None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("family photos"), "{err}");

    // A protect added after the proposal still stops execution.
    let p = fx.propose(ProposalKind::Delete, &[&other], None).unwrap();
    remember_with(
        &conn,
        NewMemory::new(
            Scope::Path,
            &s(other.parent().unwrap()),
            Kind::Protect,
            "late protect",
            "user",
        ),
        2,
        &profile,
    )
    .unwrap();
    let ex = fx.executor();
    let r = ex.approve(&p.id, &ids(&p)).unwrap();
    assert!(r.items.is_empty());
    wait(&ex, &r.job_id);
    let after = fx.get(&p.id);
    assert_eq!(after.items[0].status, "skipped");
    assert!(
        after.items[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("late protect")
    );
    assert!(other.exists());

    memory::delete(&conn, m.id).unwrap();
    assert_eq!(denied(&fx, &s(&photo)), None);
}

// Traces: POLICY-PROTECT
#[test]
fn a_wildcard_protect_glob_blocks_a_folder_that_holds_a_match_deeper_inside() {
    let fx = Fx::new();
    let conn = fx.db.conn().unwrap();
    let proj = fx.work.join("proj");
    let with = fx.file(r"proj\app\secrets\key.pem", b"pem");
    fx.file(r"proj\app\src\main.rs", b"fn main() {}");
    let clean = fx.file(r"clean\app\src\main.rs", b"fn main() {}");
    remember_with(
        &conn,
        NewMemory::new(
            Scope::Glob,
            &format!(r"{}\**\secrets\**", s(&fx.work)),
            Kind::Protect,
            "private keys",
            "user",
        ),
        1,
        &s(&fx.profile),
    )
    .unwrap();

    let unindexed = denied(&fx, &s(&proj)).expect("an unindexed folder cannot be cleared");
    assert!(unindexed.contains("index cannot confirm"), "{unindexed}");

    fx.index_work();
    let why = denied(&fx, &s(&proj)).expect("proj holds a protected folder");
    assert!(
        why.contains("private keys") && why.contains("secrets"),
        "{why}"
    );
    assert!(denied(&fx, &s(&proj.join("app"))).is_some());
    assert_eq!(denied(&fx, &s(&proj.join("app").join("src"))), None);
    assert_eq!(
        denied(&fx, &s(clean.parent().unwrap().parent().unwrap())),
        None
    );
    assert_eq!(denied(&fx, &s(&clean)), None);
    assert!(with.exists());
}

fn assert_open_in_me(reason: Option<&str>) {
    let r = reason.expect("blocked with a reason");
    assert!(r.starts_with("Open in "), "{r}");
    assert!(r.contains(&format!("(pid {})", std::process::id())), "{r}");
}

// Traces: POLICY-IN-USE
#[test]
fn open_files_block_delete_and_archive_naming_the_process() {
    let fx = Fx::new();
    let held = fx.file(r"indexed\held.bin", &data(64 * 1024, 1));
    fx.file(r"indexed\free.bin", &data(1024, 2));
    fx.index_work();
    let loose_held = fx.file(r"loose\deep\held2.bin", &data(4096, 3));
    let free = fx.file(r"loose\free.bin", &data(10, 4));
    let (dest, _) = fx.local_destination("nas");
    let h1 = std::fs::File::open(&held).unwrap();
    let h2 = std::fs::File::open(&loose_held).unwrap();

    let indexed_dir = fx.work.join("indexed");
    let loose_dir = fx.work.join("loose");
    let p = fx
        .propose(ProposalKind::Delete, &[&held, &loose_dir], None)
        .unwrap();
    // The indexed folder is checked through the index's largest files, the loose one by listing it.
    let q = fx
        .propose(ProposalKind::Delete, &[&indexed_dir], None)
        .unwrap();
    assert_eq!(p.items.len() + q.items.len(), 3);
    for it in p.items.iter().chain(&q.items) {
        assert_eq!(it.status, "blocked", "{}", it.path);
        assert!(!it.selected);
        assert_eq!(it.cap, Some(Verdict::Keep));
        assert_open_in_me(it.block_reason.as_deref());
    }
    assert_eq!(p.total_alloc, 0);
    let a = fx
        .propose(ProposalKind::Archive, &[&loose_held], Some(&dest.id))
        .unwrap();
    assert_open_in_me(a.items[0].block_reason.as_deref());

    let ex = fx.executor();
    let err = ex.approve(&p.id, &ids(&p)).unwrap_err().to_string();
    assert!(err.contains("Open in"), "{err}");
    assert_eq!(fx.get(&p.id).status, "pending");

    // Opened after the proposal: skipped at execution with the process name.
    let late = fx.propose(ProposalKind::Delete, &[&free], None).unwrap();
    assert_eq!(late.items[0].status, "pending");
    let h3 = std::fs::File::open(&free).unwrap();
    let r = ex.approve(&late.id, &ids(&late)).unwrap();
    assert!(r.items.is_empty());
    wait(&ex, &r.job_id);
    let after = fx.get(&late.id);
    assert_eq!(after.items[0].status, "skipped");
    assert_open_in_me(after.items[0].detail.as_deref());
    drop((h1, h2, h3));
    assert!(held.exists() && loose_held.exists() && free.exists());
}

// Traces: POLICY-IN-USE
#[test]
fn cloud_placeholders_block_archive_but_not_delete() {
    use std::io::Write;
    use std::os::windows::fs::OpenOptionsExt;
    let fx = Fx::new();
    let f = fx.work.join("online-only.bin");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .attributes(hoardini_win::FILE_ATTRIBUTE_OFFLINE)
        .open(&f)
        .unwrap()
        .write_all(b"pretend remote")
        .unwrap();
    let (dest, _) = fx.local_destination("nas");
    let a = fx
        .propose(ProposalKind::Archive, &[&f], Some(&dest.id))
        .unwrap();
    assert_eq!(a.items[0].status, "blocked");
    assert!(
        a.items[0]
            .block_reason
            .as_deref()
            .unwrap()
            .contains("download")
    );
    let d = fx.propose(ProposalKind::Delete, &[&f], None).unwrap();
    assert_eq!(d.items[0].status, "pending");

    let missing = fx.work.join("never-existed.bin");
    let m = fx.propose(ProposalKind::Delete, &[&missing], None).unwrap();
    assert_eq!(m.items[0].status, "blocked");
    assert_eq!(m.items[0].block_reason.as_deref(), Some("Not found"));
}

// Traces: POLICY-REVALIDATE
#[test]
fn items_changed_since_the_proposal_are_skipped_not_executed() {
    let fx = Fx::new();
    let grown = fx.file("grown.bin", &data(1000, 1));
    let touched = fx.file("touched.bin", &data(1000, 2));
    let replaced = fx.file("replaced.bin", &data(1000, 3));
    let same = fx.file("same.bin", &data(1000, 4));
    let p = fx
        .propose(
            ProposalKind::Delete,
            &[&grown, &touched, &replaced, &same],
            None,
        )
        .unwrap();
    assert!(p.items.iter().all(|i| i.status == "pending"));

    std::fs::OpenOptions::new()
        .write(true)
        .open(&grown)
        .unwrap()
        .set_len(1500)
        .unwrap();
    set_file_times(&touched, None, Some(1_600_000_000_000)).unwrap();
    let md = std::fs::metadata(&replaced).unwrap();
    std::fs::remove_file(&replaced).unwrap();
    std::fs::write(&replaced, data(1000, 9)).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&replaced)
        .unwrap()
        .set_modified(md.modified().unwrap())
        .unwrap();

    let ex = fx.executor();
    let r = ex.approve(&p.id, &ids(&p)).unwrap();
    assert_eq!(r.items, vec![item(&p, &same).id]);
    let job = wait(&ex, &r.job_id);
    assert_eq!(job.status, "done");
    let after = fx.get(&p.id);
    for f in [&grown, &touched, &replaced] {
        let it = item(&after, f);
        assert_eq!(it.status, "skipped", "{}", f.display());
        assert!(
            it.detail
                .as_deref()
                .unwrap()
                .starts_with("Changed since the proposal"),
            "{:?}",
            it.detail
        );
        assert!(f.exists());
    }
    assert_eq!(item(&after, &same).status, "done");
    assert!(!same.exists());
    assert_eq!(after.status, "partial");
}

// Traces: POLICY-VERDICT-CAP
#[test]
fn shown_verdict_never_exceeds_the_policy_cap() {
    let fx = Fx::new();
    let blob = fx.pfile(
        r".cache\huggingface\hub\models--org--m\blobs\0123abcd",
        b"blob",
    );
    let model_dir = fx
        .profile
        .join(r".cache\huggingface\hub\models--org--other");
    fx.pfile(
        r".cache\huggingface\hub\models--org--other\blobs\4567",
        b"blob",
    );
    let vhdx = fx.file(r"vm\disk.vhdx", b"vhdx");
    let nm = fx.file(r"app\node_modules\left-pad\index.js", b"js");
    let node_modules = fx.work.join(r"app\node_modules");
    let unknown = fx.file("mystery.bin", b"??");
    let keep_me = fx.file("keep-me.bin", b"k");

    let mut req = fx.request(ProposalKind::Delete, &[], None);
    let mut add = |p: &Path, v: Verdict| {
        req.items.push(NewItem {
            path: s(p),
            verdict: Some(v),
            reason: Some("llm says so".into()),
        })
    };
    add(&blob, Verdict::Safe);
    add(&model_dir, Verdict::Redownload);
    add(&vhdx, Verdict::Safe);
    add(&node_modules, Verdict::Safe);
    add(&unknown, Verdict::Safe);
    add(&keep_me, Verdict::Keep);
    let p = proposals::create_with(&fx.db, &fx.env, req).unwrap();

    let check = |path: &Path, verdict: Verdict, cap: Option<Verdict>| {
        let it = item(&p, path);
        assert_eq!(
            (it.verdict, it.cap),
            (Some(verdict), cap),
            "{}",
            path.display()
        );
    };
    check(&blob, Verdict::Review, Some(Verdict::Review));
    check(&model_dir, Verdict::Redownload, None);
    check(&vhdx, Verdict::Review, Some(Verdict::Review));
    check(&node_modules, Verdict::Safe, None);
    check(&unknown, Verdict::Review, Some(Verdict::Review));
    check(&keep_me, Verdict::Keep, Some(Verdict::Review));
    for it in &p.items {
        assert!(it.verdict >= it.cap, "{}", it.path);
    }
    assert!(!item(&p, &keep_me).selected);
    assert!(nm.exists());

    let conn = fx.db.conn().unwrap();
    assert_eq!(
        cap_for_with(&conn, &fx.env, &node_modules, None, true).unwrap(),
        Some(Verdict::Keep)
    );
    assert_eq!(
        cap_for_with(&conn, &fx.env, &fx.profile, None, false).unwrap(),
        Some(Verdict::Keep)
    );
}

// Traces: POLICY-DEFAULT-TICK
#[test]
fn small_means_under_50_mib_and_under_2_percent_of_the_card() {
    const MIB: i64 = 1024 * 1024;
    assert_eq!(SMALL_BYTES, 50 * MIB);
    let huge = 100_000 * MIB;
    assert!(is_small(50 * MIB - 1, huge));
    assert!(!is_small(50 * MIB, huge), "50 MiB itself is not small");
    assert!(!is_small(50 * MIB + 1, huge));
    assert!(is_small(1_999, 100_000));
    assert!(!is_small(2_000, 100_000), "exactly 2% is not small");
    assert!(!is_small(2_001, 100_000));
    // A card made only of small files still ticks its biggest ones.
    assert!(!is_small(3 * MIB, 3 * MIB + 10 * 4096));
    assert!(is_small(4096, 3 * MIB + 10 * 4096));
    assert!(!is_small(0, 0));
}

// Traces: POLICY-DEFAULT-TICK
#[test]
fn only_unblocked_safe_or_redownload_items_that_are_not_small_start_ticked() {
    for v in [Verdict::Safe, Verdict::Redownload] {
        assert!(default_tick(false, Some(v), false), "{v:?}");
        assert!(!default_tick(true, Some(v), false), "blocked {v:?}");
        assert!(!default_tick(false, Some(v), true), "small {v:?}");
    }
    for v in [Some(Verdict::Review), Some(Verdict::Keep), None] {
        assert!(!default_tick(false, v, false), "{v:?}");
    }
}

// Traces: POLICY-DEFAULT-TICK
#[test]
fn the_capped_verdict_decides_the_default_tick() {
    let fx = Fx::new();
    let chunk = data(64 * 1024, 1);
    fx.file(r"a\node_modules\pkg\index.js", &chunk);
    fx.file(r"b\node_modules\pkg\index.js", &chunk);
    fx.file(r"c\node_modules\pkg\index.js", &chunk);
    fx.file(r"tiny\node_modules\pkg\index.js", &data(4096, 2));
    fx.pfile(r".cache\huggingface\hub\models--org--m\blobs\0123", &chunk);
    let mystery = fx.file("mystery.bin", &chunk);
    let safe_nm = fx.work.join(r"a\node_modules");
    let review_nm = fx.work.join(r"b\node_modules");
    let bare_nm = fx.work.join(r"c\node_modules");
    let tiny_nm = fx.work.join(r"tiny\node_modules");
    let model = fx.profile.join(r".cache\huggingface\hub\models--org--m");

    let mut req = fx.request(ProposalKind::Delete, &[], None);
    for (p, v) in [
        (&safe_nm, Some(Verdict::Safe)),
        (&review_nm, Some(Verdict::Review)),
        (&bare_nm, None),
        (&tiny_nm, Some(Verdict::Safe)),
        (&model, Some(Verdict::Redownload)),
        (&mystery, Some(Verdict::Safe)),
    ] {
        req.items.push(NewItem {
            path: s(p),
            verdict: v,
            reason: None,
        });
    }
    let p = proposals::create_with(&fx.db, &fx.env, req).unwrap();
    assert!(p.items.iter().all(|i| i.status == "pending"));
    let ticked = |path: &Path| item(&p, path).selected;

    assert!(ticked(&safe_nm));
    assert!(ticked(&model), "redownload, no cap");
    assert!(!ticked(&review_nm), "the model said review");
    assert!(!ticked(&bare_nm), "no verdict counts as review");
    assert_eq!(item(&p, &mystery).verdict, Some(Verdict::Review));
    assert!(!ticked(&mystery), "safe, capped to review");
    let tiny = item(&p, &tiny_nm);
    assert_eq!(tiny.verdict, Some(Verdict::Safe));
    assert!(tiny.small && !tiny.selected, "{tiny:#?}");
    assert!(
        p.items
            .iter()
            .filter(|i| i.path != tiny.path)
            .all(|i| !i.small)
    );
}

// Traces: POLICY-ITEM-MEMORY
#[test]
fn each_item_carries_the_most_specific_non_protect_note_and_protect_still_refuses() {
    let fx = Fx::new();
    let conn = fx.db.conn().unwrap();
    let profile = s(&fx.profile);
    let old = fx.file(r"scratch\old\a.bin", &data(8192, 1));
    let loose = fx.file(r"scratch\b.bin", &data(8192, 2));
    let kept = fx.file(r"keep\d.bin", &data(8192, 3));
    let elsewhere = write(&fx.root().join(r"elsewhere\c.bin"), &data(8192, 4));
    let note = |key: &Path, kind: Kind, text: &str| {
        remember_with(
            &conn,
            NewMemory::new(Scope::Path, &s(key), kind, text, "user"),
            1,
            &profile,
        )
        .unwrap()
    };
    let broad = note(&fx.work, Kind::Fact, "work folder");
    let scratch = note(
        &fx.work.join("scratch"),
        Kind::Fact,
        "scratch is disposable",
    );
    note(
        &fx.work.join(r"scratch\old"),
        Kind::Preference,
        "old builds, fine to drop",
    );
    note(&fx.work.join("keep"), Kind::Fact, "photos live here");
    note(&fx.work.join("keep"), Kind::Protect, "family photos");

    let before = hoardini_core::store::now_ms();
    let p = fx
        .propose(
            ProposalKind::Delete,
            &[&old, &loose, &elsewhere, &kept],
            None,
        )
        .unwrap();
    assert_eq!(
        item(&p, &old).memory_note.as_deref(),
        Some("old builds, fine to drop")
    );
    assert_eq!(
        item(&p, &loose).memory_note.as_deref(),
        Some("scratch is disposable")
    );
    assert_eq!(item(&p, &elsewhere).memory_note, None);
    assert_eq!(p.items.len(), 3);
    let refused = p
        .dropped
        .iter()
        .find(|d| d.path.eq_ignore_ascii_case(&s(&kept)))
        .expect("the protected file is refused");
    assert!(
        refused.reason.contains("family photos"),
        "{}",
        refused.reason
    );

    let used = |id: i64| memory::get(&conn, id).unwrap().unwrap().last_used_ms;
    assert!(
        used(scratch.id).unwrap() >= before,
        "the shown note counts as used"
    );
    assert_eq!(
        used(broad.id),
        Some(1),
        "a covering note that is not shown stays unused"
    );
}

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hoardini_core::actions::Executor;
use hoardini_core::destinations::rclone::{
    CONFIG_PASS_REF, Endpoint, RcloneDaemon, daemon_args, test_at,
};
use hoardini_core::destinations::{self, Destination, NewDestination};
use hoardini_core::proposals::ProposalKind;
use hoardini_core::runtime::{self, RCLONE};
use hoardini_core::vault::{SecretStore, SecretString};
use serde_json::{Value, json};

use super::fake_rc::{FakeRc, Settings};
use super::*;

struct Rig {
    fx: Fx,
    fake: FakeRc,
    ex: Executor,
    dest: Destination,
}

fn rig(settings: Settings) -> Rig {
    let fx = Fx::new();
    let fake = FakeRc::start(&fx.root().join("fake-remote"), settings);
    let ex = fx.executor();
    ex.set_rclone_endpoint(Some(fake.ep.clone()));
    let dest = destinations::create(
        &fx.db.conn().unwrap(),
        NewDestination {
            name: "cloud".into(),
            kind: "rclone".into(),
            config: json!({"remote": "fake", "type": "s3", "path": "hoardini", "params": {"region": "eu"}}),
        },
    )
    .unwrap();
    Rig { fx, fake, ex, dest }
}

fn sha256_only() -> Settings {
    Settings {
        hashes: vec!["md5".into(), "sha256".into()],
        ..Settings::default()
    }
}

fn no_hashes() -> Settings {
    Settings {
        hashes: Vec::new(),
        ..Settings::default()
    }
}

fn remote_file(r: &Rig, uri: &str) -> PathBuf {
    let path = uri
        .strip_prefix("rclone:fake:")
        .unwrap_or_else(|| panic!("{uri}"));
    r.fake.root.join(path)
}

fn run_archive(
    r: &Rig,
    paths: &[&Path],
) -> (
    hoardini_core::actions::Job,
    hoardini_core::proposals::Proposal,
) {
    let p =
        r.fx.propose(ProposalKind::Archive, paths, Some(&r.dest.id))
            .unwrap();
    let a = r.ex.approve(&p.id, &ids(&p)).unwrap();
    let job = wait(&r.ex, &a.job_id);
    (job, r.fx.get(&p.id))
}

fn item_details(p: &hoardini_core::proposals::Proposal) -> Vec<String> {
    p.items
        .iter()
        .map(|i| i.detail.clone().unwrap_or_default())
        .collect()
}

fn archive_and_restore(settings: Settings, expect_hash_listing: bool) {
    let r = rig(settings);
    let original = data(3 * 1024 * 1024 + 17, 7);
    let file = r.fx.file(r"docs\model.gguf", &original);
    let dir = make_tree(&r.fx, "project");
    let dir_before = tree(&dir);

    let (job, p) = run_archive(&r, &[&file, &dir]);
    assert_eq!(job.status, "done", "{:?} {:?}", job.error, item_details(&p));
    assert!(!file.exists() && !dir.exists());
    assert_eq!(job.progress.done_bytes, job.progress.total_bytes);

    let fe = entry_for(&r.fx, &file);
    assert!(
        fe.dest_uri.starts_with("rclone:fake:hoardini/"),
        "{}",
        fe.dest_uri
    );
    assert!(
        fe.dest_uri.ends_with("/work/docs/model.gguf"),
        "{}",
        fe.dest_uri
    );
    assert_eq!(
        std::fs::read(remote_file(&r, &fe.dest_uri)).unwrap(),
        original
    );
    assert_eq!(entry_blake3(&r.fx, fe.id), blake3_hex(&original));
    let de = entry_for(&r.fx, &dir);
    assert_eq!(tree(&remote_file(&r, &de.dest_uri)), dir_before);

    let copies: Vec<Value> = r
        .fake
        .calls("operations/copyfile")
        .into_iter()
        .chain(r.fake.calls("sync/copy"))
        .collect();
    assert_eq!(copies.len(), 2);
    for c in &copies {
        assert_eq!(c["_async"], true);
        assert!(
            c["_group"].as_str().unwrap().starts_with("hoardini-a"),
            "{c}"
        );
    }
    let hashed_list = r
        .fake
        .calls("operations/list")
        .iter()
        .any(|c| c["opt"]["showHash"] == true && c["opt"]["hashTypes"] == json!(["sha256"]));
    assert_eq!(hashed_list, expect_hash_listing);
    assert_eq!(
        !r.fake.calls("operations/hashsum").is_empty(),
        !expect_hash_listing
    );
    assert_eq!(r.fake.bad_auth(), 0);
    assert!(r.fake.methods().len() > 6, "requests reached the fake");

    assert_eq!(wait(&r.ex, &r.ex.restore(fe.id).unwrap()).status, "done");
    assert_eq!(std::fs::read(&file).unwrap(), original);
    assert_eq!(wait(&r.ex, &r.ex.restore(de.id).unwrap()).status, "done");
    assert_eq!(tree(&dir), dir_before);
    assert!(entry_for(&r.fx, &file).restored_ms.is_some());
    assert!(
        remote_file(&r, &fe.dest_uri).exists(),
        "the archived copy is kept"
    );
}

// Traces: ACT-RCLONE-VERIFY
#[test]
fn archive_to_rclone_deletes_after_the_remote_hash_matches_and_restores() {
    archive_and_restore(sha256_only(), true);
}

// Traces: ACT-RCLONE-VERIFY
#[test]
fn a_remote_without_hashes_is_verified_by_reading_it_back() {
    archive_and_restore(no_hashes(), false);
}

fn tampered_archive_keeps_sources(settings: Settings) {
    let r = rig(Settings {
        tamper: true,
        ..settings
    });
    let victim = data(500_000, 9);
    let file = r.fx.file(r"docs\second.bin", &victim);
    let dir = make_tree(&r.fx, "project2");
    let dir_before = tree(&dir);
    let (job, p) = run_archive(&r, &[&file, &dir]);
    assert_eq!(job.status, "failed");
    assert_eq!(p.status, "failed");
    for it in &p.items {
        assert_eq!(it.status, "failed");
        assert!(
            it.detail
                .as_deref()
                .unwrap()
                .contains("verification failed"),
            "{:?}",
            it.detail
        );
    }
    assert_eq!(std::fs::read(&file).unwrap(), victim);
    assert_eq!(tree(&dir), dir_before);
    assert!(
        entries(&r.fx).is_empty(),
        "no archive entry for an unverified copy"
    );
    assert_eq!(job.freed_bytes, 0);
    assert!(
        r.fake.remote_files().is_empty(),
        "the bad copies are removed"
    );
}

// Traces: ACT-RCLONE-VERIFY
#[test]
fn a_remote_hash_mismatch_fails_the_item_and_keeps_the_source() {
    tampered_archive_keeps_sources(sha256_only());
}

// Traces: ACT-RCLONE-VERIFY
#[test]
fn a_read_back_mismatch_fails_the_item_and_keeps_the_source() {
    tampered_archive_keeps_sources(no_hashes());
}

// Traces: ACT-RCLONE-VERIFY
#[test]
fn a_second_archive_of_the_same_path_keeps_the_first_remote_copy() {
    let r = rig(sha256_only());
    let v1 = data(300_000, 1);
    let file = r.fx.file(r"dl\setup.exe", &v1);
    assert_eq!(run_archive(&r, &[&file]).0.status, "done");
    let first = entry_for(&r.fx, &file);
    r.fx.file(r"dl\setup.exe", &data(200_000, 2));
    assert_eq!(run_archive(&r, &[&file]).0.status, "done");
    let all = entries(&r.fx);
    assert_eq!(all.len(), 2);
    assert_ne!(all[0].dest_uri, all[1].dest_uri, "{all:#?}");
    assert_eq!(std::fs::read(remote_file(&r, &first.dest_uri)).unwrap(), v1);
    assert_eq!(wait(&r.ex, &r.ex.restore(first.id).unwrap()).status, "done");
    assert_eq!(std::fs::read(&file).unwrap(), v1);
}

// Traces: ACT-RCLONE-VERIFY
#[test]
fn a_source_that_changes_after_it_was_hashed_is_kept() {
    let fx_file: std::sync::Arc<parking_lot::Mutex<Option<PathBuf>>> = Default::default();
    let hook_file = fx_file.clone();
    let r = rig(Settings {
        after_upload: Some(Box::new(move || {
            if let Some(p) = hook_file.lock().as_ref() {
                let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
                std::io::Write::write_all(&mut f, b"!").unwrap();
            }
        })),
        ..sha256_only()
    });
    let file = r.fx.file(r"docs\busy.bin", &data(200_000, 3));
    *fx_file.lock() = Some(file.clone());
    let (job, p) = run_archive(&r, &[&file]);
    assert_eq!(job.status, "failed");
    assert!(
        item_details(&p)[0].contains("changed while"),
        "{:?}",
        item_details(&p)
    );
    assert_eq!(std::fs::read(&file).unwrap().len(), 200_001);
    assert!(entries(&r.fx).is_empty());
}

// Traces: ACT-RCLONE-PROGRESS
#[test]
fn copy_progress_comes_from_the_job_group_and_cancel_stops_the_job() {
    let r = rig(Settings {
        slow_polls: 8,
        ..sha256_only()
    });
    let mut rx = r.ex.subscribe();
    let file = r.fx.file(r"docs\big.bin", &data(900_000, 4));
    let (job, _) = run_archive(&r, &[&file]);
    assert_eq!(job.status, "done", "{:?}", job.error);
    let mut seen = Vec::new();
    while let Ok(j) = rx.try_recv() {
        if j.id == job.id {
            seen.push(j.progress.done_bytes);
        }
    }
    let total = job.progress.total_bytes;
    assert!(
        seen.iter().any(|b| *b > 0 && *b < total),
        "mid-copy progress {seen:?} of {total}"
    );
    assert!(
        r.fake
            .calls("core/stats")
            .iter()
            .all(|c| c["group"].as_str().unwrap().starts_with("hoardini-a"))
    );

    let r = rig(Settings {
        slow_polls: 10_000,
        ..sha256_only()
    });
    let file = r.fx.file(r"docs\cancel.bin", &data(100_000, 5));
    let p =
        r.fx.propose(ProposalKind::Archive, &[&file], Some(&r.dest.id))
            .unwrap();
    let a = r.ex.approve(&p.id, &ids(&p)).unwrap();
    let start = Instant::now();
    while r.fake.calls("job/status").len() < 2 {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "the copy never started"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(r.ex.cancel(&a.job_id));
    assert_eq!(wait(&r.ex, &a.job_id).status, "cancelled");
    assert_eq!(r.fake.calls("job/stop").len(), 1);
    assert!(file.exists());
    assert!(entries(&r.fx).is_empty());
}

// Traces: ACT-RCLONE-SECRETS
#[test]
fn secrets_stay_in_the_vault_and_reach_rclone_only_when_the_remote_is_created() {
    let r = rig(sha256_only());
    r.ex.secrets
        .set(
            "secret://destination/s3/secret",
            SecretString::from("SECRET-VALUE-123"),
        )
        .unwrap();
    r.ex.secrets
        .set(
            "secret://destination/s3/key",
            SecretString::from("AKIA-VALUE-456"),
        )
        .unwrap();
    let conn = r.fx.db.conn().unwrap();
    let plain = destinations::create(
        &conn,
        NewDestination {
            name: "bad".into(),
            kind: "rclone".into(),
            config: json!({"remote": "bad", "type": "s3", "params": {"secret_access_key": "SECRET-VALUE-123"}}),
        },
    );
    assert!(plain.unwrap_err().to_string().contains("secret://"));
    let dest = destinations::create(
        &conn,
        NewDestination {
            name: "s3".into(),
            kind: "rclone".into(),
            config: json!({"remote": "fake", "type": "s3", "path": "bk", "params": {
                "secret_access_key": "secret://destination/s3/secret",
                "access_key_id": "secret://destination/s3/key",
                "region": "eu-west-1"}}),
        },
    )
    .unwrap();
    let stored = destinations::get(&conn, &dest.id)
        .unwrap()
        .unwrap()
        .config
        .to_string();
    assert!(stored.contains("secret://destination/s3/secret"));
    assert!(!stored.contains("SECRET-VALUE-123") && !stored.contains("AKIA-VALUE-456"));

    let t = test_at(&dest, &*r.ex.secrets, Some(&r.fake.ep));
    assert!(t.ok, "{t:?}");
    let created = r.fake.created();
    assert_eq!(created.len(), 1);
    assert_eq!(
        (created[0].0.as_str(), created[0].1.as_str()),
        ("fake", "s3")
    );
    assert_eq!(created[0].2["secret_access_key"], "SECRET-VALUE-123");
    assert_eq!(created[0].2["access_key_id"], "AKIA-VALUE-456");
    assert_eq!(created[0].2["region"], "eu-west-1");

    let drive = destinations::create(
        &conn,
        NewDestination {
            name: "gd".into(),
            kind: "rclone".into(),
            config: json!({"remote": "gd", "type": "drive"}),
        },
    );
    assert!(drive.unwrap_err().to_string().contains("OAuth client"));
}

// Traces: ACT-RCLONE-TEST
#[test]
fn testing_a_destination_reports_free_space_or_probes_with_a_write() {
    let r = rig(sha256_only());
    let t = test_at(&r.dest, &*r.ex.secrets, Some(&r.fake.ep));
    assert!(t.ok, "{t:?}");
    assert_eq!(t.free_bytes, Some(5_000_000_000));

    let r = rig(Settings {
        about_free: None,
        ..sha256_only()
    });
    let t = test_at(&r.dest, &*r.ex.secrets, Some(&r.fake.ep));
    assert!(t.ok && t.free_bytes.is_none(), "{t:?}");
    let methods = r.fake.methods();
    let put = methods
        .iter()
        .position(|m| m == "operations/copyfile")
        .unwrap();
    let del = methods
        .iter()
        .position(|m| m == "operations/deletefile")
        .unwrap();
    assert!(put < del, "the probe file is written, then removed");
    assert!(r.fake.remote_files().is_empty());

    let wrong = Endpoint {
        pass: "wrong".into(),
        ..r.fake.ep.clone()
    };
    let t = test_at(&r.dest, &*r.ex.secrets, Some(&wrong));
    assert!(!t.ok);
    assert!(r.fake.bad_auth() > 0);
}

// Traces: ACT-RCLONE-DAEMON
#[test]
fn the_daemon_listens_on_loopback_with_random_credentials_and_auth_on() {
    let conf = Path::new(r"C:\data\rclone.conf");
    let a = Endpoint::random(40123);
    let b = Endpoint::random(40123);
    assert_ne!(a.user, b.user);
    assert_ne!(a.pass, b.pass);
    assert!(a.pass.len() >= 32);
    let args: Vec<String> = daemon_args(&a, conf)
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    assert_eq!(args[0], "rcd");
    let at = |flag: &str| args[args.iter().position(|x| x == flag).expect(flag) + 1].clone();
    assert_eq!(at("--rc-addr"), "127.0.0.1:40123");
    assert_eq!(at("--rc-user"), a.user);
    assert_eq!(at("--rc-pass"), a.pass);
    assert_eq!(at("--config"), r"C:\data\rclone.conf");
    assert!(!args.iter().any(|x| x == "--rc-no-auth"));
    assert!(
        !format!("{a:?}").contains(&a.pass),
        "credentials stay out of Debug output"
    );
}

// Traces: ACT-RCLONE-DAEMON
#[test]
fn a_binary_that_fails_verification_is_never_launched() {
    let root = tempfile::tempdir().unwrap();
    let exe = runtime::entry_path(&RCLONE, root.path());
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"not rclone").unwrap();
    let daemon = RcloneDaemon::new(root.path()).without_auto_install();
    let vault = hoardini_core::vault::DpapiVault::open(&root.path().join("v")).unwrap();
    let err = daemon
        .client(&vault)
        .err()
        .expect("must refuse")
        .to_string();
    assert!(
        err.contains("install record") || err.contains("SHA-256"),
        "{err}"
    );
    assert!(!daemon.is_running());
    assert!(
        !daemon.config_path().exists(),
        "no config was written for it"
    );
    assert!(vault.get(CONFIG_PASS_REF).unwrap().is_none());
}

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hoardini_core::index::{FakeFs, RefreshOptions, index_status, refresh};
use hoardini_core::proposals::{self, NewSystemCleanup, ProposalKind};
use hoardini_core::store::Db;
use hoardini_core::syscleanup::{
    self, CleanupCandidate, Cleared, OpResult, SystemFacts, SystemOp, WslDisk, clear_contents,
    count_shadows, diskpart_scripts, helper_params, leftover_targets, parse_helper_args,
};
use hoardini_win::RecycleBin;

use super::action_rows;
use super::support::*;

const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;
const LOCAL: &str = r"C:\Users\me\AppData\Local";

fn indexed(db: &Db, busy: bool) {
    let fs = FakeFs::new(r"C:\");
    if busy {
        fs.file(r"C:\Windows\WinSxS\amd64_old\a.dll", 6 * GIB);
        fs.file(r"C:\hiberfil.sys", 25 * GIB);
        fs.file(
            &format!(r"{LOCAL}\Packages\Ubuntu\LocalState\ext4.vhdx"),
            60 * GIB,
        );
        fs.file(
            &format!(r"{LOCAL}\Docker\wsl\disk\docker_data.vhdx"),
            40 * GIB,
        );
        fs.file(&format!(r"{LOCAL}\Docker\wsl\main\ext4.vhdx"), GIB);
        fs.set_volume(1 << 40, 800 * GIB);
    } else {
        fs.file(r"C:\Windows\WinSxS\amd64_old\a.dll", GIB);
        fs.file(r"C:\hiberfil.sys", 25 * GIB);
        fs.set_volume(1 << 40, (1 << 40) - 26 * GIB - 512 * MIB);
    }
    let out = refresh(&db.conn().unwrap(), &fs, r"C:\", &RefreshOptions::default()).unwrap();
    assert!(out.completed);
}

fn busy_facts() -> SystemFacts {
    SystemFacts {
        system_drive: r"C:\".into(),
        windows_dir: r"C:\Windows".into(),
        ram_bytes: Some(64 * GIB),
        hibernate_enabled: Some(true),
        recycle_bin: Some(RecycleBin {
            bytes: 3 * GIB,
            items: 12,
        }),
        wsl_disks: vec![WslDisk {
            distro: "Ubuntu".into(),
            path: format!(r"{LOCAL}\Packages\Ubuntu\LocalState\ext4.vhdx"),
        }],
        docker_dir: Some(format!(r"{LOCAL}\Docker\wsl")),
    }
}

// Traces: ACT-WINDOWS-CLEANUP-DETECT
#[test]
fn cleanups_are_detected_from_the_index_and_facts_read_without_elevation() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("h.db")).unwrap();
    indexed(&db, true);
    let conn = db.conn().unwrap();
    let unseen = index_status(&conn, Some(r"C:\"))
        .unwrap()
        .unwrap()
        .unaccounted_bytes
        .unwrap() as u64;
    assert!(unseen > 2 * GIB);
    let got = syscleanup::candidates(&conn, &busy_facts()).unwrap();
    let rows: Vec<(SystemOp, &str, Option<u64>, Option<u64>)> = got
        .iter()
        .map(|c| (c.op.clone(), c.target.as_str(), c.estimate, c.size))
        .collect();
    let ubuntu = format!(r"{LOCAL}\Packages\Ubuntu\LocalState\ext4.vhdx");
    let docker = format!(r"{LOCAL}\Docker\wsl\disk\docker_data.vhdx");
    assert_eq!(
        rows,
        vec![
            (
                SystemOp::ComponentStore,
                r"C:\Windows\WinSxS",
                None,
                Some(6 * GIB)
            ),
            (
                SystemOp::RestorePoints {
                    volume: "C:".into()
                },
                r"C:\System Volume Information",
                None,
                Some(unseen)
            ),
            (
                SystemOp::Hibernation,
                r"C:\hiberfil.sys",
                Some(25 * GIB - 64 * GIB / 5),
                Some(25 * GIB)
            ),
            (
                SystemOp::CompactVhdx {
                    path: ubuntu.clone()
                },
                ubuntu.as_str(),
                None,
                Some(60 * GIB)
            ),
            (
                SystemOp::CompactVhdx {
                    path: docker.clone()
                },
                docker.as_str(),
                None,
                Some(40 * GIB)
            ),
            (
                SystemOp::RecycleBin,
                r"C:\$Recycle.Bin",
                Some(3 * GIB),
                Some(3 * GIB)
            ),
        ]
    );
    assert!(got[3].note.contains("Ubuntu") && got[4].note.contains("Docker Desktop"));
    assert!(got[5].note.starts_with("12 items"), "{}", got[5].note);
}

// Traces: ACT-WINDOWS-CLEANUP-DETECT
#[test]
fn nothing_is_offered_below_the_thresholds_or_when_windows_says_no() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("h.db")).unwrap();
    indexed(&db, false);
    let facts = SystemFacts {
        ram_bytes: Some(128 * GIB),
        recycle_bin: Some(RecycleBin {
            bytes: 500 * MIB,
            items: 3,
        }),
        wsl_disks: Vec::new(),
        docker_dir: None,
        ..busy_facts()
    };
    let conn = db.conn().unwrap();
    assert_eq!(syscleanup::candidates(&conn, &facts).unwrap(), vec![]);
    let off = SystemFacts {
        ram_bytes: Some(16 * GIB),
        hibernate_enabled: Some(false),
        ..facts.clone()
    };
    assert_eq!(syscleanup::candidates(&conn, &off).unwrap(), vec![]);
    let on = SystemFacts {
        hibernate_enabled: None,
        ..off
    };
    let hib = syscleanup::candidates(&conn, &on).unwrap();
    assert_eq!(
        hib.len(),
        1,
        "unknown registry state falls back to the file"
    );
    assert_eq!(hib[0].estimate, Some(25 * GIB - 16 * GIB / 5));
    let ops = syscleanup::candidates(&conn, &facts).unwrap();
    let err = proposals::create_system_cleanup_card(&db, cleanup(ops), None).unwrap_err();
    assert!(
        err.to_string().contains("nothing Windows can clean up"),
        "{err}"
    );
}

fn cleanup(ops: Vec<CleanupCandidate>) -> NewSystemCleanup {
    NewSystemCleanup {
        conversation_id: Some("review".into()),
        tool_call_id: None,
        summary: "Cleanups only Windows can do".into(),
        ops,
        dropped: Vec::new(),
    }
}

// Traces: ACT-WINDOWS-CLEANUP-DETECT
#[test]
fn a_proposal_holds_the_wanted_operations_with_their_views_and_ticks() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("h.db")).unwrap();
    indexed(&db, true);
    let wanted = [
        "recycle_bin",
        "hibernation",
        "restore_points",
        "compact_vhdx",
    ];
    let ops = syscleanup::candidates(&db.conn().unwrap(), &busy_facts())
        .unwrap()
        .into_iter()
        .filter(|c| wanted.contains(&c.op.name()))
        .collect();
    let p = proposals::create_system_cleanup_card(&db, cleanup(ops), None).unwrap();
    assert_eq!(p.kind, ProposalKind::SystemCleanup);
    let items: Vec<(&str, i64, bool, bool, &str)> = p
        .items
        .iter()
        .map(|i| {
            let op = i.op.as_ref().unwrap();
            (
                i.path.as_str(),
                i.alloc,
                i.selected,
                op.irreversible,
                op.op.name(),
            )
        })
        .collect();
    let hib = (25 * GIB - 64 * GIB / 5) as i64;
    assert_eq!(
        items,
        vec![
            (
                r"C:\System Volume Information",
                0,
                false,
                true,
                "restore_points"
            ),
            (r"C:\hiberfil.sys", hib, true, false, "hibernation"),
            (
                r"C:\$Recycle.Bin",
                (3 * GIB) as i64,
                false,
                true,
                "recycle_bin"
            ),
        ]
    );
    assert_eq!(p.total_alloc, hib + (3 * GIB) as i64);
    assert!(p.items.iter().all(|i| !i.small && i.status == "pending"));
    let views: Vec<(bool, bool)> = p
        .items
        .iter()
        .map(|i| {
            let v = i.op.as_ref().unwrap();
            (v.elevated, v.estimate_known)
        })
        .collect();
    assert_eq!(views, vec![(true, false), (true, true), (false, true)]);
    let dropped: Vec<&str> = p.dropped.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(
        dropped.len(),
        2,
        "both vhdx files are missing on disk: {dropped:?}"
    );
    assert!(
        p.dropped
            .iter()
            .all(|d| d.reason.contains("not a virtual disk file"))
    );
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["kind"], "system_cleanup");
    assert_eq!(json["items"][0]["op"]["op"], "restore_points");
    assert_eq!(json["items"][0]["op"]["volume"], "C:");
    assert!(
        json["items"][0]["op"]["explain"]
            .as_str()
            .unwrap()
            .contains("cannot be brought back")
    );
    let normal = fx_request_refused(&db);
    assert!(normal.contains("detected operations"), "{normal}");
}

fn fx_request_refused(db: &Db) -> String {
    let req = proposals::NewProposal {
        conversation_id: None,
        tool_call_id: None,
        kind: ProposalKind::SystemCleanup,
        destination_id: None,
        format: None,
        dedupe: None,
        summary: "x".into(),
        items: vec![proposals::NewItem {
            path: r"C:\Windows\WinSxS".into(),
            verdict: None,
            reason: None,
        }],
    };
    proposals::create(db, req).unwrap_err().to_string()
}

#[derive(Default)]
struct FakeRunner {
    local: Mutex<Vec<SystemOp>>,
    elevated: Mutex<Vec<Vec<SystemOp>>>,
    results: Mutex<Option<anyhow::Result<Vec<OpResult>>>>,
}

impl hoardini_core::actions::SystemRunner for FakeRunner {
    fn local(&self, op: &SystemOp) -> anyhow::Result<OpResult> {
        self.local.lock().unwrap().push(op.clone());
        Ok(OpResult {
            ok: true,
            freed: GIB,
            ..OpResult::default()
        })
    }
    fn elevated(
        &self,
        ops: &[SystemOp],
        done: &mut dyn FnMut(usize),
    ) -> anyhow::Result<Vec<OpResult>> {
        self.elevated.lock().unwrap().push(ops.to_vec());
        done(1);
        done(2);
        self.results.lock().unwrap().take().unwrap()
    }
}

fn ok(freed: u64, detail: Option<&str>) -> OpResult {
    OpResult {
        ok: true,
        freed,
        error: None,
        detail: detail.map(str::to_string),
    }
}

fn five_ops(fx: &Fx) -> (proposals::Proposal, PathBuf) {
    let vhdx = fx.file(r"wsl\ext4.vhdx", b"vhdxfile");
    let c = |op: SystemOp, target: &str, est: Option<u64>| CleanupCandidate {
        op,
        target: target.into(),
        estimate: est,
        size: None,
        note: String::new(),
        parts: Vec::new(),
    };
    let p = proposals::create_system_cleanup_card(
        &fx.db,
        NewSystemCleanup {
            conversation_id: Some("review".into()),
            tool_call_id: None,
            summary: "Cleanups only Windows can do".into(),
            ops: vec![
                c(SystemOp::ComponentStore, r"C:\Windows\WinSxS", None),
                c(
                    SystemOp::RestorePoints {
                        volume: "C:".into(),
                    },
                    r"C:\System Volume Information",
                    None,
                ),
                c(SystemOp::Hibernation, r"C:\hiberfil.sys", Some(12 * GIB)),
                c(SystemOp::CompactVhdx { path: s(&vhdx) }, &s(&vhdx), None),
                c(SystemOp::RecycleBin, r"C:\$Recycle.Bin", Some(3 * GIB)),
            ],
            dropped: Vec::new(),
        },
        None,
    )
    .unwrap();
    (p, vhdx)
}

// Traces: ACT-WINDOWS-CLEANUP-RUN, ACT-JOURNAL
#[test]
fn one_prompt_per_job_runs_every_elevated_operation_and_journals_each() {
    let fx = Fx::new();
    let (p, vhdx) = five_ops(&fx);
    let ex = fx.executor();
    let runner = Arc::new(FakeRunner::default());
    *runner.results.lock().unwrap() = Some(Ok(vec![
        ok(3 * GIB, Some("Windows asks for a restart to finish")),
        ok(5 * GIB, Some("4 older restore points deleted")),
        ok(12 * GIB, None),
        OpResult::failed("diskpart exited with exit code: 2: the disk is in use"),
    ]));
    ex.set_system_runner(Some(runner.clone()));
    let mut rx = ex.subscribe();
    let r = ex.approve(&p.id, &ids(&p)).unwrap();
    assert_eq!(r.items, ids(&p), "operations are not revalidated as files");
    loop {
        let job = rx.blocking_recv().unwrap();
        let now = fx.get(&p.id);
        let rows = action_rows(&fx, &p.id);
        for it in now.items.iter().filter(|i| i.status == "done") {
            let row = rows.iter().find(|r| r.0 == it.id).expect("journal row");
            assert_eq!(row.1, "done");
            assert!(row.3.is_some(), "undo info for {}", it.path);
        }
        if job.id == r.job_id && matches!(job.status.as_str(), "done" | "failed" | "cancelled") {
            break;
        }
    }
    let job = wait(&ex, &r.job_id);
    assert_eq!(*runner.local.lock().unwrap(), vec![SystemOp::RecycleBin]);
    assert_eq!(
        *runner.elevated.lock().unwrap(),
        vec![vec![
            SystemOp::ComponentStore,
            SystemOp::RestorePoints {
                volume: "C:".into()
            },
            SystemOp::Hibernation,
            SystemOp::CompactVhdx { path: s(&vhdx) },
        ]]
    );
    let done = fx.get(&p.id);
    assert_eq!(done.status, "partial");
    let states: Vec<(&str, Option<&str>)> = done
        .items
        .iter()
        .map(|i| (i.status.as_str(), i.detail.as_deref()))
        .collect();
    assert_eq!(
        states,
        vec![
            ("done", Some("Windows asks for a restart to finish")),
            ("done", Some("4 older restore points deleted")),
            ("done", None),
            (
                "failed",
                Some("diskpart exited with exit code: 2: the disk is in use")
            ),
            ("done", None),
        ]
    );
    assert_eq!(job.status, "done");
    assert_eq!(job.freed_bytes, (21 * GIB) as i64);
    let rows = action_rows(&fx, &p.id);
    assert_eq!(rows.len(), 5);
    let undo = |i: usize| -> serde_json::Value {
        let id = done.items[i].id;
        let row = rows.iter().find(|r| r.0 == id).unwrap();
        serde_json::from_str(row.3.as_deref().unwrap_or("null")).unwrap()
    };
    assert_eq!(
        undo(1)["reason"],
        "deleted restore points cannot be brought back"
    );
    assert_eq!(
        undo(2)["argv"],
        serde_json::json!(["powercfg.exe", "/h", "/type", "full"])
    );
    assert_eq!(
        undo(3),
        serde_json::Value::Null,
        "a failed item has no undo"
    );
    let freed: Vec<i64> = done
        .items
        .iter()
        .map(|it| rows.iter().find(|r| r.0 == it.id).unwrap().2)
        .collect();
    assert_eq!(
        freed,
        vec![
            (3 * GIB) as i64,
            (5 * GIB) as i64,
            (12 * GIB) as i64,
            0,
            GIB as i64
        ]
    );
    assert!(
        done.follow_up_id.is_none(),
        "system cleanups get no follow-up"
    );
}

// Traces: ACT-WINDOWS-CLEANUP-RUN
#[test]
fn a_declined_prompt_fails_the_elevated_operations_only() {
    let fx = Fx::new();
    let (p, _) = five_ops(&fx);
    let ex = fx.executor();
    let runner = Arc::new(FakeRunner::default());
    *runner.results.lock().unwrap() = Some(Err(anyhow::anyhow!(
        "The administrator prompt was declined; nothing ran"
    )));
    ex.set_system_runner(Some(runner.clone()));
    let job = wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id);
    let after = fx.get(&p.id);
    assert_eq!(after.status, "partial");
    for it in &after.items[..4] {
        assert_eq!(it.status, "failed");
        assert_eq!(
            it.detail.as_deref(),
            Some("The administrator prompt was declined; nothing ran")
        );
    }
    assert_eq!(after.items[4].status, "done");
    assert_eq!(job.freed_bytes, GIB as i64);
    let failed = count(
        &fx.db,
        "SELECT COUNT(*) FROM action WHERE status = 'failed' AND error LIKE '%declined%'",
    );
    assert_eq!(failed, 4);
}

// Traces: ACT-WINDOWS-CLEANUP-RUN
#[test]
fn a_job_with_only_the_recycle_bin_never_asks_for_admin_rights() {
    let fx = Fx::new();
    let (p, _) = five_ops(&fx);
    let bin = p
        .items
        .iter()
        .find(|i| i.path == r"C:\$Recycle.Bin")
        .unwrap()
        .id;
    let ex = fx.executor();
    let runner = Arc::new(FakeRunner::default());
    ex.set_system_runner(Some(runner.clone()));
    let job = wait(&ex, &ex.approve(&p.id, &[bin]).unwrap().job_id);
    assert_eq!(job.status, "done");
    assert!(runner.elevated.lock().unwrap().is_empty());
    assert_eq!(fx.get(&p.id).status, "done");
}

// Traces: ACT-WINDOWS-CLEANUP-RUN, ACT-WINDOWS-CLEANUP-DETECT, REVIEW-BUILD
#[test]
fn a_review_cards_leftovers_go_to_the_helper_by_name_not_by_path() {
    let fx = Fx::new();
    let parts = vec![
        syscleanup::OpPart {
            path: r"C:\Windows\Temp\setup".into(),
            size: 2 * GIB,
        },
        syscleanup::OpPart {
            path: r"C:\Windows\SoftwareDistribution\Download\abc".into(),
            size: 3 * GIB,
        },
    ];
    let leftovers = syscleanup::leftovers_candidate(parts).unwrap();
    assert_eq!(
        leftovers.target,
        r"C:\Windows\SoftwareDistribution\Download\abc"
    );
    let card = proposals::ReviewCard {
        review_id: "r1".into(),
        card: "windows".into(),
        ticked: true,
    };
    let p = proposals::create_system_cleanup_card(
        &fx.db,
        NewSystemCleanup {
            conversation_id: Some("review".into()),
            tool_call_id: None,
            summary: "Cleanups only Windows can do".into(),
            ops: vec![leftovers],
            dropped: Vec::new(),
        },
        Some(&card),
    )
    .unwrap();
    assert_eq!(
        (p.review_id.as_deref(), p.card.as_deref()),
        (Some("r1"), Some("windows"))
    );
    let it = &p.items[0];
    assert!(it.selected, "safe, so it starts ticked");
    assert_eq!(it.alloc, (5 * GIB) as i64);
    assert_eq!(it.op.as_ref().unwrap().parts.len(), 2);
    let ex = fx.executor();
    let runner = Arc::new(FakeRunner::default());
    *runner.results.lock().unwrap() = Some(Ok(vec![ok(
        4 * GIB,
        Some("12 files deleted (4.0 GB); left: 3 in use"),
    )]));
    ex.set_system_runner(Some(runner.clone()));
    let job = wait(&ex, &ex.approve(&p.id, &ids(&p)).unwrap().job_id);
    assert_eq!(
        *runner.elevated.lock().unwrap(),
        vec![vec![SystemOp::Leftovers]]
    );
    assert_eq!(job.freed_bytes, (4 * GIB) as i64);
    let done = fx.get(&p.id);
    assert_eq!(done.status, "done");
    assert_eq!(
        done.items[0].detail.as_deref(),
        Some("12 files deleted (4.0 GB); left: 3 in use")
    );
    let line = helper_params(Path::new(r"C:\r.jsonl"), &[SystemOp::Leftovers]).unwrap();
    assert_eq!(line, r#"system-cleanup --result "C:\r.jsonl" "leftovers""#);
}

// Traces: ACT-WINDOWS-CLEANUP-RUN
#[test]
fn the_elevated_helper_gets_only_known_operations_on_its_command_line() {
    let result = Path::new(r"C:\Users\me\AppData\Local\Temp\hoardini-cleanup-1.jsonl");
    let ops = vec![
        SystemOp::ComponentStore,
        SystemOp::RestorePoints {
            volume: "D:".into(),
        },
        SystemOp::Hibernation,
        SystemOp::CompactVhdx {
            path: r"C:\wsl\Ubuntu Disk\ext4.vhdx".into(),
        },
        SystemOp::Leftovers,
    ];
    let line = helper_params(result, &ops).unwrap();
    assert_eq!(
        line,
        r#"system-cleanup --result "C:\Users\me\AppData\Local\Temp\hoardini-cleanup-1.jsonl" "component-store" "restore-points=D:" "hibernation-reduced" "compact-vhdx=C:\wsl\Ubuntu Disk\ext4.vhdx" "leftovers""#
    );
    assert!(helper_params(result, &[SystemOp::RecycleBin]).is_err());
    let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let (r, parsed) = parse_helper_args(&args(&[
        "--result",
        &result.to_string_lossy(),
        "component-store",
        "restore-points=D:",
        "hibernation-reduced",
        r"compact-vhdx=C:\wsl\Ubuntu Disk\ext4.vhdx",
        "leftovers",
    ]))
    .unwrap();
    assert_eq!(r, result);
    assert_eq!(parsed, ops);
    for bad in [
        "recycle-bin",
        "format-c",
        "restore-points=C:\\",
        r"compact-vhdx=C:\wsl\notes.txt",
        r"compact-vhdx=..\x.vhdx",
        r"compact-vhdx=\\server\share\x.vhdx",
        r"leftovers=C:\Users\me\Documents",
        r"leftovers:C:\Users",
        "clear-folder=C:\\Users",
    ] {
        assert!(
            parse_helper_args(&args(&["--result", r"C:\r.jsonl", bad])).is_err(),
            "{bad}"
        );
    }
    assert!(parse_helper_args(&args(&["--result", "relative.jsonl"])).is_err());
}

// Traces: ACT-WINDOWS-CLEANUP-RUN
#[test]
fn diskpart_gets_fixed_scripts_for_one_quoted_disk_path() {
    let (compact, detach) = diskpart_scripts(r"C:\wsl\Ubuntu Disk\ext4.vhdx").unwrap();
    assert_eq!(
        compact,
        "select vdisk file=\"C:\\wsl\\Ubuntu Disk\\ext4.vhdx\"\nattach vdisk readonly\ncompact vdisk\n"
    );
    assert_eq!(
        detach,
        "select vdisk file=\"C:\\wsl\\Ubuntu Disk\\ext4.vhdx\"\ndetach vdisk\n"
    );
    for bad in [
        "C:\\x\"\nselect disk 0\nclean\n.vhdx",
        "C:\\x\r\nclean.vhdx",
        "C:\\a\"b.vhdx",
    ] {
        assert!(diskpart_scripts(bad).is_err(), "{bad:?}");
    }
}

// Traces: ACT-WINDOWS-CLEANUP-RUN
#[test]
fn restore_points_are_counted_by_their_shadow_device_paths() {
    let listing = "vssadmin 1.1 - Volume Shadow Copy Service administrative command-line tool\r\n\
(C) Copyright 2001-2013 Microsoft Corp.\r\n\r\n\
Contents of shadow copy set ID: {a}\r\n\
   Contained 1 shadow copies at creation time: 01.09.2026 10:00:00\r\n\
      Shadow Copy ID: {b}\r\n\
         Original Volume: (C:)\\\\?\\Volume{c}\\\r\n\
         Shadow Copy Volume: \\\\?\\GLOBALROOT\\Device\\HarddiskVolumeShadowCopy7\r\n\
Contents of shadow copy set ID: {d}\r\n\
      Schattenkopie-Volume: \\\\?\\GLOBALROOT\\Device\\HarddiskVolumeShadowCopy9\r\n";
    assert_eq!(count_shadows(listing), 2);
    assert_eq!(
        count_shadows("No items found that satisfy the query.\r\n"),
        0
    );
}

// Traces: ACT-WINDOWS-CLEANUP-RUN, ACT-WINDOWS-CLEANUP-DETECT
#[test]
fn the_leftovers_helper_clears_only_the_catalogs_machine_wide_folders() {
    assert_eq!(
        leftover_targets(r"X:\Windows", r"X:\ProgramData"),
        [
            r"X:\Windows\Temp",
            r"X:\Windows\SoftwareDistribution\Download",
            r"X:\Windows\ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization\Cache",
            r"X:\Windows\SoftwareDistribution\DeliveryOptimization",
            r"X:\ProgramData\Microsoft\Windows\WER",
            r"X:\Windows\Minidump",
            r"X:\Windows\MEMORY.DMP",
            r"X:\Windows\LiveKernelReports",
            r"X:\Windows\Logs\CBS",
            r"X:\Windows\Logs\DISM",
            r"X:\Windows\Panther",
        ],
        "the user's own CrashDumps and WER folders are cleared as the user, not here"
    );
    let op = SystemOp::Leftovers;
    assert!(op.elevated());
    assert!(!op.irreversible());
    assert_eq!(op.name(), "leftovers");
    assert_eq!(
        serde_json::to_value(&op).unwrap(),
        serde_json::json!({"op": "leftovers"})
    );
}

fn aged(path: &Path, days: i64) {
    let ms = hoardini_core::store::now_ms() - days * 86_400_000;
    hoardini_win::set_all_times(path, ms).unwrap();
}

// Traces: ACT-WINDOWS-CLEANUP-RUN
#[test]
fn clearing_a_leftover_folder_keeps_it_new_and_open_files_and_never_follows_a_link() {
    let fx = Fx::new();
    let temp = fx.work.join("Temp");
    let old = fx.file(r"Temp\old.tmp", b"old");
    let fresh = fx.file(r"Temp\fresh.tmp", b"fresh");
    let open = fx.file(r"Temp\open.tmp", b"open");
    let nested = fx.file(r"Temp\setup\inner.cab", b"cab");
    let readonly = fx.file(r"Temp\ro.tmp", b"ro");
    let outside = fx.file(r"elsewhere\keep.txt", b"keep");
    hoardini_win::create_junction(&temp.join("link"), &fx.work.join("elsewhere")).unwrap();
    let mut perms = std::fs::metadata(&readonly).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&readonly, perms).unwrap();
    for p in [&old, &open, &nested, &readonly, &outside] {
        aged(p, 3);
    }
    aged(&temp.join("setup"), 3);
    use std::os::windows::fs::OpenOptionsExt;
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&open)
        .unwrap();

    let cutoff = hoardini_core::store::now_ms() - 86_400_000;
    let mut c = Cleared::default();
    clear_contents(&temp, cutoff, &mut c).unwrap();
    assert_eq!(
        (c.files, c.in_use, c.recent, c.failed),
        (3, 1, 1, 0),
        "{c:?}"
    );
    assert_eq!(c.bytes, 3 + 3 + 2);
    assert!(temp.is_dir(), "the folder itself stays");
    for gone in [&old, &nested, &readonly] {
        assert!(!gone.exists(), "{}", gone.display());
    }
    assert!(
        !temp.join("setup").exists(),
        "an emptied old folder goes too"
    );
    assert!(fresh.exists() && open.exists());
    assert!(outside.exists(), "a junction inside is never followed");
    assert!(temp.join("link").exists(), "nor removed");
    assert_eq!(
        c.summary(),
        "3 files deleted (8 B); left: 1 in use, 1 newer than a day"
    );
    drop(held);

    let mut m = Cleared::default();
    clear_contents(&old, cutoff, &mut m).unwrap_err();
    let dump = fx.file("MEMORY.DMP", b"dump");
    aged(&dump, 5);
    clear_contents(&dump, cutoff, &mut m).unwrap();
    assert_eq!(m.files, 1, "a file the catalog names is cleared on its own");
    assert!(!dump.exists());
}

use hoardini_core::memory::knowledge::upsert_with;
use hoardini_core::memory::location::{cleaned_for_with, was_kept_with};
use hoardini_core::memory::{Detail, Kind, Knowledge, Scope};
use serde_json::json;

use super::{P, conn};

const T: i64 = 1_800_000_000_000;
const DAY: i64 = 86_400_000;
const GIB: u64 = 1 << 30;

fn row(c: &hoardini_core::memory::Connection, key: &str, detail: Detail) {
    upsert_with(
        c,
        Knowledge {
            scope: Scope::Path,
            key: key.into(),
            kind: Kind::Verdict,
            text: "A folder.".into(),
            source: "index".into(),
            detail,
        },
        T,
        P,
    )
    .unwrap();
}

// Traces: MEM-LEARN-JOB
#[test]
fn a_kept_item_stays_out_until_it_grows_by_half() {
    let (_d, c) = conn();
    row(
        &c,
        r"D:\code",
        Detail {
            category: "unknown".into(),
            kept: vec![
                json!({"ms": T, "path": r"D:\code\big.iso", "size": 1000, "job": "p1"}),
                json!({"ms": T, "path": r"D:\code\sub\k.bin", "size": 10, "job": "p1"}),
                json!({"note": "an entry this build does not know"}),
            ],
            ..Detail::default()
        },
    );
    row(
        &c,
        r"D:\code\sub",
        Detail {
            category: "unknown".into(),
            ..Detail::default()
        },
    );
    let kept = |p: &str, size: u64| was_kept_with(&c, P, p, size).unwrap();
    assert!(kept(r"D:\code\big.iso", 1000));
    assert!(kept(r"D:\CODE\Big.iso", 1499));
    assert!(!kept(r"D:\code\big.iso", 1500), "grown by half");
    assert!(
        kept(r"D:\code\sub\k.bin", 12),
        "an entry on a farther row still counts"
    );
    assert!(!kept(r"D:\code\other.iso", 10));
    assert!(
        !kept(r"D:\code", 10),
        "a folder holding a kept item is not itself kept"
    );
}

// Traces: MEM-LEARN-JOB
#[test]
fn a_cleaned_entry_matches_by_path_or_rule_and_reads_as_a_reason() {
    let (_d, c) = conn();
    row(
        &c,
        r"D:\code",
        Detail {
            category: "unknown".into(),
            cleaned: vec![
                json!({"ms": T, "bytes": 28 * GIB, "path": r"D:\code\a\target",
                       "rule": r"D:\code\**\target", "job": "p1", "kind": "delete"}),
                json!({"ms": T + DAY, "bytes": 5 << 20, "path": r"D:\code\logs\a.log",
                       "rule": r"D:\code\logs\*.log", "job": "p2", "kind": "archive"}),
                json!({"ms": T + 2 * DAY, "bytes": 1 << 20, "path": r"D:\code\[x]\y.bin",
                       "rule": r"D:\code\[[]x[]]\y.bin", "job": "p3", "kind": "compress"}),
            ],
            ..Detail::default()
        },
    );
    row(
        &c,
        r"C:\Users\tester\scratch",
        Detail {
            category: "unknown".into(),
            cleaned: vec![json!({"ms": T, "bytes": 4096, "path": r"~\scratch\x.bin",
                                 "rule": r"~\scratch\x.bin", "job": "p4", "kind": "delete"})],
            ..Detail::default()
        },
    );
    let hit = |p: &str| cleaned_for_with(&c, P, p).unwrap();

    let t = hit(r"D:\code\b\target").expect("the rule covers another project's target");
    assert_eq!(
        (t.path.as_str(), t.job.as_str()),
        (r"D:\code\a\target", "p1")
    );
    assert_eq!(t.reason(), "you cleaned this on 01-15 (28.0 GB)");
    assert_eq!(
        hit(r"D:\code\b\target\debug\x.o").map(|c| c.job),
        Some("p1".into())
    );

    let l = hit(r"D:\code\logs\b.log").unwrap();
    assert_eq!(l.reason(), "you archived this on 01-16 (5.0 MB)");
    assert!(
        hit(r"D:\code\logs\sub\c.log").is_none(),
        "* stays within one folder"
    );
    assert!(hit(r"D:\code\other.bin").is_none());

    let x = hit(r"D:\code\[x]\y.bin").expect("brackets in a name are literal");
    assert_eq!(x.reason(), "you compressed this on 01-17 (1.0 MB)");
    assert_eq!(
        hit(r"C:\Users\Tester\scratch\x.bin").map(|c| c.job),
        Some("p4".into())
    );
}

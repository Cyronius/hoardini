use hoardini_agent::eval::{Q4NX_SEARCH, search_standin};
use hoardini_agent::tools::{Outcome, Registry, ToolError};
use hoardini_core::memory::knowledge::{Detail, filetype, remember_filetype, weight_extensions};
use hoardini_core::memory::location::location_at;
use hoardini_core::memory::{self, Kind as MemKind, NewMemory, Scope};
use serde_json::json;

use super::support::*;
use super::{FAKE_PROFILE, ctx, tool_content};

const FLM: &str = "https://github.com/FastFlowLM/FastFlowLM";

async fn with_search(h: &Harness) {
    let base = search_standin(Q4NX_SEARCH).await.unwrap();
    h.db.conn()
        .unwrap()
        .execute(
            "INSERT INTO search_provider(id, kind, config_json, is_active, created_ms) VALUES ('searxng', 'searxng', ?1, 1, 0)",
            [json!({"base_url": base}).to_string()],
        )
        .unwrap();
}

fn content(out: Result<Outcome, ToolError>) -> String {
    match out {
        Ok(out) => out.content,
        Err(e) => panic!("expected text, got {e:?}"),
    }
}

// Traces: AGENT-WEB-LEARN, MEM-WEB-IDENTITY
#[tokio::test(flavor = "multi_thread")]
async fn a_search_about_a_type_ends_in_one_small_request_and_an_identity_row() {
    let h = Harness::new(Some(FAKE_PROFILE));
    with_search(&h).await;
    let mock = MockLlm::start(vec![
        Turn::Calls(
            None,
            vec![call(
                "s1",
                "web_search",
                json!({"query": "q4nx file format", "about": "*.q4nx"}),
            )],
        ),
        Turn::Text("They are FastFlowLM NPU weights.".into()),
        Turn::Text(
            r#"Sure: {"app": "FastFlowLM", "use": "NPU model weights", "category": "model-weights", "verdict": "safe"}"#
                .into(),
        ),
    ])
    .await;
    let ev = h
        .run(
            h.provider(&mock.base_url),
            user("t-wl", "u1", "What are .q4nx files?"),
        )
        .await;
    assert_valid_sequence(&ev);
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 3);
    let ask = reqs[2].json();
    assert!(
        ask.get("tools").is_none(),
        "the learning request offers no tools"
    );
    let msgs = reqs[2].messages();
    assert_eq!(msgs.len(), 2);
    assert!(
        msgs[0]["content"]
            .as_str()
            .unwrap()
            .contains("Never say whether it can be deleted")
    );
    let q = msgs[1]["content"].as_str().unwrap();
    assert!(
        q.starts_with("What is .q4nx?\n<<<UNTRUSTED_WEB_CONTENT"),
        "{q}"
    );
    assert!(
        q.trim_end().ends_with("UNTRUSTED_WEB_CONTENT>>>") && q.contains(FLM),
        "{q}"
    );

    let conn = h.db.conn().unwrap();
    let row = filetype(&conn, "q4nx").unwrap().expect("a type row");
    assert_eq!(row.key, r"**\*.q4nx");
    assert_eq!(row.text, ".q4nx: NPU model weights (FastFlowLM).");
    assert_eq!(row.source, format!("web:{FLM}"));
    assert_eq!(row.kind, MemKind::Fact);
    let d = row.detail.as_ref().unwrap();
    assert_eq!(
        (
            d.category.as_str(),
            d.app.as_deref(),
            d.use_.as_deref(),
            d.verdict
        ),
        (
            "model-weights",
            Some("FastFlowLM"),
            Some("NPU model weights"),
            None
        )
    );
    let raw: String = conn
        .query_row(
            "SELECT detail_json FROM memory WHERE id = ?1",
            [row.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !raw.contains("verdict") && !raw.contains("actions"),
        "{raw}"
    );
    assert!(
        weight_extensions(&conn)
            .unwrap()
            .contains(&"q4nx".to_string())
    );
    assert_eq!(types(&ev).last().map(String::as_str), Some("RUN_FINISHED"));
}

// Traces: AGENT-WEB-SEARCH, AGENT-WEB-LEARN
#[tokio::test(flavor = "multi_thread")]
async fn a_known_about_answers_from_memory_unless_again() {
    let h = Harness::new(Some(FAKE_PROFILE));
    with_search(&h).await;
    let conn = h.db.conn().unwrap();
    remember_filetype(
        &conn,
        "q4nx",
        ".q4nx: NPU model weights (FastFlowLM).",
        &format!("web:{FLM}"),
        Detail {
            category: "model-weights".into(),
            ..Detail::default()
        },
        1_800_000_000_000,
    )
    .unwrap();
    let c = ctx(&h, FAKE_PROFILE);
    let r = Registry::full();
    let known = content(r.call(&c, "web_search", r#"{"query": "q4nx", "about": ".Q4NX"}"#));
    assert!(
        known.starts_with("<<<BEGIN UNTRUSTED note learned from a web page"),
        "{known}"
    );
    assert!(
        known.contains(&format!(
            "Already known: .q4nx: NPU model weights (FastFlowLM). (from {FLM}, 2027-01-15)."
        )),
        "{known}"
    );
    let again = content(r.call(
        &c,
        "web_search",
        r#"{"query": "q4nx", "about": "*.q4nx", "again": true}"#,
    ));
    assert!(
        again.starts_with("<<<UNTRUSTED_WEB_CONTENT") && again.contains(FLM),
        "{again}"
    );

    memory::remember_with(
        &conn,
        NewMemory::new(
            Scope::Path,
            r"~\.hipfire",
            MemKind::Fact,
            "hipfire's model cache",
            "user",
        ),
        1_800_000_000_000,
        FAKE_PROFILE,
    )
    .unwrap();
    let path = content(r.call(
        &c,
        "web_search",
        r#"{"query": "hipfire", "about": "~\\.hipfire"}"#,
    ));
    assert_eq!(
        path,
        "Already known: hipfire's model cache (from you, 2027-01-15). Pass again: true to search the web anyway."
    );
    match r.call(&c, "web_search", r#"{"query": "x", "about": "two words"}"#) {
        Err(ToolError::BadArgs(m)) => assert!(m.contains("*.q4nx"), "{m}"),
        other => panic!("expected bad args, got {other:?}"),
    }
}

// Traces: AGENT-WEB-LEARN
#[tokio::test(flavor = "multi_thread")]
async fn a_remember_on_the_same_key_or_an_unreadable_reply_stores_nothing() {
    let h = Harness::new(Some(FAKE_PROFILE));
    with_search(&h).await;
    let mock = MockLlm::start(vec![
        Turn::Calls(
            None,
            vec![call(
                "s1",
                "web_search",
                json!({"query": "hipfire folder", "about": "~\\.hipfire"}),
            )],
        ),
        Turn::Calls(
            None,
            vec![call(
                "r1",
                "remember",
                json!({"scope": "~\\.hipfire", "kind": "fact", "text": "hipfire keeps its models here"}),
            )],
        ),
        Turn::Text("Noted.".into()),
        Turn::Calls(
            None,
            vec![call(
                "s2",
                "web_search",
                json!({"query": "zzq file", "about": "*.zzq"}),
            )],
        ),
        Turn::Text("Not sure.".into()),
        Turn::Text("It is probably some kind of model file.".into()),
    ])
    .await;
    let first = h
        .run(
            h.provider(&mock.base_url),
            user("t-wr", "u1", "What is ~\\.hipfire?"),
        )
        .await;
    assert_eq!(
        mock.requests().len(),
        3,
        "no learning request after a remember"
    );
    let conn = h.db.conn().unwrap();
    assert!(
        location_at(&conn, FAKE_PROFILE, r"C:\Users\zq_owner\.hipfire")
            .unwrap()
            .is_none()
    );
    let second = h
        .run(
            h.provider(&mock.base_url),
            user("t-wr", "u2", "And .zzq files?"),
        )
        .await;
    assert_eq!(
        mock.requests().len(),
        6,
        "one learning request, answered in prose"
    );
    assert!(filetype(&conn, "zzq").unwrap().is_none());
    for ev in [&first, &second] {
        assert_valid_sequence(ev);
    }
    let web_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory WHERE source LIKE 'web:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(web_rows, 0);
    let notes = tool_content(&mock.requests()[2], "r1");
    assert!(notes.starts_with("Noted as memory"), "{notes}");
}

// Traces: AGENT-WEB-LEARN
#[test]
fn the_first_complete_json_object_of_the_reply_is_read() {
    use hoardini_agent::tools::parse_identity;
    let flm = Some("FastFlowLM".to_string());
    let one = r#"{"app": "FastFlowLM", "use": "NPU model weights", "category": "model-weights"}"#;
    assert_eq!(parse_identity(one).unwrap().app, flm);
    let trailing = format!("{one} Sources: {{1}}");
    assert_eq!(parse_identity(&trailing).unwrap().app, flm);
    let two = format!("{one}\n{{\"app\": \"other\", \"use\": \"x\"}}");
    assert_eq!(parse_identity(&two).unwrap().app, flm);
    let lead = format!("From result {{1}}: {one}");
    assert_eq!(parse_identity(&lead).unwrap().app, flm);
    assert!(parse_identity("no json {here}").is_none());
}

// Traces: AGENT-WEB-SEARCH
#[tokio::test(flavor = "multi_thread")]
async fn a_glob_that_matches_only_a_folder_above_is_not_already_known() {
    let h = Harness::new(Some(FAKE_PROFILE));
    with_search(&h).await;
    memory::remember_with(
        &h.db.conn().unwrap(),
        NewMemory::new(
            Scope::Glob,
            r"~\code\**\target",
            MemKind::Fact,
            "Rust build output",
            "user",
        ),
        1_800_000_000_000,
        FAKE_PROFILE,
    )
    .unwrap();
    let c = ctx(&h, FAKE_PROFILE);
    let r = Registry::full();
    let below = content(r.call(
        &c,
        "web_search",
        r#"{"query": "debug folder", "about": "~\\code\\x\\target\\debug"}"#,
    ));
    assert!(below.starts_with("<<<UNTRUSTED_WEB_CONTENT"), "{below}");
    let at = content(r.call(
        &c,
        "web_search",
        r#"{"query": "target folder", "about": "~\\code\\x\\target"}"#,
    ));
    assert!(
        at.starts_with("Already known: Rust build output (from you"),
        "{at}"
    );
}

// Traces: AGENT-WEB-LEARN
#[tokio::test(flavor = "multi_thread")]
async fn a_rate_limited_learning_request_is_not_retried_before_the_run_ends() {
    let h = Harness::new(Some(FAKE_PROFILE));
    with_search(&h).await;
    let mock = MockLlm::start(vec![
        Turn::Calls(
            None,
            vec![call(
                "s1",
                "web_search",
                json!({"query": "q4nx file format", "about": "*.q4nx"}),
            )],
        ),
        Turn::Text("They are FastFlowLM NPU weights.".into()),
        Turn::Http(429, r#"{"error": {"message": "slow down"}}"#.into()),
        Turn::Text(r#"{"app": "FastFlowLM", "use": "NPU model weights"}"#.into()),
    ])
    .await;
    let ev = h
        .run(
            h.provider(&mock.base_url),
            user("t-429", "u1", "What are .q4nx files?"),
        )
        .await;
    assert_valid_sequence(&ev);
    assert_eq!(
        mock.requests().len(),
        3,
        "the 429 ends the learning request: no retry, so no backoff wait"
    );
    assert!(filetype(&h.db.conn().unwrap(), "q4nx").unwrap().is_none());
}

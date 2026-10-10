use hoardini_agent::eval::{Q4NX_SEARCH, search_standin};
use hoardini_agent::thread;
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
        Ok(Outcome::Text { content, .. }) => content,
        other => panic!("expected text, got {other:?}"),
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

    let payload = json!({"v": 1, "kind": "learned", "items": [
        {"memory_id": row.id, "text": ".q4nx: NPU model weights (FastFlowLM).", "url": FLM, "confirmed": false}
    ]});
    let data = custom(&ev, "tool_data");
    assert_eq!(data.len(), 1);
    assert_eq!(
        data[0]["value"],
        json!({"tool_call_id": "s1", "data": payload})
    );
    let types = types(&ev);
    let at = ev.iter().position(|e| e["name"] == "tool_data").unwrap();
    assert!(
        types[..at].contains(&"TEXT_MESSAGE_END".to_string()),
        "after the answer"
    );
    assert_eq!(types.last().map(String::as_str), Some("RUN_FINISHED"));
    let stored = thread::agui_messages(&h.db, "t-wl", Some(FAKE_PROFILE)).unwrap();
    let tool = stored
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("s1"))
        .unwrap();
    assert_eq!(tool.data.as_ref(), Some(&payload));

    memory::knowledge::confirm(&conn, row.id, 2, FAKE_PROFILE).unwrap();
    let again = thread::agui_messages(&h.db, "t-wl", Some(FAKE_PROFILE)).unwrap();
    let tool = again
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("s1"))
        .unwrap();
    assert_eq!(
        tool.data.as_ref().unwrap()["items"][0]["confirmed"],
        json!(true),
        "a reload after Confirm offers no Confirm"
    );
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
    let c = ctx(&h, FAKE_PROFILE, "k1");
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
        assert!(custom(ev, "tool_data").is_empty());
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

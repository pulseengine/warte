//! Decisions in the app (REQ-DECIDE-001, warte#184).

use serde_json::{json, Value};
use warte::decisions::{answer, open};

fn notice(id: &str, rev: u64, tags: &[&str], status: &str, flow: &str) -> Value {
    json!({"schema_version": 2, "id": id, "kind": "needs-decision", "revision": rev,
           "created_at": format!("2026-11-22T0{rev}:00:00Z"), "agent": "a1", "title": format!("Decide {id}"),
           "repo": "pulseengine/relay", "tags": tags, "status": status, "flow": flow,
           "options": [{"id": "keep", "label": "Keep", "description": "", "pros": [], "cons": []},
                       {"id": "drop", "label": "Drop", "description": "", "pros": [], "cons": []}],
           "recommended": "keep"})
}

fn ao() -> Vec<String> {
    vec!["security".into(), "red-main".into()]
}

const NOW: &str = "2026-11-22T12:00:00Z";

// rivet: verifies REQ-DECIDE-001
#[test]
fn urgent_decisions_come_first_and_answered_or_superseded_ones_disappear() {
    let rows = vec![
        ("row-a".to_string(), notice("n1", 1, &[], "open", "b-flow")),
        (
            "row-b".to_string(),
            notice("n2", 1, &["security"], "open", "z-flow"),
        ),
        ("row-a".to_string(), notice("n3", 1, &[], "open", "a-flow")),
        // n4 was withdrawn by its second revision
        ("row-a".to_string(), notice("n4", 1, &[], "open", "a-flow")),
        (
            "row-a".to_string(),
            notice("n4", 2, &[], "withdrawn", "a-flow"),
        ),
    ];
    let ids = |ds: &[warte::decisions::Decision]| -> Vec<String> {
        ds.iter()
            .map(|d| d.notice["id"].as_str().unwrap().to_string())
            .collect()
    };
    let ds = open(&rows, &[], &ao());
    assert_eq!(ids(&ds), ["n2", "n3", "n1"], "urgent first, then by flow");
    assert_eq!(ds[0].agent_row, "row-b");
    // answered at its revision: gone
    let answers = vec![json!({"notice_id": "n3", "revision": 1})];
    assert_eq!(ids(&open(&rows, &answers, &ao())), ["n2", "n1"]);
    // an answer to an older revision does not hide a newer one
    let mut rows2 = rows.clone();
    rows2.push(("row-a".to_string(), notice("n3", 2, &[], "open", "a-flow")));
    assert_eq!(ids(&open(&rows2, &answers, &ao())), ["n2", "n3", "n1"]);
}

// rivet: verifies REQ-DECIDE-001
#[test]
fn only_an_open_current_offered_answer_is_made_and_it_has_the_schema_shape() {
    let rows = vec![
        ("row-a".to_string(), notice("n1", 2, &[], "open", "f")),
        ("row-a".to_string(), notice("n2", 1, &[], "answered", "f")),
    ];
    let answers = vec![json!({"notice_id": "n9", "revision": 1})];
    let (row, a) = answer(
        &rows,
        &answers,
        "n1",
        2,
        "drop",
        Some("ship it Monday"),
        NOW,
    )
    .unwrap();
    assert_eq!(row, "row-a");
    assert_eq!(
        a,
        json!({"schema_version": 1, "notice_id": "n1", "revision": 2, "option_id": "drop",
               "answered_at": NOW, "via": "warte", "note": "ship it Monday"})
    );
    // an empty note is left out
    let (_, a) = answer(&rows, &answers, "n1", 2, "keep", Some(""), NOW).unwrap();
    assert!(a.get("note").is_none());
    // planted refusals: stale revision, unoffered option, closed notice,
    // unknown id, an already-answered one, a note too long
    for (id, rev, opt, note, want) in [
        ("n1", 1, "drop", None, "revision"),
        ("n1", 2, "maybe", None, "option_id"),
        ("n2", 1, "keep", None, "no longer open"),
        ("nope", 1, "keep", None, "no such decision"),
    ] {
        let e = answer(&rows, &answers, id, rev, opt, note, NOW).unwrap_err();
        assert!(e.contains(want), "{id}/{rev}/{opt}: {e}");
    }
    let done = vec![json!({"notice_id": "n1", "revision": 2})];
    assert!(answer(&rows, &done, "n1", 2, "keep", None, NOW)
        .unwrap_err()
        .contains("already answered"));
    let long = "x".repeat(2001);
    assert!(answer(&rows, &answers, "n1", 2, "keep", Some(&long), NOW)
        .unwrap_err()
        .starts_with("note"));
}

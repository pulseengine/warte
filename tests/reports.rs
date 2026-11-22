//! Reports from producers (REQ-REPORTS-001, warte#87, #76).

use serde_json::{json, Value};
use warte::reports::{check, seams_view, write_right, SEAM_CHECK};

/// A planted change to the sample, and the error it must cause.
type Case = (fn(&mut Value), &'static str);

/// A report in the shape seam-check.py writes (seam-check.v1).
pub fn sample() -> Value {
    json!({"schema": "seam-check.v1", "now": "2026-11-17T06:41:00+00:00", "source": "github",
        "seams": [
            {"id": "notice-warte/attention-schedule", "source": {"repo": "pulseengine/warte", "issue": 181, "url": "https://github.com/pulseengine/warte/issues/181", "in": "body"},
             "status": "decided", "provider": {"repo": "pulseengine/pulseengine.eu"}, "consumers": [{"repo": "pulseengine/warte"}],
             "violations": [], "warnings": [], "open_questions": []},
            {"id": "relay-jess/estimator-identity", "source": {"repo": "pulseengine/jess", "issue": 167, "url": "https://github.com/pulseengine/jess/issues/167", "in": "body"},
             "status": "implemented", "provider": {"repo": "pulseengine/relay"}, "consumers": [{"repo": "pulseengine/jess"}],
             "violations": [{"rule": "interface", "message": "wit package version 0.11.0 is not 0.12.0"}], "warnings": [],
             "open_questions": [{"q": "who applies calibration?", "owner": "pulseengine/relay"}]}
        ],
        "issues_without_block": [], "errors": [], "exit": 1,
        "summary": {"seams": 2, "failing": 1}})
}

// rivet: verifies REQ-REPORTS-001
#[test]
fn a_seam_check_report_is_checked_by_field() {
    let ok = sample();
    assert!(check(&SEAM_CHECK, ok.to_string().as_bytes()).is_ok());
    assert_eq!(write_right(&SEAM_CHECK), "reports:write:seam-check");
    let cases: Vec<Case> = vec![
        (
            |v| v["schema"] = json!("seam-check.v2"),
            "schema: must be seam-check.v1",
        ),
        (|v| v["seams"] = json!({}), "seams: not a list"),
        (
            |v| v["seams"][1]["source"] = json!({}),
            "seams[1].source.repo",
        ),
        (
            |v| v["seams"][0]["violations"] = json!(null),
            "seams[0].violations",
        ),
        (|v| v["errors"] = json!("x"), "errors: not a list"),
        (|v| v["exit"] = json!("one"), "exit: not a number"),
        (|v| v["now"] = json!(null), "now: missing"),
    ];
    for (plant, want) in cases {
        let mut v = sample();
        plant(&mut v);
        let e = check(&SEAM_CHECK, v.to_string().as_bytes()).unwrap_err();
        assert!(e.starts_with(want), "{want}: {e}");
    }
    assert!(check(&SEAM_CHECK, b"not json")
        .unwrap_err()
        .contains("not JSON"));
    let big = vec![b' '; warte::reports::MAX_BYTES + 1];
    assert!(check(&SEAM_CHECK, &big).unwrap_err().contains("larger"));
}

// rivet: verifies REQ-REPORTS-001
#[test]
fn the_seams_view_puts_violations_first_and_flags_a_stale_report() {
    let at = chrono::DateTime::parse_from_rfc3339("2026-11-17T06:41:00+00:00")
        .unwrap()
        .timestamp();
    let html = seams_view(Some(&(sample(), at)), at + 86_400);
    let failing = html.find("relay-jess/estimator-identity").unwrap();
    let passing = html.find("notice-warte/attention-schedule").unwrap();
    assert!(failing < passing, "a violation comes first");
    assert!(html.contains("wit package version 0.11.0 is not 0.12.0") && html.contains("warnrow"));
    assert!(html.contains("who applies calibration?"));
    assert!(!html.contains("stale"));
    assert!(seams_view(Some(&(sample(), at)), at + 9 * 86_400).contains("stale"));
    assert!(seams_view(None, at).contains("No seam-check report yet"));
}

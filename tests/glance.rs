//! The night glance (REQ-GLANCE-001, warte#179).

use serde_json::{json, Value};
use warte::glance::{failed_releases, glance, Inputs, ALWAYS_OPEN, SILENT_S};
use warte::hub::model::JobRecord;

const NOW: i64 = 1_800_000_000;

fn notice(id: &str, rev: u64, tags: &[&str], status: &str, title: &str) -> Value {
    json!({"schema_version": 2, "id": id, "kind": "needs-decision", "revision": rev,
           "created_at": "2026-11-22T01:00:00Z", "agent": "a1", "title": title,
           "repo": "pulseengine/relay", "tags": tags, "status": status})
}

fn inputs() -> Inputs {
    Inputs {
        always_open: ALWAYS_OPEN.map(String::from).to_vec(),
        now: NOW,
        ..Default::default()
    }
}

// rivet: verifies REQ-GLANCE-001
#[test]
fn an_urgent_decision_turns_quiet_into_needs_you_and_a_routine_one_is_only_counted() {
    let mut i = inputs();
    // planted negative control: routine decisions never become items
    i.notices = vec![
        notice("n1", 1, &[], "open", "Pick a crate"),
        notice("n2", 1, &["style"], "open", "Rename a flag"),
    ];
    let g = glance(&i);
    assert_eq!(
        (g.verdict.as_str(), g.items.len(), g.routine_decisions),
        ("quiet", 0, 2)
    );
    i.notices.push(notice(
        "n3",
        1,
        &["security"],
        "open",
        "Rotate the leaked key",
    ));
    let g = glance(&i);
    assert_eq!(g.verdict, "needs-you");
    assert_eq!(g.items.len(), 1);
    assert_eq!(g.items[0].text, "security: Rotate the leaked key");
    assert_eq!(g.routine_decisions, 2);
    // a newer revision that was answered closes it
    i.notices.push(notice(
        "n3",
        2,
        &["security"],
        "answered",
        "Rotate the leaked key",
    ));
    assert_eq!(glance(&i).verdict, "quiet");
    // other kinds of notice are neither items nor decisions
    let mut info = notice("n4", 1, &["security"], "open", "FYI");
    info["kind"] = json!("info");
    i.notices.push(info);
    assert_eq!(glance(&i).routine_decisions, 2);
}

// rivet: verifies REQ-GLANCE-001
#[test]
fn red_main_failed_release_and_critical_disk_are_items() {
    let mut i = inputs();
    i.red_mains = vec![("rivet".into(), "CI".into())];
    i.failed_releases = vec![("spar".into(), "v0.9.0".into())];
    i.critical_disks = vec!["build-host".into()];
    let g = glance(&i);
    assert_eq!(g.verdict, "needs-you");
    let kinds: Vec<&str> = g.items.iter().map(|x| x.kind.as_str()).collect();
    assert_eq!(kinds, ["red-main", "release", "disk"]);
    assert!(
        g.items.iter().all(|x| !x.text.contains('/')),
        "{:?}",
        g.items
    );
}

fn job(run: u64, att: u32, repo: &str, tag: &str, c: i64, r: &str) -> JobRecord {
    JobRecord {
        d: "2026-11-22".into(),
        id: run * 10 + u64::from(att),
        run,
        att,
        repo: repo.into(),
        wf: "Release".into(),
        job: "build".into(),
        pool: "hosted:ubuntu-24.04".into(),
        runner: "r".into(),
        ev: "push".into(),
        c,
        s: Some(c + 10),
        e: Some(c + 100),
        r: r.into(),
        sig: None,
        br: Some(tag.into()),
    }
}

// rivet: verifies REQ-GLANCE-001
#[test]
fn a_failed_release_rerun_green_is_not_listed_and_one_not_rerun_is() {
    let recs = vec![
        // planted: failed, then the same tag rerun green
        job(1, 1, "rivet", "v1.0.0", NOW - 600, "failure"),
        job(1, 2, "rivet", "v1.0.0", NOW - 300, "success"),
        // failed and not rerun
        job(2, 1, "spar", "v0.9.0", NOW - 200, "failure"),
        // a green release
        job(3, 1, "loom", "v2.0.0", NOW - 100, "success"),
    ];
    assert_eq!(
        failed_releases(&recs),
        [("spar".to_string(), "v0.9.0".to_string())]
    );
    // a non-release workflow or a non-push event never counts
    let mut ci = job(4, 1, "gale", "main", NOW, "failure");
    ci.wf = "CI".into();
    let mut pr = job(5, 1, "gale", "v1", NOW, "failure");
    pr.ev = "pull_request".into();
    assert!(failed_releases(&[ci, pr]).is_empty());
}

// rivet: verifies REQ-GLANCE-001
#[test]
fn a_silent_agent_is_counted_never_listed_and_one_never_heard_is_unknown() {
    let mut i = inputs();
    i.heartbeats = vec![Some(NOW - 2 * 3600), Some(NOW - 60), None];
    let g = glance(&i);
    assert_eq!(g.verdict, "quiet", "silence is not an item in v1");
    assert_eq!((g.silent, g.unknown), (1, 1));
    const { assert!(SILENT_S == 30 * 60) };
}

// rivet: verifies REQ-GLANCE-001
#[test]
fn a_title_is_cut_to_80_characters() {
    let mut i = inputs();
    let long = "x".repeat(200);
    i.notices = vec![notice("n1", 1, &["red-main"], "open", &long)];
    let g = glance(&i);
    let title = g.items[0].text.strip_prefix("red-main: ").unwrap();
    assert_eq!(title.chars().count(), 80);
}

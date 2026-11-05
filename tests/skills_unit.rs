//! Skills: the view's escaping and staleness, and what is kept on disk
//! (REQ-SKILLS-001).

fn report(source: &str) -> warte::skills::SkillReport {
    serde_json::from_value(serde_json::json!({
        "schema": "skill-usage.v1",
        "source": source,
        "generated_at": "2026-11-05T12:00:00Z",
        "window": {"from": "2026-10-06", "to": "2026-11-05"},
        "sessions_total": 10,
        "sessions_with_skill": 2,
        "installed": ["skill-a"],
        "skills": [{"name": "skill-a", "plugin": "plugin-x",
                    "invocations": {"model": 1, "user_slash": 0, "subagent": 0},
                    "sessions": 1, "skill_md_reads": 0, "repos": []}]
    }))
    .unwrap()
}

// rivet: verifies REQ-SKILLS-001
#[test]
fn the_view_escapes_names_and_flags_a_stale_report() {
    let dir = tempfile::tempdir().unwrap();
    let s = warte::skills::Skills::open(dir.path().to_str().unwrap(), None);
    // stored directly, past validation: the view must still escape
    let mut r = report("ws-a");
    r.skills[0].name = "<img src=x onerror=alert(1)>".into();
    r.installed.push("<b>x</b>".into());
    let received = warte::hub::now() - 3 * 24 * 3600;
    s.record(r, received).unwrap();
    let html = warte::dashboard::skills::view(Some(&s), warte::hub::now());
    assert!(
        !html.contains("<img src=x") && !html.contains("<b>x</b>"),
        "{html}"
    );
    assert!(html.contains("&lt;img src=x"));
    assert!(html.contains("pill stale"), "a 3-day-old report is stale");
    assert!(warte::dashboard::skills::view(None, 0).contains("No skill-usage report received yet"));
}

// rivet: verifies REQ-SKILLS-001
#[test]
fn the_latest_report_per_source_is_kept_and_a_stored_file_cannot_smuggle_text() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().to_str().unwrap();
    let s = warte::skills::Skills::open(d, None);
    s.record(report("ws-a"), 100).unwrap();
    s.record(report("ws-b"), 100).unwrap();
    let mut r = report("ws-a");
    r.sessions_total = 99;
    s.record(r, 200).unwrap();
    let all = s.all();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].report.sessions_total, 99);
    assert_eq!(warte::skills::Skills::open(d, None).all().len(), 2);
    // the unit-level checks reject what serde cannot
    let mut bad = report("ws-a");
    bad.skills[0].plugin = "two words".into();
    assert!(bad.validate().is_err());
    assert!(report("ws-a").validate().is_ok());
}

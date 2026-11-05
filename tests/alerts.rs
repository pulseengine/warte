//! Outage alerts, the Overview's Outages strip, and hosted history per runner
//! type (REQ-ALERT-001, REQ-DASH-017, REQ-HIST-001).

use warte::alerts::{notice, Event, Tracker, AGENT, CLEAR_AFTER, OPEN_AFTER};
use warte::dashboard::fleet::{outages_summary, Detected};
use warte::hub::model::{hosted_by_label, HostedGroup, Point, Queue, Snapshot, HOSTED_LABELS_MAX};

fn d(key: &str, sev: u8) -> Detected {
    Detected {
        key: key.into(),
        sev,
        text: format!("{key} is wrong"),
    }
}

// rivet: verifies REQ-ALERT-001
#[test]
fn a_finding_opens_once_escalates_once_and_clears_after_it_is_gone() {
    let mut t = Tracker::default();
    let disk = d("disk:/var/lib/runners", 1);
    // not on the first sighting: one flapping snapshot does not notify
    for _ in 1..OPEN_AFTER {
        assert!(t.step(std::slice::from_ref(&disk)).is_empty());
    }
    assert_eq!(
        t.step(std::slice::from_ref(&disk)),
        vec![Event::Opened(disk.clone())]
    );
    // still open, same severity: silent however long it lasts
    for _ in 0..10 {
        assert!(t.step(std::slice::from_ref(&disk)).is_empty());
    }
    // severity rises: one escalation
    let full = d("disk:/var/lib/runners", 2);
    assert_eq!(
        t.step(std::slice::from_ref(&full)),
        vec![Event::Escalated(full.clone())]
    );
    assert!(t.step(std::slice::from_ref(&full)).is_empty());
    // gone: clears only after CLEAR_AFTER evaluations without it
    for _ in 1..CLEAR_AFTER {
        assert!(t.step(&[]).is_empty());
    }
    assert_eq!(
        t.step(&[]),
        vec![Event::Cleared {
            key: full.key.clone(),
            text: full.text.clone()
        }]
    );
    assert!(t.step(&[]).is_empty());
}

// rivet: verifies REQ-ALERT-001
#[test]
fn a_blip_shorter_than_the_debounce_never_notifies() {
    let x = d("pressure:CPU", 1);
    // present once, then gone: never opens, never clears
    let mut u = Tracker::default();
    assert!(u.step(std::slice::from_ref(&x)).is_empty());
    for _ in 0..CLEAR_AFTER + 1 {
        assert!(u.step(&[]).is_empty());
    }
    // an open finding that comes back within the clear window does not re-open
    let mut v = Tracker::default();
    for _ in 0..OPEN_AFTER {
        v.step(std::slice::from_ref(&x));
    }
    assert!(v.step(&[]).is_empty());
    assert!(v.step(std::slice::from_ref(&x)).is_empty());
}

// rivet: verifies REQ-ALERT-001
#[test]
fn notices_carry_what_the_factory_notice_schema_requires() {
    let now = chrono::Utc::now();
    let red = notice(
        &Event::Opened(d("github-incident:https://stspg.io/x", 2)),
        now,
        None,
    );
    for k in [
        "schema_version",
        "id",
        "kind",
        "revision",
        "created_at",
        "agent",
        "title",
    ] {
        assert!(red.get(k).is_some(), "missing {k}");
    }
    assert_eq!(red["schema_version"], 2);
    assert_eq!(red["agent"], AGENT);
    assert_eq!(red["kind"], "blocked");
    assert!(red["id"].as_str().unwrap().len() <= 80);
    assert!(red["tags"].as_array().unwrap().iter().any(|t| t == "down"));
    let kinds = [
        "heartbeat",
        "shipped",
        "blocked",
        "needs-decision",
        "session-summary",
        "info",
    ];
    let amber = notice(&Event::Opened(d("pressure:CPU", 1)), now, None);
    let gone = notice(
        &Event::Cleared {
            key: "pressure:CPU".into(),
            text: "CPU pressure".into(),
        },
        now,
        Some("https://example.invalid/warte/"),
    );
    for n in [&red, &amber, &gone] {
        assert!(kinds.contains(&n["kind"].as_str().unwrap()));
        assert!(n["title"].as_str().unwrap().chars().count() <= 200);
    }
    assert!(gone["title"].as_str().unwrap().starts_with("Cleared:"));
    assert_eq!(gone["links"][0], "https://example.invalid/warte/");
    assert_ne!(red["id"], amber["id"]);
}

// rivet: verifies REQ-DASH-017
#[test]
fn the_overview_strip_counts_findings_and_names_the_worst() {
    let calm = outages_summary(&[], Err("offline".into()));
    assert!(
        calm.contains("all clear")
            && calm.contains("0 finding(s)")
            && calm.contains("GitHub unknown")
    );
    let found = vec![d("disk:/x", 1), d("telemetry-stale", 2)];
    let mut sorted = found.clone();
    sorted.sort_by_key(|x| std::cmp::Reverse(x.sev));
    let s = outages_summary(&sorted, Err("x".into()));
    assert!(s.contains("badge red") && s.contains("down") && s.contains("2 finding(s)"));
    assert!(
        s.contains("telemetry-stale is wrong"),
        "the worst is named first"
    );
    assert!(s.contains(r#"hx-get="api/dashboard/outages-partial""#));
    let x = outages_summary(&[d("<script>", 1)], Err("x".into()));
    assert!(!x.contains("<script> is") && x.contains("&lt;script&gt;"));
}

fn g(labels: &str, queued: u32, running: u32) -> HostedGroup {
    HostedGroup {
        repo: "r".into(),
        labels: labels.into(),
        queued,
        running,
        since: 0,
    }
}

// rivet: verifies REQ-HIST-001
#[test]
fn history_keeps_hosted_queued_and_running_per_runner_type() {
    let groups = vec![
        g("macos-latest", 6, 5),
        g("macos-latest", 3, 0),
        g("ubuntu-latest", 1, 12),
    ];
    let m = hosted_by_label(&groups);
    assert_eq!(m.get("macos-latest"), Some(&[9, 5]));
    assert_eq!(m.get("ubuntu-latest"), Some(&[1, 12]));

    // bounded: the busiest labels survive
    let many: Vec<HostedGroup> = (0..HOSTED_LABELS_MAX + 5)
        .map(|i| g(&format!("l{i:02}"), i as u32, 0))
        .collect();
    let m = hosted_by_label(&many);
    assert_eq!(m.len(), HOSTED_LABELS_MAX);
    assert!(m.contains_key(&format!("l{:02}", HOSTED_LABELS_MAX + 4)) && !m.contains_key("l00"));

    // carried by every history point, and old points without it still parse
    let snap = Snapshot {
        queue: Some(Queue {
            hosted_groups: groups,
            ..Default::default()
        }),
        ..Default::default()
    };
    let p = Point::from_snapshot(&snap);
    assert_eq!(p.hosted_by_label.get("macos-latest"), Some(&[9, 5]));
    let back: Point = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(back.hosted_by_label, p.hosted_by_label);
    let mut old = serde_json::to_value(&p).unwrap();
    old.as_object_mut().unwrap().remove("hosted_by_label");
    let old: Point = serde_json::from_value(old).unwrap();
    assert!(old.hosted_by_label.is_empty());
}

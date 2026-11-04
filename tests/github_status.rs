//! GitHub's status page in the Outages view, and GitHub-hosted jobs stuck in
//! the queue as a detected degradation (REQ-STATUS-001, REQ-DASH-009).

use warte::dashboard::fleet;
use warte::hub::model::{HostedJob, Queue, Snapshot};
use warte::hub::telemetry::{encode_snapshot, SeqCounters};
use warte::hub::{now, Hub};
use warte::status::parse_summary;

/// Shaped like githubstatus.com's summary.json during an Actions incident;
/// synthetic values.
const SUMMARY: &str = r#"{
  "page": {"id": "x", "name": "GitHub", "url": "https://www.githubstatus.com"},
  "status": {"indicator": "major", "description": "Partial System Outage"},
  "components": [
    {"id": "a", "name": "Actions", "status": "partial_outage", "group": false},
    {"id": "b", "name": "Git Operations", "status": "operational", "group": false},
    {"id": "c", "name": "Some group", "status": "degraded_performance", "group": true}
  ],
  "incidents": [
    {"id": "i1", "name": "Incident with Actions", "status": "investigating", "impact": "major",
     "created_at": "2026-11-04T17:40:33.675Z", "shortlink": "https://stspg.io/x1"},
    {"id": "i2", "name": "Old one", "status": "resolved", "impact": "minor",
     "created_at": "2026-11-04T09:49:00Z", "shortlink": "https://stspg.io/x2"},
    {"id": "i3", "name": "<script>bad</script>", "status": "monitoring", "impact": "minor",
     "created_at": "2026-11-04T18:00:00Z", "shortlink": "javascript:alert(1)"}
  ]
}"#;

// rivet: verifies REQ-STATUS-001
#[test]
fn the_summary_keeps_only_what_is_wrong_now() {
    let st = parse_summary(SUMMARY, "https://www.githubstatus.com/api/v2").unwrap();
    assert_eq!(st.status, "outage");
    let comps: Vec<_> = st
        .components
        .iter()
        .map(|c| (c.name.as_str(), c.status.as_str()))
        .collect();
    assert_eq!(comps, vec![("Actions", "partial outage")]);
    let titles: Vec<_> = st.incidents.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["Incident with Actions", "<script>bad</script>"]
    );
    assert_eq!(st.incidents[0].impact, "major");
    assert!(st.incidents[0].started_at.is_some());

    let calm = r#"{"status":{"indicator":"none","description":"All Systems Operational"},"components":[],"incidents":[]}"#;
    assert_eq!(parse_summary(calm, "s").unwrap().status, "operational");
    assert!(parse_summary("not json", "s").is_err());
}

// rivet: verifies REQ-DASH-009
#[test]
fn the_outages_view_shows_githubs_incidents_escaped_and_linked_safely() {
    let st = parse_summary(SUMMARY, "https://www.githubstatus.com/api/v2").unwrap();
    let html = fleet::github_status(Ok(&st));
    assert!(html.contains("GitHub status · outage"));
    assert!(html.contains("Incident with Actions") && html.contains("badge red"));
    assert!(html.contains("Actions") && html.contains("partial outage"));
    assert!(
        !html.contains("Old one"),
        "resolved incidents are not shown"
    );
    assert!(!html.contains("<script>bad") && html.contains("&lt;script&gt;"));
    assert!(
        !html.contains("javascript:"),
        "only https links are rendered"
    );

    let calm = parse_summary(
        r#"{"status":{"indicator":"none","description":"ok"},"components":[],"incidents":[]}"#,
        "s",
    )
    .unwrap();
    assert!(fleet::github_status(Ok(&calm)).contains("GitHub reports no incident"));
    let down = fleet::github_status(Err("timed out".into()));
    assert!(down.contains("could not be read") && down.contains("timed out"));
}

fn hub_with(s: &Snapshot) -> (tempfile::TempDir, Hub) {
    let ci: std::net::IpAddr = [192, 0, 2, 201].into();
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let mut seq = SeqCounters::default();
    for f in encode_snapshot(s, &mut seq) {
        hub.ingest(&f, ci, &[ci]);
    }
    (dir, hub)
}

fn hosted(labels: &str, status: &str, age_s: i64) -> HostedJob {
    HostedJob {
        repo: "kiln".into(),
        workflow: "CI".into(),
        job: "test (macos)".into(),
        status: status.into(),
        labels: labels.into(),
        since: now() - age_s,
        ..Default::default()
    }
}

// rivet: verifies REQ-DASH-008
#[test]
fn hosted_jobs_stuck_in_the_queue_are_a_detected_degradation() {
    let wait = fleet::HOSTED_WAIT_S;
    let snap = |jobs: Vec<HostedJob>| Snapshot {
        agent_version: "0.1.0".into(),
        host: "ci".into(),
        taken_at: now(),
        queue: Some(Queue {
            queued_complete: true,
            hosted: jobs,
            ..Default::default()
        }),
        ..Default::default()
    };
    let (_d, hub) = hub_with(&snap(vec![
        hosted("macos-latest", "queued", wait + 600),
        hosted("macos-latest", "queued", wait + 60),
        hosted("macos-latest", "in_progress", wait + 900),
        hosted("ubuntu-latest", "queued", 120),
    ]));
    let html = fleet::incidents(Some(&hub));
    assert!(
        html.contains("GitHub-hosted macos-latest: 2 job(s) queued"),
        "{html}"
    );
    assert!(
        !html.contains("GitHub-hosted ubuntu-latest"),
        "a short wait is normal"
    );

    let (_d2, calm) = hub_with(&snap(vec![hosted("macos-latest", "queued", wait - 60)]));
    assert!(fleet::incidents(Some(&calm)).contains("all clear"));
}

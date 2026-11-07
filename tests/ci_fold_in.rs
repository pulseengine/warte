//! What the ground station's `/ci` page offered is in warte's own API and
//! dashboard, with the same answers, so `/ci` can be retired (REQ-SMITHY-006).

use std::net::Ipv4Addr;
use warte::hub::model::{Automation, Point, Snapshot, TimerState};
use warte::hub::now;

fn snapshot() -> Snapshot {
    Snapshot {
        host: "ci-host".into(),
        automation: Automation {
            dispatcher: vec!["/srv/secret/path raw log line".into()],
            timers: vec![TimerState {
                unit: "runner-<watchdog>.timer".into(),
                active: false,
                next: Some("in 5 min".into()),
            }],
            ..Default::default()
        },
        ..Default::default()
    }
}

async fn serve(with_hub: bool) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let hub_dir = dir.path().join("hub");
    std::fs::create_dir_all(&hub_dir).unwrap();
    let t = now();
    let history: Vec<Point> = (0..30)
        .map(|i| Point {
            t: t - 60 * (30 - i),
            local_busy: (i % 5) as u32,
            online: 14,
            sh_queued: Some(i as u32),
            ..Default::default()
        })
        .collect();
    std::fs::write(
        hub_dir.join("hub-state.json"),
        serde_json::json!({"latest": snapshot(), "received_at": t, "history": history}).to_string(),
    )
    .unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: with_hub.then(|| warte::config::HubConfig {
            udp: "127.0.0.1:0".into(),
            allow: vec![Ipv4Addr::LOCALHOST.into()],
            data_dir: hub_dir.to_str().unwrap().into(),
            org: "pulseengine".into(),
            probe_self: false,
            costs_file: None,
            skill_usage_file: None,
        }),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let app = warte::server::build_router(warte::server::build_state(config, db).unwrap());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (dir, url)
}

async fn get(url: String) -> (u16, String) {
    let r = reqwest::get(url).await.unwrap();
    (r.status().as_u16(), r.text().await.unwrap())
}

fn this_month() -> String {
    chrono::Utc::now().format("%Y-%m").to_string()
}

// rivet: verifies REQ-SMITHY-006
#[tokio::test]
async fn warte_api_answers_what_ci_answered() {
    let (_d, base) = serve(true).await;
    let m = this_month();
    for (own, ci) in [
        ("/api/status.md".to_string(), "/ci/status.md".to_string()),
        (
            "/api/status.md?repo=spar&pr=1".into(),
            "/ci/status.md?repo=spar&pr=1".into(),
        ),
        (
            format!("/api/report?month={m}"),
            format!("/ci/api/report?month={m}"),
        ),
        (
            format!("/api/report.md?month={m}"),
            format!("/ci/report.md?month={m}"),
        ),
        (
            "/api/history/fleet?hours=2".into(),
            "/ci/api/history?hours=2".into(),
        ),
    ] {
        let a = get(format!("{base}{own}")).await;
        let b = get(format!("{base}{ci}")).await;
        assert_eq!(a.0, 200, "{own}: {}", a.1);
        assert_eq!(a, b, "{own} differs from {ci}");
    }
    let pts: serde_json::Value =
        serde_json::from_str(&get(format!("{base}/api/history/fleet?hours=2")).await.1).unwrap();
    assert_eq!(pts.as_array().unwrap().len(), 30);
    assert_eq!(get(format!("{base}/api/report?month=../../x")).await.0, 400);
    // the whole snapshot (with the automation's raw log lines) is not republished
    assert_eq!(get(format!("{base}/api/state")).await.0, 404);
}

// rivet: verifies REQ-SMITHY-006
#[tokio::test]
async fn without_a_ground_station_they_say_so() {
    let (_d, base) = serve(false).await;
    for p in [
        "/api/status.md",
        "/api/report",
        "/api/report.md",
        "/api/history/fleet",
        "/api/eta?repo=spar&pr=1",
    ] {
        let (code, body) = get(format!("{base}{p}")).await;
        assert_eq!(code, 503, "{p}");
        assert!(body.contains("no ground station"), "{p}: {body}");
    }
    let (code, html) = get(format!("{base}/api/dashboard/ci-history-partial")).await;
    assert_eq!(code, 200);
    assert!(html.contains("Ground station not configured"), "{html}");
}

// rivet: verifies REQ-SMITHY-006
#[tokio::test]
async fn the_dashboard_shows_trends_belt_and_timers() {
    let (_d, base) = serve(true).await;
    let (code, html) = get(format!("{base}/api/dashboard/ci-history-partial?range=6h")).await;
    assert_eq!(code, 200);
    assert!(html.contains("Fleet trends"), "{html}");
    assert!(html.contains("self-hosted runners busy"), "{html}");
    assert!(html.contains("<svg"), "a chart is drawn: {html}");
    assert!(
        html.contains(r#"class="rbtn on" hx-get="api/dashboard/ci-history-partial?range=6h"#),
        "{html}"
    );
    assert!(
        html.contains(&format!("Conveyor belt · {}", this_month())),
        "{html}"
    );
    assert!(html.contains("api/status.md"), "{html}");
    // a month outside the three offered falls back to this month
    let (_, other) = get(format!(
        "{base}/api/dashboard/ci-history-partial?month=1999-01"
    ))
    .await;
    assert!(
        other.contains(&format!("Conveyor belt · {}", this_month())),
        "{other}"
    );

    let (_, health) = get(format!("{base}/api/dashboard/health-partial")).await;
    assert!(health.contains("Timers on the CI host"), "{health}");
    assert!(
        health.contains("runner-&lt;watchdog&gt;.timer"),
        "escaped: {health}"
    );
    assert!(health.contains("inactive"), "{health}");
    assert!(
        !health.contains("/srv/secret/path"),
        "the automation's raw log lines stay off: {health}"
    );

    let (_, page) = get(format!("{base}/")).await;
    assert!(
        page.contains("api/dashboard/ci-history-partial"),
        "nav button"
    );
}

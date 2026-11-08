//! GitHub-hosted runners split by label and kind, per-pool trends
//! (REQ-RUNNERS-001, REQ-RUNNERS-003).

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use warte::hub::model::{HostedGroup, Point, PoolQueue, Queue, Runner, Snapshot};

fn snapshot(t: i64) -> Snapshot {
    Snapshot {
        host: "ci".into(),
        taken_at: t,
        runners: vec![
            Runner {
                unit: "r1".into(),
                local_busy: true,
                labels: vec!["self-hosted".into(), "rust-cpu".into()],
                ..Default::default()
            },
            Runner {
                unit: "r2".into(),
                labels: vec!["self-hosted".into(), "rust-cpu".into()],
                ..Default::default()
            },
        ],
        queue: Some(Queue {
            // a snapshot from an agent that still pools hosted jobs as one
            pools: BTreeMap::from([
                (
                    "rust-cpu".to_string(),
                    PoolQueue {
                        jobs_queued: 4,
                        ..Default::default()
                    },
                ),
                (
                    "hosted".to_string(),
                    PoolQueue {
                        jobs_queued: 5,
                        ..Default::default()
                    },
                ),
            ]),
            hosted_groups: vec![
                HostedGroup {
                    repo: "pulseengine/spar".into(),
                    labels: "ubuntu-latest".into(),
                    running: 0,
                    queued: 3,
                    since: t,
                },
                HostedGroup {
                    repo: "pulseengine/spar".into(),
                    labels: "macos-15".into(),
                    running: 1,
                    queued: 2,
                    since: t,
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    }
}

// rivet: verifies REQ-RUNNERS-003
#[test]
fn a_history_point_keeps_queued_and_busy_per_pool_with_hosted_split() {
    let p = Point::from_snapshot(&snapshot(1_000));
    assert_eq!(p.pools["rust-cpu"], [4, 1]);
    assert_eq!(p.pools["hosted:ubuntu-latest"], [3, 0]);
    assert_eq!(p.pools["hosted:macos-15"], [2, 1]);
    // the agent's unsplit "hosted" is replaced by the split groups, not added
    assert!(!p.pools.contains_key("hosted"), "{:?}", p.pools);
}

// rivet: verifies REQ-RUNNERS-003
#[tokio::test]
async fn ci_history_draws_one_queued_and_one_busy_chart_per_pool() {
    let dir = tempfile::tempdir().unwrap();
    let hub_dir = dir.path().join("hub");
    std::fs::create_dir_all(&hub_dir).unwrap();
    let t = warte::hub::now();
    let mut history: Vec<Point> = (0..20)
        .map(|i| Point::from_snapshot(&snapshot(t - 60 * (20 - i))))
        .collect();
    // a point from before warte kept pools: a gap, not an idle pool
    history.push(Point {
        t: t - 30,
        ..Default::default()
    });
    std::fs::write(
        hub_dir.join("hub-state.json"),
        serde_json::json!({"latest": snapshot(t), "received_at": t, "history": history})
            .to_string(),
    )
    .unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: Some(warte::config::HubConfig {
            udp: "127.0.0.1:0".into(),
            allow: vec![Ipv4Addr::LOCALHOST.into()],
            data_dir: hub_dir.to_str().unwrap().into(),
            org: "pulseengine".into(),
            probe_self: false,
            costs_file: None,
            skill_usage_file: None,
        }),
        runners: toml::from_str(
            r#"
            [[pool]]
            labels = ["self-hosted", "rust-cpu"]
            name = "Rust builders"
            "#,
        )
        .unwrap(),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let app = warte::server::build_router(warte::server::build_state(config, db).unwrap());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    let html = reqwest::get(format!("{base}/api/dashboard/ci-history-partial?range=6h"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("Per pool"), "{html}");
    for title in [
        "rust builders (self-hosted, linux) · queued",
        "rust builders (self-hosted, linux) · busy",
        "ubuntu-latest (github-hosted, linux) · queued",
        "macos-15 (github-hosted, macos) · running",
    ] {
        assert!(html.contains(title), "{title} missing");
    }
    assert!(
        !html.contains("hosted:"),
        "pools are shown by name, not key"
    );
}

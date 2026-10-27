//! Oracles for v1.4.0 part 2: warte's own CI telemetry is gone, and the views
//! that used it render the ground station's data instead (REQ-STORE-002).

use std::net::{IpAddr, Ipv4Addr};
use warte::dashboard::fleet;
use warte::hub::model::{PoolQueue, Queue, QueuedEntry, Runner, Snapshot, WaitStats};
use warte::hub::telemetry::{encode_snapshot, SeqCounters};
use warte::hub::{now, Hub};

const CI_HOST: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 201));

fn hub_with(s: &Snapshot) -> (tempfile::TempDir, Hub) {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let mut seq = SeqCounters::default();
    for f in encode_snapshot(s, &mut seq) {
        hub.ingest(&f, CI_HOST, &[CI_HOST]);
    }
    (dir, hub)
}

fn fleet_snapshot() -> Snapshot {
    let mut pools = std::collections::BTreeMap::new();
    pools.insert(
        "rust-cpu".to_string(),
        PoolQueue {
            jobs_queued: 7,
            oldest_s: 900,
            median_s: 300,
        },
    );
    let mut waits = std::collections::BTreeMap::new();
    waits.insert(
        "rust-cpu".to_string(),
        WaitStats {
            n: 40,
            p50: 60,
            p90: 600,
            max: 1200,
        },
    );
    Snapshot {
        agent_version: "0.1.0".into(),
        host: "ci-test".into(),
        taken_at: now(),
        runners: vec![
            Runner {
                id: 1,
                local_busy: true,
                gh_status: Some("online".into()),
                ..Default::default()
            },
            Runner {
                id: 2,
                gh_status: Some("online".into()),
                ..Default::default()
            },
        ],
        queue: Some(Queue {
            scanned_at: now(),
            jobs_queued: 7,
            runs_queued: 1,
            pools,
            waits,
            queued_complete: true,
            queued: vec![QueuedEntry {
                // author-controlled: must come out escaped
                repo: "spar".into(),
                run_id: 9,
                workflow: "<img src=x onerror=alert(1)>".into(),
                event: "pull_request".into(),
                branch: "feat/x".into(),
                pr: Some(511),
                pool: "rust-cpu".into(),
                jobs: 7,
                since: now() - 600,
            }],
            ..Default::default()
        }),
        ..Default::default()
    }
}

// rivet: verifies REQ-STORE-002
#[tokio::test]
async fn ci_telemetry_tables_are_gone_after_migration() {
    let dir = tempfile::tempdir().unwrap();
    let db = warte::db::Database::connect(&dir.path().join("state.db"))
        .await
        .unwrap();
    let tables: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table'")
            .fetch_all(db.pool())
            .await
            .unwrap();
    let tables: Vec<&str> = tables.iter().map(|(n,)| n.as_str()).collect();
    assert!(
        tables.contains(&"agents"),
        "migrations did not run: {tables:?}"
    );
    for gone in ["ci_jobs", "wait_history", "ci_status"] {
        assert!(
            !tables.contains(&gone),
            "{gone} still exists; CI telemetry belongs to the ground station"
        );
    }
}

// rivet: verifies REQ-STORE-002
#[test]
fn queue_view_shows_the_hubs_queue_escaped() {
    let (_d, hub) = hub_with(&fleet_snapshot());
    let html = fleet::queue(Some(&hub));
    assert!(html.contains("rust-cpu"), "pool missing: {html}");
    assert!(html.contains("#511"), "queued PR missing");
    assert!(html.contains("1/2"), "busy/total runners missing");
    assert!(
        !html.contains("<img"),
        "author-controlled workflow name was not escaped"
    );
    assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    assert!(
        html.contains(">fresh<"),
        "a just-received snapshot is fresh"
    );
}

// rivet: verifies REQ-STORE-002
#[test]
fn critical_path_uses_measured_waits_from_the_hub() {
    let (_d, hub) = hub_with(&fleet_snapshot());
    let html = fleet::bottlenecks(Some(&hub));
    assert!(
        html.contains("spar") && html.contains("7 jobs queued"),
        "{html}"
    );
    // p50 60s, p90 10m, max 20m for rust-cpu
    assert!(
        html.contains("<td>rust-cpu</td><td>40</td><td>60s</td><td>10m</td><td>20m</td>"),
        "{html}"
    );
}

// rivet: verifies REQ-STORE-002
#[test]
fn ask_gives_the_hubs_eta_answer() {
    let (_d, hub) = hub_with(&fleet_snapshot());
    let html = fleet::ask(Some(&hub), "spar", Some(511), None);
    assert!(html.contains("ETA for spar PR #511"), "{html}");
    assert!(
        html.contains(">queued<"),
        "status from /api/eta missing: {html}"
    );
    assert!(html.contains("Queued in <code>rust-cpu</code>"), "{html}");
    let not_queued = fleet::ask(Some(&hub), "spar", Some(1), None);
    assert!(not_queued.contains(">not_queued<"), "{not_queued}");
}

// rivet: verifies REQ-STORE-002
#[test]
fn stale_or_missing_data_is_said_not_hidden() {
    let mut old = fleet_snapshot();
    old.taken_at = now() - 3600;
    // received_at is set on ingest; to get a stale view, age the stored state
    let (dir, hub) = hub_with(&old);
    drop(hub);
    let state = dir.path().join("hub-state.json");
    // state is persisted every 5th frame; write it ourselves with an old receive time
    let snap = serde_json::to_value(&old).unwrap();
    std::fs::write(
        &state,
        serde_json::json!({"latest": snap, "received_at": now() - 3600, "history": []}).to_string(),
    )
    .unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    assert!(fleet::queue(Some(&hub)).contains(">stale<"));
    assert!(fleet::health_cards(Some(&hub)).contains("(stale)"));

    let dir2 = tempfile::tempdir().unwrap();
    let empty = Hub::open(dir2.path().to_str().unwrap(), "pulseengine").unwrap();
    assert!(fleet::queue(Some(&empty)).contains("no telemetry has arrived"));
    assert!(fleet::queue(None).contains("not configured"));
}

/// Serve the full warte router (as `warte serve` builds it) on an ephemeral port.
async fn serve_app(hub: Option<warte::config::HubConfig>) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub,
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let app = warte::server::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (dir, format!("http://{addr}"))
}

async fn status(url: String) -> u16 {
    reqwest::get(url).await.unwrap().status().as_u16()
}

// rivet: verifies REQ-STORE-002
#[tokio::test]
async fn the_router_serves_api_and_dashboard_where_documented() {
    let (_d, base) = serve_app(None).await;
    // the API is under /api, once (it was under /api/api before v1.4.0)
    assert_eq!(status(format!("{base}/api/health")).await, 200);
    assert_eq!(status(format!("{base}/api/api/health")).await, 404);
    // the dashboard page is reachable and shows the real version
    let page = reqwest::get(format!("{base}/")).await.unwrap();
    assert_eq!(page.status(), 200);
    let page = page.text().await.unwrap();
    assert!(
        page.contains(&format!("warte v{}", env!("CARGO_PKG_VERSION"))),
        "footer version"
    );
    // the rewired views answer, and say why they are empty
    let q = reqwest::get(format!("{base}/api/dashboard/queue-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(q.contains("not configured"), "{q}");
    for p in ["critical-path-partial", "health-partial"] {
        assert_eq!(
            status(format!("{base}/api/dashboard/{p}")).await,
            200,
            "{p}"
        );
    }
    // removed endpoints stay removed
    let c = reqwest::Client::new();
    for p in ["/api/webhooks/github", "/api/webhooks/gitlab"] {
        let r = c.post(format!("{base}{p}")).send().await.unwrap();
        assert_eq!(r.status(), 404, "{p}");
    }
    assert_eq!(
        status(format!("{base}/api/dashboard/ask?repo=x&pr=1")).await,
        404
    );
    // no hub configured: no /ci
    assert_eq!(status(format!("{base}/ci/healthz")).await, 404);
    // Ask is a read-only GET an anonymous dashboard viewer can use (a POST
    // needs auth and came back 401 with an empty body)
    let r = reqwest::get(format!(
        "{base}/api/dashboard/ask-partial?repo=pulseengine%2Frivet&branch=main"
    ))
    .await
    .unwrap();
    assert_eq!(r.status(), 200);
    let t = r.text().await.unwrap();
    assert!(t.contains("ETA for rivet @ main"), "{t}");
    // no view claims freshness it did not measure
    for p in [
        "queue-partial",
        "health-partial",
        "agents-partial",
        "overview-partial",
    ] {
        let t = reqwest::get(format!("{base}/api/dashboard/{p}"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(
            !t.contains(">fresh<"),
            "{p} claims fresh without a snapshot: {t}"
        );
    }
}

// rivet: verifies REQ-SMITHY-001
#[tokio::test]
async fn serve_with_a_hub_mounts_the_ground_station_at_ci() {
    let hub_dir = tempfile::tempdir().unwrap();
    let cfg = warte::config::HubConfig {
        udp: "127.0.0.1:0".into(),
        allow: vec![Ipv4Addr::LOCALHOST.into()],
        data_dir: hub_dir.path().to_str().unwrap().into(),
        org: "pulseengine".into(),
    };
    let (_d, base) = serve_app(Some(cfg)).await;
    assert_eq!(status(format!("{base}/ci/healthz")).await, 200);
    assert_eq!(status(format!("{base}/ci/api/openapi.json")).await, 200);
    // before telemetry: the hub says so, and so does the view
    assert_eq!(
        status(format!("{base}/ci/api/eta?repo=spar&pr=1")).await,
        503
    );
    let q = reqwest::get(format!("{base}/api/dashboard/queue-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(q.contains("no telemetry has arrived"), "{q}");
}

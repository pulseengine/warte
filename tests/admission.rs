//! Oracles for #55: machine-readable queue and PR admission for agents.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use warte::admission::{self, Kind, Verdict};
use warte::hub::model::{HostedJob, JobContext, PoolQueue, Queue, QueuedEntry, Runner, Snapshot};
use warte::hub::now;

fn runner(id: u32, busy: bool, repo: Option<&str>, run_id: u64) -> Runner {
    Runner {
        id,
        local_busy: busy,
        gh_busy: Some(busy),
        ctx: repo.map(|r| JobContext {
            repo: format!("pulseengine/{r}"),
            run_id,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn queued(repo: &str, run_id: u64, pool: &str, jobs: u32) -> QueuedEntry {
    QueuedEntry {
        repo: repo.into(),
        run_id,
        workflow: "CI".into(),
        event: "pull_request".into(),
        branch: "b".into(),
        pr: Some(1),
        pool: pool.into(),
        jobs,
        since: now() - 60,
    }
}

/// A saturated fleet: 2/2 runners busy (one on spar), 60 self-hosted jobs
/// queued, spar has 2 queued runs, rivet none; hosted pool backed up.
fn saturated() -> Snapshot {
    let mut pools = BTreeMap::new();
    pools.insert(
        "rust-cpu".to_string(),
        PoolQueue {
            jobs_queued: 60,
            oldest_s: 1800,
            median_s: 600,
        },
    );
    pools.insert(
        "hosted".to_string(),
        PoolQueue {
            jobs_queued: 105,
            oldest_s: 1140,
            median_s: 720,
        },
    );
    Snapshot {
        host: "ci-test".into(),
        taken_at: now(),
        runners: vec![
            runner(1, true, Some("spar"), 900),
            runner(2, true, Some("gale"), 901),
        ],
        queue: Some(Queue {
            scanned_at: now(),
            jobs_queued: 165,
            pools,
            queued_complete: true,
            queued: vec![
                queued("spar", 10, "rust-cpu", 30),
                queued("spar", 11, "rust-cpu", 30),
            ],
            hosted: vec![HostedJob {
                repo: "spar".into(),
                status: "in_progress".into(),
                run_id: 12,
                ..Default::default()
            }],
            hosted_queued: 105,
            hosted_running: 1,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn idle() -> Snapshot {
    Snapshot {
        host: "ci-test".into(),
        taken_at: now(),
        runners: vec![runner(1, false, None, 0), runner(2, false, None, 0)],
        queue: Some(Queue {
            scanned_at: now(),
            queued_complete: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}

// rivet: verifies REQ-QUEUE-001
#[test]
fn queue_summarises_self_hosted_hosted_and_repo() {
    let s = saturated();
    let q = admission::queue(&s, now(), now(), Some("spar"));
    assert_eq!((q.self_hosted.busy, q.self_hosted.total), (2, 2));
    assert_eq!(
        q.self_hosted.queued_jobs, 60,
        "hosted jobs must not count as self-hosted"
    );
    assert_eq!(q.hosted.queued_jobs, 105);
    assert_eq!(
        (q.hosted.oldest_wait_s, q.hosted.median_wait_s),
        (1140, 720)
    );
    let r = q.repo.expect("repo requested");
    assert_eq!(r.queued_runs, 2, "distinct queued runs, not jobs");
    // runner 1 runs spar run 900, a hosted job runs spar run 12
    assert_eq!(r.running_runs, 2);
    assert!(!q.as_of.stale);
    assert!(admission::queue(&s, now(), now(), None).repo.is_none());
}

// rivet: verifies REQ-QUEUE-002
#[test]
fn security_and_red_main_are_always_open() {
    for kind in [Kind::Security, Kind::RedMain] {
        let a = admission::admit(&saturated(), now(), now(), "spar", kind);
        assert_eq!(a.verdict, Verdict::Open, "{kind:?} on a saturated fleet");
    }
}

// rivet: verifies REQ-QUEUE-002
#[test]
fn docs_batch_whenever_the_repo_has_ci_in_flight() {
    let s = saturated();
    assert_eq!(
        admission::admit(&s, now(), now(), "spar", Kind::Docs).verdict,
        Verdict::Batch
    );
    // rivet has nothing queued or running: docs may open even on a busy fleet
    assert_eq!(
        admission::admit(&s, now(), now(), "rivet", Kind::Docs).verdict,
        Verdict::Open
    );
}

// rivet: verifies REQ-QUEUE-002
#[test]
fn features_hold_on_a_saturated_fleet_and_open_on_an_idle_one() {
    let busy = admission::admit(&saturated(), now(), now(), "rivet", Kind::Feature);
    assert_eq!(busy.verdict, Verdict::Hold, "{}", busy.reason);
    assert!(
        busy.recheck_s.unwrap_or(0) > 0,
        "hold must say when to ask again"
    );
    let calm = admission::admit(&idle(), now(), now(), "rivet", Kind::Feature);
    assert_eq!(calm.verdict, Verdict::Open, "{}", calm.reason);
    assert_eq!(calm.recheck_s, None);
}

// rivet: verifies REQ-QUEUE-002
#[test]
fn fixes_batch_behind_their_own_repo_but_do_not_hold() {
    let a = admission::admit(&saturated(), now(), now(), "spar", Kind::Fix);
    assert_eq!(a.verdict, Verdict::Batch, "{}", a.reason);
    let b = admission::admit(&saturated(), now(), now(), "rivet", Kind::Fix);
    assert_eq!(
        b.verdict,
        Verdict::Open,
        "a fix in a quiet repo opens: {}",
        b.reason
    );
}

// rivet: verifies REQ-QUEUE-002
#[test]
fn stale_data_never_gates() {
    let s = saturated();
    let received = now() - 3600;
    for kind in [Kind::Docs, Kind::Fix, Kind::Feature] {
        let a = admission::admit(&s, received, now(), "spar", kind);
        assert_eq!(a.verdict, Verdict::Open, "{kind:?} on hour-old data");
        assert!(a.reason.contains("stale"), "{}", a.reason);
    }
}

async fn serve(snapshot: Option<&Snapshot>) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let hub_dir = dir.path().join("hub");
    std::fs::create_dir_all(&hub_dir).unwrap();
    if let Some(s) = snapshot {
        std::fs::write(
            hub_dir.join("hub-state.json"),
            serde_json::json!({"latest": s, "received_at": now(), "history": []}).to_string(),
        )
        .unwrap();
    }
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
        }),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let app = warte::server::build_router(warte::server::build_state(config, db).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (dir, format!("http://{addr}"))
}

// rivet: verifies REQ-QUEUE-001
// rivet: verifies REQ-QUEUE-002
#[tokio::test]
async fn endpoints_answer_as_documented() {
    let (_d, base) = serve(Some(&saturated())).await;
    let get = |p: &str| reqwest::get(format!("{base}{p}"));

    let q: serde_json::Value = get("/api/queue?repo=spar")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(q["self_hosted"]["busy"], 2);
    assert_eq!(q["hosted"]["queued_jobs"], 105);
    assert_eq!(q["repo"]["queued_runs"], 2);
    assert_eq!(q["as_of"]["stale"], false);

    let a: serde_json::Value = get("/api/admission?repo=rivet&kind=feature")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(a["verdict"], "hold");
    assert!(a["recheck_s"].as_i64().unwrap() > 0);
    let a: serde_json::Value = get("/api/admission?repo=spar&kind=red-main")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(a["verdict"], "open");

    for bad in [
        "/api/admission?repo=spar",
        "/api/admission?repo=spar&kind=yolo",
        "/api/admission?repo=pulseengine/spar&kind=fix",
        "/api/queue?repo=../x",
    ] {
        assert_eq!(get(bad).await.unwrap().status(), 400, "{bad}");
    }

    // the contract is published, and every path in it is served
    let doc: serde_json::Value = get("/api/openapi.json")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let paths = doc["paths"].as_object().expect("openapi lists paths");
    assert!(paths.contains_key("/api/queue") && paths.contains_key("/api/admission"));
    for p in paths.keys() {
        let q = match p.as_str() {
            "/api/admission" => "?repo=spar&kind=docs",
            "/api/watch" => "?repo=spar&pr=1&timeout=0",
            "/api/fleet" => continue, // 404 without a model; tests/fleet_model.rs covers it
            _ => "",
        };
        assert_eq!(get(&format!("{p}{q}")).await.unwrap().status(), 200, "{p}");
    }
}

// rivet: verifies REQ-QUEUE-001
#[tokio::test]
async fn no_snapshot_is_503_not_an_empty_answer() {
    let (_d, base) = serve(None).await;
    for p in ["/api/queue", "/api/admission?repo=spar&kind=fix"] {
        let r = reqwest::get(format!("{base}{p}")).await.unwrap();
        assert_eq!(r.status(), 503, "{p}");
    }
}

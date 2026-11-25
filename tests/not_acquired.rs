//! Oracles for REQ-QUEUE-003: hosted jobs GitHub never gave a runner are
//! classified apart from code failures and counted per pool and day.

use serde_json::json;
use std::net::Ipv4Addr;
use warte::hub::model::JobRecord;
use warte::requeue::{counts, may_be_not_acquired, says_not_acquired, PoolDay, NOT_ACQUIRED};

/// A job from GitHub's job listing that meets all three listing conditions.
fn unacquired_job() -> serde_json::Value {
    json!({
        "id": 7, "status": "completed", "conclusion": "cancelled",
        "runner_name": null, "runner_id": null,
        "steps": []
    })
}

// rivet: verifies REQ-QUEUE-003
#[test]
fn each_listing_condition_failing_alone_gives_no_candidate() {
    assert!(may_be_not_acquired(&unacquired_job()));
    // a step that never started still counts as no step run
    let mut j = unacquired_job();
    j["steps"] = json!([{"name": "Set up job", "started_at": null, "conclusion": null}]);
    assert!(may_be_not_acquired(&j));

    let mut j = unacquired_job();
    j["status"] = json!("in_progress");
    assert!(!may_be_not_acquired(&j), "not finished");
    for c in ["failure", "success", "timed_out", "skipped"] {
        let mut j = unacquired_job();
        j["conclusion"] = json!(c);
        assert!(!may_be_not_acquired(&j), "{c}");
    }
    let mut j = unacquired_job();
    j["runner_name"] = json!("GitHub Actions 12");
    assert!(!may_be_not_acquired(&j), "a runner took it");
    let mut j = unacquired_job();
    j["runner_id"] = json!(12);
    assert!(!may_be_not_acquired(&j), "a runner id");
    let mut j = unacquired_job();
    j["steps"] = json!([{"name": "Set up job", "started_at": "2026-11-25T10:00:00Z", "conclusion": "success"}]);
    assert!(!may_be_not_acquired(&j), "a step ran");
}

// rivet: verifies REQ-QUEUE-003
#[test]
fn only_the_not_acquired_annotation_classifies() {
    let yes = json!([
        {"message": "The operation was canceled."},
        {"message": "The job was not acquired by Runner of type hosted even after multiple attempts"}
    ]);
    assert!(says_not_acquired(&yes));
    assert!(!says_not_acquired(
        &json!([{"message": "The operation was canceled."}])
    ));
    assert!(!says_not_acquired(&json!([])));
    assert!(
        !says_not_acquired(&json!({"message": "not acquired by Runner"})),
        "not an array"
    );
}

fn rec(id: u64, repo: &str, day: &str, pool: &str, sig: Option<&str>) -> JobRecord {
    JobRecord {
        d: day.into(),
        id,
        run: id,
        att: 1,
        repo: repo.into(),
        pool: pool.into(),
        c: warte::hub::now() - 60,
        r: "cancelled".into(),
        sig: sig.map(String::from),
        ..Default::default()
    }
}

// rivet: verifies REQ-QUEUE-003
#[test]
fn counts_group_by_pool_and_day_newest_first() {
    let recs = vec![
        rec(
            1,
            "relay",
            "2026-11-25",
            "macos-15-arm64",
            Some(NOT_ACQUIRED),
        ),
        rec(
            2,
            "relay",
            "2026-11-25",
            "macos-15-arm64",
            Some(NOT_ACQUIRED),
        ),
        rec(3, "spar", "2026-11-25", "ubuntu-24.04", Some(NOT_ACQUIRED)),
        rec(
            4,
            "relay",
            "2026-11-24",
            "macos-15-arm64",
            Some(NOT_ACQUIRED),
        ),
        // a code failure and a cancelled job of another class are not counted
        rec(
            5,
            "relay",
            "2026-11-25",
            "macos-15-arm64",
            Some("timeout:example.org"),
        ),
        rec(6, "relay", "2026-11-25", "macos-15-arm64", None),
    ];
    let pd = |pool: &str, day: &str, jobs| PoolDay {
        pool: pool.into(),
        day: day.into(),
        jobs,
    };
    assert_eq!(
        counts(&recs, None),
        vec![
            pd("macos-15-arm64", "2026-11-25", 2),
            pd("ubuntu-24.04", "2026-11-25", 1),
            pd("macos-15-arm64", "2026-11-24", 1),
        ]
    );
    assert_eq!(
        counts(&recs, Some("spar")),
        vec![pd("ubuntu-24.04", "2026-11-25", 1)]
    );
    assert!(counts(&recs, Some("gale")).is_empty());
}

// rivet: verifies REQ-QUEUE-003
#[tokio::test]
async fn the_queue_api_reports_the_week() {
    let dir = tempfile::tempdir().unwrap();
    let hub_dir = dir.path().join("hub");
    std::fs::create_dir_all(&hub_dir).unwrap();
    let now = warte::hub::now();
    let today = warte::hub::report::day_of(now);
    let old = warte::hub::report::day_of(now - 10 * 86_400);
    {
        let hub = warte::hub::Hub::open(hub_dir.to_str().unwrap(), "pulseengine").unwrap();
        let mut stale = rec(9, "relay", &old, "macos-15-arm64", Some(NOT_ACQUIRED));
        stale.c = now - 10 * 86_400;
        hub.ingest_ledger(
            vec![
                rec(1, "relay", &today, "macos-15-arm64", Some(NOT_ACQUIRED)),
                rec(2, "spar", &today, "macos-15-arm64", Some(NOT_ACQUIRED)),
                stale,
            ],
            vec![],
        );
    }
    std::fs::write(
        hub_dir.join("hub-state.json"),
        json!({"latest": warte::hub::model::Snapshot { taken_at: now, ..Default::default() },
               "received_at": now, "history": []})
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
            quic: None,
            quic_names: vec![],
            require_quic: false,
        }),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let app = warte::server::build_router(warte::server::build_state(config, db).unwrap());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    let get = |p: &str| {
        let u = format!("{base}{p}");
        async move {
            reqwest::get(u)
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap()
        }
    };
    let all = get("/api/queue").await;
    assert_eq!(
        all["not_acquired"],
        json!([{"pool": "macos-15-arm64", "day": today, "jobs": 2}]),
        "the 10-day-old job is outside the week"
    );
    let spar = get("/api/queue?repo=spar").await;
    assert_eq!(spar["not_acquired"][0]["jobs"], 1);
    let gale = get("/api/queue?repo=gale").await;
    assert_eq!(gale["not_acquired"], json!([]));
}

// rivet: verifies REQ-QUEUE-003
#[tokio::test]
async fn the_ledger_reads_annotations_only_for_candidates_and_classifies() {
    use axum::{extract::Request, response::IntoResponse, Router};
    use std::sync::{Arc, Mutex};
    use warte::ghsource::{ledger, Client};
    let now = warte::hub::now();
    let iso = |t: i64| {
        chrono::DateTime::from_timestamp(t, 0)
            .unwrap()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    };
    let today = warte::hub::report::day_of(now);
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let s2 = seen.clone();
    let api = Router::new().fallback(move |req: Request| {
        let (s2, day) = (s2.clone(), today.clone());
        async move {
            let path = req.uri().path().to_string();
            let q = req.uri().query().unwrap_or_default().to_string();
            s2.lock().unwrap().push(path.clone());
            let job = |id: u64, runner: Option<&str>| {
                json!({"id": id, "run_attempt": 1, "name": "macos", "status": "completed",
                       "conclusion": "cancelled", "labels": ["macos-15"], "runner_name": runner,
                       "steps": [], "created_at": iso(now - 200), "completed_at": iso(now - 60)})
            };
            match path.as_str() {
                "/repos/acme/relay/actions/runs" if q.contains(&format!("created={day}")) => {
                    axum::Json(json!({"workflow_runs": [{"id": 8, "name": "CI", "event": "merge_group",
                        "status": "completed", "run_attempt": 1, "created_at": iso(now - 300),
                        "head_branch": "gh-readonly-queue/main/pr-636-abc"}]}))
                    .into_response()
                }
                "/repos/acme/relay/actions/runs" => {
                    axum::Json(json!({"workflow_runs": []})).into_response()
                }
                "/repos/acme/relay/actions/runs/8/jobs" => axum::Json(json!({"jobs": [
                    job(80, None), job(81, None), job(82, Some("GitHub Actions 3")),
                ]}))
                .into_response(),
                "/repos/acme/relay/check-runs/80/annotations" => axum::Json(json!([
                    {"message": "The job was not acquired by Runner of type hosted even after multiple attempts"}
                ]))
                .into_response(),
                "/repos/acme/relay/check-runs/81/annotations" => {
                    axum::Json(json!([{"message": "The operation was canceled."}])).into_response()
                }
                _ => axum::http::StatusCode::NOT_FOUND.into_response(),
            }
        }
    });
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, api).await.unwrap() });
    let c = Client::new(&warte::config::GithubConfig {
        api_url: base,
        org: "acme".into(),
        token: Some("t0ken".into()),
        ..Default::default()
    })
    .unwrap();
    let mut st = ledger::LedgerState::default();
    let (recs, _) = ledger::pass(&c, "acme", &["relay".to_string()], &mut st, now, 0, &[])
        .await
        .unwrap();
    let sig = |id: u64| recs.iter().find(|r| r.id == id).unwrap().sig.clone();
    assert_eq!(sig(80).as_deref(), Some(NOT_ACQUIRED), "{recs:?}");
    assert_eq!((sig(81), sig(82)), (None, None));
    let notes: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.ends_with("/annotations"))
        .cloned()
        .collect();
    assert_eq!(
        notes,
        vec![
            "/repos/acme/relay/check-runs/80/annotations",
            "/repos/acme/relay/check-runs/81/annotations"
        ],
        "a job a runner took is never worth a read"
    );
}

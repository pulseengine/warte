//! Oracles for REQ-RUNNERS-005: without a fleet agent, the GitHub source
//! keeps the job ledger, and a day counts as complete only once every run
//! created on it has finished and been read.

use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Router,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use warte::config::GithubConfig;
use warte::ghsource::{ledger, Client};

/// 2026-11-10T12:00:00Z
const NOON: i64 = 1_794_312_000;

fn iso(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

#[derive(Clone, Default)]
struct Mock {
    seen: Arc<Mutex<Vec<String>>>,
    /// Run 22 (created 2026-11-10) has finished.
    done22: Arc<Mutex<bool>>,
    /// The job listing of run 21 fails with a 502.
    fail21: Arc<Mutex<bool>>,
}

fn run(id: u64, status: &str, created: i64) -> serde_json::Value {
    json!({"id": id, "name": "CI", "event": "push", "status": status,
           "run_attempt": 1, "created_at": iso(created)})
}

fn job(
    id: u64,
    labels: &[&str],
    runner: &str,
    c: i64,
    s: Option<i64>,
    e: i64,
    r: &str,
) -> serde_json::Value {
    json!({"id": id, "run_attempt": 1, "name": format!("job{id}"), "status": "completed",
           "conclusion": r, "labels": labels, "runner_name": runner,
           "created_at": iso(c), "started_at": s.map(iso), "completed_at": iso(e)})
}

async fn github(State(m): State<Mock>, req: Request) -> Response {
    let uri = req.uri().to_string();
    let inm = req.headers().get("if-none-match").is_some();
    let tag = format!("\"{uri}\"");
    let body = |m: &Mock| -> Option<serde_json::Value> {
        let path = req.uri().path();
        let q = req.uri().query().unwrap_or_default();
        let done22 = *m.done22.lock().unwrap();
        Some(match path {
            "/repos/acme/spar/actions/runs" if q.contains("created=2026-11-09") => {
                json!({"workflow_runs": [
                    run(20, "completed", NOON - 86_400),
                    run(21, "completed", NOON - 80_000)
                ]})
            }
            "/repos/acme/spar/actions/runs" if q.contains("created=2026-11-10") => {
                json!({"workflow_runs": [
                    run(22, if done22 { "completed" } else { "in_progress" }, NOON - 600)
                ]})
            }
            "/repos/acme/spar/actions/runs" if q.contains("created=") => {
                json!({"workflow_runs": []})
            }
            "/repos/acme/spar/actions/runs/20/jobs" => json!({"jobs": [
                job(200, &["self-hosted", "rust-cpu"], "r1", NOON - 86_400, Some(NOON - 86_300), NOON - 86_000, "success"),
                job(201, &["ubuntu-latest"], "", NOON - 86_400, None, NOON - 86_390, "skipped")
            ]}),
            "/repos/acme/spar/actions/runs/21/jobs" => json!({"jobs": [
                job(210, &["ubuntu-latest"], "GitHub Actions 7", NOON - 80_000, Some(NOON - 79_990), NOON - 79_000, "failure")
            ]}),
            "/repos/acme/spar/actions/runs/22/jobs" => json!({"jobs": [
                job(220, &["self-hosted", "rust-cpu"], "r1", NOON - 600, Some(NOON - 500), NOON - 100, "success")
            ]}),
            _ => return None,
        })
    };
    // a listing that has not changed since its ETag answers 304, as GitHub does
    let unchanged = inm && !(uri.contains("created=2026-11-10") && *m.done22.lock().unwrap());
    if unchanged {
        m.seen.lock().unwrap().push(format!("304 {uri}"));
        return StatusCode::NOT_MODIFIED.into_response();
    }
    m.seen.lock().unwrap().push(uri.clone());
    if *m.fail21.lock().unwrap() && uri.contains("/runs/21/jobs") {
        return StatusCode::BAD_GATEWAY.into_response();
    }
    match body(&m) {
        Some(b) => {
            let mut r = axum::Json(b).into_response();
            r.headers_mut().insert("etag", tag.parse().unwrap());
            r
        }
        None => (StatusCode::NOT_FOUND, format!("mock has no {uri}")).into_response(),
    }
}

async fn mock() -> (Mock, String) {
    let m = Mock::default();
    let app = Router::new().fallback(github).with_state(m.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (m, base)
}

fn cfg(base: &str) -> GithubConfig {
    GithubConfig {
        api_url: base.into(),
        org: "acme".into(),
        token: Some("t0ken".into()),
        ..Default::default()
    }
}

// rivet: verifies REQ-RUNNERS-005
#[tokio::test]
async fn finished_jobs_are_recorded_and_a_day_closes_only_when_every_run_is_read() {
    let (m, base) = mock().await;
    let c = Client::new(&cfg(&base)).unwrap();
    let repos = vec!["spar".to_string()];
    let mut st = ledger::LedgerState::default();

    let (recs, mans) = ledger::pass(&c, "acme", &repos, &mut st, NOON, 2, &[])
        .await
        .unwrap();
    let mut ids: Vec<u64> = recs.iter().map(|r| r.id).collect();
    ids.sort();
    assert_eq!(ids, vec![200, 201, 210], "only finished runs are read");
    let r200 = recs.iter().find(|r| r.id == 200).unwrap();
    assert_eq!(r200.d, "2026-11-09");
    assert_eq!(
        (r200.repo.as_str(), r200.wf.as_str(), r200.ev.as_str()),
        ("spar", "CI", "push")
    );
    assert_eq!(
        (r200.pool.as_str(), r200.runner.as_str(), r200.r.as_str()),
        ("rust-cpu", "r1", "success")
    );
    assert_eq!(
        (r200.c, r200.s, r200.e),
        (NOON - 86_400, Some(NOON - 86_300), Some(NOON - 86_000))
    );
    let r201 = recs.iter().find(|r| r.id == 201).unwrap();
    assert_eq!(
        (r201.pool.as_str(), r201.runner.as_str(), r201.s),
        ("hosted:ubuntu-latest", "", None)
    );

    let man = |mans: &[warte::hub::model::DayManifest], d: &str| {
        mans.iter()
            .find(|x| x.day == d)
            .cloned()
            .unwrap_or_else(|| panic!("no manifest for {d}"))
    };
    let y = man(&mans, "2026-11-09");
    assert_eq!(
        (y.n, y.x, y.closed),
        (3, 200 ^ 201 ^ 210, true),
        "yesterday is over and fully read"
    );
    let t = man(&mans, "2026-11-10");
    assert_eq!((t.n, t.closed), (0, false), "run 22 is still running");
    assert!(
        man(&mans, "2026-11-08").closed,
        "an empty finished day closes too"
    );

    // second pass: closed days are not listed again, read runs are not re-read,
    // and the open day's listing goes out conditionally
    m.seen.lock().unwrap().clear();
    let (recs, _) = ledger::pass(&c, "acme", &repos, &mut st, NOON + 60, 2, &[])
        .await
        .unwrap();
    assert!(recs.is_empty());
    let seen = m.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(
        seen[0].starts_with("304 ") && seen[0].contains("created=2026-11-10"),
        "{seen:?}"
    );

    // run 22 finishes: read once; its day closes only after the day is over
    *m.done22.lock().unwrap() = true;
    let (recs, mans) = ledger::pass(&c, "acme", &repos, &mut st, NOON + 120, 2, &[])
        .await
        .unwrap();
    assert_eq!(recs.iter().map(|r| r.id).collect::<Vec<_>>(), vec![220]);
    let t = man(&mans, "2026-11-10");
    assert_eq!(
        (t.n, t.x, t.closed),
        (1, 220, false),
        "the day is not over yet"
    );
    let tomorrow = NOON + 14 * 3600; // 2026-11-11T02:00Z
    let (recs, mans) = ledger::pass(&c, "acme", &repos, &mut st, tomorrow, 2, &[])
        .await
        .unwrap();
    assert!(recs.is_empty());
    assert!(man(&mans, "2026-11-10").closed);

    // the state survives a restart: closed days stay closed, counts kept
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("ghledger.json");
    st.save(&p).unwrap();
    let back = ledger::LedgerState::load(&p);
    assert_eq!(back.manifests(), st.manifests());
}

// rivet: verifies REQ-RUNNERS-005
#[tokio::test]
async fn the_monthly_report_is_built_from_the_github_ledger_once_a_day_is_complete() {
    let (_m, base) = mock().await;
    let c = Client::new(&cfg(&base)).unwrap();
    let mut st = ledger::LedgerState::default();
    let (recs, mans) = ledger::pass(&c, "acme", &["spar".into()], &mut st, NOON, 2, &[])
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let hub = warte::hub::Hub::open(dir.path().to_str().unwrap(), "acme").unwrap();
    hub.ingest_ledger(recs, mans);
    let r = hub.month_report("2026-11").await.unwrap();
    assert!(!r.preliminary, "{:?}", r.days);
    assert_eq!(r.complete_days, 1);
    let d = r
        .days
        .iter()
        .find(|d| d.day == "2026-11-09")
        .expect("day row");
    assert_eq!(d.status, "complete");
}

// rivet: verifies REQ-RUNNERS-005
#[tokio::test]
async fn a_failed_pass_changes_nothing_so_the_next_one_reads_the_same_runs() {
    let (m, base) = mock().await;
    let c = Client::new(&cfg(&base)).unwrap();
    let repos = vec!["spar".to_string()];
    let mut st = ledger::LedgerState::default();
    *m.fail21.lock().unwrap() = true;
    assert!(ledger::pass(&c, "acme", &repos, &mut st, NOON, 2, &[])
        .await
        .is_err());
    assert_eq!(
        st,
        ledger::LedgerState::default(),
        "run 20 was read, but its records were not handed on"
    );
    *m.fail21.lock().unwrap() = false;
    let (recs, _) = ledger::pass(&c, "acme", &repos, &mut st, NOON, 2, &[])
        .await
        .unwrap();
    assert_eq!(recs.len(), 3);
}

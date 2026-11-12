//! Oracles for #81: GitHub's API as a CI source, against a mock GitHub served
//! at the root of its base URL, as `https://api.<tenant>.ghe.com` is.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use warte::config::GithubConfig;
use warte::ghsource::{self, Client, GhError};
use warte::hub::now;

fn iso(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// Request URI and Authorization header, as the mock saw them.
type Seen = Vec<(String, Option<String>)>;
/// A status code and headers to answer every request with.
type Failure = Option<(u16, Vec<(&'static str, String)>)>;

#[derive(Clone, Default)]
struct Mock {
    seen: Arc<Mutex<Seen>>,
    /// Answer every request with this status (401/403 cases).
    fail: Arc<Mutex<Failure>>,
    /// Only the runners listing fails with 403 (token without runner scope).
    runners_forbidden: Arc<Mutex<bool>>,
    /// Answer with ETags and honour If-None-Match.
    etags: Arc<Mutex<bool>>,
    /// Send `x-ratelimit-remaining` (and a reset 10 min ahead).
    remaining: Arc<Mutex<Option<i64>>>,
    /// The runners listing answers this status instead (403 or 404).
    runners_status: Arc<Mutex<Option<u16>>>,
    /// Run 10 also has a job for an ARC runner group with no self-hosted label.
    arc: Arc<Mutex<bool>>,
}

/// The mock behind ETags and rate-limit headers, as GitHub answers: a 200
/// carries `etag: "<uri>"`, a matching If-None-Match gets a bodiless 304.
async fn github(State(m): State<Mock>, req: Request) -> Response {
    let uri = req.uri().to_string();
    let inm = req
        .headers()
        .get("if-none-match")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let etags = *m.etags.lock().unwrap();
    let remaining = *m.remaining.lock().unwrap();
    let tag = format!("\"{uri}\"");
    let mut r = if etags && inm.as_deref() == Some(tag.as_str()) {
        m.seen.lock().unwrap().push((format!("304 {uri}"), None));
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        answer(State(m.clone()), req).await
    };
    if etags && r.status() == StatusCode::OK {
        r.headers_mut().insert("etag", tag.parse().unwrap());
    }
    if let Some(n) = remaining {
        r.headers_mut()
            .insert("x-ratelimit-remaining", n.to_string().parse().unwrap());
        r.headers_mut().insert(
            "x-ratelimit-reset",
            (now() + 600).to_string().parse().unwrap(),
        );
    }
    r
}

async fn answer(State(m): State<Mock>, req: Request) -> Response {
    let uri = req.uri().to_string();
    let auth = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    m.seen.lock().unwrap().push((uri.clone(), auth));
    let base = format!("http://{}", req.headers()["host"].to_str().unwrap());
    if let Some((code, headers)) = m.fail.lock().unwrap().clone() {
        let mut h = HeaderMap::new();
        for (k, v) in headers {
            h.insert(k, v.parse().unwrap());
        }
        return (
            StatusCode::from_u16(code).unwrap(),
            h,
            axum::Json(json!({"message": "Bad credentials"})),
        )
            .into_response();
    }
    let t = now();
    let path = req.uri().path();
    let q = req.uri().query().unwrap_or_default();
    let body = match path {
        "/orgs/acme/actions/runners" if m.runners_status.lock().unwrap().is_some() => {
            let code = m.runners_status.lock().unwrap().unwrap();
            return (
                StatusCode::from_u16(code).unwrap(),
                axum::Json(json!({"message": "Not Found"})),
            )
                .into_response();
        }
        "/repos/acme/spar/actions/runs/10/jobs" if *m.arc.lock().unwrap() => json!({"jobs": [
            {"name": "build", "status": "queued", "labels": ["self-hosted", "rust-cpu"], "created_at": iso(t - 120)},
            {"name": "lint", "status": "queued", "labels": ["ubuntu-latest"], "created_at": iso(t - 60)},
            {"name": "gpu", "status": "queued", "labels": ["ARC-large"], "created_at": iso(t - 30)}
        ]}),
        "/orgs/acme/actions/runners" if *m.runners_forbidden.lock().unwrap() => {
            return (
                StatusCode::FORBIDDEN,
                axum::Json(json!({"message": "Resource not accessible"})),
            )
                .into_response()
        }
        "/orgs/acme/actions/runners" if q.contains("page=2") => json!({"runners": [
            {"name": "r2", "status": "offline", "busy": false, "labels": [{"name": "self-hosted"}, {"name": "rust-cpu"}]}
        ]}),
        "/orgs/acme/actions/runners" => {
            let mut h = HeaderMap::new();
            h.insert(
                "link",
                format!(r#"<{base}/orgs/acme/actions/runners?per_page=100&page=2>; rel="next""#)
                    .parse()
                    .unwrap(),
            );
            return (h, axum::Json(json!({"runners": [
                {"name": "r1", "status": "online", "busy": true, "labels": [{"name": "self-hosted"}, {"name": "rust-cpu"}]}
            ]})))
                .into_response();
        }
        "/orgs/acme/repos" => json!([
            {"name": "spar", "archived": false, "pushed_at": iso(t - 3600)},
            {"name": "attic", "archived": true, "pushed_at": iso(t - 3600)},
            {"name": "stale", "archived": false, "pushed_at": iso(t - 90 * 86400)}
        ]),
        "/repos/acme/spar/actions/runs" if q.contains("status=queued") => json!({"workflow_runs": [
            {"id": 10, "name": "CI", "event": "pull_request", "head_branch": "feat", "pull_requests": [{"number": 511}]}
        ]}),
        "/repos/acme/spar/actions/runs" => json!({"workflow_runs": [
            {"id": 11, "name": "CI", "event": "push", "head_branch": "main", "pull_requests": []}
        ]}),
        "/repos/acme/spar/actions/runs/10/jobs" => json!({"jobs": [
            {"name": "build", "status": "queued", "labels": ["self-hosted", "rust-cpu"], "created_at": iso(t - 120)},
            {"name": "lint", "status": "queued", "labels": ["ubuntu-latest"], "created_at": iso(t - 60)}
        ]}),
        "/repos/acme/spar/actions/runs/11/jobs" => json!({"jobs": [
            {"name": "test", "status": "in_progress", "labels": ["self-hosted", "rust-cpu"],
             "created_at": iso(t - 300), "started_at": iso(t - 60), "runner_name": "r1"}
        ]}),
        _ => return (StatusCode::NOT_FOUND, format!("mock has no {uri}")).into_response(),
    };
    axum::Json(body).into_response()
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

// rivet: verifies REQ-GHSRC-001
#[tokio::test]
async fn a_scan_fills_the_ground_station_model_from_a_ghe_com_style_base() {
    let (m, base) = mock().await;
    let c = Client::new(&cfg(&base)).unwrap();
    let s = ghsource::snapshot(&c, &cfg(&base)).await.unwrap();
    // paging: the second runners page was followed
    assert_eq!(s.runners.len(), 2, "{:?}", s.runners);
    assert!(s.runners[0].local_busy && s.runners[0].gh_status.as_deref() == Some("online"));
    let q = s.queue.expect("queue");
    // archived and long-quiet repos are not scanned
    assert_eq!(q.repos_scanned, 1);
    let e = q
        .queued
        .iter()
        .find(|e| e.pool == "rust-cpu")
        .expect("self-hosted queued job");
    assert_eq!((e.repo.as_str(), e.pr, e.run_id), ("spar", Some(511), 10));
    assert_eq!(q.pools["rust-cpu"].jobs_queued, 1);
    // REQ-RUNNERS-001: hosted jobs are pooled by their runs-on label
    assert_eq!(q.pools["hosted:ubuntu-latest"].jobs_queued, 1);
    assert!(!q.pools.contains_key("hosted"));
    assert_eq!(q.hosted_queued, 1);
    // the realized wait of the running job (created -5 min, started -1 min)
    assert_eq!(q.waits["rust-cpu"].p50, 240);
    assert!(q.queued_complete && !q.truncated);
    // every request: bearer token, at the base root, never /api/v3
    let seen = m.seen.lock().unwrap();
    assert!(seen.len() >= 6);
    for (uri, auth) in seen.iter() {
        assert!(!uri.contains("/api/v3"), "{uri}");
        assert_eq!(auth.as_deref(), Some("Bearer t0ken"), "{uri}");
    }
}

// rivet: verifies REQ-GHSRC-002
#[tokio::test]
async fn refused_tokens_and_rate_limits_are_told_apart() {
    let (m, base) = mock().await;
    let c = Client::new(&cfg(&base)).unwrap();

    *m.fail.lock().unwrap() = Some((401, vec![]));
    let e = ghsource::snapshot(&c, &cfg(&base)).await.unwrap_err();
    assert!(
        matches!(e, GhError::Auth(401, ref msg) if msg == "Bad credentials"),
        "{e:?}"
    );
    assert!(
        e.to_string().contains("WARTE_GITHUB_TOKEN"),
        "says what to fix: {e}"
    );

    let reset = now() + 900;
    *m.fail.lock().unwrap() = Some((
        403,
        vec![
            ("x-ratelimit-remaining", "0".into()),
            ("x-ratelimit-reset", reset.to_string()),
        ],
    ));
    assert_eq!(
        ghsource::snapshot(&c, &cfg(&base)).await.unwrap_err(),
        GhError::RateLimited { until: reset }
    );

    // with the window spent, the client sends nothing more until the reset
    let n = m.seen.lock().unwrap().len();
    assert_eq!(
        ghsource::snapshot(&c, &cfg(&base)).await.unwrap_err(),
        GhError::RateLimited { until: reset }
    );
    assert_eq!(m.seen.lock().unwrap().len(), n, "no request while spent");

    let c = Client::new(&cfg(&base)).unwrap();
    *m.fail.lock().unwrap() = Some((429, vec![("retry-after", "30".into())]));
    match ghsource::snapshot(&c, &cfg(&base)).await.unwrap_err() {
        GhError::RateLimited { until } => assert!((until - now() - 30).abs() <= 2),
        e => panic!("secondary limit: {e:?}"),
    }

    // a token that may read the queue but not the org's runners: the queue
    // still works, and the gap is said, not hidden
    *m.fail.lock().unwrap() = None;
    *m.runners_forbidden.lock().unwrap() = true;
    let c = Client::new(&cfg(&base)).unwrap();
    let s = ghsource::snapshot(&c, &cfg(&base)).await.unwrap();
    // only the busy runner, seen through its running job (REQ-GHSRC-004)
    assert!(s.runners_inferred && s.runners.len() == 1 && s.queue.is_some());
    assert!(
        s.errors
            .iter()
            .any(|e| e.contains("runners not listed (403)")),
        "{:?}",
        s.errors
    );
}

// rivet: verifies REQ-GHSRC-003
#[tokio::test]
async fn a_second_scan_reuses_etags_and_the_reserve_stops_the_scan() {
    let (m, base) = mock().await;
    *m.etags.lock().unwrap() = true;
    let c = Client::new(&cfg(&base)).unwrap();
    let first = ghsource::snapshot(&c, &cfg(&base)).await.unwrap();
    let sent = c.calls.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(c.not_modified.load(std::sync::atomic::Ordering::Relaxed), 0);
    // the second scan sends every request conditionally and is answered 304
    // throughout, with the same model as the first
    let second = ghsource::snapshot(&c, &cfg(&base)).await.unwrap();
    assert_eq!(c.calls.load(std::sync::atomic::Ordering::Relaxed), 2 * sent);
    assert_eq!(
        c.not_modified.load(std::sync::atomic::Ordering::Relaxed),
        sent,
        "{:?}",
        m.seen.lock().unwrap()
    );
    assert_eq!(second.runners.len(), first.runners.len());
    let (q1, q2) = (first.queue.unwrap(), second.queue.unwrap());
    assert_eq!(q1.queued.len(), q2.queued.len());
    assert_eq!(q2.pools["rust-cpu"].jobs_queued, 1);

    // GitHub reports 150 left: the next request is not sent, and the scan
    // waits for the reset instead of eating the token's last requests
    let (m, base) = mock().await;
    *m.remaining.lock().unwrap() = Some(150);
    let c = Client::new(&cfg(&base)).unwrap();
    assert_eq!(GithubConfig::default().rate_reserve, 150);
    match ghsource::snapshot(&c, &cfg(&base)).await.unwrap_err() {
        GhError::RateLimited { until } => assert!((until - now() - 600).abs() <= 2),
        e => panic!("reserve: {e:?}"),
    }
    assert_eq!(m.seen.lock().unwrap().len(), 1, "one request, then stop");
    assert_eq!(c.remaining(), Some(150));
    // above the reserve the scan runs
    let (m, base) = mock().await;
    *m.remaining.lock().unwrap() = Some(151);
    let c = Client::new(&cfg(&base)).unwrap();
    ghsource::snapshot(&c, &cfg(&base)).await.unwrap();
    assert_eq!(GithubConfig::default().interval_s, 120);
}

// rivet: verifies REQ-GHSRC-004
#[tokio::test]
async fn running_self_hosted_work_is_seen_without_the_runner_list() {
    for code in [403u16, 404] {
        let (m, base) = mock().await;
        *m.runners_status.lock().unwrap() = Some(code);
        let c = Client::new(&cfg(&base)).unwrap();
        let s = ghsource::snapshot(&c, &cfg(&base)).await.unwrap();
        assert!(s.runners_inferred, "{code}");
        assert!(
            s.errors
                .iter()
                .any(|e| e.contains(&format!("runners not listed ({code})"))
                    && e.contains("idle ones are unknown")),
            "{:?}",
            s.errors
        );
        // run 11's test job runs on r1: a busy runner with its job's context
        assert_eq!(s.runners.len(), 1, "{:?}", s.runners);
        let r = &s.runners[0];
        assert!(r.local_busy && r.unit == "r1");
        let ctx = r.ctx.as_ref().unwrap();
        assert_eq!(
            (ctx.repo.as_str(), ctx.run_id, ctx.branch.as_str()),
            ("acme/spar", 11, "main")
        );
        // Ask / eta: a branch running on a self-hosted pool is running, not not_queued
        let a = warte::hub::eta::answer(&s, "acme", "spar", None, Some("main"), now());
        assert_eq!(a.status, "running", "{code}: {a:?}");
        // admission counts the run as in flight
        let q = warte::admission::queue(&s, now(), now(), Some("spar"));
        assert_eq!(q.repo.unwrap().running_runs, 1);
        assert_eq!(q.self_hosted.busy, 1);
        // and the Overview says jobs running, not runways 1/1
        let d = tempfile::tempdir().unwrap();
        let hub = warte::hub::Hub::open(d.path().to_str().unwrap(), "acme").unwrap();
        hub.ingest_snapshot(s);
        let html = warte::dashboard::fleet::overview(Some(&hub));
        assert!(html.contains("Self-hosted jobs running"), "{html}");
        assert!(html.contains("idle runners unknown"));
        assert!(!html.contains("Runways in use"));
    }
}

// rivet: verifies REQ-GHSRC-005
#[tokio::test]
async fn configured_labels_make_a_job_self_hosted() {
    let (m, base) = mock().await;
    *m.arc.lock().unwrap() = true;
    let c = Client::new(&cfg(&base)).unwrap();
    let q = ghsource::snapshot(&c, &cfg(&base))
        .await
        .unwrap()
        .queue
        .unwrap();
    // unconfigured: a hosted larger runner, its own pool
    assert_eq!(q.pools["hosted:ubuntu-latest"].jobs_queued, 1);
    assert_eq!(q.pools["hosted:arc-large"].jobs_queued, 1);
    let conf = GithubConfig {
        self_hosted_labels: vec!["arc-*".into()],
        ..cfg(&base)
    };
    let c = Client::new(&conf).unwrap();
    let q = ghsource::snapshot(&c, &conf).await.unwrap().queue.unwrap();
    assert_eq!(q.pools["hosted:ubuntu-latest"].jobs_queued, 1);
    assert_eq!(q.pools["arc-large"].jobs_queued, 1, "{:?}", q.pools);
    assert!(q
        .queued
        .iter()
        .any(|e| e.pool == "arc-large" && e.pr == Some(511)));
}

// rivet: verifies REQ-GHSRC-001
// rivet: verifies REQ-SMITHY-005
#[tokio::test]
async fn warte_serve_with_github_answers_eta_queue_and_admission() {
    let (_m, base) = mock().await;
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        github: Some(GithubConfig {
            data_dir: dir.path().join("gh").to_str().unwrap().into(),
            ..cfg(&base)
        }),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let hub = state.hub.clone().expect("a hub fed by GitHub");
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    for _ in 0..100 {
        if hub.snapshot().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    // the start estimate, under warte's own API (REQ-SMITHY-005)
    let own: serde_json::Value = reqwest::get(format!("{url}/api/eta?repo=spar&pr=511"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(own["status"], "queued", "{own}");
    let part = &own["queued"][0];
    for k in [
        "run_id", "workflow", "pool", "jobs", "waited_s", "model", "estimate",
    ] {
        assert!(part.get(k).is_some(), "queued part lacks {k}: {own}");
    }
    let q: serde_json::Value = reqwest::get(format!("{url}/api/queue?repo=spar"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(q["repo"]["queued_runs"], 1, "{q}");
    let a = reqwest::get(format!("{url}/api/admission?repo=spar&kind=docs"))
        .await
        .unwrap();
    assert_eq!(a.status(), 200);
    let ask = reqwest::get(format!("{url}/api/dashboard/ask-partial?repo=spar&pr=511"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(ask.contains("ETA for spar PR #511"), "{ask}");
}

// rivet: verifies REQ-GHSRC-002
#[tokio::test]
async fn misconfiguration_is_refused_at_startup() {
    let dir = tempfile::tempdir().unwrap();
    // both sources at once
    let both = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: Some(warte::config::HubConfig {
            udp: "127.0.0.1:0".into(),
            allow: vec![std::net::Ipv4Addr::LOCALHOST.into()],
            data_dir: dir.path().join("hub").to_str().unwrap().into(),
            org: "acme".into(),
            probe_self: false,
            costs_file: None,
            skill_usage_file: None,
        }),
        github: Some(cfg("http://127.0.0.1:9")),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&both.db_path).await.unwrap();
    let err = warte::server::build_state(both, db).err().expect("refused");
    assert!(err.to_string().contains("choose one CI source"), "{err}");
    // a CA bundle without a certificate, and no token
    let not_pem = dir.path().join("ca.txt");
    std::fs::write(&not_pem, "not a certificate").unwrap();
    let e = Client::new(&GithubConfig {
        ca_bundle: Some(not_pem),
        ..cfg("http://x")
    })
    .err()
    .unwrap();
    assert!(e.contains("no PEM certificate"), "{e}");
    let e = Client::new(&GithubConfig {
        token: None,
        ..cfg("http://x")
    })
    .err()
    .unwrap();
    assert!(e.contains("WARTE_GITHUB_TOKEN"), "{e}");
}

// rivet: verifies REQ-ALERT-002
#[tokio::test]
async fn without_a_fleet_agent_findings_come_from_github_and_say_so() {
    let (m, base) = mock().await;
    // a normal scan: no host telemetry, so no host findings can appear
    let dir = tempfile::tempdir().unwrap();
    let hub = warte::hub::Hub::open(dir.path().to_str().unwrap(), "acme").unwrap();
    let c = Client::new(&cfg(&base)).unwrap();
    hub.ingest_snapshot(ghsource::snapshot(&c, &cfg(&base)).await.unwrap());
    let (found, _) = warte::dashboard::fleet::detected(&hub).unwrap();
    for d in &found {
        assert!(
            !["disk:", "pressure:", "oom:", "link-loss", "telemetry-stale"]
                .iter()
                .any(|k| d.key.starts_with(k)),
            "a finding GitHub cannot show: {d:?}"
        );
    }

    // a secondary rate limit: the scan loop reports it as a finding worded for
    // GitHub, and wakes again without sending a request before the reset
    *m.fail.lock().unwrap() = Some((429, vec![("retry-after", "600".into())]));
    let dir = tempfile::tempdir().unwrap();
    let hub = warte::hub::Hub::open(dir.path().to_str().unwrap(), "acme").unwrap();
    let mut g = cfg(&base);
    g.interval_s = 1;
    ghsource::spawn(hub.clone(), g, Default::default()).unwrap();
    let mut said = None;
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Some((list, _)) = warte::dashboard::fleet::detected(&hub) {
            said = list.into_iter().find(|d| d.key.starts_with("agent-error"));
            if said.is_some() {
                break;
            }
        }
    }
    let said = said.expect("the rate limit is a finding");
    assert!(
        said.text.starts_with("GitHub scan: GitHub API rate limit"),
        "{}",
        said.text
    );
    let n = m.seen.lock().unwrap().len();
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    assert_eq!(
        m.seen.lock().unwrap().len(),
        n,
        "no request before the reset"
    );
}

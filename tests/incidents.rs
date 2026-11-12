//! Oracles for shared incidents (REQ-CI-005, DD-CI-002).

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use axum::{extract::Request, response::IntoResponse, Router};
use serde_json::json;
use warte::hub::model::JobRecord;
use warte::incidents::{detected, group, of_line, of_log, panel, Kind, Signature, KEEP_S};

const NOW: i64 = 1_800_000_000;

fn sig(kind: Kind, host: &str) -> Option<Signature> {
    Some(Signature {
        kind,
        host: host.into(),
    })
}

// rivet: verifies REQ-CI-005
// rivet: verifies DD-CI-002
#[test]
fn signatures_come_from_fixed_patterns_and_name_one_host() {
    use Kind::*;
    for (line, want) in [
        // apt, curl/git/cargo, npm, Go/docker as they print a DNS failure
        ("2026-11-11T09:12:01.1Z   Temporary failure resolving 'archive.ubuntu.com'", sig(Resolver, "archive.ubuntu.com")),
        ("W: Failed to fetch http://azure.archive.ubuntu.com/ubuntu/dists/noble/InRelease  Temporary failure resolving 'azure.archive.ubuntu.com'", sig(Resolver, "azure.archive.ubuntu.com")),
        ("curl: (6) Could not resolve host: index.crates.io", sig(Resolver, "index.crates.io")),
        ("npm error getaddrinfo ENOTFOUND registry.npmjs.org", sig(Resolver, "registry.npmjs.org")),
        ("dial tcp: lookup ghcr.io on 127.0.0.53:53: no such host", sig(Resolver, "ghcr.io")),
        ("Could not connect to archive.ubuntu.com:80 (185.125.190.81), connection timed out", sig(Timeout, "archive.ubuntu.com")),
        ("curl: (28) Failed to connect to github.com port 443 after 130 ms: Connection timed out", sig(Timeout, "github.com")),
        ("Failed to connect to pypi.org port 443: Connection refused", sig(Refused, "pypi.org")),
        ("npm error 503 Service Unavailable - GET https://registry.npmjs.org/left-pad", sig(ServerError, "registry.npmjs.org")),
        ("curl: (22) The requested URL returned error: 502 for https://static.rust-lang.org/dist/x", sig(ServerError, "static.rust-lang.org")),
        // not a pattern: nothing, however alarming
        ("error[E0308]: mismatched types at /home/runner/work/a/src/lib.rs:3", None),
        ("export TOKEN=ghp_abcdef; curl https://example.com", None),
        ("test result: FAILED. 3 passed; 1 failed", None),
        // an address is not a host
        ("Could not connect to 10.0.0.7:80, connection timed out", None),
        ("Temporary failure resolving '192.168.1.1'", None),
    ] {
        assert_eq!(of_line(line), want, "{line}");
    }
    // the last matching line is the one nearest the failure
    let log = "Temporary failure resolving 'archive.ubuntu.com'\n...\nnpm error getaddrinfo ENOTFOUND registry.npmjs.org\nProcess completed with exit code 1.\n";
    assert_eq!(of_log(log), sig(Resolver, "registry.npmjs.org"));
    assert_eq!(of_log("all fine\n"), None);

    // only the fixed vocabulary survives a record (an agent can send anything)
    let s = Signature::parse("resolver:archive.ubuntu.com").unwrap();
    assert_eq!(s.encode(), "resolver:archive.ubuntu.com");
    for bad in [
        "resolver:/etc/passwd",
        "panic:archive.ubuntu.com",
        "resolver:10.0.0.1",
        "resolver:",
        "resolver:host with spaces.com",
        "timeout:localhost",
        "the log line itself",
    ] {
        assert_eq!(Signature::parse(bad), None, "{bad}");
    }
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn rec(repo: &str, e: i64, r: &str, s: Option<&str>) -> JobRecord {
    JobRecord {
        d: chrono::DateTime::from_timestamp(e - 120, 0)
            .unwrap()
            .format("%Y-%m-%d")
            .to_string(),
        id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        run: 1,
        att: 1,
        repo: repo.into(),
        wf: "CI".into(),
        job: "build".into(),
        pool: "hosted:ubuntu-latest".into(),
        runner: "GitHub Actions 1".into(),
        ev: "push".into(),
        c: e - 120,
        s: Some(e - 100),
        e: Some(e),
        r: r.into(),
        sig: s.map(String::from),
    }
}

const DNS: &str = "resolver:archive.ubuntu.com";

// rivet: verifies REQ-CI-005
// rivet: verifies DD-CI-002
#[test]
fn one_error_in_two_repositories_within_an_hour_is_one_incident() {
    // the 2026-11-11 shape: apt fails the same way in warte and gale
    let recs = vec![
        rec("warte", NOW - 3000, "failure", Some(DNS)),
        rec("gale", NOW - 2000, "failure", Some(DNS)),
        rec("warte", NOW - 600, "timed_out", Some(DNS)),
        // a pass carrying a signature, and a failure without one, join nothing
        rec("spar", NOW - 900, "success", Some(DNS)),
        rec("spar", NOW - 900, "failure", None),
        // the same error in one repository only is not shared
        rec("loom", NOW - 500, "failure", Some("timeout:pypi.org")),
        rec("loom", NOW - 400, "failure", Some("timeout:pypi.org")),
    ];
    let r = group(&recs, NOW, &[]);
    assert_eq!(r.incidents.len(), 1, "{:?}", r.incidents);
    let i = &r.incidents[0];
    assert_eq!(i.repos, vec!["gale", "warte"]);
    assert_eq!(
        (i.jobs, i.first, i.last, i.ongoing),
        (3, NOW - 3000, NOW - 600, true)
    );
    assert_eq!(i.host.as_deref(), Some("archive.ubuntu.com"));
    assert_eq!(
        warte::incidents::text(i),
        "archive.ubuntu.com did not resolve: 3 failed jobs in 2 repositories (gale and warte), failing since 07:10 UTC. One shared cause, not 2 separate failures"
    );
    // ongoing: it notifies, under a key that holds no count
    let d = detected(&r);
    assert_eq!(d.len(), 1);
    assert_eq!(
        (d[0].key.as_str(), d[0].sev),
        ("shared-incident:resolver:archive.ubuntu.com", 2)
    );
    let p = panel(&r);
    assert!(
        p.contains("<h3>Shared incidents</h3>") && p.contains("gale and warte"),
        "{p}"
    );

    // more than an hour between two failures splits them; neither half is shared
    let recs = vec![
        rec("warte", NOW - 3 * 3600, "failure", Some(DNS)),
        rec("gale", NOW - 3600 - 60, "failure", Some(DNS)),
    ];
    assert!(group(&recs, NOW, &[]).incidents.is_empty());
    // but a chain of failures each within the hour is one incident
    let recs = vec![
        rec("warte", NOW - 3 * 3600, "failure", Some(DNS)),
        rec("gale", NOW - 2 * 3600 - 60, "failure", Some(DNS)),
        rec("spar", NOW - 3600 - 120, "failure", Some(DNS)),
    ];
    let r = group(&recs, NOW, &[]);
    assert_eq!(r.incidents.len(), 1);
    assert!(!r.incidents[0].ongoing, "last failure over an hour ago");
    assert!(detected(&r).is_empty(), "a past incident does not notify");
    assert!(warte::incidents::text(&r.incidents[0]).contains("from 05:00 to 06:58 UTC"));

    // different errors never merge; older than a day is gone
    let recs = vec![
        rec("warte", NOW - 60, "failure", Some(DNS)),
        rec(
            "gale",
            NOW - 60,
            "failure",
            Some("resolver:security.ubuntu.com"),
        ),
        rec(
            "warte",
            NOW - KEEP_S - 600,
            "failure",
            Some("refused:pypi.org"),
        ),
        rec(
            "gale",
            NOW - KEEP_S - 300,
            "failure",
            Some("refused:pypi.org"),
        ),
    ];
    assert!(group(&recs, NOW, &[]).incidents.is_empty());
    assert_eq!(
        panel(&group(&recs, NOW, &[])),
        "",
        "nothing to show, no panel"
    );
}

// rivet: verifies REQ-CI-005
#[test]
fn a_host_is_named_only_when_public_or_watched() {
    let internal = "resolver:artifacts.corp.example";
    let recs = vec![
        rec("a", NOW - 100, "failure", Some(internal)),
        rec("b", NOW - 50, "failure", Some(internal)),
    ];
    let r = group(&recs, NOW, &[]);
    let i = &r.incidents[0];
    assert_eq!(
        (i.host.as_deref(), i.what.as_str()),
        (None, "a host did not resolve")
    );
    let api = serde_json::to_string(&r).unwrap();
    assert!(!api.contains("corp"), "{api}");
    assert!(!panel(&r).contains("corp"));
    assert!(!detected(&r)[0].key.contains("corp") && !detected(&r)[0].text.contains("corp"));
    // a name the canaries watch is the operator's own choice to show
    let r = group(&recs, NOW, &["artifacts.corp.example".to_string()]);
    assert_eq!(
        r.incidents[0].host.as_deref(),
        Some("artifacts.corp.example")
    );
    // a suffix is not a match: notgithub.com is not github.com's
    assert!(!warte::incidents::named("notgithub.com", &[]));
    assert!(warte::incidents::named(
        "objects.githubusercontent.com",
        &[]
    ));
}

#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<String>>>);

// rivet: verifies REQ-CI-005
// rivet: verifies DD-CI-002
#[tokio::test]
async fn github_only_mode_reads_a_recent_failures_log_once_and_keeps_only_the_signature() {
    use warte::ghsource::{ledger, Client};
    let now = warte::hub::now();
    let iso = |t: i64| {
        chrono::DateTime::from_timestamp(t, 0)
            .unwrap()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    };
    let today = chrono::DateTime::from_timestamp(now, 0)
        .unwrap()
        .format("%Y-%m-%d")
        .to_string();

    // the blob host: what GitHub's redirect points at; it must not see the token
    let blob_seen = Seen::default();
    let bs = blob_seen.clone();
    let blob = Router::new().fallback(move |req: Request| {
        let bs = bs.clone();
        async move {
            let auth = req.headers().get("authorization").is_some();
            bs.0.lock()
                .unwrap()
                .push(format!("{} auth={auth}", req.uri().path()));
            let mut log = "x\n".repeat(200_000);
            log.push_str("2026-11-11T09:12:01Z Err:1 http://archive.ubuntu.com noble InRelease\n");
            log.push_str(
                "2026-11-11T09:12:01Z   Temporary failure resolving 'archive.ubuntu.com'\n",
            );
            log.push_str("##[error]Process completed with exit code 100.\n");
            log
        }
    });
    let bl = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let blob_url = format!("http://localhost:{}", bl.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(bl, blob).await.unwrap() });

    let api_seen = Seen::default();
    let s2 = api_seen.clone();
    let day = today.clone();
    let api = Router::new().fallback(move |req: Request| {
        let (s2, blob_url, day) = (s2.clone(), blob_url.clone(), day.clone());
        async move {
            let path = req.uri().path().to_string();
            let q = req.uri().query().unwrap_or_default().to_string();
            s2.0.lock().unwrap().push(path.clone());
            let job = |id: u64, r: &str, end: i64| {
                json!({"id": id, "run_attempt": 1, "name": "build", "status": "completed",
                       "conclusion": r, "labels": ["ubuntu-latest"], "runner_name": "GitHub Actions 2",
                       "created_at": iso(end - 200), "started_at": iso(end - 100), "completed_at": iso(end)})
            };
            match path.as_str() {
                "/repos/acme/warte/actions/runs" if q.contains(&format!("created={day}")) => {
                    axum::Json(json!({"workflow_runs": [{"id": 7, "name": "CI", "event": "push",
                        "status": "completed", "run_attempt": 1, "created_at": iso(now - 300)}]}))
                        .into_response()
                }
                "/repos/acme/warte/actions/runs" => axum::Json(json!({"workflow_runs": []})).into_response(),
                "/repos/acme/warte/actions/runs/7/jobs" => axum::Json(json!({"jobs": [
                    job(70, "failure", now - 60),
                    job(71, "success", now - 60),
                    job(72, "cancelled", now - 60),
                ]}))
                .into_response(),
                "/repos/acme/warte/actions/jobs/70/logs" => (
                    axum::http::StatusCode::FOUND,
                    [("location", format!("{blob_url}/blob/70.txt?sig=abc"))],
                )
                    .into_response(),
                _ => axum::http::StatusCode::NOT_FOUND.into_response(),
            }
        }
    });
    let al = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", al.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(al, api).await.unwrap() });

    let c = Client::new(&warte::config::GithubConfig {
        api_url: base,
        org: "acme".into(),
        token: Some("t0ken".into()),
        ..Default::default()
    })
    .unwrap();
    let mut st = ledger::LedgerState::default();
    let (recs, _) = ledger::pass(&c, "acme", &["warte".to_string()], &mut st, now, 0, &[])
        .await
        .unwrap();
    let by = |id: u64| recs.iter().find(|r| r.id == id).unwrap();
    assert_eq!(by(70).sig.as_deref(), Some(DNS), "{recs:?}");
    assert_eq!((by(71).sig.clone(), by(72).sig.clone()), (None, None));
    let logs: Vec<String> = api_seen
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.contains("/logs"))
        .cloned()
        .collect();
    assert_eq!(
        logs,
        vec!["/repos/acme/warte/actions/jobs/70/logs"],
        "only the failure's log"
    );
    assert_eq!(
        blob_seen.0.lock().unwrap().clone(),
        vec!["/blob/70.txt auth=false"],
        "the token stays with GitHub's API host"
    );

    // a second pass reads no log again: the job is already in the ledger
    let (recs, _) = ledger::pass(&c, "acme", &["warte".to_string()], &mut st, now, 0, &[])
        .await
        .unwrap();
    assert!(recs.is_empty());
    assert_eq!(blob_seen.0.lock().unwrap().len(), 1);
    // and an old failure is never worth a log
    let mut old = rec("warte", now - KEEP_S - 60, "failure", None);
    assert!(!ledger::wants_log(&old, now));
    old.e = Some(now - 60);
    assert!(ledger::wants_log(&old, now));
}

// rivet: verifies REQ-CI-005
#[tokio::test]
async fn the_api_overview_and_alerts_show_a_shared_incident() {
    let dir = tempfile::tempdir().unwrap();
    let hub_dir = dir.path().join("hub");
    std::fs::create_dir_all(&hub_dir).unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: Some(warte::config::HubConfig {
            udp: "127.0.0.1:0".into(),
            allow: vec![Ipv4Addr::LOCALHOST.into()],
            data_dir: hub_dir.to_str().unwrap().into(),
            org: "acme".into(),
            probe_self: false,
            costs_file: None,
            skill_usage_file: None,
        }),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let now = warte::hub::now();
    state.hub.as_ref().unwrap().ingest_ledger(
        vec![
            rec("warte", now - 900, "failure", Some(DNS)),
            rec("gale", now - 300, "failure", Some(DNS)),
            // an agent sending raw text gets nothing through
            rec(
                "spar",
                now - 200,
                "failure",
                Some("secret token in /home/x"),
            ),
            rec(
                "loom",
                now - 200,
                "failure",
                Some("secret token in /home/x"),
            ),
        ],
        vec![],
    );
    state
        .incidents
        .as_ref()
        .unwrap()
        .refresh(state.hub.as_ref().unwrap(), now)
        .await;
    let found = warte::dashboard::fleet::host_detected(&state, now);
    assert!(
        found
            .iter()
            .any(|d| d.key == "shared-incident:resolver:archive.ubuntu.com"),
        "{found:?}"
    );
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    let v: serde_json::Value = reqwest::get(format!("{base}/api/incidents"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["incidents"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["incidents"][0]["repos"], json!(["gale", "warte"]), "{v}");
    assert_eq!(v["incidents"][0]["kind"], "resolver");
    assert!(!v.to_string().contains("secret"), "{v}");
    let ov = reqwest::get(format!("{base}/api/dashboard/overview-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(ov.contains("<h3>Shared incidents</h3>"), "{ov}");
}

//! Oracles for REQ-QUEUE-004: bounded re-queue offers after a merge-group
//! ejection caused only by hosted jobs no runner acquired.

use std::collections::BTreeMap;
use warte::hub::model::JobRecord;
use warte::requeue::{detected, ejections, plan, pr_of, Ejection, NOT_ACQUIRED};

const T0: i64 = 1_764_064_800; // 2025-11-25 10:00:00 UTC
const SHA: &str = "f433a7c0c63a9918b981c018dff72d1b32dcc2a9";

// rivet: verifies REQ-QUEUE-004
#[test]
fn the_pr_comes_from_the_merge_group_branch() {
    assert_eq!(
        pr_of("gh-readonly-queue/main/pr-636-237b76758559b79f4c578083ae1e85a02d933fdc"),
        Some(636)
    );
    assert_eq!(pr_of("gh-readonly-queue/release/1.x/pr-7-abc"), Some(7));
    for b in [
        "main",
        "pr-636-abc",
        "gh-readonly-queue/main/636-abc",
        "gh-readonly-queue/main/pr-x-abc",
        "gh-readonly-queue/main/pr-0-abc",
    ] {
        assert_eq!(pr_of(b), None, "{b}");
    }
}

fn job(id: u64, run: u64, pr: u64, r: &str, sig: Option<&str>, end: i64) -> JobRecord {
    JobRecord {
        d: warte::hub::report::day_of(end),
        id,
        run,
        att: 1,
        repo: "relay".into(),
        job: format!("j{id}"),
        pool: if sig.is_some() {
            "macos-15-arm64".into()
        } else {
            "ubuntu-24.04".into()
        },
        ev: "merge_group".into(),
        c: end - 600,
        e: Some(end),
        r: r.into(),
        sig: sig.map(String::from),
        br: Some(format!("gh-readonly-queue/main/pr-{pr}-237b767")),
        ..Default::default()
    }
}

// rivet: verifies REQ-QUEUE-004
#[test]
fn only_a_run_whose_every_failure_is_unacquired_qualifies() {
    let recs = vec![
        // run 1: one unacquired job, the rest passed
        job(10, 1, 636, "success", None, T0),
        job(11, 1, 636, "cancelled", Some(NOT_ACQUIRED), T0 + 5),
        // run 2: an unacquired job and an ordinary failure
        job(20, 2, 637, "cancelled", Some(NOT_ACQUIRED), T0),
        job(21, 2, 637, "failure", None, T0),
        // run 3: all passed
        job(30, 3, 638, "success", None, T0),
        // run 5: skipped and neutral jobs are ignored, so it still qualifies
        job(50, 5, 640, "skipped", None, T0),
        job(51, 5, 640, "neutral", None, T0),
        job(52, 5, 640, "cancelled", Some(NOT_ACQUIRED), T0),
        // run 6: a plain cancelled job (no runner class) is a failure like any other
        job(60, 6, 641, "cancelled", None, T0),
        job(61, 6, 641, "cancelled", Some(NOT_ACQUIRED), T0),
        // a push run is not a merge group
        JobRecord {
            ev: "push".into(),
            ..job(40, 4, 639, "cancelled", Some(NOT_ACQUIRED), T0)
        },
    ];
    let e = ejections(&recs);
    let by = |run: u64| e.iter().find(|x| x.run == run).unwrap();
    assert_eq!(e.len(), 5);
    assert!(
        by(5).only_not_acquired && !by(5).passed,
        "skipped and neutral are ignored"
    );
    assert!(
        !by(6).only_not_acquired,
        "a cancelled job without the class disqualifies"
    );
    assert!(by(1).only_not_acquired && !by(1).passed);
    assert_eq!(
        (by(1).pr, by(1).at, by(1).pool.as_str()),
        (636, T0 + 5, "macos-15-arm64")
    );
    assert!(!by(2).only_not_acquired && !by(2).passed);
    assert!(by(3).passed && !by(3).only_not_acquired);
}

fn ej(run: u64, pr: u64, at: i64, only: bool) -> Ejection {
    Ejection {
        repo: "relay".into(),
        pr,
        run,
        att: 1,
        at,
        pool: "macos-15-arm64".into(),
        passed: false,
        only_not_acquired: only,
    }
}

fn heads(runs: &[u64]) -> BTreeMap<u64, String> {
    runs.iter().map(|r| (*r, SHA.to_string())).collect()
}

// rivet: verifies REQ-QUEUE-004
#[test]
fn backoff_and_the_daily_cap() {
    let none = BTreeMap::new();
    let mut ejs = vec![];
    for (n, want) in [(1u32, 30i64), (2, 60), (3, 120)] {
        let at = T0 + n as i64 * 3600;
        ejs.push(ej(n as u64, 636, at, true));
        let p = plan(&ejs, &heads(&[1, 2, 3]), &none, at + 60);
        let o = &p.offers_for("relay")[0];
        assert_eq!(
            (o.attempt, o.of, o.earliest, o.run_id),
            (n, 3, at + want * 60, n as u64)
        );
        assert_eq!(
            (o.pr, o.head_sha.as_str(), o.pool.as_str()),
            (636, SHA, "macos-15-arm64")
        );
        assert!(p.exhausted.is_empty());
    }
    // the fourth ejection that day: no offer, an escalation
    ejs.push(ej(4, 636, T0 + 4 * 3600, true));
    let p = plan(&ejs, &heads(&[1, 2, 3, 4]), &none, T0 + 4 * 3600 + 60);
    assert!(p.offers.is_empty());
    assert_eq!(p.exhausted.len(), 1);
    assert_eq!(p.exhausted[0].pr, 636);
    // the next UTC day starts over
    let next = T0 + 86_400;
    ejs.push(ej(5, 636, next, true));
    let p = plan(&ejs, &heads(&[5]), &none, next + 60);
    assert_eq!(p.offers_for("relay")[0].attempt, 1);
}

// rivet: verifies REQ-QUEUE-004
#[test]
fn no_offer_without_a_head_for_an_old_ejection_or_another_failure() {
    let none = BTreeMap::new();
    let one = vec![ej(1, 636, T0, true)];
    assert!(
        plan(&one, &BTreeMap::new(), &none, T0 + 60)
            .offers
            .is_empty(),
        "no head"
    );
    assert!(
        plan(&one, &heads(&[1]), &none, T0 + 86_401)
            .offers
            .is_empty(),
        "older than a day"
    );
    let other = vec![ej(1, 636, T0, false)];
    assert!(
        plan(&other, &heads(&[1]), &none, T0 + 60).offers.is_empty(),
        "an ordinary failure"
    );
    // a later run that passed: the PR merged, the offer is withdrawn
    let mut merged = one.clone();
    merged.push(Ejection {
        passed: true,
        only_not_acquired: false,
        ..ej(2, 636, T0 + 3600, false)
    });
    assert!(plan(&merged, &heads(&[1]), &none, T0 + 3700)
        .offers
        .is_empty());
}

// rivet: verifies REQ-QUEUE-004
#[test]
fn the_kill_switch_trips_holds_for_the_repo_and_clears() {
    let ejs = vec![
        ej(1, 636, T0, true),
        // the re-queued PR ejected for another reason
        ej(2, 636, T0 + 3600, false),
        // another PR's offer in the same repo
        ej(3, 700, T0 + 3700, true),
    ];
    let p = plan(&ejs, &heads(&[1, 3]), &BTreeMap::new(), T0 + 3800);
    assert_eq!(p.stopped.len(), 1);
    assert_eq!(
        (
            p.stopped[0].repo.as_str(),
            p.stopped[0].pr,
            p.stopped[0].run
        ),
        ("relay", 636, 2)
    );
    assert!(
        p.offers_for("relay").is_empty(),
        "stopped holds for the whole repo"
    );
    // red, and no offer finding while stopped
    let d = detected(&p);
    assert!(d
        .iter()
        .any(|x| x.key == "requeue-stopped:relay" && x.sev == 2));
    assert!(!d.iter().any(|x| x.key.starts_with("requeue:")), "{d:?}");
    // an ordinary failure without an offer before it trips nothing
    let p = plan(
        &[ej(9, 800, T0, false)],
        &BTreeMap::new(),
        &BTreeMap::new(),
        T0 + 60,
    );
    assert!(p.stopped.is_empty());
    // a person cleared after the trip: offers resume
    let cleared = BTreeMap::from([("relay".to_string(), T0 + 3650)]);
    let p = plan(&ejs, &heads(&[1, 3]), &cleared, T0 + 3800);
    assert!(p.stopped.is_empty());
    assert_eq!(p.offers_for("relay")[0].pr, 700);
}

// rivet: verifies REQ-QUEUE-004
#[test]
fn findings_name_the_pr_attempt_and_time() {
    let p = plan(
        &[ej(1, 636, T0, true)],
        &heads(&[1]),
        &BTreeMap::new(),
        T0 + 60,
    );
    let d = detected(&p);
    assert_eq!(d.len(), 1);
    assert_eq!((d[0].key.as_str(), d[0].sev), ("requeue:relay#636:1", 1));
    assert!(
        d[0].text.contains("relay#636")
            && d[0].text.contains("1/3")
            && d[0].text.contains("10:30 UTC"),
        "{}",
        d[0].text
    );
    assert!(!d[0].text.contains(SHA), "the head stays out of the notice");
}

// rivet: verifies REQ-QUEUE-004
#[tokio::test]
async fn the_pass_resolves_heads_once_and_the_api_serves_only_the_agreed_fields() {
    use axum::{extract::Request, response::IntoResponse, Router};
    use serde_json::json;
    use std::net::Ipv4Addr;
    use std::sync::{Arc, Mutex};
    let now = warte::hub::now();
    // GitHub: PR 636's head reads; PR 637's does not
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let s2 = seen.clone();
    let gh = Router::new().fallback(move |req: Request| {
        let s2 = s2.clone();
        async move {
            let path = req.uri().path().to_string();
            s2.lock().unwrap().push(path.clone());
            match path.as_str() {
                "/repos/acme/relay/pulls/636" => {
                    axum::Json(json!({"number": 636, "head": {"sha": SHA}})).into_response()
                }
                _ => axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
    });
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gh_base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, gh).await.unwrap() });
    let c = warte::ghsource::Client::new(&warte::config::GithubConfig {
        api_url: gh_base,
        org: "acme".into(),
        token: Some("t0ken".into()),
        ..Default::default()
    })
    .unwrap();

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
            quic: None,
            quic_names: vec![],
            require_quic: false,
        }),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let hub = state.hub.clone().unwrap();
    hub.ingest_ledger(
        vec![
            job(11, 1, 636, "cancelled", Some(NOT_ACQUIRED), now - 120),
            job(21, 2, 637, "cancelled", Some(NOT_ACQUIRED), now - 120),
        ],
        vec![],
    );
    warte::ghsource::requeue_pass(&c, "acme", &hub, now).await;
    warte::ghsource::requeue_pass(&c, "acme", &hub, now).await;
    let pulls: Vec<String> = seen.lock().unwrap().clone();
    assert_eq!(
        pulls.iter().filter(|p| p.ends_with("/pulls/636")).count(),
        1,
        "a resolved head is not read again: {pulls:?}"
    );
    assert_eq!(
        pulls.iter().filter(|p| p.ends_with("/pulls/637")).count(),
        2,
        "an unread head is tried again on the next pass"
    );
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    let r = reqwest::get(format!("{base}/api/requeue?repo=relay"))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: serde_json::Value = r.json().await.unwrap();
    assert_eq!(v["schema"], "warte.requeue.v1");
    let offers = v["offers"].as_array().unwrap();
    assert_eq!(offers.len(), 1, "no head for 637, no offer: {v}");
    let o = offers[0].as_object().unwrap();
    let keys: Vec<&str> = o.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec!["attempt", "earliest", "head_sha", "of", "pool", "pr", "run_id"],
        "only repo-level facts"
    );
    assert_eq!(
        (o["pr"].clone(), o["head_sha"].clone(), o["run_id"].clone()),
        (json!(636), json!(SHA), json!(1))
    );
    assert_eq!(o["earliest"], json!(now - 120 + 30 * 60));

    // no offers is an empty list with 200; no repo is a 400
    let v: serde_json::Value = reqwest::get(format!("{base}/api/requeue?repo=gale"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["offers"], json!([]));
    assert_eq!(
        reqwest::get(format!("{base}/api/requeue"))
            .await
            .unwrap()
            .status(),
        400
    );
    // clearing needs a login
    let r = reqwest::Client::new()
        .post(format!("{base}/api/requeue/clear"))
        .json(&json!({"repo": "relay"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

// rivet: verifies REQ-QUEUE-004
#[test]
fn a_clear_lifts_the_stop_and_outlives_a_restart() {
    use warte::requeue::Board;
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().to_str().unwrap();
    let ejs = vec![
        ej(1, 636, T0, true),
        ej(2, 636, T0 + 3600, false),
        ej(3, 700, T0 + 3700, true),
    ];
    let b = Board::open(d);
    b.set_head(1, SHA);
    b.set_head(3, SHA);
    b.update(&ejs, T0 + 3800);
    assert_eq!(b.plan().stopped.len(), 1);
    b.clear("relay", T0 + 3650);
    assert!(b.plan().stopped.is_empty(), "lifted at once");
    // a restart keeps the heads and the clear
    let b = Board::open(d);
    assert!(b.has_head(3));
    b.update(&ejs, T0 + 3800);
    let p = b.plan();
    assert!(p.stopped.is_empty());
    assert_eq!(p.offers_for("relay").len(), 1);
    // heads of runs no longer seen are dropped
    b.update(&ejs[2..], T0 + 3800);
    assert!(!b.has_head(1) && b.has_head(3));
}

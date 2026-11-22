//! Runner usage per repository over a window of ledger days (REQ-CI-009).

use std::net::Ipv4Addr;
use warte::cihealth::usage::{check, usage, MAX_DAYS};
use warte::hub::model::JobRecord;

const T: i64 = 1_795_000_000;

#[allow(clippy::too_many_arguments)]
fn rec(
    id: u64,
    run: u64,
    att: u32,
    repo: &str,
    d: &str,
    pool: &str,
    runner: &str,
    ev: &str,
    br: &str,
    wait: Option<i64>,
    minutes: i64,
) -> JobRecord {
    JobRecord {
        d: d.into(),
        id,
        run,
        att,
        repo: repo.into(),
        wf: "CI".into(),
        job: format!("job{id}"),
        pool: pool.into(),
        runner: runner.into(),
        ev: ev.into(),
        c: T,
        s: wait.map(|w| T + w),
        e: wait.map(|w| T + w + minutes * 60),
        r: if wait.is_some() {
            "success"
        } else {
            "cancelled"
        }
        .into(),
        sig: None,
        br: Some(br.into()),
    }
}

fn ledger() -> Vec<JobRecord> {
    vec![
        // run 1, a pull request on branch a: two jobs on rust-cpu, one on light
        rec(
            1,
            1,
            1,
            "warte",
            "2026-11-10",
            "rust-cpu",
            "cpu-host-7",
            "pull_request",
            "a",
            Some(60),
            10,
        ),
        rec(
            2,
            1,
            1,
            "warte",
            "2026-11-10",
            "rust-cpu",
            "cpu-host-7",
            "pull_request",
            "a",
            Some(120),
            4,
        ),
        rec(
            3,
            1,
            1,
            "warte",
            "2026-11-10",
            "light",
            "small-host-2",
            "pull_request",
            "a",
            Some(10),
            1,
        ),
        // run 1 re-run: uses runner time, but its wait is not a first attempt
        rec(
            4,
            1,
            2,
            "warte",
            "2026-11-10",
            "rust-cpu",
            "cpu-host-7",
            "pull_request",
            "a",
            Some(900),
            10,
        ),
        // run 2, a second push of branch a; run 3, branch b
        rec(
            5,
            2,
            1,
            "warte",
            "2026-11-11",
            "rust-cpu",
            "cpu-host-8",
            "pull_request",
            "a",
            Some(30),
            5,
        ),
        rec(
            6,
            3,
            1,
            "warte",
            "2026-11-12",
            "rust-cpu",
            "cpu-host-8",
            "pull_request",
            "b",
            Some(600),
            6,
        ),
        // run 4, a push to main
        rec(
            7,
            4,
            1,
            "warte",
            "2026-11-12",
            "rust-cpu",
            "cpu-host-7",
            "push",
            "main",
            Some(240),
            8,
        ),
        // planted: never got a runner, so no runner time and no wait
        rec(
            8,
            5,
            1,
            "warte",
            "2026-11-12",
            "rust-cpu",
            "",
            "pull_request",
            "c",
            None,
            0,
        ),
        // planted: another repository, and a day outside the window
        rec(
            9,
            6,
            1,
            "rivet",
            "2026-11-12",
            "rust-cpu",
            "cpu-host-7",
            "push",
            "main",
            Some(5),
            99,
        ),
        rec(
            10,
            7,
            1,
            "warte",
            "2026-11-23",
            "rust-cpu",
            "cpu-host-7",
            "push",
            "main",
            Some(5),
            99,
        ),
    ]
}

// rivet: verifies REQ-CI-009
#[test]
fn usage_sums_runner_minutes_per_pool_event_and_branch_without_runner_names() {
    let u = usage(&ledger(), "warte", "2026-11-08", "2026-11-22");
    assert_eq!(u.records, 8, "rivet's job and the 23rd are not counted");
    assert_eq!(u.runs, 5);
    // 10 + 4 + 1 + 10 (re-run) + 5 + 6 + 8
    assert_eq!(u.runner_minutes, 44.0);
    assert_eq!(u.minutes_per_run, Some(8.8));
    // branch a: 10 + 4 + 1 + 10 + 5 = 30; b: 6; c never ran
    assert_eq!(u.pr_branches, 2);
    assert_eq!(u.minutes_per_pr_branch, Some(18.0));

    let cpu = u.pools.iter().find(|p| p.pool == "rust-cpu").unwrap();
    assert_eq!(cpu.jobs, 6);
    assert_eq!(cpu.runner_minutes, 43.0);
    // first attempts with a start: 60, 120, 30, 600, 240 (not the re-run's 900)
    assert_eq!(cpu.wait_samples, 5);
    assert_eq!(cpu.wait_p50_s, Some(120.0));
    assert_eq!(cpu.wait_p90_s, Some(600.0));
    let light = u.pools.iter().find(|p| p.pool == "light").unwrap();
    assert_eq!(
        (light.jobs, light.runner_minutes, light.wait_samples),
        (1, 1.0, 1)
    );

    let pr = u.events.iter().find(|e| e.event == "pull_request").unwrap();
    assert_eq!(
        (pr.runs, pr.runner_minutes, pr.minutes_per_run),
        (4, 36.0, Some(9.0))
    );
    let push = u.events.iter().find(|e| e.event == "push").unwrap();
    assert_eq!((push.runs, push.runner_minutes), (1, 8.0));

    let json = serde_json::to_string(&u).unwrap();
    for name in ["cpu-host-7", "cpu-host-8", "small-host-2"] {
        assert!(!json.contains(name), "{name} in {json}");
    }
}

// rivet: verifies REQ-CI-009
#[test]
fn an_empty_window_has_no_rates_and_no_waits() {
    let u = usage(&ledger(), "warte", "2026-12-01", "2026-12-07");
    assert_eq!((u.records, u.runs, u.runner_minutes), (0, 0, 0.0));
    assert_eq!(u.minutes_per_run, None, "no runs is not 0 minutes per run");
    assert_eq!(u.minutes_per_pr_branch, None);
    assert!(u.pools.is_empty() && u.events.is_empty());
}

// rivet: verifies REQ-CI-009
#[test]
fn only_a_repository_name_and_a_real_window_of_at_most_31_days_pass() {
    assert!(check("warte", "2026-11-08", "2026-11-22").is_ok());
    assert!(check("rules_wasm-component.x", "2026-11-22", "2026-11-22").is_ok());
    assert!(
        check("warte", "2026-11-01", "2026-12-01").is_ok(),
        "31 days"
    );
    assert_eq!(MAX_DAYS, 31);
    for (repo, from, until) in [
        ("", "2026-11-08", "2026-11-22"),
        ("../etc", "2026-11-08", "2026-11-22"),
        ("pulseengine/warte", "2026-11-08", "2026-11-22"),
        ("a b", "2026-11-08", "2026-11-22"),
        ("warte", "2026-11-31", "2026-12-01"),
        ("warte", "2026-11-8", "2026-11-22"),
        ("warte", "", "2026-11-22"),
        ("warte", "2026-11-22", "2026-11-08"),
        ("warte", "2026-11-01", "2026-12-02"),
    ] {
        assert!(check(repo, from, until).is_err(), "{repo} {from} {until}");
    }
}

/// warte over a ledger with a private repository planted, and a briefing
/// whose public list (GitHub's `type=public` listing) is `public`; no
/// briefing at all when `public` is `None`.
async fn serve(public: Option<&[&str]>) -> (tempfile::TempDir, String) {
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
    let mut state = warte::server::build_state(config, db).unwrap();
    let mut recs = ledger();
    // planted: a private repository with runner time in the window
    recs.push(rec(
        11,
        8,
        1,
        "secret-one",
        "2026-11-12",
        "rust-cpu",
        "cpu-host-7",
        "push",
        "main",
        Some(5),
        30,
    ));
    state.hub.as_ref().unwrap().ingest_ledger(recs, vec![]);
    if let Some(public) = public {
        let store = warte::briefing::Store::open(dir.path().to_str().unwrap()).unwrap();
        store
            .put(&warte::briefing::Briefing {
                facts: warte::briefing::Facts {
                    from: "2026-11-21".into(),
                    until: "2026-11-22".into(),
                    known_repos: public.iter().map(|s| s.to_string()).collect(),
                    ..Default::default()
                },
                text: Default::default(),
                source: "model".into(),
                model: None,
                rejected: None,
                written_at: T,
                replaces: vec![],
            })
            .unwrap();
        state.briefing = Some(std::sync::Arc::new(store));
    }
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (dir, base)
}

async fn get(url: String) -> (u16, String) {
    let r = reqwest::get(url).await.unwrap();
    (r.status().as_u16(), r.text().await.unwrap())
}

const WINDOW: &str = "from=2026-11-08&until=2026-11-22";

// rivet: verifies REQ-CI-009
#[tokio::test]
async fn the_api_answers_from_the_ledger_and_refuses_a_bad_window() {
    let (_d, base) = serve(Some(&["warte", "rivet"])).await;
    let (code, body) = get(format!("{base}/api/runner-usage?repo=warte&{WINDOW}")).await;
    assert_eq!(code, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["runner_minutes"], 44.0, "{v}");
    assert_eq!(v["runs"], 5, "{v}");
    assert!(!body.contains("cpu-host"), "{body}");

    for q in [
        "repo=warte&from=2026-11-01&until=2026-12-31",
        "repo=../x&from=2026-11-08&until=2026-11-22",
        "repo=warte",
    ] {
        let (code, _) = get(format!("{base}/api/runner-usage?{q}")).await;
        assert_eq!(code, 400, "{q}");
    }
}

// rivet: verifies REQ-CI-009
#[tokio::test]
async fn a_private_or_unknown_repository_gets_the_same_404_and_is_never_named() {
    let (_d, base) = serve(Some(&["warte", "rivet"])).await;
    let (code, private) = get(format!("{base}/api/runner-usage?repo=secret-one&{WINDOW}")).await;
    assert_eq!(code, 404, "a private repository in the ledger: {private}");
    let (code, unknown) = get(format!(
        "{base}/api/runner-usage?repo=no-such-repo&{WINDOW}"
    ))
    .await;
    assert_eq!(code, 404);
    assert_eq!(
        private, unknown,
        "private and unknown must not be told apart"
    );
    assert!(!private.contains("secret-one"), "{private}");
    // a public answer never carries the private repository either
    for repo in ["warte", "rivet"] {
        let (code, body) = get(format!("{base}/api/runner-usage?repo={repo}&{WINDOW}")).await;
        assert_eq!(code, 200, "{repo}: {body}");
        assert!(!body.contains("secret-one"), "{repo}: {body}");
    }
    // no public list yet: nothing is answered
    let (_d2, bare) = serve(None).await;
    let (code, body) = get(format!("{bare}/api/runner-usage?repo=warte&{WINDOW}")).await;
    assert_eq!((code, body), (404, private));
}

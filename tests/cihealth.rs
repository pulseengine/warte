//! Oracles for CI health trends (REQ-PREDICT-005, DD-PREDICT-004).

use std::net::Ipv4Addr;
use warte::cihealth::{health, panel, Warning, WEEK_S};
use warte::hub::model::JobRecord;

const NOW: i64 = 1_800_000_000;
const THIS: i64 = NOW - 2 * 86_400;
const LAST: i64 = NOW - WEEK_S - 2 * 86_400;

struct R {
    repo: &'static str,
    wf: &'static str,
    job: &'static str,
    pool: &'static str,
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn id() -> u64 {
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn rec(r: &R, run: u64, att: u32, c: i64, wait: i64, run_s: i64, result: &str) -> JobRecord {
    JobRecord {
        d: chrono::DateTime::from_timestamp(c, 0)
            .unwrap()
            .format("%Y-%m-%d")
            .to_string(),
        id: id(),
        run,
        att,
        repo: r.repo.into(),
        wf: r.wf.into(),
        job: r.job.into(),
        pool: r.pool.into(),
        runner: "r1".into(),
        ev: "push".into(),
        c,
        s: Some(c + wait),
        e: Some(c + wait + run_s),
        r: result.into(),
        sig: None,
    }
}

/// `n` first-attempt jobs in `week`, `bad` of them failed.
fn jobs(r: &R, week: i64, n: usize, bad: usize) -> Vec<JobRecord> {
    (0..n)
        .map(|i| {
            rec(
                r,
                10_000 + week as u64 % 1000 + i as u64 * 7,
                1,
                week + i as i64 * 60,
                30,
                120,
                if i < bad { "failure" } else { "success" },
            )
        })
        .collect()
}

fn warn<'a>(ws: &'a [Warning], scope: &str, what: &str) -> Option<&'a Warning> {
    ws.iter().find(|w| w.scope == scope && w.what == what)
}

const A: R = R {
    repo: "alpha",
    wf: "CI",
    job: "test",
    pool: "ubuntu-latest",
};
const B: R = R {
    repo: "beta",
    wf: "CI",
    job: "test",
    pool: "ubuntu-latest",
};

// rivet: verifies REQ-PREDICT-005
// rivet: verifies DD-PREDICT-004
#[test]
fn a_failure_rate_that_doubled_is_called_out_with_its_numbers() {
    let mut recs = jobs(&A, LAST, 40, 2);
    recs.extend(jobs(&A, THIS, 40, 8));
    // too few jobs to judge, however bad
    recs.extend(jobs(&B, LAST, 10, 0));
    recs.extend(jobs(&B, THIS, 10, 5));
    let h = health(&recs, NOW);
    let a = h.repos.iter().find(|s| s.repo == "alpha").unwrap();
    assert_eq!(
        (a.this.jobs, a.this.failed, a.last.jobs, a.last.failed),
        (40, 8, 40, 2)
    );
    let w = warn(&h.warnings, "alpha", "failure_rate").expect("alpha's rate");
    assert_eq!(
        w.text,
        "alpha: 20% of jobs failed this week (8 of 40), 5% the week before"
    );
    assert!(
        warn(&h.warnings, "beta", "failure_rate").is_none(),
        "{:?}",
        h.warnings
    );

    // high but steady is not a trend
    let mut recs = jobs(&A, LAST, 40, 8);
    recs.extend(jobs(&A, THIS, 40, 9));
    assert!(health(&recs, NOW).warnings.is_empty());

    // cancelled and skipped jobs are not counted, and nothing older than
    // two weeks is
    let mut recs = jobs(&A, THIS, 20, 0);
    recs.push(rec(&A, 1, 1, THIS, 30, 60, "cancelled"));
    recs.push(rec(&A, 2, 1, THIS, 30, 60, "skipped"));
    recs.push(rec(&A, 3, 1, NOW - 2 * WEEK_S - 86_400, 30, 60, "failure"));
    let h = health(&recs, NOW);
    assert_eq!(
        (h.all.this.jobs, h.all.last.jobs, h.all.this.failed),
        (20, 0, 0)
    );
}

// rivet: verifies REQ-PREDICT-005
#[test]
fn a_job_that_failed_and_then_passed_on_a_re_run_is_flaky() {
    let mut recs = jobs(&A, LAST, 30, 0);
    recs.extend(jobs(&A, THIS, 30, 0));
    for run in [1, 2, 3] {
        recs.push(rec(&A, run, 1, THIS, 30, 60, "failure"));
        recs.push(rec(&A, run, 2, THIS + 600, 30, 60, "success"));
    }
    // failing on every attempt is broken, not flaky
    recs.push(rec(&A, 4, 1, THIS, 30, 60, "failure"));
    recs.push(rec(&A, 4, 2, THIS + 600, 30, 60, "failure"));
    // a passing job re-run with its run is not flaky
    recs.push(rec(&A, 6, 1, THIS, 30, 60, "success"));
    recs.push(rec(&A, 6, 2, THIS + 600, 30, 60, "success"));
    // a different job of the same run passing is not a re-run of this one
    recs.push(rec(&A, 5, 1, THIS, 30, 60, "failure"));
    recs.push(rec(&R { job: "lint", ..A }, 5, 1, THIS, 30, 60, "success"));
    let h = health(&recs, NOW);
    assert_eq!(h.all.this.flaky, 3);
    let w = warn(&h.warnings, "alpha", "flaky").expect("flaky");
    assert!(
        w.text
            .contains("3 jobs failed and then passed on a re-run this week, 0 the week before"),
        "{}",
        w.text
    );
}

// rivet: verifies REQ-PREDICT-005
#[test]
fn a_queue_wait_that_doubled_is_called_out_per_pool() {
    let mut recs = Vec::new();
    for i in 0..10 {
        recs.push(rec(&A, 100 + i, 1, LAST + i as i64 * 60, 60, 60, "success"));
        recs.push(rec(
            &A,
            200 + i,
            1,
            THIS + i as i64 * 60,
            900,
            60,
            "success",
        ));
        // a re-run keeps an old start: never a wait
        recs.push(rec(
            &A,
            300 + i,
            2,
            LAST + i as i64 * 60,
            50_000,
            60,
            "success",
        ));
    }
    // too few waits in another pool to judge
    let other = R {
        pool: "linux-16core",
        ..A
    };
    for i in 0..4 {
        recs.push(rec(&other, 400 + i, 1, THIS, 5_000, 60, "success"));
    }
    let h = health(&recs, NOW);
    let wait = h.waits.iter().find(|w| w.pool == "ubuntu-latest").unwrap();
    assert_eq!(
        (wait.this_p90_s, wait.last_p90_s),
        (Some(900.0), Some(60.0))
    );
    let w = warn(&h.warnings, "ubuntu-latest", "queue_wait").expect("wait");
    assert_eq!(
        w.text,
        "pool ubuntu-latest: 1 job in 10 waited 15 min or more for a runner this week, 60 s the week before"
    );
    let o = h.waits.iter().find(|w| w.pool == "linux-16core").unwrap();
    assert_eq!(o.this_p90_s, None);
    assert!(warn(&h.warnings, "linux-16core", "queue_wait").is_none());
}

// rivet: verifies REQ-PREDICT-005
#[test]
fn a_release_run_that_got_much_slower_is_called_out() {
    let rel = R {
        wf: "Release",
        job: "build",
        ..A
    };
    let gate = R {
        wf: "Release",
        job: "gate",
        ..A
    };
    let mut recs = Vec::new();
    for i in 0..5u64 {
        // a run is its jobs from the first start to the last end
        recs.push(rec(
            &gate,
            500 + i,
            1,
            LAST + i as i64 * 3600,
            0,
            60,
            "success",
        ));
        recs.push(rec(
            &rel,
            500 + i,
            1,
            LAST + i as i64 * 3600 + 60,
            0,
            540,
            "success",
        ));
        recs.push(rec(
            &rel,
            600 + i,
            1,
            THIS + i as i64 * 3600,
            0,
            1800,
            "success",
        ));
    }
    // failed release runs are not durations
    for i in 0..6u64 {
        recs.push(rec(
            &rel,
            700 + i,
            1,
            THIS + i as i64 * 3600,
            0,
            90_000,
            "failure",
        ));
    }
    let h = health(&recs, NOW);
    assert_eq!(h.release_last_p50_s, Some(600.0));
    assert_eq!(h.release_this_p50_s, Some(1800.0));
    let w = warn(&h.warnings, "all", "release_duration").expect("release");
    assert_eq!(
        w.text,
        "a release run took 30 min this week (median), 10 min the week before"
    );

    // slower, but not by half: not called out
    let mut recs = Vec::new();
    for i in 0..5u64 {
        recs.push(rec(
            &rel,
            800 + i,
            1,
            LAST + i as i64 * 3600,
            0,
            600,
            "success",
        ));
        recs.push(rec(
            &rel,
            900 + i,
            1,
            THIS + i as i64 * 3600,
            0,
            840,
            "success",
        ));
    }
    let h = health(&recs, NOW);
    assert_eq!(h.release_this_p50_s, Some(840.0));
    assert!(warn(&h.warnings, "all", "release_duration").is_none());
}

// rivet: verifies REQ-PREDICT-005
#[test]
fn the_panel_says_when_there_is_nothing_to_act_on() {
    let mut recs = jobs(&A, LAST, 30, 3);
    recs.extend(jobs(&A, THIS, 30, 3));
    let p = panel(&health(&recs, NOW));
    assert!(
        p.contains("<h3>CI health</h3>") && p.contains("No trend worth acting on"),
        "{p}"
    );
    assert!(p.contains("30 jobs this week, 3 failed (10%)"), "{p}");
}

// rivet: verifies REQ-PREDICT-005
#[tokio::test]
async fn the_api_and_overview_read_the_ledger() {
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
    let mut recs = jobs(&A, now - WEEK_S - 3600, 25, 1);
    recs.extend(jobs(&A, now - 3600, 25, 10));
    state.hub.as_ref().unwrap().ingest_ledger(recs, vec![]);
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    let v: serde_json::Value = reqwest::get(format!("{base}/api/ci-health"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["all"]["this"]["failed"], 10, "{v}");
    assert_eq!(v["warnings"][0]["what"], "failure_rate", "{v}");
    let ov = reqwest::get(format!("{base}/api/dashboard/overview-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        ov.contains("<h3>CI health</h3>") && ov.contains("40% of jobs failed"),
        "{ov}"
    );
}

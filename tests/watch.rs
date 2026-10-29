//! Oracles for `warte watch` and `/api/watch` (REQ-WATCH-001/002).

use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};
use warte::hub::model::{HostedJob, Queue, QueuedEntry, Runner, Snapshot};
use warte::hub::telemetry::{encode_snapshot, SeqCounters};
use warte::hub::{now, Hub};

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// spar PR 511: run 10 queued; or, once `started`, run 10 running on a
/// hosted runner and run 11 queued.
fn snapshot(started: bool) -> Snapshot {
    let queued = |run_id| QueuedEntry {
        repo: "spar".into(),
        run_id,
        workflow: "CI".into(),
        event: "pull_request".into(),
        branch: "feat".into(),
        pr: Some(511),
        pool: "rust-cpu".into(),
        jobs: 1,
        since: now() - 60,
    };
    let mut q = Queue {
        scanned_at: now(),
        queued_complete: true,
        ..Default::default()
    };
    if started {
        q.queued.push(queued(11));
        q.hosted.push(HostedJob {
            repo: "pulseengine/spar".into(),
            status: "in_progress".into(),
            run_id: 10,
            pr: Some(511),
            branch: "feat".into(),
            ..Default::default()
        });
    } else {
        q.queued.push(queued(10));
    }
    Snapshot {
        host: "ci-test".into(),
        taken_at: now(),
        runners: vec![Runner {
            id: 1,
            ..Default::default()
        }],
        queue: Some(q),
        ..Default::default()
    }
}

fn send(hub: &Hub, s: &Snapshot) {
    let mut seq = SeqCounters::default();
    for f in encode_snapshot(s, &mut seq) {
        hub.ingest(&f, LO, &[LO]);
    }
}

async fn serve() -> (tempfile::TempDir, String, Hub) {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: Some(warte::config::HubConfig {
            udp: "127.0.0.1:0".into(),
            allow: vec![LO],
            data_dir: dir.path().join("hub").to_str().unwrap().into(),
            org: "pulseengine".into(),
            probe_self: false,
            costs_file: None,
        }),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let hub = state.hub.clone().unwrap();
    send(&hub, &snapshot(false));
    let app = warte::server::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (dir, format!("http://{addr}"), hub)
}

async fn watch(base: &str, q: &str) -> (u16, serde_json::Value) {
    let r = reqwest::get(format!("{base}/api/watch?{q}")).await.unwrap();
    let code = r.status().as_u16();
    let v = r.json().await.unwrap_or(serde_json::Value::Null);
    (code, v)
}

// rivet: verifies REQ-WATCH-002
#[tokio::test]
async fn a_hold_ends_on_the_next_snapshot_that_changes_the_prs_ci() {
    let (_d, base, hub) = serve().await;
    // baseline: run 10 queued; nothing changes for a second
    let t = Instant::now();
    let (code, idle) = watch(&base, "repo=spar&pr=511&timeout=1").await;
    assert_eq!(code, 200);
    assert_eq!(idle["changed"], false);
    assert_eq!(idle["status"], "queued");
    assert!(
        t.elapsed() >= Duration::from_millis(900),
        "held until the timeout"
    );
    let cursor = idle["cursor"].as_str().unwrap().to_string();

    // hold from that cursor; run 10 starts while it is held
    let held = tokio::spawn({
        let base = base.clone();
        let cursor = cursor.clone();
        async move {
            watch(
                &base,
                &format!("repo=spar&pr=511&since={cursor}&timeout=30"),
            )
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!held.is_finished(), "no change yet: still held");
    send(&hub, &snapshot(true));
    let (code, v) = tokio::time::timeout(Duration::from_secs(5), held)
        .await
        .expect("woken by the new snapshot, not by the 30 s timeout")
        .unwrap();
    assert_eq!(code, 200);
    assert_eq!(v["changed"], true);
    assert_eq!(
        (v["from"].as_str(), v["status"].as_str()),
        (Some("queued"), Some("running+queued"))
    );
    assert_eq!(v["runs"].as_array().unwrap().len(), 2, "{v}");

    // a caller that was away resumes with its old cursor: answered at once
    let t = Instant::now();
    let (_, again) = watch(
        &base,
        &format!("repo=spar&pr=511&since={cursor}&timeout=30"),
    )
    .await;
    assert_eq!(again["changed"], true);
    assert!(t.elapsed() < Duration::from_secs(2));

    // a snapshot that changes nothing for this PR does not end the hold
    let now_cursor = v["cursor"].as_str().unwrap().to_string();
    let held = tokio::spawn({
        let base = base.clone();
        async move {
            watch(
                &base,
                &format!("repo=spar&pr=511&since={now_cursor}&timeout=2"),
            )
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    send(&hub, &snapshot(true));
    let (_, v) = held.await.unwrap();
    assert_eq!(v["changed"], false, "same runs, same states: {v}");
}

// rivet: verifies REQ-WATCH-002
#[tokio::test]
async fn bad_requests_are_refused_not_held() {
    let (_d, base, _hub) = serve().await;
    for q in [
        "pr=511",
        "repo=spar",
        "repo=spar&pr=511&branch=feat",
        "repo=pulseengine/spar&pr=511",
        "repo=spar&pr=x",
        "repo=spar&pr=511&since=garbage",
        "repo=spar&pr=511&since=done.0123456789abcdef",
        "repo=spar&pr=511&timeout=-1",
    ] {
        let t = Instant::now();
        let (code, _) = watch(&base, q).await;
        assert_eq!(code, 400, "{q}");
        assert!(t.elapsed() < Duration::from_secs(1), "{q} was held");
    }
    // the contract lists the endpoint
    let doc: serde_json::Value = reqwest::get(format!("{base}/api/openapi.json"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(doc["paths"]["/api/watch"].is_object());
}

fn cli(base: &str, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_warte"))
        .args(["watch", "--url", base])
        .args(args)
        .env("RUST_LOG", "off")
        .output()
        .unwrap()
}

// rivet: verifies REQ-WATCH-001
// multi-threaded: the CLI blocks a thread while the server answers on another
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_cli_exits_0_on_a_change_3_on_timeout_2_on_usage_1_on_error() {
    let (_d, base, hub) = serve().await;

    // timeout with no change: exit 3, and the JSON carries a cursor
    let b = base.clone();
    let out = tokio::task::spawn_blocking(move || {
        cli(
            &b,
            &["--ci", "--repo", "spar", "--pr", "511", "--timeout", "1"],
        )
    })
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["changed"], false);
    let cursor = v["cursor"].as_str().unwrap().to_string();

    // the change lands while the CLI waits: exit 0 with the change
    let b = base.clone();
    let c = cursor.clone();
    let waiting = tokio::task::spawn_blocking(move || {
        cli(
            &b,
            &[
                "--ci",
                "--repo",
                "spar",
                "--pr",
                "511",
                "--since",
                &c,
                "--timeout",
                "60",
            ],
        )
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    send(&hub, &snapshot(true));
    let out = tokio::time::timeout(Duration::from_secs(10), waiting)
        .await
        .expect("the CLI returned on the change")
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["status"], "running+queued");

    // usage errors: 2
    for args in [
        &["--ci", "--repo", "spar"][..],
        &["--repo", "spar", "--pr", "1"][..],
        &["--ci", "--repo", "spar", "--pr", "1", "--branch", "x"][..],
    ] {
        let out = cli(&base, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
    }

    // a server that refuses, or none at all: 1
    let b = base.clone();
    let out = tokio::task::spawn_blocking(move || cli(&b, &["--ci", "--repo", "a/b", "--pr", "1"]))
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let out = cli(
        "http://127.0.0.1:9",
        &["--ci", "--repo", "spar", "--pr", "1"],
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    // a --ca-cert that holds no certificate is an error up front, not a
    // TLS failure later
    let not_pem = _d.path().join("not.pem");
    std::fs::write(&not_pem, "hello").unwrap();
    let out = cli(
        &base,
        &[
            "--ci",
            "--repo",
            "spar",
            "--pr",
            "1",
            "--ca-cert",
            not_pem.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("no PEM certificate"));
}

// rivet: verifies REQ-WATCH-002
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_client_cannot_hold_every_watch() {
    let (_d, base, _hub) = serve().await;
    let cap = warte::watch::MAX_WATCHERS_PER_CLIENT;
    let held: Vec<_> = (0..cap)
        .map(|_| {
            let base = base.clone();
            tokio::spawn(async move { watch(&base, "repo=spar&pr=511&timeout=3").await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let (code, _) = watch(&base, "repo=spar&pr=511&timeout=3").await;
    assert_eq!(code, 429, "over the per-client cap");
    for h in held {
        assert_eq!(h.await.unwrap().0, 200);
    }
    // the slots came back
    let (code, _) = watch(&base, "repo=spar&pr=511&timeout=0").await;
    assert_eq!(code, 200);
}

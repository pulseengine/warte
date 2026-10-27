//! Oracles for the ground station absorbed from smithy hub (DD-SMITHY-001).
//!
//! Telemetry is produced with the agent's own encoder (`telemetry::encode_*`),
//! so these tests exercise the same bytes the smithy agent sends. Snapshots are
//! synthetic: real hub state carries operational detail that does not belong in
//! a public repo. Parity with the live hub is checked separately during the
//! parallel run.

use sha2::{Digest, Sha256};
use std::net::{IpAddr, Ipv4Addr};
use warte::hub::model::{DayManifest, JobRecord, Machine, Queue, Runner, Snapshot};
use warte::hub::telemetry::{encode_ledger, encode_snapshot, SeqCounters};
use warte::hub::{now, Hub, OPENAPI};

const CI_HOST: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 201));
const STRANGER: IpAddr = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7));

/// sha256 of smithy's `tools/smithy/web/openapi.json` at the UPSTREAM commit.
/// The vendored copy must not drift silently from the contract agents use.
const UPSTREAM_OPENAPI_SHA256: &str =
    "a18f74c14bf44b56e9bac4fc9c3d0ec7985be6be9cdc8f77046aaf7690acf9ce";

fn snapshot(host: &str) -> Snapshot {
    Snapshot {
        agent_version: "0.1.0".into(),
        host: host.into(),
        taken_at: now(),
        machine: Machine {
            ncpu: 32,
            ..Default::default()
        },
        runners: vec![Runner {
            id: 1,
            unit: "github-runner@runner1.service".into(),
            active: true,
            labels: vec!["self-hosted".into(), "rust-cpu".into()],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn send(hub: &Hub, frames: Vec<Vec<u8>>, src: IpAddr) {
    for f in frames {
        hub.ingest(&f, src, &[CI_HOST]);
    }
}

/// Serve the hub router on an ephemeral port; returns its base URL.
async fn serve(hub: &Hub) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = hub.router();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

// rivet: verifies REQ-SMITHY-001
#[tokio::test]
async fn telemetry_from_the_agent_encoder_reaches_api_state() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let base = serve(&hub).await;

    // before any telemetry the hub says so, rather than serving an empty view
    let r = reqwest::get(format!("{base}/api/eta?repo=warte&pr=1"))
        .await
        .unwrap();
    assert_eq!(r.status(), 503);

    let mut seq = SeqCounters::default();
    send(
        &hub,
        encode_snapshot(&snapshot("ci-test"), &mut seq),
        CI_HOST,
    );

    let v: serde_json::Value = reqwest::get(format!("{base}/api/state"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["snapshot"]["host"], "ci-test");
    assert_eq!(v["snapshot"]["runners"][0]["labels"][1], "rust-cpu");
    assert_eq!(v["frames"], 1);
    assert_eq!(v["foreign"], 0);
}

// rivet: verifies REQ-SMITHY-001
#[tokio::test]
async fn telemetry_from_a_non_allowed_source_is_counted_not_ingested() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let base = serve(&hub).await;

    let mut seq = SeqCounters::default();
    let frames = encode_snapshot(&snapshot("intruder"), &mut seq);
    let n = frames.len() as u64;
    send(&hub, frames, STRANGER);

    let v: serde_json::Value = reqwest::get(format!("{base}/api/state"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(v["snapshot"].is_null(), "foreign telemetry was ingested");
    assert_eq!(v["foreign"], n);
    assert_eq!(v["frames"], 0);
}

fn contract_paths() -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(OPENAPI).expect("openapi.json parses");
    let paths: Vec<String> = doc["paths"]
        .as_object()
        .expect("openapi has paths")
        .keys()
        .cloned()
        .collect();
    assert!(
        !paths.is_empty(),
        "contract lists no paths — nothing to check"
    );
    paths
}

// rivet: verifies REQ-SMITHY-002
#[test]
fn vendored_contract_is_the_upstream_one() {
    let got = hex::encode(Sha256::digest(OPENAPI.as_bytes()));
    assert_eq!(
        got, UPSTREAM_OPENAPI_SHA256,
        "src/hub/web/openapi.json differs from smithy's contract; update both \
         deliberately (UPSTREAM in src/hub/mod.rs and this hash)"
    );
}

// rivet: verifies REQ-SMITHY-002
#[tokio::test]
async fn every_contract_path_is_served() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let mut seq = SeqCounters::default();
    send(
        &hub,
        encode_snapshot(&snapshot("ci-test"), &mut seq),
        CI_HOST,
    );
    let base = serve(&hub).await;

    for p in contract_paths() {
        // the two ETA-shaped endpoints need their required parameters
        let q = match p.as_str() {
            "/api/eta" => "?repo=warte&pr=40",
            _ => "",
        };
        let r = reqwest::get(format!("{base}{p}{q}")).await.unwrap();
        assert_eq!(
            r.status(),
            200,
            "GET {p}{q} is in the contract but returned {}",
            r.status()
        );
    }
    let r = reqwest::get(format!("{base}/api/openapi.json"))
        .await
        .unwrap();
    assert_eq!(r.text().await.unwrap(), OPENAPI, "served contract differs");

    // a path outside the contract is not silently served
    let r = reqwest::get(format!("{base}/api/not-in-contract"))
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

// rivet: verifies REQ-SMITHY-002
#[tokio::test]
async fn eta_rejects_what_smithy_rejects() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let mut seq = SeqCounters::default();
    send(
        &hub,
        encode_snapshot(&snapshot("ci-test"), &mut seq),
        CI_HOST,
    );
    let base = serve(&hub).await;

    for q in ["repo=pulseengine/spar&pr=1", "repo=spar", "repo=spar&pr=0"] {
        let r = reqwest::get(format!("{base}/api/eta?{q}")).await.unwrap();
        assert_eq!(r.status(), 400, "/api/eta?{q} must be rejected");
    }
    let eta = || async {
        reqwest::get(format!("{base}/api/eta?repo=warte&pr=40"))
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap()
    };
    // no queue scan yet: smithy says "unknown", never a false "not_queued"
    let v = eta().await;
    assert_eq!(
        v["repo"], "pulseengine/warte",
        "org-qualified as smithy does"
    );
    assert_eq!(v["status"], "unknown");

    // a complete scan that does not contain the PR: nothing to wait for
    let mut s = snapshot("ci-test");
    s.queue = Some(Queue {
        scanned_at: now(),
        queued_complete: true,
        ..Default::default()
    });
    send(&hub, encode_snapshot(&s, &mut seq), CI_HOST);
    assert_eq!(eta().await["status"], "not_queued");
}

fn job(id: u64, day: &str) -> JobRecord {
    JobRecord {
        d: day.into(),
        id,
        run: 1000 + id,
        att: 1,
        repo: "warte".into(),
        wf: "CI".into(),
        job: "Build".into(),
        pool: "rust-cpu".into(),
        runner: "pulseengine-ci-01-7".into(),
        ev: "pull_request".into(),
        c: 1_793_000_000,
        s: Some(1_793_000_060),
        e: Some(1_793_000_330),
        r: "success".into(),
    }
}

// rivet: verifies REQ-SMITHY-003
#[test]
fn ledger_is_per_day_jsonl_in_smithys_format_and_deduplicated() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_str().unwrap();
    let hub = Hub::open(data, "pulseengine").unwrap();
    let mut seq = SeqCounters::default();
    let manifest = DayManifest {
        day: "2026-10-27".into(),
        n: 2,
        x: 1 ^ 2,
        closed: false,
    };
    let frames = encode_ledger(
        &[job(1, "2026-10-27"), job(2, "2026-10-27")],
        &[manifest],
        &mut seq,
    );
    send(&hub, frames.clone(), CI_HOST);
    // the agent re-sends days; the hub must not duplicate them
    send(&hub, frames, CI_HOST);
    // a day that is not a date must never become a file name
    send(
        &hub,
        encode_ledger(&[job(3, "../etc")], &[], &mut seq),
        CI_HOST,
    );

    let ledger = dir.path().join("ledger");
    let files: Vec<String> = std::fs::read_dir(&ledger)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, vec!["2026-10-27.jsonl"]);
    // nor may it escape the ledger directory: the data dir holds only what
    // smithy hub's layout has
    let root: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "hub-state.json" && n != "hub-state.json.tmp")
        .collect();
    assert_eq!(
        root,
        vec!["ledger"],
        "a ledger record escaped the ledger directory"
    );
    let text = std::fs::read_to_string(ledger.join("2026-10-27.jsonl")).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "re-sent records were duplicated");
    let keys: Vec<&str> = lines[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    // smithy hub's on-disk record: an existing /var/lib/smithy-hub reads as-is
    for k in [
        "d", "id", "run", "att", "repo", "wf", "job", "pool", "runner", "ev", "c", "s", "e", "r",
    ] {
        assert!(keys.contains(&k), "ledger record lacks `{k}`: {keys:?}");
    }
}

// rivet: verifies REQ-SMITHY-003
#[tokio::test]
async fn hub_state_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_str().unwrap();
    {
        let hub = Hub::open(data, "pulseengine").unwrap();
        let mut seq = SeqCounters::default();
        // state is written every 5th frame, as smithy hub does
        for _ in 0..5 {
            send(
                &hub,
                encode_snapshot(&snapshot("ci-test"), &mut seq),
                CI_HOST,
            );
        }
    }
    assert!(dir.path().join("hub-state.json").exists());
    let hub = Hub::open(data, "pulseengine").unwrap();
    let base = serve(&hub).await;
    let v: serde_json::Value = reqwest::get(format!("{base}/api/state"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        v["snapshot"]["host"], "ci-test",
        "latest snapshot not restored"
    );
    let h: Vec<serde_json::Value> = reqwest::get(format!("{base}/api/history?hours=1"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(h.len(), 5, "history not restored");
}

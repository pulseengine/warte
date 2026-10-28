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
        runner: "ci-host-7".into(),
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

/// Check a JSON value against one OpenAPI property schema (type, nullable, enum).
fn conforms(v: &serde_json::Value, schema: &serde_json::Value) -> Result<(), String> {
    if v.is_null() {
        return if schema["nullable"] == true {
            Ok(())
        } else {
            Err("null for a non-nullable field".into())
        };
    }
    let ok = match schema["type"].as_str() {
        Some("string") => v.is_string(),
        Some("integer") => v.is_i64() || v.is_u64(),
        Some("boolean") => v.is_boolean(),
        Some("array") => v.is_array(),
        Some("object") => v.is_object(),
        Some(t) => return Err(format!("schema type {t} not handled by this check")),
        None => true,
    };
    if !ok {
        return Err(format!("{v} is not a {}", schema["type"]));
    }
    if let Some(allowed) = schema["enum"].as_array() {
        if !allowed.contains(v) {
            return Err(format!("{v} not in {allowed:?}"));
        }
    }
    Ok(())
}

fn eta_schema() -> serde_json::Value {
    let doc: serde_json::Value = serde_json::from_str(OPENAPI).unwrap();
    doc["components"]["schemas"]["Eta"].clone()
}

// rivet: verifies REQ-SMITHY-002
#[tokio::test]
async fn eta_answers_match_the_contract_schema() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let base = serve(&hub).await;
    let props = eta_schema()["properties"].clone();
    let props = props.as_object().expect("Eta has properties");
    assert!(!props.is_empty());

    let mut seq = SeqCounters::default();
    let mut s = snapshot("ci-test");
    s.queue = Some(Queue {
        scanned_at: now(),
        queued_complete: true,
        queued: vec![warte::hub::model::QueuedEntry {
            repo: "warte".into(),
            run_id: 7,
            workflow: "CI".into(),
            event: "pull_request".into(),
            branch: "feat/x".into(),
            pr: Some(41),
            pool: "rust-cpu".into(),
            jobs: 3,
            since: now() - 120,
        }],
        ..Default::default()
    });
    send(&hub, encode_snapshot(&s, &mut seq), CI_HOST);

    // queued (by PR and by branch) and not queued
    let mut statuses = vec![];
    for q in [
        "repo=warte&pr=41",
        "repo=warte&branch=feat%2Fx",
        "repo=warte&pr=999",
    ] {
        let r = reqwest::get(format!("{base}/api/eta?{q}")).await.unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = r.json().await.unwrap();
        for (k, val) in v.as_object().unwrap() {
            let schema = props
                .get(k)
                .unwrap_or_else(|| panic!("/api/eta?{q}: field `{k}` is not in the contract"));
            conforms(val, schema).unwrap_or_else(|e| panic!("/api/eta?{q}: `{k}`: {e}"));
        }
        statuses.push(v["status"].as_str().unwrap().to_string());
    }
    assert_eq!(statuses, ["queued", "queued", "not_queued"]);
}

// rivet: verifies REQ-SMITHY-002
#[tokio::test]
async fn every_declared_status_code_is_produced() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let base = serve(&hub).await;
    let get = |p: &'static str| {
        let base = base.clone();
        async move {
            reqwest::get(format!("{base}{p}"))
                .await
                .unwrap()
                .status()
                .as_u16()
        }
    };
    // before the first snapshot
    let eta_503 = get("/api/eta?repo=warte&pr=1").await;
    let status_503 = get("/status.md").await;

    let mut seq = SeqCounters::default();
    send(
        &hub,
        encode_snapshot(&snapshot("ci-test"), &mut seq),
        CI_HOST,
    );
    let produced = [
        ("/api/eta", 503, eta_503),
        // not declared in the contract (inherited from upstream), but smithy does it
        ("/status.md", 503, status_503),
        ("/api/eta", 200, get("/api/eta?repo=warte&pr=1").await),
        ("/api/eta", 400, get("/api/eta?repo=warte").await),
        ("/report.md", 200, get("/report.md?month=2026-10").await),
        ("/report.md", 400, get("/report.md?month=bad").await),
        ("/api/report", 200, get("/api/report?month=2026-10").await),
        ("/api/report", 400, get("/api/report?month=2026/10").await),
        ("/api/history", 200, get("/api/history?hours=500").await),
        (
            "/status.md",
            200,
            get("/status.md?repo=warte&branch=main").await,
        ),
    ];
    for (p, want, got) in produced {
        assert_eq!(got, want, "{p}: expected {want}");
    }
    // every code the contract declares is one this test produces
    let doc: serde_json::Value = serde_json::from_str(OPENAPI).unwrap();
    for (p, ops) in doc["paths"].as_object().unwrap() {
        for code in ops["get"]["responses"].as_object().unwrap().keys() {
            let c: u16 = code.parse().unwrap();
            let covered = c == 200 || produced.iter().any(|(q, w, _)| q == p && *w == c);
            assert!(
                covered,
                "contract declares {c} for {p}; no test produces it"
            );
        }
    }
}

// rivet: verifies REQ-SMITHY-002
#[tokio::test]
async fn only_get_is_served_like_smithy() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let base = serve(&hub).await;
    let c = reqwest::Client::new();
    for m in [
        reqwest::Method::HEAD,
        reqwest::Method::POST,
        reqwest::Method::DELETE,
    ] {
        let r = c
            .request(m.clone(), format!("{base}/healthz"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "{m} /healthz");
        assert_eq!(r.headers()["cache-control"], "no-store", "{m} /healthz");
    }
    for p in ["/", "/index.html"] {
        let r = reqwest::get(format!("{base}{p}")).await.unwrap();
        assert_eq!(r.status(), 200, "{p}");
        assert!(
            r.text().await.unwrap().contains("SMITHY"),
            "{p} serves the web view"
        );
    }
}

// rivet: verifies REQ-SMITHY-001
#[tokio::test]
async fn ingest_over_a_real_udp_socket() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let base = serve(&hub).await;
    let loopback: IpAddr = Ipv4Addr::LOCALHOST.into();
    let addr = hub.spawn_ingest("127.0.0.1:0", vec![loopback]).unwrap();
    assert!(
        hub.spawn_ingest("127.0.0.1:0", vec![]).is_err(),
        "an empty allow-list must refuse to start"
    );

    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut seq = SeqCounters::default();
    for f in encode_snapshot(&snapshot("over-udp"), &mut seq) {
        sock.send_to(&f, addr).unwrap();
    }
    let mut host = serde_json::Value::Null;
    for _ in 0..50 {
        let v: serde_json::Value = reqwest::get(format!("{base}/api/state"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        host = v["snapshot"]["host"].clone();
        if host == "over-udp" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(host, "over-udp");
}

// rivet: verifies REQ-SMITHY-003
#[tokio::test]
async fn implausible_ledger_records_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let base = serve(&hub).await;
    let day = "2026-10-27";
    let mut bad = vec![];
    // ends before it starts; ends ~290 billion years out; overflows; negative
    for (i, (c, s, e)) in [
        (1_793_000_000, 1_793_000_060, 1_793_000_000),
        (0, 0, i64::MAX),
        (i64::MIN, i64::MAX, i64::MAX),
        (-5, 0, 10),
    ]
    .into_iter()
    .enumerate()
    {
        let mut r = job(100 + i as u64, day);
        (r.c, r.s, r.e) = (c, Some(s), Some(e));
        bad.push(r);
    }
    let mut seq = SeqCounters::default();
    send(&hub, encode_ledger(&bad, &[], &mut seq), CI_HOST);
    let f = dir.path().join("ledger").join(format!("{day}.jsonl"));
    assert!(!f.exists(), "an implausible record was stored");

    // a re-run attempt legitimately starts before its run was created (GitHub
    // keeps started_at of a reused job); it must be kept, as smithy keeps it
    let mut rerun = job(200, day);
    (rerun.att, rerun.c, rerun.s, rerun.e) =
        (2, 1_793_000_000, Some(1_792_990_000), Some(1_792_990_300));
    send(&hub, encode_ledger(&[rerun], &[], &mut seq), CI_HOST);
    assert!(f.exists(), "a real re-run record was rejected");
    std::fs::remove_file(&f).unwrap();

    // a forged file on disk is filtered on open too, and the report stays fast
    std::fs::write(&f, serde_json::to_string(&bad[1]).unwrap() + "\n").unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let base2 = serve(&hub).await;
    let t = std::time::Instant::now();
    let r = reqwest::get(format!("{base2}/api/report?month=2026-10"))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(t.elapsed().as_secs() < 5, "report took {:?}", t.elapsed());
    drop(base);
}

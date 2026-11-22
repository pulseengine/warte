//! Security findings from hosts (REQ-FINDINGS-001, warte#114).

use serde_json::{json, Value};
use warte::findings::{parse, view, Severity, Store, MAX_FINDINGS, SILENT_S};
use warte::probe::{enroll, send};

fn finding(rule: &str, severity: &str, subject: &str) -> Value {
    json!({"source": "drift", "rule": rule, "severity": severity, "subject": subject,
           "first_seen": "2026-11-22T01:00:00Z", "last_seen": "2026-11-22T02:00:00Z", "count": 1})
}

fn report(findings: Vec<Value>) -> Value {
    json!({"schema": "security-findings.v1", "generated_at": "2026-11-22T02:00:00Z", "findings": findings})
}

// rivet: verifies REQ-FINDINGS-001
#[test]
fn a_report_carries_metadata_only_and_every_field_is_checked() {
    let ok = report(vec![
        finding("new-ssh-key", "high", "SHA256:abc"),
        finding("suid", "low", "/usr/bin/x"),
    ]);
    assert_eq!(parse(ok.to_string().as_bytes()).unwrap().findings.len(), 2);
    // planted: a field for contents, a wrong schema, an unknown source or
    // severity, an empty or long rule, a bad time, too many findings
    let mut contents = finding("aide-change", "medium", "/etc/shadow");
    contents["contents"] = json!("root:$6$...");
    let mut src = finding("x", "low", "y");
    src["source"] = json!("rumour");
    let mut time = finding("x", "low", "y");
    time["last_seen"] = json!("yesterday");
    let cases = vec![
        (report(vec![contents]), "schema:"),
        (
            json!({"schema": "security-findings.v2", "generated_at": "x", "findings": []}),
            "schema: must be",
        ),
        (report(vec![src]), "findings[0].source"),
        (report(vec![finding("x", "dire", "y")]), "schema:"),
        (report(vec![finding("", "low", "y")]), "findings[0].rule"),
        (
            report(vec![finding(&"r".repeat(81), "low", "y")]),
            "findings[0].rule",
        ),
        (report(vec![time]), "findings[0].last_seen"),
        (
            report(
                (0..=MAX_FINDINGS)
                    .map(|i| finding(&format!("r{i}"), "low", "s"))
                    .collect(),
            ),
            "findings: at most",
        ),
    ];
    for (v, want) in cases {
        let e = parse(v.to_string().as_bytes()).unwrap_err();
        assert!(e.starts_with(want), "{want}: {e}");
        assert!(!e.contains("$6$"), "a value was echoed: {e}");
    }
}

async fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let db = warte::db::Database::connect(&dir.path().join("state.db"))
        .await
        .unwrap();
    (dir, Store::new(db.pool().clone()))
}

// rivet: verifies REQ-FINDINGS-001
#[tokio::test]
async fn each_report_replaces_the_hosts_set_a_replay_is_refused_and_the_baseline_silences() {
    let (_d, s) = store().await;
    let r1 = parse(
        report(vec![
            finding("new-ssh-key", "high", "SHA256:abc"),
            finding("suid", "low", "/x"),
        ])
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    assert!(s.record("h1", 100, 1000, &r1).await.unwrap());
    let open = s.open().await;
    assert_eq!(open.len(), 2);
    assert_eq!(open[0].finding.severity, Severity::High, "highest first");
    // planted: the same report time again is a replay
    assert!(!s.record("h1", 100, 1001, &r1).await.unwrap());
    // a newer report replaces the set
    let r2 = parse(
        report(vec![finding("suid", "low", "/x")])
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    assert!(s.record("h1", 200, 1002, &r2).await.unwrap());
    assert_eq!(s.open().await.len(), 1);
    // accepting into the baseline closes it; an unknown finding is refused
    assert!(s
        .accept("h1", "drift", "suid", "/x", "owner")
        .await
        .unwrap());
    assert!(s.open().await.is_empty());
    assert!(!s
        .accept("h1", "drift", "nope", "/x", "owner")
        .await
        .unwrap());
    // the baseline holds when the same finding is reported again
    assert!(s.record("h1", 300, 1003, &r2).await.unwrap());
    assert!(s.open().await.is_empty());
    // a host silent for longer than 36 hours is a stopped detector
    assert!(s.silent(1003 + SILENT_S - 1).await.is_empty());
    assert_eq!(s.silent(1003 + SILENT_S + 1).await, ["h1"]);
}

// rivet: verifies REQ-FINDINGS-001
#[test]
fn high_findings_and_a_silent_detector_reach_the_glance_and_the_view() {
    let i = warte::glance::Inputs {
        security_findings: vec![("h1".into(), "new-ssh-key".into())],
        silent_detectors: vec!["h2".into()],
        always_open: vec!["security".into()],
        now: 0,
        ..Default::default()
    };
    let g = warte::glance::glance(&i);
    assert_eq!(g.verdict, "needs-you");
    assert_eq!(g.items.len(), 2);
    assert!(g.items.iter().all(|x| x.kind == "security"));
    let open = vec![warte::findings::Open {
        host: "h1".into(),
        finding: serde_json::from_value(finding("new-ssh-key", "critical", "SHA256:abc")).unwrap(),
    }];
    let html = view(&open, &["h2".to_string()], Some("csrf"));
    assert!(
        html.contains("warnrow") && html.contains("Accept") && html.contains("h2"),
        "{html}"
    );
    assert!(
        !view(&open, &[], None).contains("data-host="),
        "only the owner sees accept buttons"
    );
}

async fn serve() -> (tempfile::TempDir, String, warte::db::Database) {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db.clone()).unwrap();
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (dir, base, db)
}

fn signed(dir: &std::path::Path, body: &Value, at: i64) -> send::SignedFeed {
    use base64::Engine;
    let packets = warte::hub::telemetry::segment(
        warte::hub::telemetry::APID_FEED_SECURITY,
        &serde_json::to_vec(body).unwrap(),
    )
    .unwrap();
    let key = std::fs::read_to_string(dir.join("probe-key.pem")).unwrap();
    send::SignedFeed {
        sig: hex::encode(
            send::sign_bytes(&key, &send::feed_message("security", at, &packets)).unwrap(),
        ),
        cert: std::fs::read_to_string(dir.join("probe-cert.pem")).unwrap(),
        name: "security".into(),
        ccsds: base64::engine::general_purpose::STANDARD.encode(&packets),
        at,
    }
}

async fn post(base: &str, f: &send::SignedFeed) -> u16 {
    reqwest::Client::new()
        .post(format!("{base}/api/probe/feed/security"))
        .json(f)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

// rivet: verifies REQ-FINDINGS-001
#[tokio::test]
async fn an_enrolled_host_sends_its_findings_signed_and_a_forgery_or_replay_is_refused() {
    let (dir, base, db) = serve().await;
    let probe = dir.path().join("probe");
    let token = enroll::mint(&db, "h1", warte::hub::now()).await.unwrap();
    enroll::enroll(&base, &token, "h1", &probe, None)
        .await
        .unwrap();
    let body = report(vec![finding("new-ssh-key", "high", "SHA256:abc")]);
    let now = warte::hub::now();
    let f = signed(&probe, &body, now);
    assert_eq!(post(&base, &f).await, 204);
    let s = Store::new(db.pool().clone());
    assert_eq!(
        s.open().await[0].host,
        "h1",
        "the host is the certificate's, not the body's"
    );
    // planted: the same feed again, an altered signature, a body outside the schema
    assert_eq!(post(&base, &f).await, 403, "a replay");
    let mut forged = signed(&probe, &body, now + 1);
    forged.sig = "00".repeat(64);
    assert_eq!(post(&base, &forged).await, 403);
    let mut bad = body.clone();
    bad["findings"][0]["secret"] = json!("value");
    assert_eq!(post(&base, &signed(&probe, &bad, now + 2)).await, 422);
}

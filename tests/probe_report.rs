//! Oracles for signed probe reports (REQ-PROBE-006): an enrolled machine's
//! report lands; a replay, another host's name, a foreign certificate and a
//! changed report are refused.

use warte::probe::{enroll, send};

async fn serve() -> (tempfile::TempDir, String, warte::db::Database) {
    let dir = tempfile::tempdir().unwrap();
    let hub: warte::config::HubConfig = serde_json::from_value(serde_json::json!({
        "udp": "127.0.0.1:0", "allow": ["127.0.0.1"],
        "data_dir": dir.path().join("hub").to_str().unwrap(), "probe_self": false
    }))
    .unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: Some(hub),
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

async fn enrolled(
    base: &str,
    db: &warte::db::Database,
    host: &str,
    dir: &std::path::Path,
) -> (String, String) {
    let token = enroll::mint(db, host, warte::hub::now()).await.unwrap();
    enroll::enroll(base, &token, host, dir, None).await.unwrap();
    (
        std::fs::read_to_string(dir.join("probe-key.pem")).unwrap(),
        std::fs::read_to_string(dir.join("probe-cert.pem")).unwrap(),
    )
}

fn report(host: &str, taken_at: i64) -> String {
    serde_json::json!({
        "probe_version": "t", "host": host, "taken_at": taken_at, "uptime_s": 1,
        "os": {"name": "macOS 27", "kernel": "27", "arch": "arm64"},
        "cpu": {"ncpu": 10, "load": [0.0, 0.0, 0.0]},
        "memory": {"total_mb": 1, "avail_mb": 1, "swap_total_mb": 0, "swap_used_mb": 0},
        "disks": [], "tools": [], "sessions": []
    })
    .to_string()
}

async fn post(base: &str, body: &send::SignedReport) -> u16 {
    reqwest::Client::new()
        .post(format!("{base}/api/probe/report"))
        .json(body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn hosts(base: &str) -> Vec<String> {
    let v: serde_json::Value = reqwest::get(format!("{base}/api/machines"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v.as_array()
        .or_else(|| v["machines"].as_array())
        .unwrap()
        .iter()
        .map(|m| m["host"].as_str().unwrap_or_default().to_string())
        .collect()
}

// rivet: verifies REQ-PROBE-006
#[tokio::test]
async fn an_enrolled_machine_reports_and_a_replay_is_refused() {
    let (dir, base, db) = serve().await;
    let (key, cert) = enrolled(&base, &db, "Mac.fritz.box", &dir.path().join("p")).await;
    let now = warte::hub::now();
    let r = report("Mac.fritz.box", now);
    let body = send::SignedReport {
        sig: send::sign(&key, &r).unwrap(),
        cert: cert.clone(),
        report: r,
    };
    assert_eq!(post(&base, &body).await, 204);
    assert!(hosts(&base).await.iter().any(|h| h == "Mac.fritz.box"));
    // the same report again: a replay
    assert_eq!(post(&base, &body).await, 403);
    // a newer one is fine
    let r2 = report("Mac.fritz.box", now + 1);
    let b2 = send::SignedReport {
        sig: send::sign(&key, &r2).unwrap(),
        cert,
        report: r2,
    };
    assert_eq!(post(&base, &b2).await, 204);
}

// rivet: verifies REQ-PROBE-006
#[tokio::test]
async fn a_machine_cannot_speak_for_another_or_alter_or_forge() {
    let (dir, base, db) = serve().await;
    let (key, cert) = enrolled(&base, &db, "PowerBook", &dir.path().join("p")).await;
    let now = warte::hub::now();

    // another host's name under PowerBook's certificate
    let r = report("fourpi", now);
    let b = send::SignedReport {
        sig: send::sign(&key, &r).unwrap(),
        cert: cert.clone(),
        report: r,
    };
    assert_eq!(post(&base, &b).await, 403);

    // the report changed after signing
    let r = report("PowerBook", now);
    let sig = send::sign(&key, &r).unwrap();
    let b = send::SignedReport {
        sig,
        cert: cert.clone(),
        report: r.replace("\"ncpu\":10", "\"ncpu\":99"),
    };
    assert_eq!(post(&base, &b).await, 403);

    // too old
    let r = report("PowerBook", now - send::MAX_SKEW_S - 60);
    let b = send::SignedReport {
        sig: send::sign(&key, &r).unwrap(),
        cert: cert.clone(),
        report: r,
    };
    assert_eq!(post(&base, &b).await, 403);

    // a self-made certificate with the right name, not from warte's CA
    let k = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut p = rcgen::CertificateParams::new(vec!["PowerBook".into()]).unwrap();
    p.distinguished_name
        .push(rcgen::DnType::CommonName, "PowerBook");
    let forged = p.self_signed(&k).unwrap().pem();
    let r = report("PowerBook", now);
    let b = send::SignedReport {
        sig: send::sign(&k.serialize_pem(), &r).unwrap(),
        cert: forged,
        report: r,
    };
    assert_eq!(post(&base, &b).await, 403);

    assert!(!hosts(&base)
        .await
        .iter()
        .any(|h| h == "PowerBook" || h == "fourpi"));
}

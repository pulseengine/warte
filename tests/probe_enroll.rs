//! Oracles for probe enrollment with a one-time token (DD-PROBE-002).

use warte::probe::enroll::{self, EnrollRequest};

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

fn csr_for(name: &str) -> String {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut p = rcgen::CertificateParams::new(vec![name.to_string()]).unwrap();
    p.distinguished_name.push(rcgen::DnType::CommonName, name);
    p.serialize_request(&key).unwrap().pem().unwrap()
}

async fn post(base: &str, token: &str, host: &str, csr: String) -> (u16, String) {
    let r = reqwest::Client::new()
        .post(format!("{base}/api/probe/enroll"))
        .json(&EnrollRequest {
            token: token.into(),
            host: host.into(),
            csr,
        })
        .send()
        .await
        .unwrap();
    (r.status().as_u16(), r.text().await.unwrap())
}

fn subject_cn(pem: &str) -> String {
    let (_, p) = x509_parser::pem::parse_x509_pem(pem.as_bytes()).unwrap();
    let cert = p.parse_x509().unwrap();
    let cn = cert
        .subject()
        .iter_common_name()
        .next()
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    cn
}

// rivet: verifies REQ-SEC-001
#[tokio::test]
async fn a_machine_enrolls_with_its_own_key_and_the_tokens_host() {
    let (dir, base, db) = serve().await;
    let token = enroll::mint(&db, "wohl", warte::hub::now()).await.unwrap();
    let state = dir.path().join("probe");
    enroll::enroll(&base, &token, "wohl", &state, None)
        .await
        .unwrap();

    use std::os::unix::fs::PermissionsExt;
    let key = std::fs::read_to_string(state.join("probe-key.pem")).unwrap();
    assert!(key.contains("PRIVATE KEY"));
    assert_eq!(
        std::fs::metadata(state.join("probe-key.pem"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let cert = std::fs::read_to_string(state.join("probe-cert.pem")).unwrap();
    assert_eq!(subject_cn(&cert), "wohl");
    // it chains to warte's CA: the issuer is the CA's subject
    let ca = std::fs::read_to_string(state.join("ca.pem")).unwrap();
    let (_, cp) = x509_parser::pem::parse_x509_pem(ca.as_bytes()).unwrap();
    let (_, pp) = x509_parser::pem::parse_x509_pem(cert.as_bytes()).unwrap();
    assert_eq!(
        pp.parse_x509().unwrap().issuer(),
        cp.parse_x509().unwrap().subject()
    );

    // the token worked once
    let (code, body) = post(&base, &token, "wohl", csr_for("wohl")).await;
    assert_eq!((code, body.as_str()), (403, "enrollment refused"));
}

// rivet: verifies REQ-SEC-001
#[tokio::test]
async fn a_token_is_bound_to_its_host_its_time_and_one_use() {
    let (_d, base, db) = serve().await;
    let now = warte::hub::now();
    // another host's token
    let t = enroll::mint(&db, "gale", now).await.unwrap();
    assert_eq!(post(&base, &t, "wohl", csr_for("wohl")).await.0, 403);
    // expired
    let old = enroll::mint(&db, "wohl", now - enroll::TOKEN_TTL_S - 1)
        .await
        .unwrap();
    assert_eq!(post(&base, &old, "wohl", csr_for("wohl")).await.0, 403);
    // made up
    assert_eq!(post(&base, "wpe_00", "wohl", csr_for("wohl")).await.0, 403);
    // a CSR cannot choose its identity: the certificate names the token's host
    let t2 = enroll::mint(&db, "wohl", now).await.unwrap();
    let (code, body) = post(&base, &t2, "wohl", csr_for("tower-pi")).await;
    assert_eq!(code, 200, "{body}");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(subject_cn(resp["cert"].as_str().unwrap()), "wohl");
    // and nothing secret comes back
    assert!(!body.contains("PRIVATE KEY"));
}

// rivet: verifies REQ-SEC-001
#[tokio::test]
async fn the_exemption_is_enrollment_only() {
    let (_d, base, _db) = serve().await;
    let c = reqwest::Client::new();
    for p in [
        "/api/agents/register",
        "/api/access/grant",
        "/api/probe/enroll/x",
    ] {
        let r = c
            .post(format!("{base}{p}"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401, "{p} without a bearer token");
    }
    // a malformed request is refused, not served
    let r = c
        .post(format!("{base}/api/probe/enroll"))
        .body("not json")
        .header("content-type", "application/json")
        .send()
        .await
        .unwrap();
    assert!(r.status().is_client_error());
}

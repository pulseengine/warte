//! Oracles for probe certificate renewal (REQ-PROBE-007): an enrolled probe
//! renews with its current key and no token; another key, an expired
//! certificate and a revoked host are refused; a revoked host enrolls again
//! with a new token.

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

async fn enrolled(base: &str, db: &warte::db::Database, host: &str, dir: &std::path::Path) {
    let token = enroll::mint(db, host, warte::hub::now()).await.unwrap();
    enroll::enroll(base, &token, host, dir, None).await.unwrap();
}

fn read(dir: &std::path::Path, f: &str) -> String {
    std::fs::read_to_string(dir.join(f)).unwrap()
}

fn der(pem: &str) -> Vec<u8> {
    x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .unwrap()
        .1
        .contents
}

fn validity(cert_pem: &str) -> (i64, i64) {
    let d = der(cert_pem);
    let (_, c) = x509_parser::parse_x509_certificate(&d).unwrap();
    (
        c.validity().not_before.timestamp(),
        c.validity().not_after.timestamp(),
    )
}

fn not_after(cert_pem: &str) -> i64 {
    validity(cert_pem).1
}

async fn report(base: &str, dir: &std::path::Path, host: &str, at: i64) -> u16 {
    let r = serde_json::json!({
        "probe_version": "t", "host": host, "taken_at": at, "uptime_s": 1,
        "os": {"name": "Linux", "kernel": "6", "arch": "arm64"},
        "cpu": {"ncpu": 4, "load": [0.0, 0.0, 0.0]},
        "memory": {"total_mb": 1, "avail_mb": 1, "swap_total_mb": 0, "swap_used_mb": 0},
        "disks": [], "tools": [], "sessions": []
    })
    .to_string();
    let body = send::SignedReport {
        sig: send::sign(&read(dir, "probe-key.pem"), &r).unwrap(),
        cert: read(dir, "probe-cert.pem"),
        report: r,
    };
    reqwest::Client::new()
        .post(format!("{base}/api/probe/report"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn post_renew(base: &str, body: &send::RenewRequest) -> u16 {
    reqwest::Client::new()
        .post(format!("{base}/api/probe/renew"))
        .json(body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

/// A renewal request signed with `key_pem` for a CSR of `csr_key`.
fn renewal(
    cert: &str,
    key_pem: &str,
    csr_key: &rcgen::KeyPair,
    host: &str,
    at: i64,
) -> send::RenewRequest {
    let csr = enroll::csr_pem(host, csr_key).unwrap();
    send::RenewRequest {
        sig: hex::encode(send::sign_bytes(key_pem, &send::renew_message(at, &csr)).unwrap()),
        cert: cert.into(),
        csr,
        at,
    }
}

// rivet: verifies REQ-PROBE-007
#[tokio::test]
async fn a_probe_renews_with_its_key_and_no_token() {
    let (dir, base, db) = serve().await;
    let p = dir.path().join("p");
    enrolled(&base, &db, "gale", &p).await;
    let (key, old) = (read(&p, "probe-key.pem"), read(&p, "probe-cert.pem"));
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    send::renew(&reqwest::Client::new(), &base, &p, warte::hub::now())
        .await
        .unwrap();
    let new = read(&p, "probe-cert.pem");
    assert_ne!(new, old, "a new certificate");
    assert!(not_after(&new) > not_after(&old), "with a later end");
    assert_eq!(read(&p, "probe-key.pem"), key, "for the same key");
    assert!(!p.join("probe-cert.pem.tmp").exists());
    // reports under the renewed certificate land
    assert_eq!(report(&base, &p, "gale", warte::hub::now()).await, 204);
}

// rivet: verifies REQ-PROBE-007
#[tokio::test]
async fn another_key_a_stale_or_forged_request_is_refused() {
    let (dir, base, db) = serve().await;
    let p = dir.path().join("p");
    enrolled(&base, &db, "wohl", &p).await;
    let (key, cert) = (read(&p, "probe-key.pem"), read(&p, "probe-cert.pem"));
    let own = rcgen::KeyPair::from_pem(&key).unwrap();
    let now = warte::hub::now();
    assert_eq!(
        post_renew(&base, &renewal(&cert, &key, &own, "wohl", now)).await,
        200
    );

    // a CSR for a different key, even signed by the enrolled one
    let other = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    assert_eq!(
        post_renew(&base, &renewal(&cert, &key, &other, "wohl", now)).await,
        403
    );
    // signed by a key that is not the certificate's
    assert_eq!(
        post_renew(
            &base,
            &renewal(&cert, &other.serialize_pem(), &own, "wohl", now)
        )
        .await,
        403
    );
    // too old: a captured request does not work later
    assert_eq!(
        post_renew(
            &base,
            &renewal(&cert, &key, &own, "wohl", now - send::MAX_SKEW_S - 5)
        )
        .await,
        403
    );
    // a certificate from another CA
    let (dir2, base2, db2) = serve().await;
    let q = dir2.path().join("q");
    enrolled(&base2, &db2, "wohl", &q).await;
    let foreign = renewal(
        &read(&q, "probe-cert.pem"),
        &read(&q, "probe-key.pem"),
        &rcgen::KeyPair::from_pem(&read(&q, "probe-key.pem")).unwrap(),
        "wohl",
        now,
    );
    assert_eq!(post_renew(&base, &foreign).await, 403);
}

// rivet: verifies REQ-PROBE-007
#[tokio::test]
async fn an_expired_certificate_needs_a_new_enrollment() {
    let (dir, base, db) = serve().await;
    let p = dir.path().join("p");
    enrolled(&base, &db, "netcup", &p).await;
    let (key, cert) = (read(&p, "probe-key.pem"), read(&p, "probe-cert.pem"));
    let own = rcgen::KeyPair::from_pem(&key).unwrap();
    let ca = std::fs::read_to_string(dir.path().join("ca.pem")).unwrap();
    let ca_der = der(&ca);
    let later = not_after(&cert) + 1;
    let req = renewal(&cert, &key, &own, "netcup", later);
    let e = send::verify_renew(&ca_der, &req, later).unwrap_err();
    assert!(e.to_string().contains("validity"), "{e}");
    // the same request while the certificate is valid passes
    let now = warte::hub::now();
    assert!(send::verify_renew(&ca_der, &renewal(&cert, &key, &own, "netcup", now), now).is_ok());
}

// rivet: verifies REQ-PROBE-007
#[tokio::test]
async fn a_revoked_host_is_refused_until_it_enrolls_again() {
    let (dir, base, db) = serve().await;
    let p = dir.path().join("p");
    enrolled(&base, &db, "PowerBook", &p).await;
    assert_eq!(report(&base, &p, "PowerBook", warte::hub::now()).await, 204);
    enroll::revoke(&db, "PowerBook", warte::hub::now())
        .await
        .unwrap();
    assert_eq!(
        report(&base, &p, "PowerBook", warte::hub::now() + 1).await,
        403
    );
    let e = send::renew(&reqwest::Client::new(), &base, &p, warte::hub::now())
        .await
        .unwrap_err();
    assert!(e.to_string().contains("403"), "{e}");
    // a new token, a new key: accepted again (certificate times are whole
    // seconds; an enrollment in the revocation's own second counts as before it)
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let q = dir.path().join("q");
    enrolled(&base, &db, "PowerBook", &q).await;
    assert_eq!(
        report(&base, &q, "PowerBook", warte::hub::now() + 2).await,
        204
    );
    // the old certificate stays refused
    assert_eq!(
        report(&base, &p, "PowerBook", warte::hub::now() + 3).await,
        403
    );
}

// rivet: verifies REQ-PROBE-007
#[tokio::test]
async fn renewal_is_due_at_two_thirds_of_the_life() {
    let (dir, base, db) = serve().await;
    let p = dir.path().join("p");
    enrolled(&base, &db, "mini", &p).await;
    let cert = read(&p, "probe-cert.pem");
    let (nb, na) = validity(&cert);
    let due = nb + (na - nb) * 2 / 3;
    assert!(!send::renewal_due(&cert, due - 1).unwrap());
    assert!(send::renewal_due(&cert, due).unwrap());
    assert!(
        !send::renewal_due(&cert, warte::hub::now()).unwrap(),
        "not right after enrolling"
    );
}

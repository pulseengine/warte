//! Fleet agent transport v2: QUIC + mutual TLS + relay-sec (REQ-SEC-001, #41),
//! exercised end to end with a real QUIC client playing the fleet agent.

use relay_sec::frame::SecurityChannel;
use relay_sec::session::{Config as SaConfig, Initiator};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use warte::ca::CertificateAuthority;
use warte::hub::model::{Machine, Snapshot};
use warte::hub::quic::{self, Listener, SaStore, ALPN, CH_DATAGRAM, CH_LEDGER};
use warte::hub::telemetry::{encode_snapshot, SeqCounters};
use warte::hub::{now, Hub};

struct Station {
    _dir: tempfile::TempDir,
    hub: Hub,
    db: warte::db::Database,
    ca: Arc<CertificateAuthority>,
    sas: SaStore,
    addr: std::net::SocketAddr,
}

async fn station() -> Station {
    let dir = tempfile::tempdir().unwrap();
    let ca = Arc::new(
        CertificateAuthority::load_or_create(
            &dir.path().join("ca.pem"),
            &dir.path().join("ca-key.pem"),
            24,
        )
        .unwrap(),
    );
    let db = warte::db::Database::connect(&dir.path().join("state.db"))
        .await
        .unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "acme").unwrap();
    let (cert, key) = ca.issue_server_cert(&["localhost".into()], 1).unwrap();
    let cfg = quic::server_config(ca.ca_cert_der().to_vec(), cert, key).unwrap();
    let sas = SaStore::new(db.pool().clone());
    let l = Arc::new(Listener {
        hub: hub.clone(),
        db: db.clone(),
        sas: sas.clone(),
        epoch: quic::EpochFile::new(dir.path().join("quic-epoch")),
        counts: hub.quic.clone(),
    });
    let addr = quic::spawn("127.0.0.1:0".parse().unwrap(), cfg, l).unwrap();
    Station {
        _dir: dir,
        hub,
        db,
        ca,
        sas,
        addr,
    }
}

/// A client certificate for `host` from `ca` and its PKCS#8 key.
async fn client_cert(ca: &CertificateAuthority, host: &str) -> (Vec<u8>, Vec<u8>) {
    let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let csr = warte::probe::enroll::csr_pem(host, &kp).unwrap();
    let pem = ca.sign_probe_csr(&csr, host, 1).await.unwrap();
    let der = CertificateDer::from_pem_slice(pem.as_bytes()).unwrap();
    (der.to_vec(), kp.serialize_der())
}

async fn connect(s: &Station, cert: (Vec<u8>, Vec<u8>)) -> Result<quinn::Connection, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(s.ca.ca_cert_der().to_vec()))
        .unwrap();
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots)
        .with_client_auth_cert(
            vec![CertificateDer::from(cert.0)],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.1)),
        )
        .unwrap();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let qc = quinn::crypto::rustls::QuicClientConfig::try_from(tls).unwrap();
    let mut ep = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
    ep.set_default_client_config(quinn::ClientConfig::new(Arc::new(qc)));
    ep.connect(s.addr, "localhost")
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())
}

async fn msg(w: &mut quinn::SendStream, b: &[u8]) {
    w.write_all(&(b.len() as u16).to_be_bytes()).await.unwrap();
    w.write_all(b).await.unwrap();
}

async fn read(r: &mut quinn::RecvStream) -> Option<Vec<u8>> {
    let mut l = [0u8; 2];
    r.read_exact(&mut l).await.ok()?;
    let mut b = vec![0u8; usize::from(u16::from_be_bytes(l))];
    r.read_exact(&mut b).await.ok()?;
    Some(b)
}

/// The agent's side of the handshake; the session's MAC key, or None.
async fn handshake(
    conn: &quinn::Connection,
    spi: u16,
    k_id: [u8; 16],
    epoch: u64,
) -> Option<[u8; 16]> {
    let (mut tx, mut rx) = conn.open_bi().await.ok()?;
    let cfg = SaConfig {
        spi,
        channel_id: 0,
        k_id,
    };
    let (init, hello) = Initiator::start(cfg, rand::random(), epoch);
    msg(&mut tx, &quic::hello_bytes(&hello)).await;
    let resp = quic::response_from(&read(&mut rx).await?)?;
    let (keys, confirm) = init.finish(&resp).ok()?;
    msg(&mut tx, &confirm.tag).await;
    let _ = tx.finish();
    Some(keys.mac)
}

fn packets(host: &str, seq: &mut SeqCounters) -> Vec<Vec<u8>> {
    let s = Snapshot {
        agent_version: "0.1.0".into(),
        host: host.into(),
        taken_at: now(),
        machine: Machine {
            ncpu: 4,
            ..Default::default()
        },
        ..Default::default()
    };
    encode_snapshot(&s, seq)
}

fn seal(ch: &mut SecurityChannel, p: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; p.len() + 64];
    let n = ch.wrap(p, &mut out).unwrap();
    out.truncate(n);
    out
}

fn host_of(hub: &Hub) -> Option<String> {
    hub.snapshot().map(|(s, _)| s.host)
}

async fn until<F: Fn() -> bool>(f: F) -> bool {
    for _ in 0..100 {
        if f() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    false
}

fn key(hex_key: &str) -> [u8; 16] {
    let mut k = [0u8; 16];
    hex::decode_to_slice(hex_key, &mut k).unwrap();
    k
}

// rivet: verifies REQ-SEC-001
#[tokio::test]
async fn an_enrolled_agent_s_sealed_packets_reach_the_hub_over_quic() {
    let s = station().await;
    let (spi, k) = s.sas.issue("agent1").await.unwrap();
    let conn = connect(&s, client_cert(&s.ca, "agent1").await)
        .await
        .unwrap();
    let mac = handshake(&conn, spi, key(&k), 1).await.expect("handshake");
    let mut seq = SeqCounters::default();
    let mut ch = SecurityChannel::new(spi, CH_DATAGRAM, mac);
    let frames: Vec<Vec<u8>> = packets("via-quic", &mut seq)
        .iter()
        .map(|p| {
            assert!(p.len() <= quic::MAX_DATAGRAM_PACKET);
            seal(&mut ch, p)
        })
        .collect();
    for f in &frames {
        conn.send_datagram(f.clone().into()).unwrap();
    }
    assert!(until(|| host_of(&s.hub).as_deref() == Some("via-quic")).await);
    let c = &s.hub.quic;
    assert_eq!(c.connected.load(Ordering::Relaxed), 1);

    // planted: the same sealed frames again, and a tampered one
    for f in &frames {
        conn.send_datagram(f.clone().into()).unwrap();
    }
    let mut bad = seal(&mut ch, &packets("tampered", &mut seq)[0]);
    let mid = bad.len() / 2;
    bad[mid] ^= 1;
    conn.send_datagram(bad.into()).unwrap();
    assert!(until(|| c.replayed.load(Ordering::Relaxed) >= frames.len() as u64).await);
    assert!(until(|| c.bad_tag.load(Ordering::Relaxed) >= 1).await);
    assert_eq!(host_of(&s.hub).as_deref(), Some("via-quic"));

    // the ledger stream (channel 2) carries sealed frames too
    let mut ledger = SecurityChannel::new(spi, CH_LEDGER, mac);
    let mut w = conn.open_uni().await.unwrap();
    for p in packets("via-stream", &mut seq) {
        msg(&mut w, &seal(&mut ledger, &p)).await;
    }
    let _ = w.finish();
    assert!(until(|| host_of(&s.hub).as_deref() == Some("via-stream")).await);
    // the counts reach Health, Ground station internals
    let health = warte::dashboard::fleet::health_view(Some(&s.hub));
    assert!(health.contains("Agents connected over QUIC"), "{health}");
    assert!(health.contains("Refused: replayed (replayed)"), "{health}");
}

// rivet: verifies REQ-SEC-001
#[tokio::test]
async fn a_replayed_epoch_a_wrong_key_or_an_unenrolled_agent_gets_no_session() {
    let s = station().await;
    let (spi, k) = s.sas.issue("agent1").await.unwrap();
    let cert = client_cert(&s.ca, "agent1").await;
    let c1 = connect(&s, cert.clone()).await.unwrap();
    assert!(handshake(&c1, spi, key(&k), 5).await.is_some());
    // the same epoch again: refused, counted replayed
    let c2 = connect(&s, cert.clone()).await.unwrap();
    assert!(handshake(&c2, spi, key(&k), 5).await.is_none());
    assert!(until(|| s.hub.quic.replayed.load(Ordering::Relaxed) >= 1).await);
    // a key other than the enrolled k_id: refused, counted bad_tag
    let c3 = connect(&s, cert.clone()).await.unwrap();
    assert!(handshake(&c3, spi, [9; 16], 6).await.is_none());
    assert!(until(|| s.hub.quic.bad_tag.load(Ordering::Relaxed) >= 1).await);
    // a certificate whose host has no security association
    let c4 = connect(&s, client_cert(&s.ca, "stranger").await)
        .await
        .unwrap();
    assert!(handshake(&c4, spi, key(&k), 7).await.is_none());
    assert!(until(|| s.hub.quic.unknown_spi.load(Ordering::Relaxed) >= 1).await);
    // re-enrolment replaces the SA: the old k_id no longer opens a session
    let (spi2, k2) = s.sas.issue("agent1").await.unwrap();
    let c5 = connect(&s, cert.clone()).await.unwrap();
    assert!(handshake(&c5, spi, key(&k), 8).await.is_none());
    let c6 = connect(&s, cert.clone()).await.unwrap();
    assert!(
        handshake(&c6, spi2, key(&k2), 1).await.is_some(),
        "a new SA starts its epochs again"
    );
}

// rivet: verifies REQ-SEC-001
#[tokio::test]
async fn a_foreign_or_revoked_certificate_is_refused_and_v1_can_be_switched_off() {
    let s = station().await;
    let (spi, k) = s.sas.issue("agent1").await.unwrap();
    // a certificate from another CA never completes TLS
    let other = tempfile::tempdir().unwrap();
    let foreign = CertificateAuthority::load_or_create(
        &other.path().join("ca.pem"),
        &other.path().join("ca-key.pem"),
        24,
    )
    .unwrap();
    let conn = connect(&s, client_cert(&foreign, "agent1").await).await;
    let refused = match conn {
        Err(_) => true,
        Ok(c) => handshake(&c, spi, key(&k), 1).await.is_none(),
    };
    assert!(refused, "a foreign certificate got a session");
    // revoked after the certificate was issued: refused
    let cert = client_cert(&s.ca, "agent1").await;
    warte::probe::enroll::revoke(&s.db, "agent1", now() + 1)
        .await
        .unwrap();
    let c = connect(&s, cert).await.unwrap();
    assert!(handshake(&c, spi, key(&k), 2).await.is_none());
    assert!(until(|| s.hub.quic.tls_identity.load(Ordering::Relaxed) >= 1).await);

    // require_quic drops wire v1 over UDP and counts it
    let ci: std::net::IpAddr = "192.0.2.201".parse().unwrap();
    let mut seq = SeqCounters::default();
    for p in packets("v1-udp", &mut seq) {
        s.hub.ingest(&p, ci, &[ci]);
    }
    assert_eq!(host_of(&s.hub).as_deref(), Some("v1-udp"));
    s.hub.set_require_quic(true);
    for p in packets("v1-late", &mut seq) {
        s.hub.ingest(&p, ci, &[ci]);
    }
    assert_eq!(
        host_of(&s.hub).as_deref(),
        Some("v1-udp"),
        "a v1 packet got in"
    );
}

// rivet: verifies REQ-SEC-003
#[tokio::test]
async fn the_hub_epoch_never_falls_below_the_clock_after_a_restore() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("quic-epoch");
    // planted: a quic-epoch restored from an old backup
    std::fs::write(&path, "5").unwrap();
    let e = quic::EpochFile::new(&path);
    let before = warte::hub::now_ms() as u64;
    let a = e.next().await.unwrap();
    assert!(a >= before, "{a} < {before}");
    let b = e.next().await.unwrap();
    assert!(b > a, "the epoch went up");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), b.to_string());
}

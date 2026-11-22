//! Fleet agent transport v2 (REQ-SEC-001, DD-TRANSPORT-001, #41): QUIC with
//! mutual TLS 1.3, and every wire v1 packet sealed with relay-sec under a
//! per-connection session. The contract is `docs/topics/ground-station.md`,
//! "Contract: fleet agent transport v2".
//!
//! An agent's identity is its client certificate from warte's CA; its
//! security association (SPI and `k_id`) comes from `warte probe enroll`
//! ([`SaStore`]). The first bidirectional stream carries relay-sec's
//! handshake; then DATAGRAM frames (channel 1) and unidirectional streams
//! (channel 2) carry sealed packets, which go to the v1 assembler unchanged.

use super::Hub;
use relay_sec::frame::SecurityChannel;
use relay_sec::session::{Config as SaConfig, Confirm, Hello, Responder};
use sqlx::SqlitePool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub const ALPN: &[u8] = b"warte-telemetry/1";
pub const HELLO_LEN: usize = 56;
pub const CONFIRM_LEN: usize = 16;
pub const CH_DATAGRAM: u8 = 1;
pub const CH_LEDGER: u8 = 2;
/// The CCSDS packet inside a DATAGRAM frame (1200 − 11 − 16).
pub const MAX_DATAGRAM_PACKET: usize = 1173;
/// Longest sealed frame accepted on a ledger stream.
pub const MAX_STREAM_FRAME: usize = 4096;

/// What the QUIC listener refused, for Health (contract: "What the hub checks").
#[derive(Default, Debug)]
pub struct Counts {
    pub tls_identity: AtomicU64,
    pub unknown_spi: AtomicU64,
    pub bad_tag: AtomicU64,
    pub replayed: AtomicU64,
    pub connected: AtomicU64,
    pub accepted: AtomicU64,
}

impl Counts {
    pub fn snapshot(&self) -> serde_json::Value {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        serde_json::json!({
            "tls_identity": g(&self.tls_identity),
            "unknown_spi": g(&self.unknown_spi),
            "bad_tag": g(&self.bad_tag),
            "replayed": g(&self.replayed),
            "connected": g(&self.connected),
            "accepted": g(&self.accepted),
        })
    }
}

/// One agent's security association.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sa {
    pub host: String,
    pub spi: u16,
    pub k_id: [u8; 16],
    pub last_epoch: u64,
}

/// The security associations, issued at enrollment, in warte's database.
#[derive(Clone)]
pub struct SaStore {
    pool: SqlitePool,
}

fn db(e: sqlx::Error) -> String {
    format!("database error: {e}")
}

impl SaStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// A new SA for `host`, replacing any older one (re-enrolment revokes the
    /// old SPI). Returns the SPI and `k_id` as hex, handed out once.
    pub async fn issue(&self, host: &str) -> Result<(u16, String), String> {
        use rand::RngCore;
        let mut k = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut k);
        let k_hex = hex::encode(k);
        for _ in 0..64 {
            let spi = (rand::random::<u16>() % 0xFFFE) + 1;
            let mut tx = self.pool.begin().await.map_err(db)?;
            sqlx::query("DELETE FROM telemetry_sa WHERE host = ?")
                .bind(host)
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            let r = sqlx::query(
                "INSERT INTO telemetry_sa (host, spi, k_id, last_epoch, created_at) VALUES (?, ?, ?, 0, ?)",
            )
            .bind(host)
            .bind(i64::from(spi))
            .bind(&k_hex)
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&mut *tx)
            .await;
            match r {
                Ok(_) => {
                    tx.commit().await.map_err(db)?;
                    return Ok((spi, k_hex));
                }
                // the SPI is taken: try another
                Err(sqlx::Error::Database(e)) if e.is_unique_violation() => continue,
                Err(e) => return Err(db(e)),
            }
        }
        Err("no free SPI".into())
    }

    pub async fn by_host(&self, host: &str) -> Result<Option<Sa>, String> {
        let row: Option<(String, i64, String, i64)> = sqlx::query_as(
            "SELECT host, spi, k_id, last_epoch FROM telemetry_sa WHERE host = ? COLLATE NOCASE",
        )
        .bind(host)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.and_then(|(host, spi, k, e)| {
            let mut k_id = [0u8; 16];
            hex::decode_to_slice(&k, &mut k_id).ok()?;
            Some(Sa {
                host,
                spi: u16::try_from(spi).ok()?,
                k_id,
                last_epoch: u64::try_from(e).unwrap_or(0),
            })
        }))
    }

    /// Record an accepted initiator epoch; false when it did not go up (a
    /// concurrent or replayed handshake), so the session is refused.
    pub async fn advance_epoch(&self, spi: u16, epoch: u64) -> Result<bool, String> {
        let Ok(e) = i64::try_from(epoch) else {
            return Ok(false);
        };
        let r =
            sqlx::query("UPDATE telemetry_sa SET last_epoch = ? WHERE spi = ? AND last_epoch < ?")
                .bind(e)
                .bind(i64::from(spi))
                .bind(e)
                .execute(&self.pool)
                .await
                .map_err(db)?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn revoke(&self, host: &str) -> Result<bool, String> {
        let r = sqlx::query("DELETE FROM telemetry_sa WHERE host = ? COLLATE NOCASE")
            .bind(host)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(r.rows_affected() > 0)
    }
}

/// The hub's own reboot-persistent responder epoch: a counter in a file that
/// only ever goes up, never below the clock in milliseconds, written and
/// synced before it is used.
pub struct EpochFile {
    path: std::path::PathBuf,
    lock: tokio::sync::Mutex<()>,
}

impl EpochFile {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Default::default(),
        }
    }

    pub async fn next(&self) -> Result<u64, String> {
        let _g = self.lock.lock().await;
        let cur = std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);
        // never below the clock in milliseconds: a quic-epoch restored from an
        // older backup cannot make an epoch repeat (the hub runs NTP)
        let clock = u64::try_from(crate::hub::now_ms()).unwrap_or(0);
        let next = cur.checked_add(1).ok_or("epoch exhausted")?.max(clock);
        let tmp = self.path.with_extension("tmp");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
            f.write_all(next.to_string().as_bytes())
                .and_then(|_| f.sync_all())
                .map_err(|e| e.to_string())?;
        }
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())?;
        Ok(next)
    }
}

/// The wire form of relay-sec's handshake messages (contract: 56/56/16 bytes).
pub fn hello_from(b: &[u8]) -> Option<Hello> {
    if b.len() != HELLO_LEN {
        return None;
    }
    Some(Hello {
        e_pub: b[..32].try_into().ok()?,
        epoch: u64::from_be_bytes(b[32..40].try_into().ok()?),
        tag: b[40..56].try_into().ok()?,
    })
}

pub fn hello_bytes(h: &Hello) -> Vec<u8> {
    [&h.e_pub[..], &h.epoch.to_be_bytes(), &h.tag].concat()
}

pub fn response_bytes(r: &relay_sec::session::Response) -> Vec<u8> {
    [&r.e_pub[..], &r.epoch.to_be_bytes(), &r.tag].concat()
}

pub fn response_from(b: &[u8]) -> Option<relay_sec::session::Response> {
    if b.len() != HELLO_LEN {
        return None;
    }
    Some(relay_sec::session::Response {
        e_pub: b[..32].try_into().ok()?,
        epoch: u64::from_be_bytes(b[32..40].try_into().ok()?),
        tag: b[40..56].try_into().ok()?,
    })
}

async fn read_msg(r: &mut quinn::RecvStream, max: usize) -> Option<Vec<u8>> {
    let mut len = [0u8; 2];
    r.read_exact(&mut len).await.ok()?;
    let n = usize::from(u16::from_be_bytes(len));
    if n > max {
        return None;
    }
    let mut b = vec![0u8; n];
    r.read_exact(&mut b).await.ok()?;
    Some(b)
}

async fn write_msg(w: &mut quinn::SendStream, b: &[u8]) -> Option<()> {
    let n = u16::try_from(b.len()).ok()?;
    w.write_all(&n.to_be_bytes()).await.ok()?;
    w.write_all(b).await.ok()
}

/// The common name and not-before (unix seconds) of the peer's certificate.
fn peer_identity(conn: &quinn::Connection) -> Option<(String, i64)> {
    use x509_parser::prelude::*;
    let certs = conn
        .peer_identity()?
        .downcast::<Vec<rustls::pki_types::CertificateDer<'static>>>()
        .ok()?;
    let (_, c) = X509Certificate::from_der(certs.first()?.as_ref()).ok()?;
    let cn = c
        .subject()
        .iter_common_name()
        .next()?
        .as_str()
        .ok()?
        .to_string();
    Some((cn, c.validity().not_before.timestamp()))
}

/// What a connection needs from the server around it.
pub struct Listener {
    pub hub: Hub,
    pub db: crate::db::Database,
    pub sas: SaStore,
    pub epoch: EpochFile,
    pub counts: Arc<Counts>,
}

impl Listener {
    /// Serve one connection until it closes.
    pub async fn connection(self: Arc<Self>, conn: quinn::Connection) {
        let c = &self.counts;
        let Some((host, issued_at)) = peer_identity(&conn) else {
            c.tls_identity.fetch_add(1, Ordering::Relaxed);
            conn.close(1u32.into(), b"identity");
            return;
        };
        match crate::probe::enroll::revoked(&self.db, &host, issued_at).await {
            Ok(false) => {}
            _ => {
                c.tls_identity.fetch_add(1, Ordering::Relaxed);
                conn.close(1u32.into(), b"revoked");
                return;
            }
        }
        let Ok(Some(sa)) = self.sas.by_host(&host).await else {
            c.unknown_spi.fetch_add(1, Ordering::Relaxed);
            conn.close(2u32.into(), b"no security association");
            return;
        };
        let Some(keys) = self.handshake(&conn, &sa).await else {
            conn.close(3u32.into(), b"handshake");
            return;
        };
        c.connected.fetch_add(1, Ordering::Relaxed);
        let datagrams = {
            let me = self.clone();
            let conn = conn.clone();
            let mut ch = SecurityChannel::new(sa.spi, CH_DATAGRAM, keys.mac);
            tokio::spawn(async move {
                while let Ok(d) = conn.read_datagram().await {
                    me.frame(&mut ch, &d, MAX_DATAGRAM_PACKET);
                }
            })
        };
        let me = self.clone();
        let streams = {
            let conn = conn.clone();
            tokio::spawn(async move {
                // channel 2 keeps one window across the connection's streams
                let ch = Arc::new(tokio::sync::Mutex::new(SecurityChannel::new(
                    sa.spi, CH_LEDGER, keys.mac,
                )));
                while let Ok(mut r) = conn.accept_uni().await {
                    let me = me.clone();
                    let ch = ch.clone();
                    tokio::spawn(async move {
                        while let Some(f) = read_msg(&mut r, MAX_STREAM_FRAME).await {
                            me.frame(&mut *ch.lock().await, &f, usize::MAX);
                        }
                    });
                }
            })
        };
        let _ = conn.closed().await;
        datagrams.abort();
        streams.abort();
        c.connected.fetch_sub(1, Ordering::Relaxed);
    }

    async fn handshake(
        &self,
        conn: &quinn::Connection,
        sa: &Sa,
    ) -> Option<relay_sec::session::SessionKeys> {
        let c = &self.counts;
        let (mut tx, mut rx) = conn.accept_bi().await.ok()?;
        let hello = read_msg(&mut rx, HELLO_LEN)
            .await
            .and_then(|b| hello_from(&b));
        let Some(hello) = hello else {
            c.bad_tag.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        let cfg = SaConfig {
            spi: sa.spi,
            channel_id: 0,
            k_id: sa.k_id,
        };
        let e_priv: [u8; 32] = rand::random();
        let epoch_r = self.epoch.next().await.ok()?;
        let (resp, response) = match Responder::receive(cfg, &hello, e_priv, epoch_r, sa.last_epoch)
        {
            Ok(x) => x,
            Err(relay_sec::session::HandshakeError::EpochRegression) => {
                c.replayed.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            Err(_) => {
                c.bad_tag.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };
        write_msg(&mut tx, &response_bytes(&response)).await?;
        let confirm = read_msg(&mut rx, CONFIRM_LEN).await.and_then(|b| {
            Some(Confirm {
                tag: b.try_into().ok()?,
            })
        });
        let Some(keys) = confirm.and_then(|cf| resp.finish(&cf).ok()) else {
            c.bad_tag.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        // the epoch is recorded only once the session is confirmed, and only
        // if it still goes up (a concurrent handshake with the same epoch loses)
        if !self.sas.advance_epoch(sa.spi, hello.epoch).await.ok()? {
            c.replayed.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let _ = tx.finish();
        Some(keys)
    }

    /// One sealed frame: verified, then its CCSDS packet goes to the assembler.
    fn frame(&self, ch: &mut SecurityChannel, f: &[u8], max_packet: usize) {
        use relay_sec::frame::VerifyError;
        match ch.verify(f) {
            Ok(p) if p.len() <= max_packet => {
                self.counts.accepted.fetch_add(1, Ordering::Relaxed);
                self.hub.ingest_trusted(p);
            }
            Ok(_) => {
                self.counts.bad_tag.fetch_add(1, Ordering::Relaxed);
            }
            Err(VerifyError::Replay) => {
                self.counts.replayed.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.counts.bad_tag.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// The QUIC server configuration: warte's CA as the only client root, the
/// hub's server certificate, TLS 1.3 only, ALPN `warte-telemetry/1`.
pub fn server_config(
    ca_der: Vec<u8>,
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
) -> anyhow::Result<quinn::ServerConfig> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(ca_der))?;
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        provider.clone(),
    )
    .build()?;
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![CertificateDer::from(cert_der)],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)),
        )?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls)?;
    Ok(quinn::ServerConfig::with_crypto(Arc::new(crypto)))
}

/// Bind the QUIC listener and serve connections on the runtime.
pub fn spawn(
    addr: std::net::SocketAddr,
    cfg: quinn::ServerConfig,
    l: Arc<Listener>,
) -> anyhow::Result<std::net::SocketAddr> {
    let ep = quinn::Endpoint::server(cfg, addr)?;
    let local = ep.local_addr()?;
    tokio::spawn(async move {
        while let Some(inc) = ep.accept().await {
            let l = l.clone();
            tokio::spawn(async move {
                match inc.await {
                    Ok(conn) => l.connection(conn).await,
                    Err(_) => {
                        l.counts.tls_identity.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
    });
    Ok(local)
}

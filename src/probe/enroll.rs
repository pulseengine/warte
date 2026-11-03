//! Probe enrollment with a one-time token (DD-PROBE-002).
//!
//! `warte probe token --host <h>` (ground station) mints a token: 15 minutes,
//! single use, bound to the host, stored as SHA-256 only. `warte probe enroll
//! <token> --ground <url>` (machine) makes a key locally, sends only a CSR to
//! POST /api/probe/enroll; warte's CA signs a 30-day client certificate whose
//! name is the token's host, never the CSR's. Key 0600, certificate and CA in
//! the probe's state dir.

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::Database;

/// How long a minted token stays valid, in seconds (15 minutes).
pub const TOKEN_TTL_S: i64 = 15 * 60;
/// Lifetime of the issued probe certificate, in days.
pub const CERT_DAYS: u32 = 30;

/// Request body of POST /api/probe/enroll. The CSR is the only key material
/// that ever leaves the machine.
#[derive(Debug, Serialize, Deserialize)]
pub struct EnrollRequest {
    pub host: String,
    pub token: String,
    pub csr: String,
}

/// Response body of POST /api/probe/enroll.
#[derive(Debug, Serialize, Deserialize)]
pub struct EnrollResponse {
    pub cert: String,
    pub ca: String,
}

/// A host name we are willing to put in a certificate: DNS hostname shape,
/// at most 253 chars. Anything else is refused before it reaches the CA.
pub fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

fn token_sha256(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Mint a one-time token for `host`, valid for [`TOKEN_TTL_S`] from `now`.
/// Returns the raw token; only its SHA-256 is stored.
pub async fn mint(db: &Database, host: &str, now: i64) -> anyhow::Result<String> {
    if !valid_host(host) {
        return Err(anyhow!("invalid host name: {host:?}"));
    }
    let mut raw = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
    let token = hex::encode(raw);
    let expires_at = now + TOKEN_TTL_S;
    sqlx::query(
        "INSERT INTO probe_tokens (token_sha256, host, expires_at, used) VALUES (?, ?, ?, 0)",
    )
    .bind(token_sha256(&token))
    .bind(host)
    .bind(expires_at)
    .execute(db.pool())
    .await
    .context("storing probe token")?;
    Ok(token)
}

/// Consume a token: true iff it exists, is for `host`, is unused and unexpired
/// at `now`; marks it used. Single-use is enforced by the UPDATE's WHERE clause.
pub async fn consume(db: &Database, token: &str, host: &str, now: i64) -> anyhow::Result<bool> {
    let res = sqlx::query(
        "UPDATE probe_tokens SET used = 1 WHERE token_sha256 = ? AND host = ? AND used = 0 AND expires_at > ?",
    )
    .bind(token_sha256(token))
    .bind(host)
    .bind(now)
    .execute(db.pool())
    .await
    .context("consuming probe token")?;
    Ok(res.rows_affected() == 1)
}

/// A CSR for `host` with `key_pair`, as enrollment and renewal send it.
pub fn csr_pem(host: &str, key_pair: &rcgen::KeyPair) -> anyhow::Result<String> {
    let mut csr_params =
        rcgen::CertificateParams::new(vec![host.to_string()]).context("building CSR params")?;
    csr_params.distinguished_name = rcgen::DistinguishedName::new();
    csr_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, host);
    csr_params
        .distinguished_name
        .push(rcgen::DnType::OrganizationName, "PulseEngine Probe");
    csr_params
        .serialize_request(key_pair)
        .context("serializing CSR")?
        .pem()
        .context("serializing CSR to PEM")
}

/// Revoke `host` (REQ-PROBE-007): every certificate it holds now is refused
/// for reports and renewal; it needs a new token.
pub async fn revoke(db: &Database, host: &str, now: i64) -> anyhow::Result<()> {
    if !valid_host(host) {
        return Err(anyhow!("invalid host name: {host:?}"));
    }
    sqlx::query(
        "INSERT INTO probe_revocations (host, revoked_at) VALUES (lower(?), ?) \
         ON CONFLICT(host) DO UPDATE SET revoked_at = excluded.revoked_at",
    )
    .bind(host)
    .bind(now)
    .execute(db.pool())
    .await
    .context("storing revocation")?;
    Ok(())
}

/// True if `host` was revoked after a certificate issued at `issued_at`.
/// Certificates are back-dated 5 minutes, so one issued right after a
/// revocation still counts as newer.
pub async fn revoked(db: &Database, host: &str, issued_at: i64) -> anyhow::Result<bool> {
    let r: Option<(i64,)> =
        sqlx::query_as("SELECT revoked_at FROM probe_revocations WHERE host = lower(?)")
            .bind(host)
            .fetch_optional(db.pool())
            .await
            .context("reading revocations")?;
    Ok(r.is_some_and(|(at,)| issued_at + 5 * 60 <= at))
}

/// Client side of enrollment: generate a key locally, send only the CSR, store
/// key (0600), certificate and CA in `state_dir`. If `ca_cert` is given, it is
/// added as a TLS root for the request to `--ground` (a reverse proxy's local
/// CA), not as a pin of the response.
pub async fn enroll(
    ground: &str,
    token: &str,
    host: &str,
    state_dir: &std::path::Path,
    ca_cert: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    if !valid_host(host) {
        return Err(anyhow!("invalid host name: {host:?}"));
    }
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .context("generating probe key")?;
    let csr = csr_pem(host, &key_pair)?;

    let url = format!("{}/api/probe/enroll", ground.trim_end_matches('/'));
    let mut client = reqwest::Client::builder();
    if let Some(ca_path) = ca_cert {
        let pem = std::fs::read(ca_path).context("reading --ca-cert")?;
        let cert = reqwest::Certificate::from_pem(&pem).context("parsing --ca-cert PEM")?;
        client = client.add_root_certificate(cert);
    }
    let resp = client
        .build()
        .context("building HTTP client")?
        .post(&url)
        .json(&EnrollRequest {
            host: host.to_string(),
            token: token.to_string(),
            csr,
        })
        .send()
        .await
        .context("POSTing enrollment request")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("enrollment refused by {ground}: {status} {body}"));
    }
    let enrolled: EnrollResponse = resp.json().await.context("reading enrollment response")?;

    std::fs::create_dir_all(state_dir).context("creating probe state dir")?;
    let key_path = state_dir.join("probe-key.pem");
    let cert_path = state_dir.join("probe-cert.pem");
    let ca_path = state_dir.join("ca.pem");
    use std::os::unix::fs::PermissionsExt;
    // Created 0600: writing first and chmod-ing after left the key readable
    // by other local users for a moment.
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&key_path)
            .context("creating probe key")?;
        f.write_all(key_pair.serialize_pem().as_bytes())
            .context("writing probe key")?;
        // an existing file keeps its old mode under create(); make sure
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
            .context("chmod 0600 on probe key")?;
    }
    std::fs::write(&cert_path, &enrolled.cert).context("writing probe certificate")?;
    std::fs::write(&ca_path, &enrolled.ca).context("writing CA certificate")?;
    Ok(())
}

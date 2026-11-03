//! Signed probe reports (REQ-PROBE-006, minimal): an enrolled machine sends
//! its report to POST /api/probe/report, signed with the key it made at
//! enrollment. The ground station accepts it only if
//!
//! - the certificate was issued by warte's CA and is within its validity,
//! - the certificate's name is the report's host (a machine reports itself
//!   only),
//! - the signature over the exact report bytes verifies with the
//!   certificate's key, and
//! - the report is fresh (taken within [`MAX_SKEW_S`] of now) and newer than
//!   the last one accepted for that host, so a captured report cannot be
//!   replayed.
//!
//! No TLS client certificate is needed, so this works behind a reverse proxy
//! that terminates TLS.

use anyhow::{anyhow, Context};
use ring::signature;
use serde::{Deserialize, Serialize};
use x509_parser::prelude::*;

/// How far a report's `taken_at` may be from the ground station's clock.
pub const MAX_SKEW_S: i64 = 10 * 60;

/// Body of POST /api/probe/report.
#[derive(Debug, Serialize, Deserialize)]
pub struct SignedReport {
    /// The probe certificate (PEM) from enrollment.
    pub cert: String,
    /// The report as JSON, exactly the bytes that were signed.
    pub report: String,
    /// ECDSA P-256 SHA-256 signature (ASN.1 DER), hex.
    pub sig: String,
}

fn pem_body(pem: &str, label: &str) -> anyhow::Result<Vec<u8>> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let s = pem.find(&begin).ok_or_else(|| anyhow!("no {begin}"))? + begin.len();
    let e = pem[s..].find(&end).ok_or_else(|| anyhow!("no {end}"))? + s;
    let b64: String = pem[s..e].chars().filter(|c| !c.is_whitespace()).collect();
    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.decode(b64)?)
}

/// Sign `report_json` with the PKCS#8 PEM key from enrollment.
pub fn sign(key_pem: &str, report_json: &str) -> anyhow::Result<String> {
    let der = pem_body(key_pem, "PRIVATE KEY")?;
    let rng = ring::rand::SystemRandom::new();
    let kp =
        signature::EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P256_SHA256_ASN1_SIGNING, &der, &rng)
            .map_err(|e| anyhow!("probe key unusable: {e}"))?;
    let sig = kp
        .sign(&rng, report_json.as_bytes())
        .map_err(|e| anyhow!("signing failed: {e}"))?;
    Ok(hex::encode(sig.as_ref()))
}

/// What the ground station verified: the report and its host.
#[derive(Debug)]
pub struct Verified {
    pub report: super::Report,
}

/// Verify a signed report against warte's CA (DER). `now` is unix seconds.
/// The caller still checks that `taken_at` is newer than the last accepted
/// report for the host.
pub fn verify(ca_der: &[u8], body: &SignedReport, now: i64) -> anyhow::Result<Verified> {
    let (_, ca) = X509Certificate::from_der(ca_der).context("CA certificate")?;
    let cert_der = pem_body(&body.cert, "CERTIFICATE")?;
    let (_, cert) = X509Certificate::from_der(&cert_der).context("probe certificate")?;

    // issued by our CA: the signature over the TBS part, with the CA's key
    if cert.issuer() != ca.subject() {
        return Err(anyhow!("certificate not issued by this ground station"));
    }
    let ca_key = signature::UnparsedPublicKey::new(
        &signature::ECDSA_P256_SHA256_ASN1,
        ca.public_key().subject_public_key.data.as_ref(),
    );
    ca_key
        .verify(
            cert.tbs_certificate.as_ref(),
            cert.signature_value.data.as_ref(),
        )
        .map_err(|_| anyhow!("certificate signature does not verify"))?;
    let v = cert.validity();
    if now < v.not_before.timestamp() || now > v.not_after.timestamp() {
        return Err(anyhow!("certificate outside its validity"));
    }

    // the report, signed by the certificate's key
    let sig = hex::decode(&body.sig).context("signature hex")?;
    let key = signature::UnparsedPublicKey::new(
        &signature::ECDSA_P256_SHA256_ASN1,
        cert.public_key().subject_public_key.data.as_ref(),
    );
    key.verify(body.report.as_bytes(), &sig)
        .map_err(|_| anyhow!("report signature does not verify"))?;

    let report: super::Report = serde_json::from_str(&body.report).context("report JSON")?;
    let cn = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|c| c.as_str().ok())
        .unwrap_or_default();
    if !cn.eq_ignore_ascii_case(&report.host) {
        return Err(anyhow!("report host is not the certificate's host"));
    }
    if (report.taken_at - now).abs() > MAX_SKEW_S {
        return Err(anyhow!("report too old or from the future"));
    }
    Ok(Verified { report })
}

/// Client side: collect, sign with the enrolled key, POST to the ground station.
pub async fn send(
    ground: &str,
    state_dir: &std::path::Path,
    ca_cert: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let key = std::fs::read_to_string(state_dir.join("probe-key.pem"))
        .context("reading probe key; enroll first")?;
    let cert = std::fs::read_to_string(state_dir.join("probe-cert.pem"))
        .context("reading probe certificate; enroll first")?;
    let mut report = super::collect(Some(&state_dir.join("io-prev.json"))).await;
    // the certificate names the host; report under that name
    let cert_der = pem_body(&cert, "CERTIFICATE")?;
    let (_, c) = X509Certificate::from_der(&cert_der).context("probe certificate")?;
    if let Some(cn) = c
        .subject()
        .iter_common_name()
        .next()
        .and_then(|c| c.as_str().ok())
    {
        report.host = cn.to_string();
    }
    let report = serde_json::to_string(&report)?;
    let body = SignedReport {
        sig: sign(&key, &report)?,
        cert,
        report,
    };
    let mut client = reqwest::Client::builder();
    if let Some(p) = ca_cert {
        let pem = std::fs::read(p).context("reading --ca-cert")?;
        client = client.add_root_certificate(reqwest::Certificate::from_pem(&pem)?);
    }
    let url = format!("{}/api/probe/report", ground.trim_end_matches('/'));
    let resp = client.build()?.post(&url).json(&body).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        return Err(anyhow!(
            "report refused by {ground}: {status} {}",
            resp.text().await.unwrap_or_default()
        ));
    }
    Ok(())
}

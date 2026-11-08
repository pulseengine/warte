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
    Ok(hex::encode(sign_bytes(key_pem, report_json.as_bytes())?))
}

pub fn sign_bytes(key_pem: &str, msg: &[u8]) -> anyhow::Result<Vec<u8>> {
    let der = pem_body(key_pem, "PRIVATE KEY")?;
    let rng = ring::rand::SystemRandom::new();
    let kp =
        signature::EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P256_SHA256_ASN1_SIGNING, &der, &rng)
            .map_err(|e| anyhow!("probe key unusable: {e}"))?;
    Ok(kp
        .sign(&rng, msg)
        .map_err(|e| anyhow!("signing failed: {e}"))?
        .as_ref()
        .to_vec())
}

/// What the ground station verified: the report and its host.
#[derive(Debug)]
pub struct Verified {
    pub report: super::Report,
    /// The certificate's not-before, to check against a revocation.
    pub issued_at: i64,
    /// The certificate's not-after (REQ-PROBE-013).
    pub cert_until: i64,
}

/// A probe certificate that checked out against warte's CA.
struct CertInfo {
    cn: String,
    public_key: Vec<u8>,
    not_before: i64,
    not_after: i64,
}

/// The certificate was issued by warte's CA (DER) and is valid at `now`.
fn check_cert(ca_der: &[u8], cert_pem: &str, now: i64) -> anyhow::Result<CertInfo> {
    let (_, ca) = X509Certificate::from_der(ca_der).context("CA certificate")?;
    let cert_der = pem_body(cert_pem, "CERTIFICATE")?;
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
    let cn = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|c| c.as_str().ok())
        .unwrap_or_default()
        .to_string();
    let info = CertInfo {
        cn,
        public_key: cert.public_key().subject_public_key.data.to_vec(),
        not_before: v.not_before.timestamp(),
        not_after: v.not_after.timestamp(),
    };
    Ok(info)
}

fn verify_sig(public_key: &[u8], msg: &[u8], sig_hex: &str) -> anyhow::Result<()> {
    let sig = hex::decode(sig_hex).context("signature hex")?;
    signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_ASN1, public_key)
        .verify(msg, &sig)
        .map_err(|_| anyhow!("signature does not verify"))
}

/// Verify a signed report against warte's CA (DER). `now` is unix seconds.
/// The caller still checks that `taken_at` is newer than the last accepted
/// report for the host, and that the host is not revoked.
pub fn verify(ca_der: &[u8], body: &SignedReport, now: i64) -> anyhow::Result<Verified> {
    let c = check_cert(ca_der, &body.cert, now)?;
    verify_sig(&c.public_key, body.report.as_bytes(), &body.sig)
        .map_err(|_| anyhow!("report signature does not verify"))?;
    let report: super::Report = serde_json::from_str(&body.report).context("report JSON")?;
    if !c.cn.eq_ignore_ascii_case(&report.host) {
        return Err(anyhow!("report host is not the certificate's host"));
    }
    if (report.taken_at - now).abs() > MAX_SKEW_S {
        return Err(anyhow!("report too old or from the future"));
    }
    Ok(Verified {
        report,
        issued_at: c.not_before,
        cert_until: c.not_after,
    })
}

/// Body of POST /api/probe/renew (REQ-PROBE-007): a new CSR for the same
/// key, signed with that key alongside the certificate it replaces.
#[derive(Debug, Serialize, Deserialize)]
pub struct RenewRequest {
    pub cert: String,
    pub csr: String,
    /// Unix seconds when the request was made; within [`MAX_SKEW_S`].
    pub at: i64,
    /// Signature over [`renew_message`], hex.
    pub sig: String,
}

/// The bytes a renewal signs: a fixed prefix, so a report signature can
/// never pass as one.
pub fn renew_message(at: i64, csr: &str) -> Vec<u8> {
    format!("warte-probe-renew\n{at}\n{csr}").into_bytes()
}

/// What a renewal verified: the host to issue for, and when its current
/// certificate was issued.
#[derive(Debug)]
pub struct Renewal {
    pub host: String,
    pub issued_at: i64,
}

/// Verify a renewal: a current certificate from warte's CA, a fresh request
/// signed by its key, and a CSR for that same key. An expired certificate
/// fails here: it needs a new enrollment.
pub fn verify_renew(ca_der: &[u8], req: &RenewRequest, now: i64) -> anyhow::Result<Renewal> {
    let c = check_cert(ca_der, &req.cert, now)?;
    if (req.at - now).abs() > MAX_SKEW_S {
        return Err(anyhow!("renewal too old or from the future"));
    }
    verify_sig(&c.public_key, &renew_message(req.at, &req.csr), &req.sig)
        .map_err(|_| anyhow!("renewal signature does not verify"))?;
    let der = pem_body(&req.csr, "CERTIFICATE REQUEST")?;
    let (_, csr) = x509_parser::certification_request::X509CertificationRequest::from_der(&der)
        .context("certificate request")?;
    if csr
        .certification_request_info
        .subject_pki
        .subject_public_key
        .data
        .as_ref()
        != c.public_key.as_slice()
    {
        return Err(anyhow!("renewal asks for a different key"));
    }
    Ok(Renewal {
        host: c.cn,
        issued_at: c.not_before,
    })
}

/// Body of POST /api/probe/feed/<name> (REQ-TEMPER-002): a file another
/// service on the probe's machine writes (temper's report), sent signed with
/// the probe's key.
#[derive(Debug, Serialize, Deserialize)]
pub struct SignedFeed {
    pub cert: String,
    /// The feed's name, e.g. `temper`; it is part of what is signed.
    pub name: String,
    /// The file as one CCSDS segmented sequence on the feed's APID
    /// (`hub::telemetry::segment`), base64. The same packets will travel over
    /// the QUIC + relay-sec transport (DD-TRANSPORT-001).
    pub ccsds: String,
    /// Unix seconds when it was sent; within [`MAX_SKEW_S`].
    pub at: i64,
    /// Signature over [`feed_message`], hex.
    pub sig: String,
}

/// The bytes a feed signs: a fixed prefix, so neither a report nor a renewal
/// signature can pass as one, and the name, so a feed cannot be replayed
/// under another.
pub fn feed_message(name: &str, at: i64, packets: &[u8]) -> Vec<u8> {
    let mut m = format!("warte-probe-feed\n{name}\n{at}\n").into_bytes();
    m.extend_from_slice(packets);
    m
}

/// The APID a feed travels on; only known feeds have one.
pub fn feed_apid(name: &str) -> Option<u16> {
    match name {
        "temper" => Some(crate::hub::telemetry::APID_FEED_TEMPER),
        _ => None,
    }
}

/// What a feed verified: the sending host, its certificate's issue time,
/// and the reassembled file.
#[derive(Debug)]
pub struct VerifiedFeed {
    pub host: String,
    pub issued_at: i64,
    pub data: Vec<u8>,
}

/// Verify a feed: a known feed name, a current certificate from warte's CA,
/// a fresh request signed by its key, and a complete CCSDS sequence on the
/// feed's APID. The caller checks the host is the one the feed may come
/// from, revocation and replay.
pub fn verify_feed(ca_der: &[u8], f: &SignedFeed, now: i64) -> anyhow::Result<VerifiedFeed> {
    let apid = feed_apid(&f.name).ok_or_else(|| anyhow!("unknown feed"))?;
    let c = check_cert(ca_der, &f.cert, now)?;
    if (f.at - now).abs() > MAX_SKEW_S {
        return Err(anyhow!("feed too old or from the future"));
    }
    use base64::Engine;
    let packets = base64::engine::general_purpose::STANDARD
        .decode(&f.ccsds)
        .context("feed packets base64")?;
    verify_sig(
        &c.public_key,
        &feed_message(&f.name, f.at, &packets),
        &f.sig,
    )
    .map_err(|_| anyhow!("feed signature does not verify"))?;
    let data = crate::hub::telemetry::reassemble(apid, &packets)
        .map_err(|e| anyhow!("feed packets: {e}"))?;
    Ok(VerifiedFeed {
        host: c.cn,
        issued_at: c.not_before,
        data,
    })
}

/// Client side: send the file at `path` as feed `name`.
pub async fn send_feed(
    client: &reqwest::Client,
    ground: &str,
    state_dir: &std::path::Path,
    name: &str,
    path: &std::path::Path,
    now: i64,
) -> anyhow::Result<()> {
    let apid = feed_apid(name).ok_or_else(|| anyhow!("unknown feed {name:?} (known: temper)"))?;
    let key = std::fs::read_to_string(state_dir.join("probe-key.pem"))?;
    let cert = std::fs::read_to_string(state_dir.join("probe-cert.pem"))?;
    let meta =
        std::fs::metadata(path).with_context(|| format!("feed {name}: {}", path.display()))?;
    if meta.len() > 2 << 20 {
        return Err(anyhow!(
            "feed {name}: {} is larger than 2 MiB",
            path.display()
        ));
    }
    let body = std::fs::read(path).with_context(|| format!("feed {name}: {}", path.display()))?;
    let packets =
        crate::hub::telemetry::segment(apid, &body).map_err(|e| anyhow!("feed {name}: {e}"))?;
    use base64::Engine;
    let f = SignedFeed {
        sig: hex::encode(sign_bytes(&key, &feed_message(name, now, &packets))?),
        cert,
        name: name.into(),
        ccsds: base64::engine::general_purpose::STANDARD.encode(&packets),
        at: now,
    };
    let url = format!("{}/api/probe/feed/{name}", ground.trim_end_matches('/'));
    let resp = client.post(&url).json(&f).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        return Err(anyhow!(
            "feed {name} refused by {ground}: {status} {}",
            resp.text().await.unwrap_or_default()
        ));
    }
    Ok(())
}

/// True once two thirds of the certificate's life have passed.
pub fn renewal_due(cert_pem: &str, now: i64) -> anyhow::Result<bool> {
    let der = pem_body(cert_pem, "CERTIFICATE")?;
    let (_, c) = X509Certificate::from_der(&der).context("probe certificate")?;
    let (nb, na) = (
        c.validity().not_before.timestamp(),
        c.validity().not_after.timestamp(),
    );
    Ok(now >= nb + (na - nb) * 2 / 3)
}

/// Client side of renewal: a CSR for the enrolled key, signed with it; the
/// new certificate replaces the old one atomically.
pub async fn renew(
    client: &reqwest::Client,
    ground: &str,
    state_dir: &std::path::Path,
    now: i64,
) -> anyhow::Result<()> {
    let key = std::fs::read_to_string(state_dir.join("probe-key.pem"))?;
    let cert = std::fs::read_to_string(state_dir.join("probe-cert.pem"))?;
    let der = pem_body(&cert, "CERTIFICATE")?;
    let (_, c) = X509Certificate::from_der(&der).context("probe certificate")?;
    let host = c
        .subject()
        .iter_common_name()
        .next()
        .and_then(|c| c.as_str().ok())
        .ok_or_else(|| anyhow!("probe certificate has no name"))?
        .to_string();
    let kp = rcgen::KeyPair::from_pem(&key).context("probe key")?;
    let csr = super::enroll::csr_pem(&host, &kp)?;
    let body = RenewRequest {
        sig: hex::encode(sign_bytes(&key, &renew_message(now, &csr))?),
        cert,
        csr,
        at: now,
    };
    let url = format!("{}/api/probe/renew", ground.trim_end_matches('/'));
    let resp = client.post(&url).json(&body).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        return Err(anyhow!(
            "renewal refused by {ground}: {status} {}",
            resp.text().await.unwrap_or_default()
        ));
    }
    let r: super::enroll::EnrollResponse = resp.json().await.context("renewal response")?;
    check_cert(&pem_body(&r.ca, "CERTIFICATE")?, &r.cert, now)
        .context("the renewed certificate does not check out")?;
    let tmp = state_dir.join("probe-cert.pem.tmp");
    std::fs::write(&tmp, &r.cert)?;
    std::fs::rename(&tmp, state_dir.join("probe-cert.pem"))?;
    Ok(())
}

/// Client side: collect, sign with the enrolled key, POST to the ground station.
pub async fn send(
    ground: &str,
    state_dir: &std::path::Path,
    ca_cert: Option<&std::path::Path>,
    feeds: &[(String, std::path::PathBuf)],
) -> anyhow::Result<()> {
    let mut client = reqwest::Client::builder();
    if let Some(p) = ca_cert {
        let pem = std::fs::read(p).context("reading --ca-cert")?;
        client = client.add_root_certificate(reqwest::Certificate::from_pem(&pem)?);
    }
    let client = client.build()?;
    let cert = std::fs::read_to_string(state_dir.join("probe-cert.pem"))
        .context("reading probe certificate; enroll first")?;
    // renew at two thirds of the certificate's life (REQ-PROBE-007); a failed
    // renewal is said and retried next time, the current certificate still works
    let now = chrono::Utc::now().timestamp();
    if renewal_due(&cert, now)? {
        match renew(&client, ground, state_dir, now).await {
            Ok(()) => eprintln!("probe certificate renewed"),
            Err(e) => eprintln!("probe certificate renewal failed, retrying next run: {e:#}"),
        }
    }
    let key = std::fs::read_to_string(state_dir.join("probe-key.pem"))
        .context("reading probe key; enroll first")?;
    let cert = std::fs::read_to_string(state_dir.join("probe-cert.pem"))
        .context("reading probe certificate; enroll first")?;
    let mut report = super::collect(Some(&state_dir.join("io-prev.json"))).await;
    // REQ-PRESSURE-002: size what agents hold while a volume is low
    super::pressure::attach(
        &mut report,
        state_dir,
        &super::pressure::Thresholds::from_env(),
        now,
    );
    // the certificate names the host; report under that name
    if let Some(cn) = cert_host(&cert)? {
        report.host = cn;
    }
    let report = serde_json::to_string(&report)?;
    let body = SignedReport {
        sig: sign(&key, &report)?,
        cert,
        report,
    };
    let url = format!("{}/api/probe/report", ground.trim_end_matches('/'));
    let resp = client.post(&url).json(&body).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        return Err(anyhow!(
            "report refused by {ground}: {status} {}",
            resp.text().await.unwrap_or_default()
        ));
    }
    // feeds after the report: a missing or refused feed does not hold the
    // machine's own report back, but the run still fails so it is seen
    let mut failed = vec![];
    for (name, path) in feeds {
        if let Err(e) = send_feed(&client, ground, state_dir, name, path, now).await {
            failed.push(format!("{e:#}"));
        }
    }
    if !failed.is_empty() {
        return Err(anyhow!("{}", failed.join("; ")));
    }
    Ok(())
}

/// The host a probe certificate (PEM) names: the name its reports carry.
pub fn cert_host(cert_pem: &str) -> anyhow::Result<Option<String>> {
    let cert_der = pem_body(cert_pem, "CERTIFICATE")?;
    let (_, c) = X509Certificate::from_der(&cert_der).context("probe certificate")?;
    let host = c
        .subject()
        .iter_common_name()
        .next()
        .and_then(|c| c.as_str().ok())
        .map(str::to_string);
    Ok(host)
}

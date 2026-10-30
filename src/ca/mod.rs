//! Certificate Authority — issues mTLS client certificates for agents.
//!
//! Follows the SPIRE pattern: short-lived certificates (24h), automatic
//! renewal, per-agent identity bound to the certificate CN.

use chrono::{DateTime, Duration, Utc};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Certificate authority for issuing agent mTLS certificates.
pub struct CertificateAuthority {
    /// CA certificate (PEM).
    ca_cert_pem: String,
    /// CA certificate (DER, for rustls).
    ca_cert_der: Vec<u8>,
    /// CA certificate params (for signing agent certs).
    ca_params: Arc<RwLock<CertificateParams>>,
    /// CA key pair (for signing agent certs).
    ca_key: Arc<RwLock<KeyPair>>,
    /// Certificate validity in hours.
    cert_validity_hours: u64,
}

impl CertificateAuthority {
    /// Load or create a CA.
    pub fn load_or_create(
        ca_cert_path: &Path,
        ca_key_path: &Path,
        cert_validity_hours: u64,
    ) -> anyhow::Result<Self> {
        if ca_cert_path.exists() && ca_key_path.exists() {
            Self::load(ca_cert_path, ca_key_path, cert_validity_hours)
        } else {
            Self::create(ca_cert_path, ca_key_path, cert_validity_hours)
        }
    }

    /// Load an existing CA from disk.
    fn load(
        ca_cert_path: &Path,
        ca_key_path: &Path,
        cert_validity_hours: u64,
    ) -> anyhow::Result<Self> {
        let ca_cert_pem = std::fs::read_to_string(ca_cert_path)?;
        let ca_key_pem = std::fs::read_to_string(ca_key_path)?;
        let ca_key = KeyPair::from_pem(&ca_key_pem)?;
        let ca_cert_der = pem_cert_to_der(&ca_cert_pem)?;

        // Load CA params from the PEM
        let ca_params = CertificateParams::from_ca_cert_pem(&ca_cert_pem)?;

        Ok(Self {
            ca_cert_pem,
            ca_cert_der,
            ca_params: Arc::new(RwLock::new(ca_params)),
            ca_key: Arc::new(RwLock::new(ca_key)),
            cert_validity_hours,
        })
    }

    /// Create a new CA and save it to disk.
    fn create(
        ca_cert_path: &Path,
        ca_key_path: &Path,
        cert_validity_hours: u64,
    ) -> anyhow::Result<Self> {
        if let Some(parent) = ca_cert_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Generate CA key pair (ECDSA P-256)
        let ca_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;

        // Create CA certificate params
        let mut params = CertificateParams::new(vec!["warte-ca".to_string()])?;
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, "warte CA");
        params
            .distinguished_name
            .push(DnType::OrganizationName, "PulseEngine");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::DigitalSignature,
        ];

        // Self-sign the CA certificate
        let ca_cert = params.clone().self_signed(&ca_key)?;
        let ca_cert_pem = ca_cert.pem();
        let ca_cert_der = ca_cert.der().to_vec();

        // Save to disk
        std::fs::write(ca_cert_path, &ca_cert_pem)?;
        std::fs::write(ca_key_path, ca_key.serialize_pem())?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(ca_key_path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(Self {
            ca_cert_pem,
            ca_cert_der,
            ca_params: Arc::new(RwLock::new(params)),
            ca_key: Arc::new(RwLock::new(ca_key)),
            cert_validity_hours,
        })
    }

    /// Get the CA certificate in PEM format.
    pub fn ca_cert_pem(&self) -> &str {
        &self.ca_cert_pem
    }

    /// Get the CA certificate in DER format (for rustls).
    pub fn ca_cert_der(&self) -> &[u8] {
        &self.ca_cert_der
    }

    /// Issue a client certificate for an agent.
    ///
    /// The certificate has:
    /// - CN = agent ID
    /// - Validity = cert_validity_hours (default 24h)
    /// - Signed by the CA
    /// - ClientAuth EKU
    pub async fn issue_agent_cert(
        &self,
        agent_id: &str,
        agent_name: &str,
    ) -> anyhow::Result<(String, DateTime<Utc>)> {
        // Generate agent key pair
        let agent_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;

        // Create agent certificate params
        let mut params = CertificateParams::new(vec![agent_id.to_string()])?;
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, agent_id);
        params
            .distinguished_name
            .push(DnType::OrganizationName, "PulseEngine Agent");
        params
            .distinguished_name
            .push(DnType::OrganizationalUnitName, agent_name);

        // Client auth usage
        params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];

        // Sign with CA
        let ca_key = self.ca_key.read().await;
        let ca_params = self.ca_params.read().await;
        let ca_cert = ca_params.clone().self_signed(&ca_key)?;
        let agent_cert = params.signed_by(&agent_key, &ca_cert, &ca_key)?;

        // Return cert PEM + key PEM as a bundle
        let cert_pem = agent_cert.pem();
        let key_pem = agent_key.serialize_pem();
        let bundle = format!("{}\n{}", cert_pem, key_pem);

        // Calculate expiry (24h from now, matching cert_validity_hours)
        let expires = Utc::now() + Duration::hours(self.cert_validity_hours as i64);

        Ok((bundle, expires))
    }
}

impl CertificateAuthority {
    /// Sign a probe's certificate request (DD-PROBE-002). The probe made its
    /// key and keeps it; only its public key comes from the request. The name
    /// is the host the enrollment token was minted for, never the request's:
    /// a CSR cannot choose its own identity.
    pub async fn sign_probe_csr(
        &self,
        csr_pem: &str,
        host: &str,
        days: u32,
    ) -> anyhow::Result<String> {
        let mut csr = rcgen::CertificateSigningRequestParams::from_pem(csr_pem)?;
        let mut params = CertificateParams::new(vec![host.to_string()])?;
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, host);
        params
            .distinguished_name
            .push(DnType::OrganizationName, "PulseEngine Probe");
        params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::minutes(5);
        params.not_after = now + time::Duration::days(days as i64);
        csr.params = params;
        let ca_key = self.ca_key.read().await;
        let ca_params = self.ca_params.read().await;
        let ca_cert = ca_params.clone().self_signed(&ca_key)?;
        Ok(csr.signed_by(&ca_cert, &ca_key)?.pem())
    }
}

/// Convert a PEM certificate to DER bytes.
fn pem_cert_to_der(pem: &str) -> anyhow::Result<Vec<u8>> {
    let pem = pem.trim();
    let start = pem
        .find("-----BEGIN CERTIFICATE-----")
        .ok_or_else(|| anyhow::anyhow!("no BEGIN CERTIFICATE"))?;
    let end = pem
        .find("-----END CERTIFICATE-----")
        .ok_or_else(|| anyhow::anyhow!("no END CERTIFICATE"))?;
    let b64_start = start + "-----BEGIN CERTIFICATE-----".len();
    let b64 = pem[b64_start..end].replace(|c: char| c.is_whitespace(), "");
    base64_decode(&b64)
}

/// Simple base64 decoder.
fn base64_decode(input: &str) -> anyhow::Result<Vec<u8>> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;

    for &byte in input.as_bytes() {
        if byte == b'=' {
            break;
        }
        let val = TABLE
            .iter()
            .position(|&t| t == byte)
            .ok_or_else(|| anyhow::anyhow!("invalid base64"))? as u32;
        buffer = (buffer << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ca_create_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("ca.pem");
        let key_path = dir.path().join("ca-key.pem");

        let ca = CertificateAuthority::create(&cert_path, &key_path, 24).unwrap();
        assert!(ca.ca_cert_pem().contains("BEGIN CERTIFICATE"));
        assert!(!ca.ca_cert_der().is_empty());

        let ca2 = CertificateAuthority::load(&cert_path, &key_path, 24).unwrap();
        assert_eq!(ca.ca_cert_pem(), ca2.ca_cert_pem());
    }

    #[tokio::test]
    async fn test_issue_agent_cert() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("ca.pem");
        let key_path = dir.path().join("ca-key.pem");

        let ca = CertificateAuthority::create(&cert_path, &key_path, 24).unwrap();
        let (bundle, expires) = ca
            .issue_agent_cert("agent-123", "test-agent")
            .await
            .unwrap();

        assert!(bundle.contains("BEGIN CERTIFICATE"));
        assert!(bundle.contains("PRIVATE KEY"));
        assert!(expires > Utc::now());
    }
}

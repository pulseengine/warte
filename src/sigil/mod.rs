//! Sigil/wsc signature verification for artifacts.
//!
//! REQ-SIGIL-001: Artifact integrity proofs SHALL support sigil/wsc
//! detached signatures in addition to SHA-256 hashes. The wsc binary
//! SHALL be resolved through the varve layer (varve which wsc).
//! SHA-256 SHALL remain as a fallback.
//!
//! wsc commands used:
//! - wsc keygen: generate a signing key pair
//! - wsc sign: sign an artifact (produces detached signature)
//! - wsc verify: verify a detached signature
//! - wsc detach: detach a signature from a signed artifact

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

/// Result of a wsc signature operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WscSignature {
    /// The detached signature (base64 or raw bytes).
    pub signature: Vec<u8>,
    /// The public key used for signing (PEM or raw).
    pub public_key: Vec<u8>,
    /// The signing algorithm (e.g., "ed25519").
    pub algorithm: String,
    /// SHA-256 of the signed content (for quick verification).
    pub content_hash: String,
}

/// Sigil/wsc integration for artifact signing and verification.
pub struct SigilVerifier {
    /// Path to the wsc binary (resolved through varve).
    wsc_path: String,
    /// Whether wsc is available.
    available: bool,
}

impl SigilVerifier {
    /// Create a new sigil verifier. Resolves wsc through varve.
    pub fn new() -> Self {
        let wsc_path = Self::resolve_wsc();
        let available = !wsc_path.is_empty();
        if available {
            tracing::info!(wsc_path = %wsc_path, "wsc resolved through varve");
        } else {
            tracing::warn!("wsc not found — falling back to SHA-256 only");
        }
        Self {
            wsc_path,
            available,
        }
    }

    /// Resolve wsc through the varve layer.
    fn resolve_wsc() -> String {
        // Try varve which wsc first
        if let Ok(output) = Command::new("varve").args(["which", "wsc"]).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !path.is_empty() && Path::new(&path).exists() {
                    return path;
                }
            }
        }
        // Fall back to PATH
        if let Ok(output) = Command::new("which").arg("wsc").output() {
            if output.status.success() {
                return String::from_utf8_lossy(&output.stdout).trim().to_string();
            }
        }
        String::new()
    }

    /// Check if wsc is available.
    pub fn is_available(&self) -> bool {
        self.available
    }

    /// Sign content with wsc. Returns a detached signature.
    ///
    /// Falls back to SHA-256 hash if wsc is not available.
    pub fn sign(&self, content: &[u8], key_path: &Path) -> anyhow::Result<WscSignature> {
        let content_hash = hex::encode(Sha256::digest(content));

        if !self.available {
            // Fallback: SHA-256 only
            return Ok(WscSignature {
                signature: Sha256::digest(content).to_vec(),
                public_key: vec![],
                algorithm: "sha256".to_string(),
                content_hash,
            });
        }

        // Write content to a temp file for wsc to sign
        let temp_dir = std::env::temp_dir().join(format!("warte-sigil-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&temp_dir)?;
        let content_path = temp_dir.join("artifact.bin");
        std::fs::write(&content_path, content)?;

        // Sign with wsc
        let sig_path = temp_dir.join("artifact.sig");
        let output = Command::new(&self.wsc_path)
            .args(["sign", "--key", key_path.to_str().unwrap_or(""), "--output"])
            .arg(&sig_path)
            .arg(&content_path)
            .output()?;

        if !output.status.success() {
            anyhow::bail!(
                "wsc sign failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let signature = std::fs::read(&sig_path)?;

        // Read the public key
        let pub_key_path = key_path.with_extension("pub");
        let public_key = std::fs::read(&pub_key_path).unwrap_or_default();

        Ok(WscSignature {
            signature,
            public_key,
            algorithm: "ed25519".to_string(),
            content_hash,
        })
    }

    /// Verify a wsc signature against content.
    ///
    /// Falls back to SHA-256 comparison if wsc is not available or
    /// the signature is a SHA-256 hash.
    pub fn verify(&self, content: &[u8], signature: &WscSignature) -> anyhow::Result<bool> {
        // SHA-256 fallback
        if signature.algorithm == "sha256" {
            let computed = hex::encode(Sha256::digest(content));
            return Ok(computed == signature.content_hash);
        }

        if !self.available {
            // Can't verify wsc signature without wsc — fall back to hash
            let computed = hex::encode(Sha256::digest(content));
            return Ok(computed == signature.content_hash);
        }

        // Write content and signature to temp files
        let temp_dir = std::env::temp_dir().join(format!("warte-sigil-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&temp_dir)?;
        let content_path = temp_dir.join("artifact.bin");
        let sig_path = temp_dir.join("artifact.sig");
        let pub_key_path = temp_dir.join("key.pub");
        std::fs::write(&content_path, content)?;
        std::fs::write(&sig_path, &signature.signature)?;
        std::fs::write(&pub_key_path, &signature.public_key)?;

        // Verify with wsc
        let output = Command::new(&self.wsc_path)
            .args(["verify", "--signature"])
            .arg(&sig_path)
            .arg("--key")
            .arg(&pub_key_path)
            .arg(&content_path)
            .output()?;

        Ok(output.status.success())
    }

    /// Generate a new wsc key pair.
    pub fn keygen(&self, key_path: &Path) -> anyhow::Result<()> {
        if !self.available {
            anyhow::bail!("wsc not available — cannot generate key pair");
        }

        let output = Command::new(&self.wsc_path)
            .args(["keygen", "--output"])
            .arg(key_path)
            .output()?;

        if !output.status.success() {
            anyhow::bail!(
                "wsc keygen failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }
}

impl Default for SigilVerifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sigil_verifier_creation() {
        let verifier = SigilVerifier::new();
        // wsc may or may not be available depending on environment
        // Just verify the struct is created without panic
        let _ = verifier.is_available();
    }

    #[test]
    fn test_sha256_fallback_sign() {
        let verifier = SigilVerifier {
            wsc_path: String::new(),
            available: false,
        };

        let content = b"hello warte";
        let temp_dir =
            std::env::temp_dir().join(format!("warte-sigil-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let key_path = temp_dir.join("test.key");

        let sig = verifier.sign(content, &key_path).unwrap();
        assert_eq!(sig.algorithm, "sha256");
        assert!(!sig.content_hash.is_empty());
    }

    #[test]
    fn test_sha256_fallback_verify() {
        let verifier = SigilVerifier {
            wsc_path: String::new(),
            available: false,
        };

        let content = b"hello warte";
        let temp_dir =
            std::env::temp_dir().join(format!("warte-sigil-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let key_path = temp_dir.join("test.key");

        let sig = verifier.sign(content, &key_path).unwrap();
        assert!(verifier.verify(content, &sig).unwrap());

        // Tampered content should fail
        let tampered = b"hello warte!";
        assert!(!verifier.verify(tampered, &sig).unwrap());
    }

    #[test]
    fn test_wsc_signature_serialization() {
        let sig = WscSignature {
            signature: vec![1, 2, 3],
            public_key: vec![4, 5, 6],
            algorithm: "ed25519".to_string(),
            content_hash: "abc123".to_string(),
        };

        let json = serde_json::to_string(&sig).unwrap();
        let parsed: WscSignature = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.algorithm, "ed25519");
        assert_eq!(parsed.content_hash, "abc123");
    }
}

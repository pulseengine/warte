//! API authentication — bearer tokens for mutating operations.
//!
//! REQ-API-002: All mutating API operations (POST, PUT, DELETE, PATCH)
//! SHALL require authentication. Read operations MAY be public but SHALL
//! be rate-limited.
//!
//! Token format: `warte1.<base64url(payload)>.<base64url(signature)>`
//! Payload: { agent_id, issued_at, expires_at, rights: [...] }
//! Signature: Ed25519 over the payload, verified with the CA public key.

use axum::{
    extract::Request,
    http::{header::AUTHORIZATION, StatusCode},
    middleware::Next,
    response::Response,
};
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Token payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPayload {
    pub agent_id: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub rights: Vec<String>,
}

/// Token manager — issues and verifies bearer tokens.
pub struct TokenManager {
    /// Signing key (Ed25519).
    signing_key: Arc<SigningKey>,
    /// Verifying key (for token verification).
    verifying_key: VerifyingKey,
    /// Token validity in hours.
    token_validity_hours: i64,
}

impl TokenManager {
    /// Create a new token manager with a fresh signing key.
    pub fn new(token_validity_hours: i64) -> Self {
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key: Arc::new(signing_key),
            verifying_key,
            token_validity_hours,
        }
    }

    /// Load a token manager from a signing key (PEM or raw bytes).
    pub fn from_seed(seed: &[u8; 32], token_validity_hours: i64) -> Self {
        let signing_key = SigningKey::from_bytes(seed);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key: Arc::new(signing_key),
            verifying_key,
            token_validity_hours,
        }
    }

    /// Get the verifying key (for mTLS cert verification).
    pub fn verifying_key(&self) -> &VerifyingKey {
        &self.verifying_key
    }

    /// Issue a bearer token for an agent.
    pub fn issue_token(&self, agent_id: &str, rights: Vec<String>) -> anyhow::Result<String> {
        let now = Utc::now();
        let expires = now + Duration::hours(self.token_validity_hours);

        let payload = TokenPayload {
            agent_id: agent_id.to_string(),
            issued_at: now,
            expires_at: expires,
            rights,
        };

        let payload_json = serde_json::to_vec(&payload)?;
        let payload_b64 = base64url_encode(&payload_json);
        let signature = self.signing_key.sign(payload_b64.as_bytes());
        let sig_b64 = base64url_encode(&signature.to_bytes());

        Ok(format!("warte1.{}.{}", payload_b64, sig_b64))
    }

    /// Verify a bearer token. Returns the payload if valid.
    pub fn verify_token(&self, token: &str) -> anyhow::Result<TokenPayload> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 || parts[0] != "warte1" {
            anyhow::bail!("invalid token format");
        }

        let payload_b64 = parts[1];
        let sig_b64 = parts[2];

        // Verify signature
        let sig_bytes = base64url_decode(sig_b64)?;
        let signature = Signature::from_slice(&sig_bytes)?;
        self.verifying_key
            .verify(payload_b64.as_bytes(), &signature)
            .map_err(|_| anyhow::anyhow!("invalid signature"))?;

        // Decode payload
        let payload_json = base64url_decode(payload_b64)?;
        let payload: TokenPayload = serde_json::from_slice(&payload_json)?;

        // Check expiry
        if payload.expires_at < Utc::now() {
            anyhow::bail!("token expired");
        }

        Ok(payload)
    }
}

/// Extracted auth info from a request.
#[derive(Debug, Clone)]
pub struct AuthInfo {
    pub agent_id: String,
    pub rights: Vec<String>,
}

/// Authentication middleware for mutating endpoints.
///
/// Extracts the bearer token from the Authorization header and verifies it.
/// On success, inserts AuthInfo into the request extensions.
pub async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<crate::server::AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Only check mutating methods
    let method = req.method().clone();
    if method == axum::http::Method::GET
        || method == axum::http::Method::HEAD
        || method == axum::http::Method::OPTIONS
    {
        return Ok(next.run(req).await);
    }

    // Skip webhook endpoints (they use HMAC signature verification, not bearer tokens)
    let path = req.uri().path();
    if path.starts_with("/api/webhooks/") {
        return Ok(next.run(req).await);
    }

    // Extract Authorization header
    let auth_header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Verify token
    let payload = state
        .tokens
        .verify_token(token)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    // Insert auth info into request extensions
    req.extensions_mut().insert(AuthInfo {
        agent_id: payload.agent_id,
        rights: payload.rights,
    });

    Ok(next.run(req).await)
}

/// Base64url encode (no padding).
fn base64url_encode(input: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::new();
    let mut buffer = 0u32;
    let mut bits = 0;

    for &byte in input {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            output.push(TABLE[(buffer >> bits) as usize & 0x3F] as char);
        }
    }
    if bits > 0 {
        output.push(TABLE[(buffer << (6 - bits)) as usize & 0x3F] as char);
    }
    output
}

/// Base64url decode (no padding).
fn base64url_decode(input: &str) -> anyhow::Result<Vec<u8>> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;

    for &byte in input.as_bytes() {
        let val = TABLE
            .iter()
            .position(|&t| t == byte)
            .ok_or_else(|| anyhow::anyhow!("invalid base64url"))? as u32;
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
    fn test_token_issue_and_verify() {
        let tm = TokenManager::new(24);
        let token = tm
            .issue_token("agent-123", vec!["artifacts:download".to_string()])
            .unwrap();
        let payload = tm.verify_token(&token).unwrap();
        assert_eq!(payload.agent_id, "agent-123");
        assert_eq!(payload.rights, vec!["artifacts:download".to_string()]);
    }

    #[test]
    fn test_token_expired() {
        let tm = TokenManager::new(-1); // Already expired
        let token = tm.issue_token("agent-123", vec![]).unwrap();
        assert!(tm.verify_token(&token).is_err());
    }

    #[test]
    fn test_token_invalid_format() {
        let tm = TokenManager::new(24);
        assert!(tm.verify_token("invalid").is_err());
        assert!(tm.verify_token("warte1.invalid.invalid").is_err());
    }

    #[test]
    fn test_token_tampered() {
        let tm = TokenManager::new(24);
        let token = tm.issue_token("agent-123", vec![]).unwrap();
        // Tamper with the payload
        let parts: Vec<&str> = token.split('.').collect();
        let tampered = format!("warte1.{}.{}", base64url_encode(b"tampered"), parts[2]);
        assert!(tm.verify_token(&tampered).is_err());
    }

    #[test]
    fn test_base64url_roundtrip() {
        let input = b"hello warte";
        let encoded = base64url_encode(input);
        let decoded = base64url_decode(&encoded).unwrap();
        assert_eq!(decoded, input);
    }
}

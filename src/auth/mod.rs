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

/// POST routes that carry their own authentication and pass without a
/// bearer token: enrollment (one-time token, DD-PROBE-002), signed reports
/// (REQ-PROBE-006), renewals (REQ-PROBE-007) and signed feeds such as
/// temper's report (REQ-TEMPER-002). An auth proxy in front must
/// leave exactly these open (docs: deployment, REQ-DOCS-010).
pub const PROBE_OPEN_PATHS: [&str; 4] = [
    "/api/probe/enroll",
    "/api/probe/report",
    "/api/probe/renew",
    "/api/probe/feed/temper",
];

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

/// Rights an agent token carries by default: the three write routes an agent
/// uses, for its own agent id only (REQ-API-004).
pub const AGENT_RIGHTS: [&str; 3] = ["agents:register", "agents:data", "agents:heartbeat"];

/// The right that lifts the agent binding: a token holding it may write as
/// any agent id, not only its own. The right to the route is still required.
pub const RIGHT_ANY_AGENT: &str = "agents:any";

/// Whose id a route's request names, which must be the token's agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// The request names no agent (the right alone decides).
    None,
    /// The `{id}` segment of the path is an agents row id; the name of that
    /// row must be the token's agent (an unknown id is refused).
    PathId,
    /// The `name` in the JSON body: register mints its own agent ids, so the
    /// identity a caller asks for is the name it registers.
    BodyName,
}

/// One mutating route and what a token needs to call it.
#[derive(Debug, Clone, Copy)]
pub struct RouteRule {
    pub method: &'static str,
    /// The full path, with `{id}` for the agent segment.
    pub pattern: &'static str,
    pub right: &'static str,
    pub binding: Binding,
}

/// REQ-API-004: every mutating route and the right it needs, in one place.
/// A mutating request that matches no row here is refused (403) even with a
/// valid token, so a route added without a row is closed, not open; the
/// test `every_mutating_route_in_the_router_has_a_right` also fails.
/// The probe routes (`PROBE_OPEN_PATHS`) authenticate themselves and are not here.
pub const MUTATING_ROUTES: [RouteRule; 9] = [
    RouteRule {
        method: "POST",
        pattern: "/api/agents/register",
        right: "agents:register",
        binding: Binding::BodyName,
    },
    RouteRule {
        method: "POST",
        pattern: "/api/agents/{id}/heartbeat",
        right: "agents:heartbeat",
        binding: Binding::PathId,
    },
    RouteRule {
        method: "POST",
        pattern: "/api/agents/{id}/data",
        right: "agents:data",
        binding: Binding::PathId,
    },
    RouteRule {
        method: "POST",
        pattern: "/api/access/grant",
        right: "access:grant",
        binding: Binding::None,
    },
    RouteRule {
        method: "POST",
        pattern: "/api/access/revoke",
        right: "access:revoke",
        binding: Binding::None,
    },
    RouteRule {
        method: "POST",
        pattern: "/api/sync/update",
        right: "sync:update",
        binding: Binding::None,
    },
    RouteRule {
        method: "POST",
        pattern: "/api/costs",
        right: "costs:write",
        binding: Binding::None,
    },
    RouteRule {
        method: "POST",
        pattern: "/api/briefing",
        right: "briefing:write",
        binding: Binding::None,
    },
    RouteRule {
        method: "POST",
        pattern: "/api/skill-usage",
        right: "skills:write",
        binding: Binding::None,
    },
];

/// Is `right` one a token can carry: a route's right or `agents:any`?
pub fn is_known_right(right: &str) -> bool {
    right == RIGHT_ANY_AGENT || MUTATING_ROUTES.iter().any(|r| r.right == right)
}

/// Every right a token can carry, for messages and docs.
pub fn known_rights() -> Vec<&'static str> {
    let mut v: Vec<&str> = MUTATING_ROUTES.iter().map(|r| r.right).collect();
    v.push(RIGHT_ANY_AGENT);
    v
}

/// The rule for a request, and the `{id}` segment (percent-decoded) if the
/// pattern has one. `None` when no row matches.
fn find_rule(method: &str, path: &str) -> Option<(&'static RouteRule, Option<String>)> {
    'rules: for rule in MUTATING_ROUTES.iter().filter(|r| r.method == method) {
        let want: Vec<&str> = rule.pattern.split('/').collect();
        let got: Vec<&str> = path.split('/').collect();
        if want.len() != got.len() {
            continue;
        }
        let mut id = None;
        for (w, g) in want.iter().zip(&got) {
            if *w == "{id}" {
                // An id the router would decode differently than we do is no
                // match: the request is refused, never compared wrongly.
                match pct_decode(g).filter(|d| !d.is_empty()) {
                    Some(d) => id = Some(d),
                    None => continue 'rules,
                }
            } else if w != g {
                continue 'rules;
            }
        }
        return Some((rule, id));
    }
    None
}

/// Percent-decode as the router does before a handler sees a path segment.
fn pct_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Largest register body the binding check reads (a name and a key).
const REGISTER_BODY_LIMIT: usize = 64 * 1024;

/// May this token call `rule`? `row_name` is the name of the agents row the path
/// names, `body_name` the name register was asked for.
fn authorize(
    info: &AuthInfo,
    rule: &RouteRule,
    row_name: Option<&str>,
    body_name: Option<&str>,
) -> bool {
    let has = |r: &str| info.rights.iter().any(|x| x == r);
    if !has(rule.right) {
        return false;
    }
    let named = match rule.binding {
        Binding::None => return true,
        Binding::PathId => row_name,
        Binding::BodyName => body_name,
    };
    has(RIGHT_ANY_AGENT) || named == Some(info.agent_id.as_str())
}

/// Longest validity `warte token mint` accepts: one year.
pub const MAX_TOKEN_HOURS: i64 = 24 * 365;

/// Load the 32-byte signing seed from `path`; create it (random, mode 0600,
/// never overwriting) if there is none. A seed that is not 64 hex digits, or
/// that other users can read, is an error that never quotes its contents.
/// Without a stored seed every restart would invalidate every issued token.
pub fn load_or_create_seed(path: &std::path::Path) -> anyhow::Result<[u8; 32]> {
    use anyhow::Context;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    if !path.exists() {
        let mut seed = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut seed);
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        match opts.open(path) {
            Ok(mut f) => {
                use std::io::Write;
                writeln!(f, "{}", hex::encode(seed))
                    .with_context(|| format!("writing {}", path.display()))?;
            }
            // Another process created it between exists() and open(): read theirs.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e).with_context(|| format!("creating {}", path.display())),
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)?.permissions().mode();
        anyhow::ensure!(
            mode & 0o077 == 0,
            "{} is readable by other users (mode {:o}); chmod 600 it",
            path.display(),
            mode & 0o777
        );
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let bytes = hex::decode(text.trim())
        .map_err(|_| anyhow::anyhow!("{} is not 64 hex digits", path.display()))?;
    <[u8; 32]>::try_from(bytes.as_slice())
        .map_err(|_| anyhow::anyhow!("{} is not 64 hex digits", path.display()))
}

/// Mint an agent token from the seed file of `config`, valid `hours`, with
/// the default `AGENT_RIGHTS`.
pub fn mint_agent_token(
    config: &crate::Config,
    agent_id: &str,
    hours: i64,
) -> anyhow::Result<String> {
    mint_agent_token_with(config, agent_id, hours, &[])
}

/// Like `mint_agent_token`, plus `extra` rights (`agents:any`, `costs:write`,
/// ...). A right no route knows is an error: a typo would otherwise mint a
/// token that silently grants nothing.
pub fn mint_agent_token_with(
    config: &crate::Config,
    agent_id: &str,
    hours: i64,
    extra: &[String],
) -> anyhow::Result<String> {
    anyhow::ensure!(!agent_id.trim().is_empty(), "agent id is empty");
    anyhow::ensure!(
        (1..=MAX_TOKEN_HOURS).contains(&hours),
        "--valid-hours must be between 1 and {MAX_TOKEN_HOURS}"
    );
    for r in extra {
        anyhow::ensure!(
            is_known_right(r),
            "unknown right {r:?}; known rights: {}",
            known_rights().join(", ")
        );
    }
    let mut rights: Vec<String> = AGENT_RIGHTS.iter().map(|r| r.to_string()).collect();
    for r in extra {
        if !rights.contains(r) {
            rights.push(r.clone());
        }
    }
    let seed = load_or_create_seed(&config.token_seed_file())?;
    TokenManager::from_seed(&seed, hours).issue_token(agent_id, rights)
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

    // The exemptions: probe enrollment authenticates with its one-time
    // token, checked by the handler (DD-PROBE-002). Exact path and method.
    // Signed probe reports authenticate with the enrolled key, checked by the
    // handler (REQ-PROBE-006), and so do renewals (REQ-PROBE-007) and feeds
    // (REQ-TEMPER-002). One list: PROBE_OPEN_PATHS, which the docs test reads.
    if method == axum::http::Method::POST && PROBE_OPEN_PATHS.contains(&req.uri().path()) {
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

    let info = AuthInfo {
        agent_id: payload.agent_id,
        rights: payload.rights,
    };

    // REQ-API-004: the route's right, then the agent binding. A missing
    // right is 403; the 401s above are for a missing or invalid token.
    let path = req.uri().path().to_string();
    let Some((rule, path_id)) = find_rule(method.as_str(), &path) else {
        return Err(StatusCode::FORBIDDEN);
    };
    let mut body_name = None;
    if rule.binding == Binding::BodyName {
        let (parts, body) = req.into_parts();
        let bytes = axum::body::to_bytes(body, REGISTER_BODY_LIMIT)
            .await
            .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
        #[derive(Deserialize)]
        struct Named {
            name: String,
        }
        // A body with no name cannot register (the handler refuses it), so
        // it is compared as "no name" and, without agents:any, refused here.
        body_name = serde_json::from_slice::<Named>(&bytes).ok().map(|n| n.name);
        req = Request::from_parts(parts, axum::body::Body::from(bytes));
    }
    // The path names the agents row (register assigned the id); the token
    // names the agent by the name it registered under. Resolve one to the
    // other, and skip the lookup for a token the binding does not apply to.
    let mut row_name = None;
    if rule.binding == Binding::PathId && !info.rights.iter().any(|r| r == RIGHT_ANY_AGENT) {
        if let Some(id) = path_id.as_deref() {
            row_name = state
                .agents
                .get(id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                .map(|a| a.name);
        }
    }
    if !authorize(&info, rule, row_name.as_deref(), body_name.as_deref()) {
        return Err(StatusCode::FORBIDDEN);
    }

    // Insert auth info into request extensions
    req.extensions_mut().insert(info);

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

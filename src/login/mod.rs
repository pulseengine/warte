//! Login through an OIDC provider, Zitadel first (DD-AUTH-001,
//! REQ-AUTH-001..003). warte never sees a password: it verifies the
//! provider's signed tokens, either passed by a proxy in front
//! (`Authorization: Bearer <JWT>`) or obtained by handing the browser over
//! to the provider's login (authorization code flow with PKCE), after which
//! warte keeps a server-side session behind a random cookie.
//!
//! Without `[auth]` nothing changes: every read stays open, as before.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, RwLock};
use std::time::Duration;

/// The session cookie's name.
pub const COOKIE: &str = "warte_session";
/// Seconds a token may be past its `exp` and still count (clock skew).
const LEEWAY_S: i64 = 60;
/// Unknown key ids refetch the JWKS at most this often.
const JWKS_REFETCH_S: i64 = 60;
/// A login that does not come back within this many seconds is forgotten.
const PENDING_S: i64 = 600;
const PENDING_MAX: usize = 1000;
const SESSIONS_MAX: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    /// The provider's issuer URL, exactly as in its tokens' `iss`.
    pub issuer: String,
    /// The OIDC application's client id (PKCE, no secret).
    pub client_id: String,
    /// The Zitadel project id: its roles claim and audience.
    #[serde(default)]
    pub project_id: Option<String>,
    /// warte's external URL, e.g. `https://fourpi.local/warte`: the
    /// redirect URI is `<base_url>/auth/callback`.
    pub base_url: String,
    /// Provider role key -> warte role (`viewer`, `member`, `owner`).
    /// Default: each of the three maps to itself.
    #[serde(default)]
    pub roles: BTreeMap<String, String>,
    #[serde(default = "default_session_hours")]
    pub session_hours: i64,
    /// Extra PEM roots for the provider's TLS, e.g. a LAN CA.
    #[serde(default)]
    pub ca_bundle: Option<std::path::PathBuf>,
}

fn default_session_hours() -> i64 {
    12
}

/// warte's roles, lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Member,
    Owner,
}

impl Role {
    fn parse(s: &str) -> Option<Role> {
        match s {
            "viewer" => Some(Role::Viewer),
            "member" => Some(Role::Member),
            "owner" => Some(Role::Owner),
            _ => None,
        }
    }
}

/// Who is signed in.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Identity {
    pub sub: String,
    pub name: String,
    pub role: Role,
}

#[derive(Debug, Clone, Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    #[serde(default)]
    end_session_endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    #[serde(default)]
    kid: Option<String>,
    kty: String,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
    #[serde(default)]
    crv: Option<String>,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    y: Option<String>,
}

struct Pending {
    verifier: String,
    nonce: String,
    created: i64,
    back: String,
}

struct Session {
    who: Identity,
    expires: i64,
}

pub struct Login {
    pub cfg: AuthConfig,
    http: reqwest::Client,
    discovery: RwLock<Option<Discovery>>,
    jwks: RwLock<(Vec<Jwk>, i64)>,
    pending: Mutex<HashMap<String, Pending>>,
    sessions: Mutex<HashMap<String, Session>>,
}

fn random(bytes: usize) -> String {
    use rand::RngCore;
    let mut b = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut b);
    B64.encode(b)
}

fn challenge(verifier: &str) -> String {
    use sha2::Digest;
    B64.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A path to return to after login: a local absolute path from a short
/// allow-list, never another site. No control character (a browser drops a
/// tab, so "/\t/evil" would become "//evil"), no backslash, and no second
/// slash first.
fn local_path(p: &str) -> String {
    let ok = p.len() <= 200
        && p.starts_with('/')
        && !p[1..].starts_with('/')
        && p.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/_.-~?=&%".contains(c));
    if ok {
        p.to_string()
    } else {
        "/".into()
    }
}

/// The login cookie: binds a login's state to the browser that started it.
pub const LOGIN_COOKIE: &str = "warte_login";

fn state_hash(state: &str) -> String {
    use sha2::Digest;
    B64.encode(sha2::Sha256::digest(state.as_bytes()))
}

/// Is `issuer` https, or plain http to this very machine (tests)?
fn issuer_ok(issuer: &str) -> bool {
    if issuer.starts_with("https://") {
        return true;
    }
    let Some(rest) = issuer.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    let (h, port) = host.split_once(':').unwrap_or((host, ""));
    h == "127.0.0.1" && port.bytes().all(|b| b.is_ascii_digit())
}

impl Login {
    pub fn new(cfg: AuthConfig) -> Result<Login, String> {
        if !issuer_ok(&cfg.issuer) {
            return Err("[auth] issuer must be an https URL".into());
        }
        for r in cfg.roles.values() {
            Role::parse(r)
                .ok_or_else(|| format!("[auth] roles: {r} is not viewer, member or owner"))?;
        }
        let mut b = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(concat!("warte/", env!("CARGO_PKG_VERSION")));
        if let Some(p) = &cfg.ca_bundle {
            for c in crate::ghsource::extra_roots(p)? {
                b = b.add_root_certificate(c);
            }
        }
        Ok(Login {
            http: b.build().map_err(|e| e.to_string())?,
            cfg,
            discovery: RwLock::new(None),
            jwks: RwLock::new((vec![], 0)),
            pending: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
        })
    }

    fn base_path(&self) -> String {
        // the path part of base_url, for the cookie and the redirects
        let rest = self.cfg.base_url.splitn(4, '/').nth(3).unwrap_or("");
        let p = format!("/{}", rest.trim_end_matches('/'));
        if p == "/" {
            "/".into()
        } else {
            p
        }
    }

    async fn discovery(&self) -> Result<Discovery, String> {
        if let Some(d) = self.discovery.read().ok().and_then(|d| d.clone()) {
            return Ok(d);
        }
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.cfg.issuer.trim_end_matches('/')
        );
        let d: Discovery = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("provider not reached: {}", e.without_url()))?
            .json()
            .await
            .map_err(|_| "provider discovery is not JSON".to_string())?;
        if d.issuer != self.cfg.issuer {
            return Err("provider discovery names another issuer".into());
        }
        if let Ok(mut w) = self.discovery.write() {
            *w = Some(d.clone());
        }
        Ok(d)
    }

    async fn key(&self, kid: Option<&str>, now: i64) -> Result<Jwk, String> {
        let find = |keys: &[Jwk]| {
            keys.iter()
                .find(|k| kid.is_none() || k.kid.as_deref() == kid)
                .cloned()
        };
        let (have, fetched) = {
            let g = self.jwks.read().map_err(|_| "jwks lock")?;
            (find(&g.0), g.1)
        };
        if let Some(k) = have {
            return Ok(k);
        }
        if now - fetched < JWKS_REFETCH_S {
            return Err("unknown signing key".into());
        }
        // the attempt counts, not only a success: a provider that is down
        // or slow is asked at most once a minute
        {
            let mut w = self.jwks.write().map_err(|_| "jwks lock")?;
            if now - w.1 < JWKS_REFETCH_S {
                return Err("unknown signing key".into());
            }
            w.1 = now;
        }
        let d = self.discovery().await?;
        #[derive(Deserialize)]
        struct Set {
            keys: Vec<Jwk>,
        }
        let set: Set = self
            .http
            .get(&d.jwks_uri)
            .send()
            .await
            .map_err(|e| format!("provider keys not reached: {}", e.without_url()))?
            .json()
            .await
            .map_err(|_| "provider keys are not JSON".to_string())?;
        let k = find(&set.keys);
        if let Ok(mut w) = self.jwks.write() {
            *w = (set.keys, now);
        }
        k.ok_or_else(|| "unknown signing key".into())
    }

    /// Verify a JWT from this provider and map it to an identity
    /// (REQ-AUTH-002). `nonce`: the ID token's expected nonce, if any.
    pub async fn verify(
        &self,
        jwt: &str,
        nonce: Option<&str>,
        now: i64,
    ) -> Result<Identity, String> {
        let parts: Vec<&str> = jwt.split('.').collect();
        if parts.len() != 3 {
            return Err("not a JWT".into());
        }
        let header: Value =
            serde_json::from_slice(&B64.decode(parts[0]).map_err(|_| "bad header")?)
                .map_err(|_| "bad header")?;
        let claims: Value =
            serde_json::from_slice(&B64.decode(parts[1]).map_err(|_| "bad claims")?)
                .map_err(|_| "bad claims")?;
        let sig = B64.decode(parts[2]).map_err(|_| "bad signature")?;
        if !header["crit"].is_null() {
            return Err("critical header extensions are not understood".into());
        }
        let alg = header["alg"].as_str().unwrap_or_default();
        let key = self.key(header["kid"].as_str(), now).await?;
        let msg = format!("{}.{}", parts[0], parts[1]);
        let d = |s: &Option<String>| s.as_deref().and_then(|v| B64.decode(v).ok());
        let ok = match (alg, key.kty.as_str()) {
            ("RS256", "RSA") => match (d(&key.n), d(&key.e)) {
                (Some(n), Some(e)) => ring::signature::RsaPublicKeyComponents { n: &n, e: &e }
                    .verify(
                        &ring::signature::RSA_PKCS1_2048_8192_SHA256,
                        msg.as_bytes(),
                        &sig,
                    )
                    .is_ok(),
                _ => false,
            },
            ("ES256", "EC") if key.crv.as_deref() == Some("P-256") => {
                match (d(&key.x), d(&key.y)) {
                    (Some(x), Some(y)) if x.len() == 32 && y.len() == 32 => {
                        let mut pt = vec![4u8];
                        pt.extend(x);
                        pt.extend(y);
                        ring::signature::UnparsedPublicKey::new(
                            &ring::signature::ECDSA_P256_SHA256_FIXED,
                            pt,
                        )
                        .verify(msg.as_bytes(), &sig)
                        .is_ok()
                    }
                    _ => false,
                }
            }
            // "none", HS256 (a shared secret warte does not have) and the rest
            _ => return Err(format!("signature algorithm {alg:?} is not accepted")),
        };
        if !ok {
            return Err("signature does not verify".into());
        }
        if claims["iss"].as_str() != Some(self.cfg.issuer.as_str()) {
            return Err("issued by another issuer".into());
        }
        let auds: Vec<&str> = match &claims["aud"] {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(|v| v.as_str()).collect(),
            _ => vec![],
        };
        let mine = auds
            .iter()
            .any(|a| *a == self.cfg.client_id || self.cfg.project_id.as_deref() == Some(*a));
        if !mine {
            return Err("issued for another audience".into());
        }
        let exp = claims["exp"].as_i64().ok_or("no expiry")?;
        if now > exp + LEEWAY_S {
            return Err("expired".into());
        }
        if let Some(n) = nonce {
            if claims["nonce"].as_str() != Some(n) {
                return Err("nonce does not match".into());
            }
        }
        let role = self.role(&claims).ok_or("no warte role for this login")?;
        let sub = claims["sub"].as_str().ok_or("no subject")?.to_string();
        let name = ["preferred_username", "name", "email"]
            .iter()
            .find_map(|k| claims[*k].as_str())
            .unwrap_or(&sub)
            .chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect();
        Ok(Identity { sub, name, role })
    }

    /// The highest warte role the token's project roles map to. With a
    /// project id only that project's roles count: the generic claim holds
    /// the roles of whichever project asked for the token.
    fn role(&self, claims: &Value) -> Option<Role> {
        let claim = match &self.cfg.project_id {
            Some(pid) => format!("urn:zitadel:iam:org:project:{pid}:roles"),
            None => "urn:zitadel:iam:org:project:roles".to_string(),
        };
        let keys: Vec<String> = claims[claim.as_str()]
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        keys.iter()
            .filter_map(|k| match self.cfg.roles.get(k) {
                Some(r) => Role::parse(r),
                None if self.cfg.roles.is_empty() => Role::parse(k),
                None => None,
            })
            .max()
    }

    // ---------- the hand-over (REQ-AUTH-003) ----------

    /// Where to send the browser to log in, and the login cookie's value.
    pub async fn login_url(&self, back: &str, now: i64) -> Result<(String, String), String> {
        let d = self.discovery().await?;
        let state = random(24);
        let verifier = random(48);
        let nonce = random(24);
        let url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}&code_challenge={}&code_challenge_method=S256",
            d.authorization_endpoint,
            enc(&self.cfg.client_id),
            enc(&self.redirect_uri()),
            enc(&self.scope()),
            enc(&state),
            enc(&nonce),
            challenge(&verifier)
        );
        let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        p.retain(|_, v| now - v.created < PENDING_S);
        // a flood of /auth/login pushes out the oldest; it never locks out
        while p.len() >= PENDING_MAX {
            let oldest = p
                .iter()
                .min_by_key(|(_, v)| v.created)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => p.remove(&k),
                None => break,
            };
        }
        let bound = state_hash(&state);
        p.insert(
            state,
            Pending {
                verifier,
                nonce,
                created: now,
                back: local_path(back),
            },
        );
        Ok((url, bound))
    }

    fn redirect_uri(&self) -> String {
        format!("{}/auth/callback", self.cfg.base_url.trim_end_matches('/'))
    }

    fn scope(&self) -> String {
        let mut s = "openid profile email".to_string();
        if let Some(pid) = &self.cfg.project_id {
            s += &format!(
                " urn:zitadel:iam:org:project:id:{pid}:aud urn:zitadel:iam:org:projects:roles"
            );
        }
        s
    }

    /// The provider sent the browser back: check the state, trade the code,
    /// verify the ID token. `bound`: the login cookie this browser holds.
    /// Returns the session id and where to go.
    pub async fn callback(
        &self,
        code: &str,
        state: &str,
        bound: Option<&str>,
        now: i64,
    ) -> Result<(String, String), String> {
        // the browser that comes back must be the one that set out (login CSRF)
        if bound != Some(state_hash(state).as_str()) {
            return Err("this browser did not start that login".into());
        }
        let p = {
            let mut g = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            g.remove(state).ok_or("unknown or used login")?
        };
        if now - p.created >= PENDING_S {
            return Err("the login took too long".into());
        }
        let d = self.discovery().await?;
        let r = self
            .http
            .post(&d.token_endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &self.redirect_uri()),
                ("client_id", &self.cfg.client_id),
                ("code_verifier", &p.verifier),
            ])
            .send()
            .await
            .map_err(|e| format!("provider not reached: {}", e.without_url()))?;
        if !r.status().is_success() {
            return Err(format!(
                "the provider refused the code ({})",
                r.status().as_u16()
            ));
        }
        let t: Value = r.json().await.map_err(|_| "the token answer is not JSON")?;
        let id_token = t["id_token"].as_str().ok_or("no ID token")?;
        let who = self.verify(id_token, Some(&p.nonce), now).await?;
        let exp = serde_json::from_slice::<Value>(
            &B64.decode(id_token.split('.').nth(1).unwrap_or_default())
                .unwrap_or_default(),
        )
        .ok()
        .and_then(|c| c["exp"].as_i64())
        .unwrap_or(now);
        let expires = exp.min(now + self.cfg.session_hours * 3600);
        if expires <= now {
            return Err("the login has already expired".into());
        }
        let id = random(32);
        let mut s = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        s.retain(|_, v| v.expires > now);
        if s.len() >= SESSIONS_MAX {
            return Err("too many sessions".into());
        }
        s.insert(id.clone(), Session { who, expires });
        Ok((id, p.back))
    }

    pub fn session(&self, id: &str, now: i64) -> Option<Identity> {
        let s = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        s.get(id).filter(|v| v.expires > now).map(|v| v.who.clone())
    }

    pub fn drop_session(&self, id: &str) {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    pub fn cookie(&self, id: &str, max_age: i64) -> String {
        format!(
            "{COOKIE}={id}; Path={}; HttpOnly; Secure; SameSite=Lax; Max-Age={max_age}",
            self.base_path()
        )
    }

    pub async fn logout_url(&self) -> String {
        let back = format!("{}/", self.cfg.base_url.trim_end_matches('/'));
        match self
            .discovery()
            .await
            .ok()
            .and_then(|d| d.end_session_endpoint)
        {
            Some(e) => format!(
                "{e}?client_id={}&post_logout_redirect_uri={}",
                enc(&self.cfg.client_id),
                enc(&back)
            ),
            None => back,
        }
    }
}

// ---------- which route needs what (REQ-AUTH-001) ----------

/// A read route's data class.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Class {
    /// Fleet status: anyone on the LAN.
    Public,
    /// Personal or team data: a login.
    SignedIn,
    /// A login, or a warte agent token carrying this right (a scheduled job
    /// that reads what it writes back).
    SignedInOrRight(&'static str),
}

/// Read routes (GET) a login is needed for. Every other GET route is public;
/// tests/auth.rs fails when a route is in neither list.
pub const SIGNED_IN: &[&str] = &[
    "/api/costs",
    "/api/skill-usage",
    "/api/agents",
    "/api/agents/{id}",
    "/api/agents/{id}/data",
    "/api/access/list/{agent_id}",
    "/api/artifacts",
    "/api/artifacts/{id}/download",
    "/api/sync/state",
    "/api/dashboard/costs-partial",
    "/api/dashboard/skills-partial",
    "/api/dashboard/briefing-partial",
    "/api/dashboard/agents-partial",
    "/api/dashboard/artifacts-partial",
    "/api/dashboard/ask-partial",
    "/api/dashboard/ask-live",
    "/api/dashboard/friction-partial",
    "/api/dashboard/health-partial",
];

/// Read routes a scheduled job reads with its write right.
pub const SIGNED_IN_OR_RIGHT: &[(&str, &str)] = &[
    ("/api/briefing", "briefing:write"),
    ("/api/briefing/facts", "briefing:write"),
    ("/api/triage", "triage:write"),
];

/// Read routes that stay open with `[auth]` (fleet status).
pub const PUBLIC: &[&str] = &[
    "/",
    "/dashboard.css",
    "/htmx.min.js",
    "/idle.js",
    "/airport.js",
    "/auth/login",
    "/auth/callback",
    "/auth/logout",
    "/api/me",
    "/api/health",
    "/api/admission",
    "/api/eta",
    "/api/queue",
    "/api/watch",
    "/api/status.md",
    "/api/report",
    "/api/report.md",
    "/api/pressure",
    "/api/fleet",
    "/api/jobs",
    "/api/machines",
    "/api/history/fleet",
    "/api/history/machines",
    "/api/history/machines/{host}/metrics",
    "/api/history/machines/{host}/series",
    "/api/openapi.json",
    "/api/airport",
    "/api/compliance",
    "/api/dashboard/overview",
    "/api/dashboard/overview-partial",
    "/api/dashboard/airport-partial",
    "/api/dashboard/machines-partial",
    "/api/dashboard/machine-history-partial",
    "/api/dashboard/jobs-partial",
    "/api/dashboard/compliance-partial",
    "/api/dashboard/queue-partial",
    "/api/dashboard/ci-history-partial",
    "/api/dashboard/critical-path-partial",
    "/api/dashboard/resources-partial",
    "/api/dashboard/outages-partial",
    "/api/dashboard/schedule-partial",
];

fn matches(pattern: &str, path: &str) -> bool {
    let a: Vec<&str> = pattern.split('/').collect();
    let b: Vec<&str> = path.split('/').collect();
    a.len() == b.len()
        && a.iter()
            .zip(&b)
            .all(|(p, s)| (p.starts_with('{') && p.ends_with('}') && !s.is_empty()) || p == s)
}

/// The class of a GET path. Unknown paths are signed-in: a route added
/// without a class fails closed (and the test names it).
pub fn class_of(path: &str) -> Class {
    if PUBLIC.iter().any(|p| matches(p, path)) {
        return Class::Public;
    }
    if let Some((_, r)) = SIGNED_IN_OR_RIGHT.iter().find(|(p, _)| matches(p, path)) {
        return Class::SignedInOrRight(r);
    }
    Class::SignedIn
}

/// The sign-in panel a signed-in view shows to someone not signed in.
pub fn sign_in_panel() -> String {
    r#"<div class="overview"><h2>Sign in</h2><p class="empty">This view shows personal data. <a href="auth/login">Sign in</a> to see it.</p></div>"#.into()
}

// ---------- the gate and the routes ----------

use axum::extract::{Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};

fn named_cookie(h: &HeaderMap, name: &str) -> Option<String> {
    h.get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| {
            kv.trim()
                .strip_prefix(&format!("{name}="))
                .map(String::from)
        })
}

fn cookie_value(h: &HeaderMap) -> Option<String> {
    named_cookie(h, COOKIE)
}

fn bearer(h: &HeaderMap) -> Option<String> {
    h.get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(String::from)
}

fn deny(path: &str, code: StatusCode) -> Response {
    if path.starts_with("/api/dashboard/") {
        // a view inside the page: say how to get in, not a bare error
        return Html(sign_in_panel()).into_response();
    }
    (
        code,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        if code == StatusCode::FORBIDDEN {
            "this login has no warte role for that"
        } else {
            "sign in: a session, or a bearer token from the provider"
        },
    )
        .into_response()
}

/// Let a read through (`None`), or the answer that refuses it
/// (REQ-AUTH-001/002). When it is let through, the identity, if any, is in
/// the request's extensions.
pub async fn gate(
    l: &Login,
    tokens: &crate::auth::TokenManager,
    req: &mut Request,
) -> Option<Response> {
    let path = req.uri().path().to_string();
    let class = class_of(&path);
    if class == Class::Public {
        return None;
    }
    let now = crate::hub::now();
    if let Some(who) = cookie_value(req.headers()).and_then(|id| l.session(&id, now)) {
        req.extensions_mut().insert(who);
        return None;
    }
    let Some(b) = bearer(req.headers()) else {
        return Some(deny(&path, StatusCode::UNAUTHORIZED));
    };
    if b.starts_with("warte1.") {
        // a warte agent token reads only what its job writes back
        let ok = matches!(class, Class::SignedInOrRight(r)
            if tokens.verify_token(&b).is_ok_and(|p| p.rights.iter().any(|x| x == r)));
        return (!ok).then(|| deny(&path, StatusCode::FORBIDDEN));
    }
    match l.verify(&b, None, now).await {
        Ok(who) => {
            req.extensions_mut().insert(who);
            None
        }
        Err(e) if e.starts_with("no warte role") => Some(deny(&path, StatusCode::FORBIDDEN)),
        Err(_) => Some(deny(&path, StatusCode::UNAUTHORIZED)),
    }
}

#[derive(Deserialize)]
pub struct LoginQuery {
    #[serde(default)]
    back: Option<String>,
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

fn redirect(to: &str, cookie: Option<String>) -> Response {
    let mut r = (StatusCode::SEE_OTHER, [(header::LOCATION, to.to_string())]).into_response();
    if let Some(c) = cookie {
        if let Ok(v) = c.parse() {
            r.headers_mut().insert(header::SET_COOKIE, v);
        }
    }
    r.headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    r
}

fn no_auth() -> Response {
    (StatusCode::NOT_FOUND, "no [auth] configured").into_response()
}

pub async fn login_route(
    State(s): State<crate::server::AppState>,
    Query(q): Query<LoginQuery>,
) -> Response {
    let Some(l) = s.login.as_deref() else {
        return no_auth();
    };
    let back = q.back.unwrap_or_else(|| "/".into());
    match l.login_url(&back, crate::hub::now()).await {
        Ok((u, bound)) => redirect(
            &u,
            Some(format!(
                "{LOGIN_COOKIE}={bound}; Path={}; HttpOnly; Secure; SameSite=Lax; Max-Age={PENDING_S}",
                l.base_path()
            )),
        ),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

pub async fn callback_route(
    State(s): State<crate::server::AppState>,
    Query(q): Query<CallbackQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(l) = s.login.as_deref() else {
        return no_auth();
    };
    if q.error.is_some() {
        return (StatusCode::UNAUTHORIZED, "the provider did not log you in").into_response();
    }
    let (Some(code), Some(state)) = (q.code, q.state) else {
        return (StatusCode::BAD_REQUEST, "no code or state").into_response();
    };
    let now = crate::hub::now();
    let bound = named_cookie(&headers, LOGIN_COOKIE);
    match l.callback(&code, &state, bound.as_deref(), now).await {
        Ok((id, back)) => {
            let max = l.cfg.session_hours * 3600;
            let to = format!("{}{}", l.base_path().trim_end_matches('/'), back);
            redirect(&to, Some(l.cookie(&id, max)))
        }
        // the reason names a check, never a token
        Err(e) => (StatusCode::UNAUTHORIZED, format!("login refused: {e}")).into_response(),
    }
}

pub async fn logout_route(
    State(s): State<crate::server::AppState>,
    headers: HeaderMap,
) -> Response {
    let Some(l) = s.login.as_deref() else {
        return no_auth();
    };
    if let Some(id) = cookie_value(&headers) {
        l.drop_session(&id);
    }
    let to = l.logout_url().await;
    redirect(&to, Some(l.cookie("", 0)))
}

/// Who the browser is signed in as, for the header: `{auth: false}` without
/// `[auth]`, `{auth: true, user: null}` when not signed in.
pub async fn me_route(State(s): State<crate::server::AppState>, headers: HeaderMap) -> Response {
    let Some(l) = s.login.as_deref() else {
        return axum::Json(serde_json::json!({"auth": false})).into_response();
    };
    let who = cookie_value(&headers).and_then(|id| l.session(&id, crate::hub::now()));
    axum::Json(serde_json::json!({"auth": true, "user": who})).into_response()
}

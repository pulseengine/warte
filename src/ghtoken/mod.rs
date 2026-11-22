//! Per-agent GitHub tokens (REQ-GHTOKEN-001..004, #189).
//!
//! An agent exchanges its warte bearer token (right `github:token`, bound to
//! its own name) for a GitHub App installation token that lasts one hour and
//! reaches only the repositories the operator assigned to it on the host.
//! warte signs the App JWT with the App's private key, which the operator
//! places on the host; warte never generates, uploads or logs it.
//!
//! Nothing is kept at rest: issued tokens live in memory only, so that an
//! agent removed or re-scoped on the host has its live tokens revoked within
//! a minute (`sweep`). The audit log names the agent, repositories,
//! permissions, expiry and outcome; never the token, nor a hash of it.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Most repositories one agent may be assigned, and one token may reach.
pub const MAX_REPOS: usize = 10;
/// A live token with more than this left is handed out again instead of a
/// new one being minted.
pub const REUSE_MIN_LEFT_S: i64 = 15 * 60;
/// How often live tokens are checked against the agents and assignments.
pub const SWEEP_EVERY_S: u64 = 60;

/// The permissions a token may carry, and the most each may be. `actions`
/// write and `workflows` are never in the configured set: the operator grants
/// `actions:write` and `workflows:write` per agent (see [`Grant`]).
const CEILING: [(&str, &str); 5] = [
    ("contents", "write"),
    ("pull_requests", "write"),
    ("issues", "write"),
    ("actions", "read"),
    ("metadata", "read"),
];

fn level(v: &str) -> Option<u8> {
    match v {
        "read" => Some(1),
        "write" => Some(2),
        _ => None,
    }
}

fn default_api() -> String {
    "https://api.github.com".into()
}
fn default_permissions() -> BTreeMap<String, String> {
    CEILING
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}
fn default_per_agent() -> u32 {
    12
}
fn default_global() -> u32 {
    300
}

/// `[github_app]` in warte's config. Without it the route answers 404.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GithubAppConfig {
    /// The App's id.
    pub app_id: u64,
    /// The installation whose tokens are minted.
    pub installation_id: u64,
    /// The account (organisation or user) the installation belongs to; every
    /// repository a token reaches must be under it.
    pub account: String,
    /// The App's private key (PEM, PKCS#1 or PKCS#8), readable by the owner only.
    pub key_path: PathBuf,
    #[serde(default = "default_api")]
    pub api_base: String,
    /// The permissions every token carries; may only lower the ceiling.
    #[serde(default = "default_permissions")]
    pub permissions: BTreeMap<String, String>,
    #[serde(default = "default_per_agent")]
    pub per_agent_per_hour: u32,
    #[serde(default = "default_global")]
    pub global_per_hour: u32,
}

impl GithubAppConfig {
    /// Refuse a plain-http API, a permission above the ceiling, or limits
    /// that make no sense.
    pub fn check(&self) -> Result<(), String> {
        let loopback = ["http://127.0.0.1:", "http://localhost:"]
            .iter()
            .any(|p| self.api_base.starts_with(p));
        if !self.api_base.starts_with("https://") && !loopback {
            return Err("[github_app] api_base must be https (plain http only on loopback)".into());
        }
        if !valid_part(&self.account) {
            return Err("[github_app] account is not a GitHub account name".into());
        }
        if self.permissions.is_empty() {
            return Err("[github_app] permissions is empty".into());
        }
        for (k, v) in &self.permissions {
            let Some((_, max)) = CEILING.iter().find(|(c, _)| c == k) else {
                if k == "workflows" {
                    return Err(
                        "[github_app] workflows is never in the configured set; grant workflows:write per agent with `warte agents repos grant`".into(),
                    );
                }
                return Err(format!(
                    "[github_app] permission {k:?} is not one of {}",
                    CEILING.map(|(c, _)| c).join(", ")
                ));
            };
            match (level(v), level(max)) {
                (Some(a), Some(b)) if a <= b => {}
                (Some(_), _) => {
                    return Err(format!(
                        "[github_app] {k} may be at most {max}{}",
                        if *k == "actions" {
                            "; actions write is granted per agent with `warte agents repos grant`"
                        } else {
                            ""
                        }
                    ))
                }
                _ => return Err(format!("[github_app] {k} must be read or write")),
            }
        }
        if !(1..=60).contains(&self.per_agent_per_hour) {
            return Err("[github_app] per_agent_per_hour must be 1..=60".into());
        }
        if !(1..=5000).contains(&self.global_per_hour) {
            return Err("[github_app] global_per_hour must be 1..=5000".into());
        }
        Ok(())
    }
}

/// One part of `owner/name`: 1 to 100 of `A-Za-z0-9_.-`, not `.` or `..`.
fn valid_part(s: &str) -> bool {
    (1..=100).contains(&s.len())
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

/// Is `s` an `owner/name` repository?
pub fn valid_repo(s: &str) -> bool {
    matches!(s.split_once('/'), Some((o, n)) if valid_part(o) && valid_part(n))
}

/// Read the App's private key. A key other users can read is refused; no
/// error quotes the file's contents.
pub fn load_key(path: &Path) -> Result<ring::rsa::KeyPair, String> {
    let shown = path.display();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map_err(|e| format!("{shown}: {}", e.kind()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(format!(
                "{shown} is readable by other users (mode {:o}); chmod 600 it",
                mode & 0o777
            ));
        }
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{shown}: {}", e.kind()))?;
    let block = |kind: &str| -> Option<Vec<u8>> {
        let begin = format!("-----BEGIN {kind}-----");
        let end = format!("-----END {kind}-----");
        let a = text.find(&begin)? + begin.len();
        let b = a + text[a..].find(&end)?;
        let body: String = text[a..b].chars().filter(|c| !c.is_whitespace()).collect();
        STANDARD.decode(body).ok()
    };
    let key = if let Some(der) = block("RSA PRIVATE KEY") {
        ring::rsa::KeyPair::from_der(&der)
    } else if let Some(der) = block("PRIVATE KEY") {
        ring::rsa::KeyPair::from_pkcs8(&der)
    } else {
        return Err(format!("{shown} holds no PEM RSA private key"));
    };
    key.map_err(|_| format!("{shown} is not a usable RSA private key"))
}

/// The App JWT GitHub asks for: RS256, issued a minute back for clock skew,
/// valid nine minutes (GitHub's limit is ten).
pub fn app_jwt(key: &ring::rsa::KeyPair, app_id: u64, now: i64) -> Result<String, String> {
    let head = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = serde_json::json!({"iat": now - 60, "exp": now + 540, "iss": app_id.to_string()});
    let body = URL_SAFE_NO_PAD.encode(claims.to_string());
    let msg = format!("{head}.{body}");
    let mut sig = vec![0u8; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        msg.as_bytes(),
        &mut sig,
    )
    .map_err(|_| "signing the App JWT failed".to_string())?;
    Ok(format!("{msg}.{}", URL_SAFE_NO_PAD.encode(sig)))
}

/// The agent name a warte token names, read without verifying it: for the
/// client, which needs the name for the path. The server verifies.
pub fn token_agent(token: &str) -> Option<String> {
    let mut parts = token.split('.');
    if parts.next()? != "warte1" {
        return None;
    }
    let payload = URL_SAFE_NO_PAD.decode(parts.next()?).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    v.get("agent_id")?.as_str().map(str::to_string)
}

/// A permission the operator may grant one agent beyond the configured set.
/// Nothing else can be granted per agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// `actions: write`, to re-run jobs.
    ActionsWrite,
    /// `workflows: write`, to push changes under `.github/workflows/`
    /// (REQ-GHTOKEN-005; owner decision 2026-11-22).
    WorkflowsWrite,
}

impl Grant {
    /// The CLI spelling, `actions:write` or `workflows:write`.
    pub fn parse(s: &str) -> Option<Grant> {
        match s {
            "actions:write" => Some(Grant::ActionsWrite),
            "workflows:write" => Some(Grant::WorkflowsWrite),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Grant::ActionsWrite => "actions:write",
            Grant::WorkflowsWrite => "workflows:write",
        }
    }
}

/// What the operator assigned to one agent.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Assignment {
    pub repos: Vec<String>,
    pub actions_write: bool,
    pub workflows_write: bool,
}

/// The operator-set scope, in warte's database (REQ-GHTOKEN-002).
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

fn db(e: sqlx::Error) -> String {
    format!("database error: {e}")
}

impl Store {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, name: &str) -> Result<Option<Assignment>, String> {
        let row: Option<(String, i64, i64)> = sqlx::query_as(
            "SELECT repos, actions_write, workflows_write FROM agent_github WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.map(|(r, a, w)| Assignment {
            repos: serde_json::from_str(&r).unwrap_or_default(),
            actions_write: a != 0,
            workflows_write: w != 0,
        }))
    }

    pub async fn list(&self) -> Result<Vec<(String, Assignment)>, String> {
        let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
            "SELECT name, repos, actions_write, workflows_write FROM agent_github ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows
            .into_iter()
            .map(|(n, r, a, w)| {
                let repos = serde_json::from_str(&r).unwrap_or_default();
                (
                    n,
                    Assignment {
                        repos,
                        actions_write: a != 0,
                        workflows_write: w != 0,
                    },
                )
            })
            .collect())
    }

    /// Replace an agent's repositories (1..=MAX_REPOS, each `owner/name`,
    /// under `account` when one is configured). Keeps its grants.
    pub async fn set_repos(
        &self,
        name: &str,
        repos: &[String],
        account: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let mut v: Vec<String> = Vec::new();
        for r in repos {
            if !valid_repo(r) {
                return Err(format!("{r:?} is not owner/name"));
            }
            if let Some(a) = account {
                if !r.split('/').next().unwrap_or("").eq_ignore_ascii_case(a) {
                    return Err(format!("{r} is not under the App's account {a}"));
                }
            }
            if !v.iter().any(|x| x.eq_ignore_ascii_case(r)) {
                v.push(r.clone());
            }
        }
        if v.is_empty() || v.len() > MAX_REPOS {
            return Err(format!("assign 1 to {MAX_REPOS} repositories"));
        }
        v.sort();
        sqlx::query(
            "INSERT INTO agent_github (name, repos, actions_write, updated_at) VALUES (?, ?, 0, ?)
             ON CONFLICT(name) DO UPDATE SET repos = excluded.repos, updated_at = excluded.updated_at",
        )
        .bind(name)
        .bind(serde_json::to_string(&v).unwrap_or_default())
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(v)
    }

    /// Remove an agent's assignment; its live tokens are revoked by the next sweep.
    pub async fn clear(&self, name: &str) -> Result<bool, String> {
        let r = sqlx::query("DELETE FROM agent_github WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(r.rows_affected() > 0)
    }

    /// Grant or take back `actions: write` for an agent that has repositories.
    pub async fn set_actions_write(&self, name: &str, on: bool) -> Result<(), String> {
        self.set_grant(name, Grant::ActionsWrite, on).await
    }

    /// Grant or take back `workflows: write` for an agent that has
    /// repositories (REQ-GHTOKEN-005).
    pub async fn set_workflows_write(&self, name: &str, on: bool) -> Result<(), String> {
        self.set_grant(name, Grant::WorkflowsWrite, on).await
    }

    /// Grant or take back one per-agent permission.
    pub async fn set_grant(&self, name: &str, grant: Grant, on: bool) -> Result<(), String> {
        // the column comes from a closed enum, never from input
        let sql = match grant {
            Grant::ActionsWrite => {
                "UPDATE agent_github SET actions_write = ?, updated_at = ? WHERE name = ?"
            }
            Grant::WorkflowsWrite => {
                "UPDATE agent_github SET workflows_write = ?, updated_at = ? WHERE name = ?"
            }
        };
        let r = sqlx::query(sql)
            .bind(on as i64)
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        if r.rows_affected() == 0 {
            return Err(format!(
                "{name} has no repositories assigned; set them first"
            ));
        }
        Ok(())
    }

    /// Is there an agents row with this name (a removed agent mints nothing)?
    pub async fn agent_exists(&self, name: &str) -> Result<bool, String> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM agents WHERE name = ? LIMIT 1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(row.is_some())
    }
}

/// The optional body of a token request: it may only narrow.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct TokenRequest {
    #[serde(default)]
    pub repositories: Option<Vec<String>>,
    #[serde(default)]
    pub permissions: Option<BTreeMap<String, String>>,
}

/// What the route returns. `token` is GitHub's installation token.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Issued {
    pub token: String,
    pub expires_at: String,
    pub repositories: Vec<String>,
    pub permissions: BTreeMap<String, String>,
}

/// Why a request was refused, with the HTTP status to answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Refused {
    pub status: u16,
    pub why: String,
}

fn refuse(status: u16, why: impl Into<String>) -> Refused {
    Refused {
        status,
        why: why.into(),
    }
}

#[derive(Clone)]
struct Live {
    token: String,
    expires_at: i64,
    expires_text: String,
    repos: Vec<String>,
    permissions: BTreeMap<String, String>,
}

/// The broker: one per warte with `[github_app]`.
pub struct Broker {
    cfg: GithubAppConfig,
    key: ring::rsa::KeyPair,
    http: reqwest::Client,
    store: Store,
    audit: PathBuf,
    live: Mutex<HashMap<String, Vec<Live>>>,
    issued: Mutex<VecDeque<(String, i64)>>,
}

/// GitHub's answer to an installation token request (the fields read).
#[derive(Deserialize)]
struct GhToken {
    token: String,
    expires_at: String,
    #[serde(default)]
    permissions: BTreeMap<String, String>,
}

impl Broker {
    /// Check the config and load the key. `dir` holds the audit log.
    pub fn new(cfg: GithubAppConfig, store: Store, dir: &Path) -> Result<Self, String> {
        cfg.check()?;
        let key = load_key(&cfg.key_path)?;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("warte/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(Self {
            cfg,
            key,
            http,
            store,
            audit: dir.join("github-tokens.jsonl"),
            live: Mutex::new(HashMap::new()),
            issued: Mutex::new(VecDeque::new()),
        })
    }

    fn log(
        &self,
        agent: &str,
        outcome: &str,
        repos: &[String],
        perms: &BTreeMap<String, String>,
        expires: Option<&str>,
        why: Option<&str>,
    ) {
        let line = serde_json::json!({
            "at": chrono::Utc::now().to_rfc3339(),
            "agent": agent,
            "outcome": outcome,
            "repos": repos,
            "permissions": perms,
            "expires_at": expires,
            "why": why,
        });
        use std::io::Write;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.audit);
        match file {
            Ok(mut f) => {
                let _ = writeln!(f, "{line}");
            }
            Err(e) => tracing::warn!("github-tokens audit: {}", e.kind()),
        }
    }

    fn refused(&self, agent: &str, r: Refused) -> Refused {
        self.log(agent, "refused", &[], &BTreeMap::new(), None, Some(&r.why));
        r
    }

    /// Exchange for `agent` (already authenticated and bound by the
    /// middleware): REQ-GHTOKEN-001..003.
    pub async fn mint(&self, agent: &str, req: TokenRequest, now: i64) -> Result<Issued, Refused> {
        let fail = |r| Err(self.refused(agent, r));
        match self.store.agent_exists(agent).await {
            Ok(true) => {}
            Ok(false) => return fail(refuse(403, "no agent of this name is registered")),
            Err(e) => {
                tracing::warn!("github-token: {e}");
                return fail(refuse(500, "database error"));
            }
        }
        let assigned = match self.store.get(agent).await {
            Ok(Some(a)) if !a.repos.is_empty() => a,
            Ok(_) => return fail(refuse(
                403,
                "no repositories are assigned to this agent; the operator assigns them on the host",
            )),
            Err(e) => {
                tracing::warn!("github-token: {e}");
                return fail(refuse(500, "database error"));
            }
        };
        let mut repos: Vec<String> = Vec::new();
        for r in req.repositories.unwrap_or_else(|| assigned.repos.clone()) {
            let Some(a) = assigned.repos.iter().find(|a| a.eq_ignore_ascii_case(&r)) else {
                return fail(refuse(403, format!("{r} is not assigned to this agent")));
            };
            if !repos.contains(a) {
                repos.push(a.clone());
            }
        }
        if repos.is_empty() || repos.len() > MAX_REPOS {
            return fail(refuse(
                400,
                format!("ask for 1 to {MAX_REPOS} repositories"),
            ));
        }
        for r in &repos {
            let owner = r.split('/').next().unwrap_or("");
            if !owner.eq_ignore_ascii_case(&self.cfg.account) {
                return fail(refuse(403, format!("{r} is not under the App's account")));
            }
        }
        repos.sort();

        let mut base = self.cfg.permissions.clone();
        if assigned.actions_write {
            base.insert("actions".into(), "write".into());
        }
        if assigned.workflows_write {
            base.insert("workflows".into(), "write".into());
        }
        let perms = match req.permissions {
            None => base.clone(),
            Some(want) => {
                for (k, v) in &want {
                    match (base.get(k).and_then(|b| level(b)), level(v)) {
                        (Some(b), Some(w)) if w <= b => {}
                        _ => {
                            return fail(refuse(
                                403,
                                format!("{k}: {v} is above what this agent may have"),
                            ))
                        }
                    }
                }
                if want.is_empty() {
                    return fail(refuse(400, "permissions is empty"));
                }
                want
            }
        };

        // A live token for the same scope with time left is handed out again.
        let reuse = self.live.lock().ok().and_then(|l| {
            l.get(agent)?
                .iter()
                .find(|x| {
                    x.repos == repos
                        && x.permissions == perms
                        && x.expires_at - now > REUSE_MIN_LEFT_S
                })
                .cloned()
        });
        if let Some(x) = reuse {
            self.log(agent, "reused", &repos, &perms, Some(&x.expires_text), None);
            return Ok(Issued {
                token: x.token,
                expires_at: x.expires_text,
                repositories: repos,
                permissions: perms,
            });
        }

        if let Ok(mut q) = self.issued.lock() {
            while q.front().is_some_and(|(_, t)| *t <= now - 3600) {
                q.pop_front();
            }
            let mine = q.iter().filter(|(a, _)| a == agent).count() as u32;
            if mine >= self.cfg.per_agent_per_hour {
                drop(q);
                return fail(refuse(
                    429,
                    "this agent's token limit for the hour is reached",
                ));
            }
            if q.len() as u32 >= self.cfg.global_per_hour {
                drop(q);
                return fail(refuse(429, "the token limit for the hour is reached"));
            }
            q.push_back((agent.to_string(), now));
        }

        let jwt = match app_jwt(&self.key, self.cfg.app_id, now) {
            Ok(j) => j,
            Err(e) => return fail(refuse(500, e)),
        };
        let names: Vec<&str> = repos.iter().filter_map(|r| r.split('/').nth(1)).collect();
        let url = format!(
            "{}/app/installations/{}/access_tokens",
            self.cfg.api_base.trim_end_matches('/'),
            self.cfg.installation_id
        );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&jwt)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&serde_json::json!({"repositories": names, "permissions": perms}))
            .send()
            .await;
        let resp = match resp {
            Ok(r) => r,
            Err(_) => return fail(refuse(502, "GitHub could not be reached")),
        };
        let status = resp.status();
        if status.as_u16() != 201 {
            return fail(refuse(502, format!("GitHub answered {}", status.as_u16())));
        }
        let got: GhToken = match resp.json().await {
            Ok(g) => g,
            Err(_) => return fail(refuse(502, "GitHub's answer did not parse")),
        };
        // Defence in depth: GitHub must not grant more than was asked.
        let over = got.permissions.iter().find(|(k, v)| {
            let asked = perms.get(*k).and_then(|a| level(a));
            let granted = level(v);
            !(k.as_str() == "metadata" && v.as_str() == "read")
                && !matches!((asked, granted), (Some(a), Some(g)) if g <= a)
        });
        if let Some((k, v)) = over {
            let why = format!("GitHub granted {k}: {v}, more than asked; revoked");
            self.revoke(&got.token).await;
            return fail(refuse(502, why));
        }
        let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&got.expires_at) else {
            self.revoke(&got.token).await;
            return fail(refuse(502, "GitHub's expiry did not parse; revoked"));
        };
        let live = Live {
            token: got.token.clone(),
            expires_at: exp.timestamp(),
            expires_text: got.expires_at.clone(),
            repos: repos.clone(),
            permissions: perms.clone(),
        };
        if let Ok(mut l) = self.live.lock() {
            l.entry(agent.to_string()).or_default().push(live);
        }
        self.log(agent, "issued", &repos, &perms, Some(&got.expires_at), None);
        Ok(Issued {
            token: got.token,
            expires_at: got.expires_at,
            repositories: repos,
            permissions: perms,
        })
    }

    async fn revoke(&self, token: &str) -> bool {
        let url = format!(
            "{}/installation/token",
            self.cfg.api_base.trim_end_matches('/')
        );
        match self
            .http
            .delete(&url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
        {
            Ok(r) => r.status().as_u16() == 204,
            Err(_) => false,
        }
    }

    /// How many live (unexpired) tokens warte holds, per agent.
    pub fn live_counts(&self, now: i64) -> BTreeMap<String, usize> {
        let Ok(l) = self.live.lock() else {
            return BTreeMap::new();
        };
        l.iter()
            .map(|(a, v)| (a.clone(), v.iter().filter(|x| x.expires_at > now).count()))
            .filter(|(_, n)| *n > 0)
            .collect()
    }

    /// Drop expired tokens; revoke those whose agent is gone or whose scope
    /// the operator narrowed since (REQ-GHTOKEN-003).
    pub async fn sweep(&self, now: i64) {
        let snapshot: Vec<(String, Vec<Live>)> = match self.live.lock() {
            Ok(mut l) => {
                for v in l.values_mut() {
                    v.retain(|x| x.expires_at > now);
                }
                l.retain(|_, v| !v.is_empty());
                l.iter().map(|(a, v)| (a.clone(), v.clone())).collect()
            }
            Err(_) => return,
        };
        for (agent, lives) in snapshot {
            let exists = self.store.agent_exists(&agent).await;
            let assigned = self.store.get(&agent).await;
            let (Ok(exists), Ok(assigned)) = (exists, assigned) else {
                continue; // a database error revokes nothing; next sweep retries
            };
            for x in lives {
                let still = exists
                    && assigned.as_ref().is_some_and(|a| {
                        x.repos.iter().all(|r| a.repos.contains(r))
                            && (a.actions_write
                                || x.permissions.get("actions").map(String::as_str)
                                    != Some("write"))
                            && (a.workflows_write || !x.permissions.contains_key("workflows"))
                    });
                if still {
                    continue;
                }
                let why = if !exists {
                    "the agent was removed"
                } else {
                    "the operator narrowed the agent's scope"
                };
                let done = self.revoke(&x.token).await;
                if let Ok(mut l) = self.live.lock() {
                    if let Some(v) = l.get_mut(&agent) {
                        v.retain(|y| y.token != x.token);
                    }
                }
                let why = if done {
                    why.to_string()
                } else {
                    format!(
                        "{why}; GitHub did not confirm, it expires at {}",
                        x.expires_text
                    )
                };
                self.log(
                    &agent,
                    "revoked",
                    &x.repos,
                    &x.permissions,
                    Some(&x.expires_text),
                    Some(&why),
                );
            }
        }
    }
}

/// Sweep every minute.
pub fn spawn(b: Arc<Broker>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(SWEEP_EVERY_S));
        loop {
            tick.tick().await;
            b.sweep(chrono::Utc::now().timestamp()).await;
        }
    });
}

/// The client side (REQ-GHTOKEN-004): ask warte at `base` for a token.
pub async fn fetch(
    base: &str,
    ca_cert: Option<&Path>,
    warte_token: &str,
    repos: &[String],
) -> Result<Issued, String> {
    let agent = token_agent(warte_token).ok_or("the warte token is not a warte1 token")?;
    let mut http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30));
    if let Some(p) = ca_cert {
        let pem = std::fs::read(p).map_err(|e| format!("{}: {}", p.display(), e.kind()))?;
        let certs = reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|_| format!("{}: not a PEM certificate", p.display()))?;
        for c in certs {
            http = http.add_root_certificate(c);
        }
    }
    let http = http.build().map_err(|e| format!("http client: {e}"))?;
    let url = format!(
        "{}/api/agents/{}/github-token",
        base.trim_end_matches('/'),
        agent
    );
    let mut body = serde_json::json!({});
    if !repos.is_empty() {
        body["repositories"] = serde_json::json!(repos);
    }
    let resp = http
        .post(&url)
        .bearer_auth(warte_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("warte could not be reached: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let why = resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or_default();
        return Err(format!("warte answered {}: {why}", status.as_u16()));
    }
    resp.json()
        .await
        .map_err(|_| "warte's answer did not parse".into())
}

/// git's credential-helper protocol, `get` only: answer for https on
/// github.com, stay silent for anything else (git then asks the next helper).
pub fn git_credential_wants(input: &str) -> bool {
    let mut protocol = None;
    let mut host = None;
    for line in input.lines() {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once('=') {
            match k {
                "protocol" => protocol = Some(v),
                "host" => host = Some(v),
                _ => {}
            }
        }
    }
    protocol == Some("https") && host.is_some_and(|h| h.eq_ignore_ascii_case("github.com"))
}

/// The credential-helper answer for a token.
pub fn git_credential_answer(token: &str) -> String {
    format!("username=x-access-token\npassword={token}\n")
}

/// The token for the client: `--token`, else the first line of `--token-file`.
pub fn read_warte_token(token: Option<String>, file: Option<&Path>) -> Result<String, String> {
    let t = match (token, file) {
        (Some(t), _) => t,
        (None, Some(p)) => std::fs::read_to_string(p)
            .map_err(|e| format!("{}: {}", p.display(), e.kind()))?
            .lines()
            .next()
            .unwrap_or("")
            .to_string(),
        (None, None) => {
            return Err("give --token or --token-file (or WARTE_TOKEN / WARTE_TOKEN_FILE)".into())
        }
    };
    let t = t.trim().to_string();
    if t.is_empty() || !t.is_ascii() || t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("the warte token is empty or has whitespace".into());
    }
    Ok(t)
}

/// The command-line halves: the operator's `warte agents repos` and the
/// agent's `warte agent github-token` / `git-credential`.
pub mod cli {
    use super::*;
    use crate::cli::{AgentClientCommands, ReposCommands};

    fn show(name: &str, a: &Assignment) {
        let grants: Vec<&str> = [
            (a.actions_write, "actions:write"),
            (a.workflows_write, "workflows:write"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, g)| *g)
        .collect();
        let grant = if grants.is_empty() {
            String::new()
        } else {
            format!("  ({})", grants.join(", "))
        };
        println!("{name}: {}{grant}", a.repos.join(" "));
    }

    /// `warte agents repos …`, on the host, against the database directly.
    pub async fn repos(config_path: Option<&Path>, action: ReposCommands) -> i32 {
        let run = async {
            let config = crate::config::Config::load_at(config_path).map_err(|e| e.to_string())?;
            let account = config.github_app.as_ref().map(|g| g.account.clone());
            let db = crate::db::Database::connect(&config.db_path)
                .await
                .map_err(|e| e.to_string())?;
            let store = Store::new(db.pool().clone());
            match action {
                ReposCommands::List => {
                    for (n, a) in store.list().await? {
                        show(&n, &a);
                    }
                }
                ReposCommands::Show { name } => match store.get(&name).await? {
                    Some(a) => show(&name, &a),
                    None => return Err(format!("{name} has no repositories assigned")),
                },
                ReposCommands::Set { name, repos } => {
                    if !store.agent_exists(&name).await? {
                        eprintln!("note: no agent named {name} is registered yet");
                    }
                    let v = store.set_repos(&name, &repos, account.as_deref()).await?;
                    println!("{name}: {}", v.join(" "));
                }
                ReposCommands::Clear { name } => {
                    if !store.clear(&name).await? {
                        return Err(format!("{name} had no repositories assigned"));
                    }
                    println!("{name}: cleared; live tokens are revoked within a minute");
                }
                ReposCommands::Grant { name, permission }
                | ReposCommands::Revoke { name, permission }
                    if Grant::parse(&permission).is_none() =>
                {
                    return Err(format!(
                        "{name}: only actions:write and workflows:write can be granted per agent, not {permission:?}"
                    ));
                }
                ReposCommands::Grant { name, permission } => {
                    let g = Grant::parse(&permission).expect("checked above");
                    store.set_grant(&name, g, true).await?;
                    println!("{name}: {} granted", g.as_str());
                }
                ReposCommands::Revoke { name, permission } => {
                    let g = Grant::parse(&permission).expect("checked above");
                    store.set_grant(&name, g, false).await?;
                    println!(
                        "{name}: {} taken back; live tokens with it are revoked within a minute",
                        g.as_str()
                    );
                }
            }
            Ok::<(), String>(())
        };
        match run.await {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("warte agents repos: {e}");
                1
            }
        }
    }

    /// `warte agent …`, on the agent's machine. Nothing is written to a file.
    pub async fn client(action: AgentClientCommands) -> i32 {
        let (conn, repos, mode) = match action {
            AgentClientCommands::GithubToken { conn, repos, json } => {
                (conn, repos, if json { "json" } else { "token" })
            }
            AgentClientCommands::GitCredential { operation, conn } => {
                if operation != "get" {
                    return 0; // store / erase: nothing is kept
                }
                let mut input = String::new();
                use std::io::Read;
                let _ = std::io::stdin().take(64 * 1024).read_to_string(&mut input);
                if !git_credential_wants(&input) {
                    return 0;
                }
                (conn, Vec::new(), "git")
            }
        };
        let token = match read_warte_token(conn.token, conn.token_file.as_deref()) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("warte agent: {e}");
                return 1;
            }
        };
        match fetch(&conn.url, conn.ca_cert.as_deref(), &token, &repos).await {
            Ok(t) => {
                match mode {
                    "json" => println!("{}", serde_json::to_string(&t).unwrap_or_default()),
                    "git" => print!("{}", git_credential_answer(&t.token)),
                    _ => println!("{}", t.token),
                }
                0
            }
            Err(e) => {
                eprintln!("warte agent: {e}");
                1
            }
        }
    }
}

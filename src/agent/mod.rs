//! Agent registry — manages agent registrations, identity, and heartbeats.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;

/// How an agent proved its identity at registration.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
pub enum IdentityType {
    /// mTLS client certificate signed by the warte CA.
    Mtls,
    /// Ed25519 signed challenge-response.
    SignedChallenge,
    /// Hardware-backed attestation (TPM 2.0, Secure Enclave, Keystore).
    HardwareAttestation,
}

impl std::fmt::Display for IdentityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityType::Mtls => write!(f, "mtls"),
            IdentityType::SignedChallenge => write!(f, "signed-challenge"),
            IdentityType::HardwareAttestation => write!(f, "hardware-attestation"),
        }
    }
}

/// Health status of an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "healthy"),
            HealthStatus::Degraded => write!(f, "degraded"),
            HealthStatus::Unhealthy => write!(f, "unhealthy"),
            HealthStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// A registered agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub repo: Option<String>,
    pub identity_type: String,
    pub cert_expires_at: Option<DateTime<Utc>>,
    pub access_rights: Vec<String>,
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub health_status: HealthStatus,
    pub instance_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request to register a new agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    pub repo: Option<String>,
    pub identity_type: String,
    pub public_key: String,
}

/// Response to a registration request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub agent_id: String,
    pub cert_pem: Option<String>,
    pub cert_expires_at: Option<DateTime<Utc>>,
    pub access_rights: Vec<String>,
}

/// Agent registry backed by SQLite.
pub struct AgentRegistry {
    pool: SqlitePool,
    instance_id: String,
    ca: Option<Arc<crate::ca::CertificateAuthority>>,
}

impl AgentRegistry {
    pub fn new(pool: SqlitePool, instance_id: String) -> Self {
        Self {
            pool,
            instance_id,
            ca: None,
        }
    }

    /// Create a registry with a certificate authority for mTLS issuance.
    pub fn with_ca(
        pool: SqlitePool,
        instance_id: String,
        ca: Arc<crate::ca::CertificateAuthority>,
    ) -> Self {
        Self {
            pool,
            instance_id,
            ca: Some(ca),
        }
    }

    /// Register a new agent. Returns the agent ID and optionally a certificate.
    /// Register an agent by name (REQ-API-006). A name already registered
    /// gets its newest row back (by name only: an unconfigured instance id
    /// changes at every start), with a fresh certificate and
    /// the request's repo, identity type and key: registering twice never
    /// makes a second agent.
    pub async fn register(&self, req: RegisterRequest) -> anyhow::Result<RegisterResponse> {
        let existing: Option<(String, String)> = sqlx::query_as(
            "SELECT id, access_rights FROM agents WHERE name = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&req.name)
        .fetch_optional(&self.pool)
        .await?;
        let (agent_id, access_rights) = match &existing {
            Some((id, rights)) => (
                id.clone(),
                serde_json::from_str::<Vec<String>>(rights).unwrap_or_default(),
            ),
            None => (Uuid::now_v7().to_string(), vec![]),
        };
        let now = Utc::now();
        let cert_expires = now + Duration::hours(24);

        // Issue mTLS certificate if CA is available
        let (cert_pem, cert_expires_at) = if let Some(ref ca) = self.ca {
            let (bundle, expires) = ca.issue_agent_cert(&agent_id, &req.name).await?;
            (Some(bundle), Some(expires))
        } else {
            (None, Some(cert_expires))
        };

        if existing.is_some() {
            sqlx::query(
                r#"
                UPDATE agents
                SET repo = ?, identity_type = ?, public_key = ?, cert_pem = ?,
                    cert_expires_at = ?, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(&req.repo)
            .bind(&req.identity_type)
            .bind(req.public_key.as_bytes())
            .bind(cert_pem.as_ref().map(|s| s.as_bytes()))
            .bind(cert_expires_at.map(|d| d.to_rfc3339()))
            .bind(now.to_rfc3339())
            .bind(&agent_id)
            .execute(&self.pool)
            .await?;
            return Ok(RegisterResponse {
                agent_id,
                cert_pem,
                cert_expires_at,
                access_rights,
            });
        }

        sqlx::query(
            r#"
            INSERT INTO agents (id, name, repo, identity_type, public_key, cert_pem, cert_expires_at,
                                access_rights, health_status, instance_id, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, '[]', 'unknown', ?, ?, ?)
            "#,
        )
        .bind(&agent_id)
        .bind(&req.name)
        .bind(&req.repo)
        .bind(&req.identity_type)
        .bind(req.public_key.as_bytes())
        .bind(cert_pem.as_ref().map(|s| s.as_bytes()))
        .bind(cert_expires_at.map(|d| d.to_rfc3339()))
        .bind(&self.instance_id)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(RegisterResponse {
            agent_id,
            cert_pem,
            cert_expires_at,
            access_rights: vec![],
        })
    }

    /// Remove one agent row, run by the operator on the host (REQ-API-006,
    /// `warte agents rm`). Refused for an unknown id, for an agent that holds
    /// access policies (revoke them first), and for one with data rows unless
    /// `with_data`, which removes those rows too. Returns the agent's name and
    /// how many data rows went with it.
    pub async fn remove(&self, agent_id: &str, with_data: bool) -> Result<(String, u64), String> {
        let db = |e: sqlx::Error| format!("database error: {e}");
        let name: Option<(String,)> = sqlx::query_as("SELECT name FROM agents WHERE id = ?")
            .bind(agent_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        let Some((name,)) = name else {
            return Err(format!("no agent {agent_id}"));
        };
        let (rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM agent_data WHERE agent_id = ?")
            .bind(agent_id)
            .fetch_one(&self.pool)
            .await
            .map_err(db)?;
        let (policies,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM access_policies WHERE agent_id = ?")
                .bind(agent_id)
                .fetch_one(&self.pool)
                .await
                .map_err(db)?;
        if policies > 0 {
            return Err(format!(
                "agent {agent_id} holds {policies} access polic(ies): revoke them first"
            ));
        }
        if rows > 0 && !with_data {
            return Err(format!(
                "agent {agent_id} has {rows} data row(s): --with-data removes them too"
            ));
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("DELETE FROM agent_data WHERE agent_id = ?")
            .bind(agent_id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(agent_id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok((name, rows as u64))
    }

    /// Record a heartbeat from an agent.
    pub async fn heartbeat(&self, agent_id: &str) -> anyhow::Result<()> {
        self.heartbeat_with(agent_id, None).await
    }

    /// A heartbeat with the optional status of seam warte-plugin/heartbeat-status
    /// (ended, idle or working); None for a bodyless beat, which clears it.
    pub async fn heartbeat_with(&self, agent_id: &str, status: Option<&str>) -> anyhow::Result<()> {
        let now = Utc::now();
        sqlx::query(
            r#"
            UPDATE agents
            SET last_heartbeat = ?, health_status = 'healthy', updated_at = ?, heartbeat_status = ?
            WHERE id = ?
            "#,
        )
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(status)
        .bind(agent_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// List all registered agents.
    pub async fn list(&self) -> anyhow::Result<Vec<Agent>> {
        let rows = sqlx::query_as::<_, AgentRow>("SELECT * FROM agents ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.into_iter().map(Agent::from).collect())
    }

    /// Get a single agent by ID.
    pub async fn get(&self, agent_id: &str) -> anyhow::Result<Option<Agent>> {
        let row = sqlx::query_as::<_, AgentRow>("SELECT * FROM agents WHERE id = ?")
            .bind(agent_id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(Agent::from))
    }

    /// Mark agents as unhealthy if their heartbeat is stale (> 5 minutes).
    pub async fn mark_stale_agents(&self) -> anyhow::Result<usize> {
        let cutoff = (Utc::now() - Duration::minutes(5)).to_rfc3339();
        let result = sqlx::query(
            r#"
            UPDATE agents
            SET health_status = 'unhealthy'
            WHERE last_heartbeat < ? AND health_status != 'unhealthy'
            "#,
        )
        .bind(&cutoff)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() as usize)
    }
}

#[derive(sqlx::FromRow)]
struct AgentRow {
    id: String,
    name: String,
    repo: Option<String>,
    identity_type: String,
    cert_expires_at: Option<String>,
    access_rights: String,
    last_heartbeat: Option<String>,
    health_status: String,
    instance_id: String,
    created_at: String,
    updated_at: String,
}

impl From<AgentRow> for Agent {
    fn from(row: AgentRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            repo: row.repo,
            identity_type: row.identity_type,
            cert_expires_at: row.cert_expires_at.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|d| d.with_timezone(&Utc))
            }),
            access_rights: serde_json::from_str(&row.access_rights).unwrap_or_default(),
            last_heartbeat: row.last_heartbeat.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|d| d.with_timezone(&Utc))
            }),
            health_status: match row.health_status.as_str() {
                "healthy" => HealthStatus::Healthy,
                "degraded" => HealthStatus::Degraded,
                "unhealthy" => HealthStatus::Unhealthy,
                _ => HealthStatus::Unknown,
            },
            instance_id: row.instance_id,
            created_at: DateTime::parse_from_rfc3339(&row.created_at)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: DateTime::parse_from_rfc3339(&row.updated_at)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        }
    }
}

/// `warte agents rm`: on the host, against its own database, never over
/// HTTP (REQ-API-006). Exit 0 removed, 1 refused or failed (the reason on
/// stderr). The removal is logged with its reason in `agents-removed.jsonl`
/// next to the database.
pub async fn rm_cli(
    config_path: Option<&std::path::Path>,
    id: &str,
    reason: &str,
    with_data: bool,
) -> i32 {
    let reason = reason.trim();
    if reason.is_empty() || reason.chars().count() > 200 || reason.chars().any(char::is_control) {
        eprintln!("warte agents rm: --reason must be 1 to 200 characters, no control characters");
        return 1;
    }
    let run = async {
        let config = crate::config::Config::load_at(config_path).map_err(|e| e.to_string())?;
        let db = crate::db::Database::connect(&config.db_path)
            .await
            .map_err(|e| e.to_string())?;
        let reg = AgentRegistry::new(db.pool().clone(), config.instance_id());
        let (name, rows) = reg.remove(id, with_data).await?;
        let log = config
            .db_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("agents-removed.jsonl");
        let line = serde_json::json!({
            "at": Utc::now().to_rfc3339(),
            "id": id,
            "name": name,
            "data_rows": rows,
            "reason": reason,
        });
        use std::io::Write;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .and_then(|mut f| writeln!(f, "{line}"))
            .map_err(|e| format!("removed, but the log {} failed: {e}", log.display()))?;
        Ok::<_, String>(format!("removed agent {id} ({name}), {rows} data row(s)"))
    };
    match run.await {
        Ok(msg) => {
            println!("{msg}");
            0
        }
        Err(e) => {
            eprintln!("warte agents rm: {e}");
            1
        }
    }
}

/// `warte agents`: list this host's agents from the configured `db_path`,
/// never over HTTP, like `warte agents rm`. `/api/agents` needs `agents:read`,
/// and the host CLI holds no token. `health` matches case-insensitively.
/// Exit 0 listed (also when empty), 1 when the database is missing or
/// unreadable.
pub async fn list_cli(
    config_path: Option<&std::path::Path>,
    health: Option<&str>,
    json: bool,
) -> i32 {
    let run = async {
        let config = crate::config::Config::load_at(config_path).map_err(|e| e.to_string())?;
        if !config.db_path.exists() {
            return Err(format!("no database at {}", config.db_path.display()));
        }
        let db = crate::db::Database::connect(&config.db_path)
            .await
            .map_err(|e| e.to_string())?;
        let reg = AgentRegistry::new(db.pool().clone(), config.instance_id());
        let agents: Vec<Agent> = reg
            .list()
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|a| health.is_none_or(|h| a.health_status.to_string().eq_ignore_ascii_case(h)))
            .collect();
        if json {
            return serde_json::to_string_pretty(&agents).map_err(|e| e.to_string());
        }
        let mut out = format!(
            "{:<38} {:<12} {:<10} LAST SEEN\n",
            "AGENT ID", "NAME", "HEALTH"
        );
        for a in &agents {
            let seen = a
                .last_heartbeat
                .map_or_else(|| "never".to_string(), |t| t.to_rfc3339());
            out.push_str(&format!(
                "{:<38} {:<12} {:<10} {}\n",
                a.id, a.name, a.health_status, seen
            ));
        }
        Ok(out.trim_end().to_string())
    };
    match run.await {
        Ok(out) => {
            println!("{out}");
            0
        }
        Err(e) => {
            eprintln!("warte agents: {e}");
            1
        }
    }
}

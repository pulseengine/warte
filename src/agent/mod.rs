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
    pub async fn register(&self, req: RegisterRequest) -> anyhow::Result<RegisterResponse> {
        let agent_id = Uuid::now_v7().to_string();
        let now = Utc::now();
        let cert_expires = now + Duration::hours(24);

        // Issue mTLS certificate if CA is available
        let (cert_pem, cert_expires_at) = if let Some(ref ca) = self.ca {
            let (bundle, expires) = ca.issue_agent_cert(&agent_id, &req.name).await?;
            (Some(bundle), Some(expires))
        } else {
            (None, Some(cert_expires))
        };

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

    /// Record a heartbeat from an agent.
    pub async fn heartbeat(&self, agent_id: &str) -> anyhow::Result<()> {
        let now = Utc::now();
        sqlx::query(
            r#"
            UPDATE agents
            SET last_heartbeat = ?, health_status = 'healthy', updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
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

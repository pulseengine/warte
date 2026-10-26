//! Access control service — RBAC with grant/revoke and push-based revocation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashSet;
use tokio::sync::broadcast;
use uuid::Uuid;

/// An access right: resource × action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AccessRight {
    pub resource: String,
    pub action: String,
}

impl AccessRight {
    pub fn new(resource: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            resource: resource.into(),
            action: action.into(),
        }
    }
}

impl std::fmt::Display for AccessRight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.resource, self.action)
    }
}

/// A role with a set of access rights.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub name: String,
    pub rights: HashSet<AccessRight>,
}

/// Predefined roles.
pub fn default_roles() -> Vec<Role> {
    vec![
        Role {
            name: "admin".to_string(),
            rights: [AccessRight::new("*", "*")].into_iter().collect(),
        },
        Role {
            name: "operator".to_string(),
            rights: [
                AccessRight::new("agents", "read"),
                AccessRight::new("agents", "control"),
                AccessRight::new("access", "grant"),
                AccessRight::new("access", "revoke"),
                AccessRight::new("access", "read"),
                AccessRight::new("dashboard", "read"),
                AccessRight::new("artifacts", "read"),
            ]
            .into_iter()
            .collect(),
        },
        Role {
            name: "agent".to_string(),
            rights: [
                AccessRight::new("artifacts", "download"),
                AccessRight::new("data", "write"),
                AccessRight::new("heartbeat", "write"),
            ]
            .into_iter()
            .collect(),
        },
        Role {
            name: "viewer".to_string(),
            rights: [
                AccessRight::new("dashboard", "read"),
                AccessRight::new("agents", "read"),
            ]
            .into_iter()
            .collect(),
        },
    ]
}

/// Revocation event broadcast to all instances.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevocationEvent {
    pub agent_id: String,
    pub rights: Vec<AccessRight>,
    pub revoked_at: DateTime<Utc>,
    pub revoked_by: String,
    pub instance_id: String,
}

/// Access control service.
pub struct AccessControl {
    pool: SqlitePool,
    instance_id: String,
    revocation_tx: broadcast::Sender<RevocationEvent>,
}

impl AccessControl {
    pub fn new(pool: SqlitePool, instance_id: String) -> Self {
        let (revocation_tx, _) = broadcast::channel(256);
        Self {
            pool,
            instance_id,
            revocation_tx,
        }
    }

    /// Subscribe to revocation events.
    pub fn subscribe_revocations(&self) -> broadcast::Receiver<RevocationEvent> {
        self.revocation_tx.subscribe()
    }

    /// Grant access rights to an agent.
    pub async fn grant(
        &self,
        agent_id: &str,
        rights: &[AccessRight],
        granted_by: &str,
    ) -> anyhow::Result<()> {
        for right in rights {
            let id = Uuid::now_v7().to_string();
            sqlx::query(
                r#"
                INSERT INTO access_policies (id, agent_id, resource, action, granted_by)
                VALUES (?, ?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(agent_id)
            .bind(&right.resource)
            .bind(&right.action)
            .bind(granted_by)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// Revoke access rights from an agent. Push-based: broadcasts to all instances.
    pub async fn revoke(
        &self,
        agent_id: &str,
        rights: &[AccessRight],
        revoked_by: &str,
    ) -> anyhow::Result<()> {
        let now = Utc::now();

        for right in rights {
            sqlx::query(
                r#"
                UPDATE access_policies
                SET revoked_at = ?
                WHERE agent_id = ? AND resource = ? AND action = ? AND revoked_at IS NULL
                "#,
            )
            .bind(now.to_rfc3339())
            .bind(agent_id)
            .bind(&right.resource)
            .bind(&right.action)
            .execute(&self.pool)
            .await?;
        }

        // Broadcast revocation to all instances
        let event = RevocationEvent {
            agent_id: agent_id.to_string(),
            rights: rights.to_vec(),
            revoked_at: now,
            revoked_by: revoked_by.to_string(),
            instance_id: self.instance_id.clone(),
        };
        let _ = self.revocation_tx.send(event);

        Ok(())
    }

    /// Check if an agent has a specific access right.
    pub async fn check(
        &self,
        agent_id: &str,
        resource: &str,
        action: &str,
    ) -> anyhow::Result<bool> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM access_policies
            WHERE agent_id = ? AND resource = ? AND action = ? AND revoked_at IS NULL
            "#,
        )
        .bind(agent_id)
        .bind(resource)
        .bind(action)
        .fetch_one(&self.pool)
        .await?;

        // Also check wildcard
        if count.0 == 0 {
            let wildcard: (i64,) = sqlx::query_as(
                r#"
                SELECT COUNT(*) FROM access_policies
                WHERE agent_id = ? AND resource = '*' AND action = '*' AND revoked_at IS NULL
                "#,
            )
            .bind(agent_id)
            .fetch_one(&self.pool)
            .await?;
            return Ok(wildcard.0 > 0);
        }

        Ok(count.0 > 0)
    }

    /// List all active access rights for an agent.
    pub async fn list_rights(&self, agent_id: &str) -> anyhow::Result<Vec<AccessRight>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            r#"
            SELECT resource, action FROM access_policies
            WHERE agent_id = ? AND revoked_at IS NULL
            "#,
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(resource, action)| AccessRight { resource, action })
            .collect())
    }
}

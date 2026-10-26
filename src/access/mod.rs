//! Access control service — RBAC with OR-Set fail-safe revocation.
//!
//! OR-Set semantics: grants and tombstones are both additive. A right is
//! active if there exists a grant without a matching tombstone. Concurrent
//! grant+revoke resolves to REVOKED (tombstone wins) — fail-safe, never
//! fail-open. This is a security requirement (TS-003, R3 from research).

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

/// Access control service with OR-Set fail-safe revocation.
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
    ///
    /// OR-Set: grants are additive. A grant is a new row in access_policies.
    pub async fn grant(
        &self,
        agent_id: &str,
        rights: &[AccessRight],
        granted_by: &str,
    ) -> anyhow::Result<Vec<String>> {
        let mut policy_ids = Vec::new();
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
            policy_ids.push(id);
        }
        Ok(policy_ids)
    }

    /// Revoke access rights from an agent.
    ///
    /// OR-Set fail-safe: revocation adds a tombstone. A right is active only
    /// if there is a grant WITHOUT a matching tombstone. Concurrent grant+revoke
    /// resolves to REVOKED (tombstone wins) — never fail-open.
    pub async fn revoke(
        &self,
        agent_id: &str,
        rights: &[AccessRight],
        revoked_by: &str,
    ) -> anyhow::Result<()> {
        let now = Utc::now();

        for right in rights {
            // Find all active grants for this agent+resource+action
            let grants: Vec<(String,)> = sqlx::query_as(
                r#"
                SELECT ap.id FROM access_policies ap
                WHERE ap.agent_id = ? AND ap.resource = ? AND ap.action = ?
                  AND NOT EXISTS (
                      SELECT 1 FROM access_policy_tombstones t
                      WHERE t.policy_id = ap.id
                  )
                "#,
            )
            .bind(agent_id)
            .bind(&right.resource)
            .bind(&right.action)
            .fetch_all(&self.pool)
            .await?;

            // Add a tombstone for each active grant (OR-Set: additive)
            for (policy_id,) in grants {
                let tombstone_id = Uuid::now_v7().to_string();
                sqlx::query(
                    r#"
                    INSERT INTO access_policy_tombstones
                        (id, policy_id, agent_id, resource, action, revoked_at, revoked_by, instance_id)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                    "#,
                )
                .bind(&tombstone_id)
                .bind(&policy_id)
                .bind(agent_id)
                .bind(&right.resource)
                .bind(&right.action)
                .bind(now.to_rfc3339())
                .bind(revoked_by)
                .bind(&self.instance_id)
                .execute(&self.pool)
                .await?;
            }
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
    ///
    /// OR-Set: a right is active if there exists a grant without a tombstone.
    pub async fn check(
        &self,
        agent_id: &str,
        resource: &str,
        action: &str,
    ) -> anyhow::Result<bool> {
        // Check for active grant (grant exists, no tombstone)
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM access_policies ap
            WHERE ap.agent_id = ? AND ap.resource = ? AND ap.action = ?
              AND NOT EXISTS (
                  SELECT 1 FROM access_policy_tombstones t
                  WHERE t.policy_id = ap.id
              )
            "#,
        )
        .bind(agent_id)
        .bind(resource)
        .bind(action)
        .fetch_one(&self.pool)
        .await?;

        if count.0 > 0 {
            return Ok(true);
        }

        // Check wildcard
        let wildcard: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM access_policies ap
            WHERE ap.agent_id = ? AND ap.resource = '*' AND ap.action = '*'
              AND NOT EXISTS (
                  SELECT 1 FROM access_policy_tombstones t
                  WHERE t.policy_id = ap.id
              )
            "#,
        )
        .bind(agent_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(wildcard.0 > 0)
    }

    /// List all active access rights for an agent.
    pub async fn list_rights(&self, agent_id: &str) -> anyhow::Result<Vec<AccessRight>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            r#"
            SELECT DISTINCT ap.resource, ap.action FROM access_policies ap
            WHERE ap.agent_id = ?
              AND NOT EXISTS (
                  SELECT 1 FROM access_policy_tombstones t
                  WHERE t.policy_id = ap.id
              )
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

    /// List all tombstones for an agent (for sync/debugging).
    pub async fn list_tombstones(
        &self,
        agent_id: &str,
    ) -> anyhow::Result<Vec<(String, String, String)>> {
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            r#"
            SELECT resource, action, revoked_at FROM access_policy_tombstones
            WHERE agent_id = ?
            ORDER BY revoked_at DESC
            "#,
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

//! Multi-instance sync — CRDT-based state synchronization across warte instances.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;

/// Version vector for causality tracking.
pub type VersionVector = HashMap<String, u64>;

/// A sync update to be propagated to other instances.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncUpdate {
    pub instance_id: String,
    pub table: String,
    pub record_id: String,
    pub operation: SyncOperation,
    pub version_vector: VersionVector,
    pub timestamp: DateTime<Utc>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncOperation {
    Insert,
    Update,
    Delete,
}

/// Sync state for an instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    pub instance_id: String,
    pub version_vector: VersionVector,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub divergence_detected: bool,
}

/// Sync service for multi-instance coordination.
pub struct SyncService {
    pool: SqlitePool,
    instance_id: String,
}

impl SyncService {
    pub fn new(pool: SqlitePool, instance_id: String) -> Self {
        Self { pool, instance_id }
    }

    /// Get the current version vector for this instance.
    pub async fn get_version_vector(&self) -> anyhow::Result<VersionVector> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT version_vector FROM sync_state WHERE instance_id = ?")
                .bind(&self.instance_id)
                .fetch_optional(&self.pool)
                .await?;

        match row {
            Some((vv,)) => Ok(serde_json::from_str(&vv).unwrap_or_default()),
            None => Ok(VersionVector::new()),
        }
    }

    /// Increment the version vector for this instance.
    pub async fn increment(&self) -> anyhow::Result<VersionVector> {
        let mut vv = self.get_version_vector().await?;
        *vv.entry(self.instance_id.clone()).or_insert(0) += 1;

        let vv_json = serde_json::to_string(&vv)?;
        sqlx::query(
            r#"
            INSERT INTO sync_state (instance_id, version_vector, last_sync_at)
            VALUES (?, ?, ?)
            ON CONFLICT(instance_id) DO UPDATE SET
                version_vector = excluded.version_vector,
                last_sync_at = excluded.last_sync_at
            "#,
        )
        .bind(&self.instance_id)
        .bind(&vv_json)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(vv)
    }

    /// Merge a remote version vector into ours (CRDT merge: max per key).
    pub async fn merge_version_vector(
        &self,
        remote: &VersionVector,
    ) -> anyhow::Result<VersionVector> {
        let mut local = self.get_version_vector().await?;
        for (instance, &remote_count) in remote {
            let local_count = local.entry(instance.clone()).or_insert(0);
            *local_count = (*local_count).max(remote_count);
        }

        let vv_json = serde_json::to_string(&local)?;
        sqlx::query(
            r#"
            INSERT INTO sync_state (instance_id, version_vector, last_sync_at)
            VALUES (?, ?, ?)
            ON CONFLICT(instance_id) DO UPDATE SET
                version_vector = excluded.version_vector,
                last_sync_at = excluded.last_sync_at
            "#,
        )
        .bind(&self.instance_id)
        .bind(&vv_json)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(local)
    }

    /// Detect divergence: compare our version vector with a remote one.
    /// Returns the set of instances where we're behind.
    pub fn detect_divergence(local: &VersionVector, remote: &VersionVector) -> Vec<String> {
        let mut behind = Vec::new();
        for (instance, &remote_count) in remote {
            let local_count = local.get(instance).copied().unwrap_or(0);
            if remote_count > local_count {
                behind.push(instance.clone());
            }
        }
        behind
    }

    /// Get the current sync state.
    pub async fn get_state(&self) -> anyhow::Result<SyncState> {
        let vv = self.get_version_vector().await?;
        let row: Option<(Option<String>, i64)> = sqlx::query_as(
            "SELECT last_sync_at, divergence_detected FROM sync_state WHERE instance_id = ?",
        )
        .bind(&self.instance_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(SyncState {
            instance_id: self.instance_id.clone(),
            version_vector: vv,
            last_sync_at: row
                .as_ref()
                .and_then(|(ts, _)| ts.as_ref())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc)),
            divergence_detected: row.map(|(_, d)| d != 0).unwrap_or(false),
        })
    }

    /// Record a sync update from a remote instance.
    pub async fn apply_remote_update(&self, update: &SyncUpdate) -> anyhow::Result<()> {
        // Merge version vectors
        self.merge_version_vector(&update.version_vector).await?;

        // TODO: apply the update to the local database based on table + operation
        // This is where CRDT merge logic lives for commutative state

        Ok(())
    }
}

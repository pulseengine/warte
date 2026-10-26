//! ETA estimation — "when will this PR run?" from measured waits.
//!
//! REQ-ASK-001/002/003: The ask feature. Computes ETA from measured
//! wait times (how long jobs in the same runner pool waited before
//! starting over the last 3 hours). Not from queue position — GitHub
//! does not start jobs in creation order.
//!
//! Algorithm:
//! 1. Store measured waits: job entered queue at T1, started at T2 → wait = T2 - T1
//! 2. For a given repo + PR, determine the runner pool
//! 3. Query historical waits for that pool (last 3 hours, minimum 5 samples)
//! 4. Compute p50 (median) and p90 estimates
//! 5. Return estimate with confidence level and sample count
//! 6. If < 5 samples, return "no estimate" rather than a guess

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;

/// A measured wait record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitRecord {
    pub id: String,
    pub repo: String,
    pub pr_number: Option<u64>,
    pub runner_pool: String,
    pub queued_at: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    pub wait_seconds: u64,
    pub recorded_at: DateTime<Utc>,
}

/// ETA estimate result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtaEstimate {
    pub repo: String,
    pub pr_number: Option<u64>,
    pub runner_pool: String,
    /// Estimated wait in seconds (p50 — median).
    pub estimate_seconds: Option<u64>,
    /// p90 estimate (worst case).
    pub estimate_p90_seconds: Option<u64>,
    /// Confidence level: "high" (>= 20 samples), "medium" (>= 5), "low" (< 5).
    pub confidence: String,
    /// Number of samples used.
    pub sample_count: usize,
    /// Time window of samples.
    pub window_hours: u64,
    /// Currently running jobs in the pool.
    pub running_jobs: u32,
    /// Currently queued jobs in the pool.
    pub queued_jobs: u32,
    /// When the estimate was computed.
    pub computed_at: DateTime<Utc>,
    /// Human-readable basis.
    pub basis: String,
}

/// ETA estimator backed by SQLite.
pub struct EtaEstimator {
    pool: SqlitePool,
}

impl EtaEstimator {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Record a measured wait.
    pub async fn record_wait(
        &self,
        repo: &str,
        pr_number: Option<u64>,
        runner_pool: &str,
        queued_at: DateTime<Utc>,
        started_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let wait_seconds = (started_at - queued_at).num_seconds().max(0) as u64;
        let id = uuid::Uuid::now_v7().to_string();

        sqlx::query(
            r#"
            INSERT INTO wait_history (id, repo, pr_number, runner_pool, queued_at, started_at, wait_seconds, recorded_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(repo)
        .bind(pr_number.map(|n| n as i64))
        .bind(runner_pool)
        .bind(queued_at.to_rfc3339())
        .bind(started_at.to_rfc3339())
        .bind(wait_seconds as i64)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Estimate ETA for a repo + PR.
    ///
    /// Uses measured waits from the last 3 hours for the runner pool.
    /// Returns "no estimate" if fewer than 5 samples.
    pub async fn estimate(
        &self,
        repo: &str,
        pr_number: Option<u64>,
        runner_pool: &str,
    ) -> anyhow::Result<EtaEstimate> {
        let window_start = Utc::now() - Duration::hours(3);

        // Query historical waits for this pool in the last 3 hours
        let rows: Vec<(i64,)> = sqlx::query_as(
            r#"
            SELECT wait_seconds FROM wait_history
            WHERE runner_pool = ? AND started_at > ?
            ORDER BY started_at DESC
            "#,
        )
        .bind(runner_pool)
        .bind(window_start.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        let sample_count = rows.len();
        let waits: Vec<u64> = rows.into_iter().map(|(w,)| w as u64).collect();

        // Count running and queued jobs (from ci_status cache)
        let running: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM ci_status WHERE status = 'pending'")
                .fetch_one(&self.pool)
                .await
                .unwrap_or((0,));

        let queued: (i64,) = sqlx::query_as("SELECT COALESCE(SUM(queue_depth), 0) FROM ci_status")
            .fetch_one(&self.pool)
            .await
            .unwrap_or((0,));

        if sample_count < 5 {
            return Ok(EtaEstimate {
                repo: repo.to_string(),
                pr_number,
                runner_pool: runner_pool.to_string(),
                estimate_seconds: None,
                estimate_p90_seconds: None,
                confidence: "low".to_string(),
                sample_count,
                window_hours: 3,
                running_jobs: running.0 as u32,
                queued_jobs: queued.0 as u32,
                computed_at: Utc::now(),
                basis: format!(
                    "no estimate — only {} samples in the last 3 hours (need >= 5)",
                    sample_count
                ),
            });
        }

        // Compute p50 (median) and p90
        let mut sorted = waits.clone();
        sorted.sort();
        let p50 = percentile(&sorted, 50);
        let p90 = percentile(&sorted, 90);

        let confidence = if sample_count >= 20 { "high" } else { "medium" };

        Ok(EtaEstimate {
            repo: repo.to_string(),
            pr_number,
            runner_pool: runner_pool.to_string(),
            estimate_seconds: Some(p50),
            estimate_p90_seconds: Some(p90),
            confidence: confidence.to_string(),
            sample_count,
            window_hours: 3,
            running_jobs: running.0 as u32,
            queued_jobs: queued.0 as u32,
            computed_at: Utc::now(),
            basis: format!(
                "p50={}s, p90={}s from {} measured waits in pool '{}' over the last 3 hours",
                p50, p90, sample_count, runner_pool
            ),
        })
    }

    /// Get wait history statistics per runner pool.
    pub async fn pool_stats(&self) -> anyhow::Result<HashMap<String, PoolStats>> {
        let rows: Vec<(String, i64, i64, i64)> = sqlx::query_as(
            r#"
            SELECT runner_pool,
                   COUNT(*) as count,
                   AVG(wait_seconds) as avg_wait,
                   MAX(wait_seconds) as max_wait
            FROM wait_history
            WHERE started_at > ?
            GROUP BY runner_pool
            "#,
        )
        .bind((Utc::now() - Duration::hours(24)).to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(pool, count, avg, max)| {
                (
                    pool.clone(),
                    PoolStats {
                        runner_pool: pool,
                        sample_count: count as usize,
                        avg_wait_seconds: avg as u64,
                        max_wait_seconds: max as u64,
                    },
                )
            })
            .collect())
    }

    /// Clean up old wait history (> 7 days).
    pub async fn cleanup(&self) -> anyhow::Result<usize> {
        let cutoff = Utc::now() - Duration::days(7);
        let result = sqlx::query("DELETE FROM wait_history WHERE recorded_at < ?")
            .bind(cutoff.to_rfc3339())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() as usize)
    }
}

/// Statistics for a runner pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolStats {
    pub runner_pool: String,
    pub sample_count: usize,
    pub avg_wait_seconds: u64,
    pub max_wait_seconds: u64,
}

/// Compute a percentile from a sorted slice.
fn percentile(sorted: &[u64], p: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (sorted.len() * p / 100).min(sorted.len() - 1);
    sorted[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_eta_estimator_no_samples() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = crate::db::Database::connect(&db_path).await.unwrap();
        let estimator = EtaEstimator::new(db.pool().clone());

        let estimate = estimator
            .estimate("pulseengine/rivet", Some(511), "ubuntu-latest")
            .await
            .unwrap();
        assert_eq!(estimate.estimate_seconds, None);
        assert_eq!(estimate.confidence, "low");
        assert_eq!(estimate.sample_count, 0);
        assert!(estimate.basis.contains("no estimate"));
    }

    #[tokio::test]
    async fn test_eta_estimator_with_samples() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = crate::db::Database::connect(&db_path).await.unwrap();
        let estimator = EtaEstimator::new(db.pool().clone());

        // Record 10 waits of varying lengths
        let now = Utc::now();
        for i in 0..10 {
            let wait = (i + 1) * 60; // 60s, 120s, ..., 600s
            estimator
                .record_wait(
                    "pulseengine/rivet",
                    Some(511),
                    "ubuntu-latest",
                    now - Duration::seconds(wait + 10),
                    now - Duration::seconds(10),
                )
                .await
                .unwrap();
        }

        let estimate = estimator
            .estimate("pulseengine/rivet", Some(511), "ubuntu-latest")
            .await
            .unwrap();
        assert!(estimate.estimate_seconds.is_some());
        assert_eq!(estimate.confidence, "medium"); // 10 samples = medium
        assert_eq!(estimate.sample_count, 10);
        assert!(estimate.basis.contains("10 measured waits"));

        // p50 of [60, 120, 180, 240, 300, 360, 420, 480, 540, 600] = 300 or 360
        let p50 = estimate.estimate_seconds.unwrap();
        assert!(p50 >= 300 && p50 <= 360, "p50 should be ~330, got {}", p50);
    }

    #[test]
    fn test_percentile() {
        let sorted = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        // Simple index-based percentile: idx = len * p / 100
        assert_eq!(percentile(&sorted, 50), 60); // idx=5 → 60
        assert_eq!(percentile(&sorted, 90), 100); // idx=9 → 100
        assert_eq!(percentile(&sorted, 0), 10); // idx=0 → 10
        assert_eq!(percentile(&sorted, 100), 100); // idx=9 (clamped) → 100
        assert_eq!(percentile(&[], 50), 0); // empty
    }
}

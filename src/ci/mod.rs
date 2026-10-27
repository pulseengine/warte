//! GitHub CI data collection — webhook receiver + status poller.
//!
//! REQ-CI-DATA-001: Collect CI data from GitHub (and GitLab) to feed
//! the ETA estimator and dashboard.
//!
//! Collection methods:
//! - GitHub Actions webhook receiver (workflow_run events)
//! - Periodic status poller (fallback)
//! - Manual CLI import
//!
//! Collected data feeds:
//! - wait_history table (for ETA estimation)
//! - ci_status cache (for dashboard)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// GitHub Actions workflow_run webhook payload (subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunEvent {
    pub action: String,
    pub workflow_run: WorkflowRun,
    pub repository: Repository,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: u64,
    pub name: String,
    pub head_branch: String,
    pub head_sha: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub run_number: u64,
    pub run_attempt: u64,
    pub created_at: String,
    pub updated_at: String,
    pub run_started_at: Option<String>,
    pub html_url: String,
    pub head_repository: Option<HeadRepository>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadRepository {
    pub full_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub name: String,
    pub full_name: String,
    pub html_url: String,
}

/// A collected CI job record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiJobRecord {
    pub id: String,
    pub repo: String,
    pub run_id: u64,
    pub run_number: u64,
    pub branch: String,
    pub sha: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub queued_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub runner_pool: String,
    pub url: String,
    pub recorded_at: DateTime<Utc>,
}

/// CI data collector — receives webhooks and stores job records.
pub struct CiDataCollector {
    pool: SqlitePool,
}

impl CiDataCollector {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Process a GitHub Actions workflow_run webhook event.
    ///
    /// Stores the job record and updates wait_history if the job
    /// transitioned from queued to in_progress (measured wait).
    pub async fn process_webhook(&self, event: &WorkflowRunEvent) -> anyhow::Result<()> {
        let run = &event.workflow_run;
        let repo = &event.repository.full_name;

        let queued_at = parse_datetime(&run.created_at);
        let started_at = run.run_started_at.as_deref().and_then(parse_datetime);
        let completed_at = match run.status.as_str() {
            "completed" => Some(parse_datetime(&run.updated_at).unwrap_or_else(Utc::now)),
            _ => None,
        };

        // Determine runner pool from workflow name or labels
        let runner_pool = Self::infer_runner_pool(&run.name);

        // Store the job record
        let id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO ci_jobs
                (id, repo, run_id, run_number, branch, sha, status, conclusion,
                 queued_at, started_at, completed_at, runner_pool, url, recorded_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(repo)
        .bind(run.id as i64)
        .bind(run.run_number as i64)
        .bind(&run.head_branch)
        .bind(&run.head_sha)
        .bind(&run.status)
        .bind(&run.conclusion)
        .bind(queued_at.map(|d| d.to_rfc3339()))
        .bind(started_at.map(|d| d.to_rfc3339()))
        .bind(completed_at.map(|d| d.to_rfc3339()))
        .bind(&runner_pool)
        .bind(&run.html_url)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        // If the job transitioned from queued to in_progress, record the wait
        if let (Some(queued), Some(started)) = (queued_at, started_at) {
            let wait_seconds = (started - queued).num_seconds().max(0) as u64;
            sqlx::query(
                r#"
                INSERT INTO wait_history
                    (id, repo, pr_number, runner_pool, queued_at, started_at, wait_seconds, recorded_at)
                VALUES (?, ?, NULL, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(repo)
            .bind(&runner_pool)
            .bind(queued.to_rfc3339())
            .bind(started.to_rfc3339())
            .bind(wait_seconds as i64)
            .bind(Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await?;
        }

        // Update ci_status cache for the dashboard
        let ci_status = match run.conclusion.as_deref() {
            Some("success") => "green",
            Some("failure") => "red",
            _ => "pending",
        };
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO ci_status (repo, status, updated_at, run_url, queue_depth)
            VALUES (?, ?, ?, ?, 0)
            "#,
        )
        .bind(repo)
        .bind(ci_status)
        .bind(Utc::now().to_rfc3339())
        .bind(&run.html_url)
        .execute(&self.pool)
        .await?;

        tracing::debug!(
            repo = %repo,
            run_id = run.id,
            status = %run.status,
            "CI job record stored"
        );

        Ok(())
    }

    /// Infer the runner pool from the workflow name or job labels.
    fn infer_runner_pool(workflow_name: &str) -> String {
        let name = workflow_name.to_lowercase();
        if name.contains("hardware") || name.contains("hil") || name.contains("silicon") {
            "hardware".to_string()
        } else if name.contains("wasm") || name.contains("component") {
            "wasm".to_string()
        } else if name.contains("proof") || name.contains("formal") {
            "proof".to_string()
        } else {
            "default".to_string()
        }
    }

    /// Get CI status for a repo (from cache).
    pub async fn get_repo_status(
        &self,
        repo: &str,
    ) -> anyhow::Result<Option<(String, DateTime<Utc>)>> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT status, updated_at FROM ci_status WHERE repo = ?")
                .bind(repo)
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.and_then(|(status, updated)| {
            DateTime::parse_from_rfc3339(&updated)
                .ok()
                .map(|d| (status, d.with_timezone(&Utc)))
        }))
    }

    /// Get all CI statuses (for dashboard overview).
    pub async fn get_all_statuses(&self) -> anyhow::Result<Vec<(String, String, DateTime<Utc>)>> {
        let rows: Vec<(String, String, String)> =
            sqlx::query_as("SELECT repo, status, updated_at FROM ci_status ORDER BY repo")
                .fetch_all(&self.pool)
                .await?;

        Ok(rows
            .into_iter()
            .filter_map(|(repo, status, updated)| {
                DateTime::parse_from_rfc3339(&updated)
                    .ok()
                    .map(|d| (repo, status, d.with_timezone(&Utc)))
            })
            .collect())
    }
}

/// Parse a datetime string (RFC3339 or GitHub format).
fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
        .or_else(|| {
            // GitHub format: "2026-10-26T12:00:00Z"
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ")
                .ok()
                .map(|d| d.and_utc())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_datetime() {
        assert!(parse_datetime("2026-10-26T12:00:00Z").is_some());
        assert!(parse_datetime("2026-10-26T12:00:00+00:00").is_some());
        assert!(parse_datetime("invalid").is_none());
    }

    #[test]
    fn test_infer_runner_pool() {
        assert_eq!(CiDataCollector::infer_runner_pool("CI"), "default");
        assert_eq!(
            CiDataCollector::infer_runner_pool("Hardware HIL"),
            "hardware"
        );
        assert_eq!(CiDataCollector::infer_runner_pool("Wasm Component"), "wasm");
        assert_eq!(CiDataCollector::infer_runner_pool("Formal Proof"), "proof");
    }

    #[test]
    fn test_webhook_deserialization() {
        let json = r#"{
            "action": "completed",
            "workflow_run": {
                "id": 12345,
                "name": "CI",
                "head_branch": "main",
                "head_sha": "abc123",
                "status": "completed",
                "conclusion": "success",
                "run_number": 42,
                "run_attempt": 1,
                "created_at": "2026-10-26T12:00:00Z",
                "updated_at": "2026-10-26T12:05:00Z",
                "run_started_at": "2026-10-26T12:01:00Z",
                "html_url": "https://github.com/pulseengine/rivet/actions/runs/12345"
            },
            "repository": {
                "name": "rivet",
                "full_name": "pulseengine/rivet",
                "html_url": "https://github.com/pulseengine/rivet"
            }
        }"#;

        let event: WorkflowRunEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.action, "completed");
        assert_eq!(event.workflow_run.id, 12345);
        assert_eq!(event.repository.full_name, "pulseengine/rivet");
    }

    #[tokio::test]
    async fn test_process_webhook() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = crate::db::Database::connect(&db_path).await.unwrap();
        let collector = CiDataCollector::new(db.pool().clone());

        let event = WorkflowRunEvent {
            action: "completed".to_string(),
            workflow_run: WorkflowRun {
                id: 12345,
                name: "CI".to_string(),
                head_branch: "main".to_string(),
                head_sha: "abc123".to_string(),
                status: "completed".to_string(),
                conclusion: Some("success".to_string()),
                run_number: 42,
                run_attempt: 1,
                created_at: "2026-10-26T12:00:00Z".to_string(),
                updated_at: "2026-10-26T12:05:00Z".to_string(),
                run_started_at: Some("2026-10-26T12:01:00Z".to_string()),
                html_url: "https://github.com/pulseengine/rivet/actions/runs/12345".to_string(),
                head_repository: None,
            },
            repository: Repository {
                name: "rivet".to_string(),
                full_name: "pulseengine/rivet".to_string(),
                html_url: "https://github.com/pulseengine/rivet".to_string(),
            },
        };

        collector.process_webhook(&event).await.unwrap();

        // Check ci_status cache
        let status = collector
            .get_repo_status("pulseengine/rivet")
            .await
            .unwrap();
        assert!(status.is_some());
        let (status_str, _) = status.unwrap();
        assert_eq!(status_str, "green");
    }
}

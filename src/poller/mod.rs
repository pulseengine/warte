//! Periodic CI status poller — fallback when webhooks are not configured.
//!
//! REQ-POLLER-001: The system SHALL provide a periodic CI status poller
//! that queries the GitHub/GitLab API every 60 seconds (configurable)
//! to collect CI status for all repos. The poller SHALL be the fallback
//! when webhooks are not configured. The poller SHALL record measured
//! waits (queued_at → started_at) in the wait_history table. The poller
//! SHALL update the ci_status cache for the dashboard. The poller SHALL
//! run as a background task started by warte serve.

use crate::ci::CiDataCollector;
use crate::platform::{CiStatus, FleetAggregator, GitHubPlatform, GitLabPlatform};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::sync::Arc;

/// Last known job states: (status, started_at).
type LastStates = Arc<RwLock<std::collections::HashMap<String, (String, Option<DateTime<Utc>>)>>>;
use std::time::Duration as StdDuration;
use tokio::sync::RwLock;

/// Poller configuration.
#[derive(Debug, Clone)]
pub struct PollerConfig {
    /// Poll interval in seconds (default: 60).
    pub interval_secs: u64,
    /// GitHub organization (optional).
    pub github_org: Option<String>,
    /// GitHub token (optional).
    pub github_token: Option<String>,
    /// GitHub Enterprise URL (optional).
    pub ghe_url: Option<String>,
    /// GitLab group (optional).
    pub gitlab_group: Option<String>,
    /// GitLab token (optional).
    pub gitlab_token: Option<String>,
    /// GitLab URL (optional, for self-hosted).
    pub gitlab_url: Option<String>,
}

impl Default for PollerConfig {
    fn default() -> Self {
        Self {
            interval_secs: 60,
            github_org: None,
            github_token: None,
            ghe_url: None,
            gitlab_group: None,
            gitlab_token: None,
            gitlab_url: None,
        }
    }
}

/// CI status poller — periodically queries CI platforms.
#[allow(dead_code)]
pub struct CiPoller {
    config: PollerConfig,
    collector: Arc<CiDataCollector>,
    pool: SqlitePool,
    shutdown: Arc<RwLock<bool>>,
    /// Last known job states (for detecting queued → in_progress transitions).
    last_states: LastStates,
}

impl CiPoller {
    /// Create a new CI poller.
    pub fn new(config: PollerConfig, pool: SqlitePool) -> Self {
        Self {
            config,
            collector: Arc::new(CiDataCollector::new(pool.clone())),
            pool,
            shutdown: Arc::new(RwLock::new(false)),
            last_states: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Build the fleet aggregator from config.
    fn build_fleet(&self) -> FleetAggregator {
        let mut fleet = FleetAggregator::new();

        if let (Some(org), Some(token)) = (&self.config.github_org, &self.config.github_token) {
            if let Some(ghe_url) = &self.config.ghe_url {
                fleet.add_platform(Box::new(GitHubPlatform::enterprise(ghe_url, org, token)));
            } else {
                fleet.add_platform(Box::new(GitHubPlatform::cloud(org, token)));
            }
        }

        if let (Some(group), Some(token)) = (&self.config.gitlab_group, &self.config.gitlab_token) {
            if let Some(url) = &self.config.gitlab_url {
                fleet.add_platform(Box::new(GitLabPlatform::self_hosted(url, group, token)));
            } else {
                fleet.add_platform(Box::new(GitLabPlatform::cloud(group, token)));
            }
        }

        fleet
    }

    /// Start the poller as a background task.
    pub fn start(self: Arc<Self>) {
        let poller = self.clone();
        tokio::spawn(async move {
            poller.run().await;
        });
        tracing::info!(
            interval_secs = self.config.interval_secs,
            "CI status poller started"
        );
    }

    /// Stop the poller.
    pub async fn stop(&self) {
        *self.shutdown.write().await = true;
    }

    /// Run the poll loop.
    async fn run(&self) {
        let interval = StdDuration::from_secs(self.config.interval_secs);
        let mut ticker = tokio::time::interval(interval);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if *self.shutdown.read().await {
                        break;
                    }
                    self.poll_once().await;
                }
            }
        }

        tracing::info!("CI status poller stopped");
    }

    /// Poll all platforms once.
    async fn poll_once(&self) {
        let fleet = self.build_fleet();
        let status = match fleet.get_fleet_status().await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "poll failed");
                return;
            }
        };

        tracing::debug!(
            total_repos = status.total_repos,
            green = status.green,
            red = status.red,
            pending = status.pending,
            "poll complete"
        );

        // Update ci_status cache for each repo
        for repo in &status.repos {
            let ci_status = match repo.ci_status {
                CiStatus::Green => "green",
                CiStatus::Red => "red",
                CiStatus::Pending => "pending",
                CiStatus::Unknown => "unknown",
            };

            let result = sqlx::query(
                r#"
                INSERT OR REPLACE INTO ci_status (repo, status, updated_at, run_url, queue_depth)
                VALUES (?, ?, ?, ?, ?)
                "#,
            )
            .bind(&repo.full_name)
            .bind(ci_status)
            .bind(Utc::now().to_rfc3339())
            .bind(&repo.run_url)
            .bind(repo.queue_depth as i64)
            .execute(&self.pool)
            .await;

            if let Err(e) = result {
                tracing::warn!(repo = %repo.full_name, error = %e, "failed to update ci_status");
            }

            // Track job state transitions for measured wait recording (REQ-POLLER-002)
            let job_key = format!(
                "{}:{}",
                repo.full_name,
                repo.run_url.as_deref().unwrap_or("")
            );
            let current_status = ci_status.to_string();
            let now = Utc::now();

            let mut states = self.last_states.write().await;
            if let Some((prev_status, prev_time)) = states.get(&job_key) {
                // Detect queued → in_progress transition
                if prev_status == "pending" && current_status == "green" || current_status == "red"
                {
                    if let Some(queued_at) = prev_time {
                        let wait_seconds = (now - *queued_at).num_seconds().max(0) as u64;
                        let wait_id = uuid::Uuid::now_v7().to_string();
                        let wait_result = sqlx::query(
                            r#"
                            INSERT INTO wait_history
                                (id, repo, pr_number, runner_pool, queued_at, started_at, wait_seconds, recorded_at)
                            VALUES (?, ?, NULL, 'polled', ?, ?, ?, ?)
                            "#,
                        )
                        .bind(&wait_id)
                        .bind(&repo.full_name)
                        .bind(queued_at.to_rfc3339())
                        .bind(now.to_rfc3339())
                        .bind(wait_seconds as i64)
                        .bind(now.to_rfc3339())
                        .execute(&self.pool)
                        .await;

                        if let Err(e) = wait_result {
                            tracing::warn!(repo = %repo.full_name, error = %e, "failed to record wait");
                        } else {
                            tracing::debug!(
                                repo = %repo.full_name,
                                wait_seconds = wait_seconds,
                                "measured wait recorded from poller"
                            );
                        }
                    }
                }
            }
            // Update last known state
            let timestamp = if current_status == "pending" {
                Some(now)
            } else {
                None
            };
            states.insert(job_key, (current_status, timestamp));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_poller_config_default() {
        let config = PollerConfig::default();
        assert_eq!(config.interval_secs, 60);
        assert!(config.github_org.is_none());
        assert!(config.gitlab_group.is_none());
    }

    #[test]
    fn test_poller_config_custom() {
        let config = PollerConfig {
            interval_secs: 30,
            github_org: Some("pulseengine".to_string()),
            github_token: Some("fake-token".to_string()),
            ghe_url: Some("https://ghe.internal".to_string()),
            gitlab_group: None,
            gitlab_token: None,
            gitlab_url: None,
        };
        assert_eq!(config.interval_secs, 30);
        assert_eq!(config.github_org, Some("pulseengine".to_string()));
    }

    #[tokio::test]
    async fn test_poller_creation() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = crate::db::Database::connect(&db_path).await.unwrap();

        let config = PollerConfig {
            interval_secs: 60,
            github_org: Some("pulseengine".to_string()),
            github_token: Some("fake-token".to_string()),
            ghe_url: None,
            gitlab_group: None,
            gitlab_token: None,
            gitlab_url: None,
        };

        let poller = CiPoller::new(config, db.pool().clone());
        // Just verify it's created without panic
        poller.stop().await;
    }
}

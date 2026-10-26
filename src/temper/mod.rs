//! Temper integration — HTTP link to the temper GitHub App.
//!
//! REQ-TEMPER-001: Temper is linked, not absorbed. Warte reads temper's
//! API to show repo compliance status in the dashboard. Temper remains
//! a separate deployment (Probot/Node.js); warte links to it via HTTP.
//!
//! Integration points (from DD-TEMPER-001):
//! - Dashboard shows temper's repo hardening status (fetched from temper's API)
//! - Warte access control can read temper's config.yml for role mapping
//! - Warte CLI can query temper's API for repo compliance status

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Temper's repo compliance status (from temper's API).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemperRepoStatus {
    pub repo: String,
    pub compliant: bool,
    pub checks: Vec<TemperCheck>,
    pub last_checked: Option<DateTime<Utc>>,
}

/// A single compliance check from temper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemperCheck {
    pub name: String,
    pub passed: bool,
    pub details: Option<String>,
}

/// Temper's organization overview (from temper's API).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemperOrgOverview {
    pub organization: String,
    pub total_repos: usize,
    pub compliant_repos: usize,
    pub non_compliant_repos: usize,
    pub repos: Vec<TemperRepoStatus>,
    pub last_updated: Option<DateTime<Utc>>,
}

/// HTTP client for the temper API.
pub struct TemperClient {
    /// Temper API base URL.
    api_base: String,
    /// Bearer token for temper's API.
    token: String,
    /// HTTP client.
    client: reqwest::Client,
}

impl TemperClient {
    /// Create a new temper client.
    pub fn new(api_base: &str, token: &str) -> Self {
        Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Get the compliance status for a specific repo.
    pub async fn get_repo_status(&self, repo: &str) -> anyhow::Result<TemperRepoStatus> {
        let url = format!("{}/api/org/repos/{}/status", self.api_base, repo);
        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("temper API returned {}", response.status());
        }

        Ok(response.json().await?)
    }

    /// Get the organization-wide compliance overview.
    pub async fn get_org_overview(&self) -> anyhow::Result<TemperOrgOverview> {
        let url = format!("{}/api/org/health", self.api_base);
        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("temper API returned {}", response.status());
        }

        Ok(response.json().await?)
    }

    /// Check if temper is reachable.
    pub async fn health_check(&self) -> anyhow::Result<bool> {
        let url = format!("{}/health", self.api_base);
        match self.client.get(&url).send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}

/// Temper link — integrates temper's compliance data into warte's dashboard.
pub struct TemperLink {
    client: TemperClient,
}

impl TemperLink {
    /// Create a new temper link.
    pub fn new(api_base: &str, token: &str) -> Self {
        Self {
            client: TemperClient::new(api_base, token),
        }
    }

    /// Get the temper client.
    pub fn client(&self) -> &TemperClient {
        &self.client
    }

    /// Get compliance status for the dashboard.
    ///
    /// Returns a summary suitable for the warte dashboard:
    /// which repos are compliant, which are not, and what checks are failing.
    pub async fn get_dashboard_summary(&self) -> anyhow::Result<TemperDashboardSummary> {
        let overview = self.client.get_org_overview().await?;

        let failing_checks: Vec<(String, String)> = overview
            .repos
            .iter()
            .flat_map(|repo| {
                repo.checks
                    .iter()
                    .filter(|c| !c.passed)
                    .map(|c| (repo.repo.clone(), c.name.clone()))
            })
            .collect();

        Ok(TemperDashboardSummary {
            organization: overview.organization,
            total_repos: overview.total_repos,
            compliant_repos: overview.compliant_repos,
            non_compliant_repos: overview.non_compliant_repos,
            compliance_rate: if overview.total_repos > 0 {
                overview.compliant_repos as f64 / overview.total_repos as f64
            } else {
                0.0
            },
            failing_checks,
            last_updated: overview.last_updated,
        })
    }
}

/// Dashboard summary of temper compliance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemperDashboardSummary {
    pub organization: String,
    pub total_repos: usize,
    pub compliant_repos: usize,
    pub non_compliant_repos: usize,
    pub compliance_rate: f64,
    pub failing_checks: Vec<(String, String)>,
    pub last_updated: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temper_client_creation() {
        let client = TemperClient::new("http://temper.internal:3000", "test-token");
        assert_eq!(client.api_base, "http://temper.internal:3000");
        assert_eq!(client.token, "test-token");
    }

    #[test]
    fn test_temper_client_trailing_slash() {
        let client = TemperClient::new("http://temper.internal:3000/", "test-token");
        assert_eq!(client.api_base, "http://temper.internal:3000");
    }

    #[test]
    fn test_temper_link_creation() {
        let link = TemperLink::new("http://temper.internal:3000", "test-token");
        assert_eq!(link.client().api_base, "http://temper.internal:3000");
    }

    #[test]
    fn test_dashboard_summary_serialization() {
        let summary = TemperDashboardSummary {
            organization: "pulseengine".to_string(),
            total_repos: 10,
            compliant_repos: 8,
            non_compliant_repos: 2,
            compliance_rate: 0.8,
            failing_checks: vec![
                ("rivet".to_string(), "signed-commits".to_string()),
                ("spar".to_string(), "branch-protection".to_string()),
            ],
            last_updated: None,
        };

        let json = serde_json::to_string(&summary).unwrap();
        let parsed: TemperDashboardSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.organization, "pulseengine");
        assert_eq!(parsed.compliance_rate, 0.8);
        assert_eq!(parsed.failing_checks.len(), 2);
    }
}

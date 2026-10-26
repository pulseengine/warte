//! CI platform abstraction — multi-platform support (GitHub, GitLab, others).
//!
//! REQ-PLATFORM-001: Support enterprise GitHub (GHE) in addition to github.com.
//! REQ-PLATFORM-002: Support GitLab CI (cloud + self-hosted).
//! REQ-PLATFORM-003: Platform abstraction layer (CiPlatform trait).
//!
//! Architecture (from DD-PLATFORM-001):
//! - CiPlatform trait: uniform interface for repo listing, CI status, PR/MR listing
//! - GitHubCloud: github.com API
//! - GitHubEnterprise: GHE API (configurable base URL)
//! - GitLabCloud: gitlab.com API
//! - GitLabSelfHosted: self-hosted GitLab API
//! - Dashboard aggregates across all configured platforms

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// CI status of a repository.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CiStatus {
    Green,
    Red,
    Pending,
    Unknown,
}

/// A repository on a CI platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    pub name: String,
    pub full_name: String,
    pub platform: String,
    pub default_branch: String,
    pub ci_status: CiStatus,
    pub last_updated: Option<DateTime<Utc>>,
    pub run_url: Option<String>,
    pub queue_depth: u32,
}

/// A pull/merge request on a CI platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestInfo {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub state: String,
    pub ci_status: CiStatus,
    pub created_at: DateTime<Utc>,
    pub url: String,
    pub head_ref: String,
    pub base_ref: String,
}

/// ETA estimate for a PR/MR to run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtaEstimate {
    pub repo: String,
    pub pr_number: Option<u64>,
    pub estimate_seconds: Option<u64>,
    pub confidence: String,
    pub basis: String,
}

/// CI platform abstraction.
#[async_trait]
pub trait CiPlatform: Send + Sync {
    /// Platform name (e.g., "github", "gitlab").
    fn name(&self) -> &str;

    /// Platform instance (e.g., "github.com", "ghe.internal", "gitlab.com").
    fn instance(&self) -> &str;

    /// List all repositories in the organization.
    async fn list_repos(&self) -> anyhow::Result<Vec<RepoInfo>>;

    /// Get CI status for a specific repository.
    async fn get_repo_status(&self, repo: &str) -> anyhow::Result<RepoInfo>;

    /// List open pull/merge requests for a repository.
    async fn list_prs(&self, repo: &str) -> anyhow::Result<Vec<PullRequestInfo>>;

    /// Estimate when a PR/MR will run (the "ask" feature).
    async fn estimate_eta(&self, repo: &str, pr_number: Option<u64>)
        -> anyhow::Result<EtaEstimate>;
}

/// GitHub platform (cloud or enterprise).
pub struct GitHubPlatform {
    /// API base URL (https://api.github.com or GHE URL).
    api_base: String,
    /// Organization name.
    org: String,
    /// HTTP client.
    client: reqwest::Client,
    /// Instance label.
    instance: String,
}

impl GitHubPlatform {
    /// Create a GitHub platform for github.com.
    pub fn cloud(org: &str, token: &str) -> Self {
        Self {
            api_base: "https://api.github.com".to_string(),
            org: org.to_string(),
            client: reqwest::Client::builder()
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert(
                        reqwest::header::AUTHORIZATION,
                        format!("Bearer {}", token).parse().unwrap(),
                    );
                    headers.insert(
                        reqwest::header::ACCEPT,
                        "application/vnd.github+json".parse().unwrap(),
                    );
                    headers
                })
                .build()
                .unwrap(),
            instance: "github.com".to_string(),
        }
    }

    /// Create a GitHub platform for GitHub Enterprise.
    pub fn enterprise(ghe_url: &str, org: &str, token: &str) -> Self {
        Self {
            api_base: format!("{}/api/v3", ghe_url.trim_end_matches('/')),
            org: org.to_string(),
            client: reqwest::Client::builder()
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert(
                        reqwest::header::AUTHORIZATION,
                        format!("Bearer {}", token).parse().unwrap(),
                    );
                    headers.insert(
                        reqwest::header::ACCEPT,
                        "application/vnd.github+json".parse().unwrap(),
                    );
                    headers
                })
                .build()
                .unwrap(),
            instance: ghe_url.to_string(),
        }
    }
}

#[async_trait]
impl CiPlatform for GitHubPlatform {
    fn name(&self) -> &str {
        "github"
    }

    fn instance(&self) -> &str {
        &self.instance
    }

    async fn list_repos(&self) -> anyhow::Result<Vec<RepoInfo>> {
        let url = format!("{}/orgs/{}/repos?per_page=100", self.api_base, self.org);
        let repos: Vec<serde_json::Value> = self.client.get(&url).send().await?.json().await?;

        let mut result = Vec::new();
        for repo in repos {
            let name = repo["name"].as_str().unwrap_or("").to_string();
            let full_name = repo["full_name"].as_str().unwrap_or("").to_string();
            let default_branch = repo["default_branch"]
                .as_str()
                .unwrap_or("main")
                .to_string();

            // Get latest CI run
            let ci_status = self.get_repo_status(&name).await.unwrap_or(RepoInfo {
                name: name.clone(),
                full_name: full_name.clone(),
                platform: format!("github/{}", self.instance),
                default_branch: default_branch.clone(),
                ci_status: CiStatus::Unknown,
                last_updated: None,
                run_url: None,
                queue_depth: 0,
            });

            result.push(ci_status);
        }
        Ok(result)
    }

    async fn get_repo_status(&self, repo: &str) -> anyhow::Result<RepoInfo> {
        let url = format!(
            "{}/repos/{}/{}/actions/runs?per_page=1",
            self.api_base, self.org, repo
        );
        let response: serde_json::Value = self.client.get(&url).send().await?.json().await?;

        let run = &response["workflow_runs"][0];
        let status = match run["conclusion"].as_str() {
            Some("success") => CiStatus::Green,
            Some("failure") => CiStatus::Red,
            _ => match run["status"].as_str() {
                Some("in_progress") | Some("queued") => CiStatus::Pending,
                _ => CiStatus::Unknown,
            },
        };

        Ok(RepoInfo {
            name: repo.to_string(),
            full_name: format!("{}/{}", self.org, repo),
            platform: format!("github/{}", self.instance),
            default_branch: "main".to_string(),
            ci_status: status,
            last_updated: run["updated_at"]
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc)),
            run_url: run["html_url"].as_str().map(String::from),
            queue_depth: 0,
        })
    }

    async fn list_prs(&self, repo: &str) -> anyhow::Result<Vec<PullRequestInfo>> {
        let url = format!(
            "{}/repos/{}/{}/pulls?state=open&per_page=20",
            self.api_base, self.org, repo
        );
        let prs: Vec<serde_json::Value> = self.client.get(&url).send().await?.json().await?;

        Ok(prs
            .iter()
            .map(|pr| PullRequestInfo {
                number: pr["number"].as_u64().unwrap_or(0),
                title: pr["title"].as_str().unwrap_or("").to_string(),
                author: pr["user"]["login"].as_str().unwrap_or("").to_string(),
                state: pr["state"].as_str().unwrap_or("open").to_string(),
                ci_status: CiStatus::Unknown, // TODO: fetch PR check status
                created_at: DateTime::parse_from_rfc3339(pr["created_at"].as_str().unwrap_or(""))
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                url: pr["html_url"].as_str().unwrap_or("").to_string(),
                head_ref: pr["head"]["ref"].as_str().unwrap_or("").to_string(),
                base_ref: pr["base"]["ref"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }

    async fn estimate_eta(
        &self,
        repo: &str,
        pr_number: Option<u64>,
    ) -> anyhow::Result<EtaEstimate> {
        // TODO: implement ETA from measured waits (the "ask" feature)
        // For now, return no estimate
        Ok(EtaEstimate {
            repo: repo.to_string(),
            pr_number,
            estimate_seconds: None,
            confidence: "low".to_string(),
            basis: "no historical data".to_string(),
        })
    }
}

/// GitLab platform (cloud or self-hosted).
pub struct GitLabPlatform {
    /// API base URL (https://gitlab.com/api/v4 or self-hosted URL).
    api_base: String,
    /// Group name (GitLab equivalent of org).
    group: String,
    /// HTTP client.
    client: reqwest::Client,
    /// Instance label.
    instance: String,
}

impl GitLabPlatform {
    /// Create a GitLab platform for gitlab.com.
    pub fn cloud(group: &str, token: &str) -> Self {
        Self {
            api_base: "https://gitlab.com/api/v4".to_string(),
            group: group.to_string(),
            client: reqwest::Client::builder()
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert("PRIVATE-TOKEN", token.parse().unwrap());
                    headers
                })
                .build()
                .unwrap(),
            instance: "gitlab.com".to_string(),
        }
    }

    /// Create a GitLab platform for self-hosted GitLab.
    pub fn self_hosted(url: &str, group: &str, token: &str) -> Self {
        Self {
            api_base: format!("{}/api/v4", url.trim_end_matches('/')),
            group: group.to_string(),
            client: reqwest::Client::builder()
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert("PRIVATE-TOKEN", token.parse().unwrap());
                    headers
                })
                .build()
                .unwrap(),
            instance: url.to_string(),
        }
    }
}

#[async_trait]
impl CiPlatform for GitLabPlatform {
    fn name(&self) -> &str {
        "gitlab"
    }

    fn instance(&self) -> &str {
        &self.instance
    }

    async fn list_repos(&self) -> anyhow::Result<Vec<RepoInfo>> {
        let url = format!(
            "{}/groups/{}/projects?per_page=100",
            self.api_base, self.group
        );
        let repos: Vec<serde_json::Value> = self.client.get(&url).send().await?.json().await?;

        Ok(repos
            .iter()
            .map(|repo| RepoInfo {
                name: repo["name"].as_str().unwrap_or("").to_string(),
                full_name: repo["path_with_namespace"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                platform: format!("gitlab/{}", self.instance),
                default_branch: repo["default_branch"]
                    .as_str()
                    .unwrap_or("main")
                    .to_string(),
                ci_status: CiStatus::Unknown, // TODO: fetch pipeline status
                last_updated: None,
                run_url: None,
                queue_depth: 0,
            })
            .collect())
    }

    async fn get_repo_status(&self, repo: &str) -> anyhow::Result<RepoInfo> {
        // TODO: implement GitLab pipeline status
        Ok(RepoInfo {
            name: repo.to_string(),
            full_name: format!("{}/{}", self.group, repo),
            platform: format!("gitlab/{}", self.instance),
            default_branch: "main".to_string(),
            ci_status: CiStatus::Unknown,
            last_updated: None,
            run_url: None,
            queue_depth: 0,
        })
    }

    async fn list_prs(&self, repo: &str) -> anyhow::Result<Vec<PullRequestInfo>> {
        let url = format!(
            "{}/groups/{}/projects/{}/merge_requests?state=opened&per_page=20",
            self.api_base, self.group, repo
        );
        let mrs: Vec<serde_json::Value> = self.client.get(&url).send().await?.json().await?;

        Ok(mrs
            .iter()
            .map(|mr| PullRequestInfo {
                number: mr["iid"].as_u64().unwrap_or(0),
                title: mr["title"].as_str().unwrap_or("").to_string(),
                author: mr["author"]["username"].as_str().unwrap_or("").to_string(),
                state: mr["state"].as_str().unwrap_or("opened").to_string(),
                ci_status: CiStatus::Unknown,
                created_at: DateTime::parse_from_rfc3339(mr["created_at"].as_str().unwrap_or(""))
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                url: mr["web_url"].as_str().unwrap_or("").to_string(),
                head_ref: mr["source_branch"].as_str().unwrap_or("").to_string(),
                base_ref: mr["target_branch"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }

    async fn estimate_eta(
        &self,
        repo: &str,
        pr_number: Option<u64>,
    ) -> anyhow::Result<EtaEstimate> {
        Ok(EtaEstimate {
            repo: repo.to_string(),
            pr_number,
            estimate_seconds: None,
            confidence: "low".to_string(),
            basis: "no historical data".to_string(),
        })
    }
}

/// Aggregated CI status across all platforms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetStatus {
    pub platforms: Vec<String>,
    pub total_repos: usize,
    pub green: usize,
    pub red: usize,
    pub pending: usize,
    pub unknown: usize,
    pub repos: Vec<RepoInfo>,
    pub timestamp: DateTime<Utc>,
}

/// Fleet aggregator — queries all configured platforms.
pub struct FleetAggregator {
    platforms: Vec<Box<dyn CiPlatform>>,
}

impl FleetAggregator {
    pub fn new() -> Self {
        Self {
            platforms: Vec::new(),
        }
    }

    /// Add a platform.
    pub fn add_platform(&mut self, platform: Box<dyn CiPlatform>) {
        self.platforms.push(platform);
    }

    /// Get fleet-wide status.
    pub async fn get_fleet_status(&self) -> anyhow::Result<FleetStatus> {
        let mut all_repos = Vec::new();
        for platform in &self.platforms {
            match platform.list_repos().await {
                Ok(repos) => all_repos.extend(repos),
                Err(e) => {
                    tracing::warn!(
                        platform = platform.name(),
                        instance = platform.instance(),
                        error = %e,
                        "failed to list repos"
                    );
                }
            }
        }

        let green = all_repos
            .iter()
            .filter(|r| r.ci_status == CiStatus::Green)
            .count();
        let red = all_repos
            .iter()
            .filter(|r| r.ci_status == CiStatus::Red)
            .count();
        let pending = all_repos
            .iter()
            .filter(|r| r.ci_status == CiStatus::Pending)
            .count();
        let unknown = all_repos
            .iter()
            .filter(|r| r.ci_status == CiStatus::Unknown)
            .count();

        Ok(FleetStatus {
            platforms: self
                .platforms
                .iter()
                .map(|p| format!("{}/{}", p.name(), p.instance()))
                .collect(),
            total_repos: all_repos.len(),
            green,
            red,
            pending,
            unknown,
            repos: all_repos,
            timestamp: Utc::now(),
        })
    }
}

impl Default for FleetAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_github_cloud_creation() {
        let gh = GitHubPlatform::cloud("pulseengine", "fake-token");
        assert_eq!(gh.name(), "github");
        assert_eq!(gh.instance(), "github.com");
        assert_eq!(gh.org, "pulseengine");
    }

    #[test]
    fn test_github_enterprise_creation() {
        let ghe = GitHubPlatform::enterprise(
            "https://ghe.internal.example.com",
            "pulseengine",
            "fake-token",
        );
        assert_eq!(ghe.name(), "github");
        assert_eq!(ghe.instance(), "https://ghe.internal.example.com");
        assert!(ghe
            .api_base
            .starts_with("https://ghe.internal.example.com/api/v3"));
    }

    #[test]
    fn test_gitlab_cloud_creation() {
        let gl = GitLabPlatform::cloud("pulseengine", "fake-token");
        assert_eq!(gl.name(), "gitlab");
        assert_eq!(gl.instance(), "gitlab.com");
    }

    #[test]
    fn test_gitlab_self_hosted_creation() {
        let gl = GitLabPlatform::self_hosted(
            "https://gitlab.internal.example.com",
            "pulseengine",
            "fake-token",
        );
        assert_eq!(gl.name(), "gitlab");
        assert!(gl
            .api_base
            .starts_with("https://gitlab.internal.example.com/api/v4"));
    }

    #[tokio::test]
    async fn test_fleet_aggregator() {
        let mut fleet = FleetAggregator::new();
        // Add platforms (with fake tokens — list_repos will fail gracefully)
        fleet.add_platform(Box::new(GitHubPlatform::cloud("pulseengine", "fake")));
        fleet.add_platform(Box::new(GitLabPlatform::cloud("pulseengine", "fake")));

        let status = fleet.get_fleet_status().await.unwrap();
        assert_eq!(status.platforms.len(), 2);
        assert_eq!(status.total_repos, 0); // API calls fail with fake tokens
    }
}

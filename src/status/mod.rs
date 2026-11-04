//! GitHub status integration: real status from githubstatus.com (statuspage.io).
//!
//! REQ-STATUS-001: The outages view SHALL query the real GitHub status
//! from status.github.com (or GHE status page) instead of hardcoded
//! 'operational'. The status SHALL be cached (5-minute TTL) and shown
//! in the outages view with freshness indicator.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Cached status with timestamp.
type StatusCache = Arc<RwLock<Option<(GitHubStatus, DateTime<Utc>)>>>;
use tokio::sync::RwLock;

/// GitHub status (from status.github.com API).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubStatus {
    /// Overall status: "operational", "degraded", "outage".
    pub status: String,
    /// Components that are not operational, e.g. Actions "partial outage".
    #[serde(default)]
    pub components: Vec<Component>,
    /// Unresolved incidents.
    pub incidents: Vec<Incident>,
    /// When the status was fetched.
    pub fetched_at: DateTime<Utc>,
    /// Source URL.
    pub source: String,
}

/// A status-page component that is not operational.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    pub name: String,
    pub status: String,
}

/// A GitHub status incident.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incident {
    pub title: String,
    pub status: String,
    /// none, minor, major or critical.
    #[serde(default)]
    pub impact: String,
    pub started_at: Option<DateTime<Utc>>,
    pub url: String,
}

/// statuspage.io `summary.json`: overall indicator, components, and the
/// incidents that are not resolved, in one request.
#[derive(Debug, Deserialize)]
struct SummaryApi {
    status: StatusApiStatus,
    #[serde(default)]
    components: Vec<ComponentApi>,
    #[serde(default)]
    incidents: Vec<IncidentApi>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct StatusApiStatus {
    indicator: String,
    description: String,
}

#[derive(Debug, Deserialize)]
struct ComponentApi {
    name: String,
    status: String,
    #[serde(default)]
    group: bool,
}

#[derive(Debug, Deserialize)]
struct IncidentApi {
    name: String,
    status: String,
    #[serde(default)]
    impact: String,
    created_at: String,
    #[serde(default)]
    shortlink: String,
}

/// Parse a statuspage.io `summary.json` body. Only unresolved incidents and
/// components that are not operational are kept.
pub fn parse_summary(body: &str, source: &str) -> anyhow::Result<GitHubStatus> {
    let s: SummaryApi = serde_json::from_str(body)?;
    let status = match s.status.indicator.as_str() {
        "none" => "operational",
        "minor" => "degraded",
        "major" | "critical" => "outage",
        _ => "unknown",
    };
    let components = s
        .components
        .into_iter()
        .filter(|c| !c.group && c.status != "operational")
        .map(|c| Component {
            name: c.name,
            status: c.status.replace('_', " "),
        })
        .collect();
    let incidents = s
        .incidents
        .into_iter()
        .filter(|i| i.status != "resolved" && i.status != "postmortem")
        .map(|i| Incident {
            title: i.name,
            status: i.status,
            impact: i.impact,
            started_at: DateTime::parse_from_rfc3339(&i.created_at)
                .ok()
                .map(|d| d.with_timezone(&Utc)),
            url: i.shortlink,
        })
        .collect();
    Ok(GitHubStatus {
        status: status.to_string(),
        components,
        incidents,
        fetched_at: Utc::now(),
        source: source.to_string(),
    })
}

/// GitHub status client with caching.
pub struct StatusClient {
    /// Status API URL (default: https://status.github.com/api/v2).
    api_base: String,
    /// HTTP client.
    client: reqwest::Client,
    /// Cached status.
    cache: StatusCache,
    /// Cache TTL (default: 5 minutes).
    cache_ttl: Duration,
}

impl StatusClient {
    /// Create a new status client for github.com.
    pub fn github() -> Self {
        Self::new("https://www.githubstatus.com/api/v2")
    }

    /// Create a new status client for a custom status page (e.g., GHE).
    pub fn new(api_base: &str) -> Self {
        Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            cache: Arc::new(RwLock::new(None)),
            cache_ttl: Duration::minutes(5),
        }
    }

    /// Get the current GitHub status (cached).
    pub async fn get_status(&self) -> anyhow::Result<GitHubStatus> {
        if self.api_base == "off" {
            anyhow::bail!("not monitored here (status_url = \"off\")");
        }
        // Check cache
        {
            let cache = self.cache.read().await;
            if let Some((status, cached_at)) = cache.as_ref() {
                if Utc::now() - *cached_at < self.cache_ttl {
                    return Ok(status.clone());
                }
            }
        }

        // Fetch fresh status
        let status = self.fetch_status().await?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            *cache = Some((status.clone(), Utc::now()));
        }

        Ok(status)
    }

    /// Fetch fresh status from the status page (not GitHub's API: no rate
    /// limit is spent).
    async fn fetch_status(&self) -> anyhow::Result<GitHubStatus> {
        let url = format!("{}/summary.json", self.api_base);
        let body = self
            .client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        parse_summary(&body, &self.api_base)
    }

    /// Invalidate the cache (force fresh fetch on next call).
    pub async fn invalidate(&self) {
        let mut cache = self.cache.write().await;
        *cache = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_client_creation() {
        let client = StatusClient::github();
        assert_eq!(client.api_base, "https://www.githubstatus.com/api/v2");
        assert_eq!(client.cache_ttl, Duration::minutes(5));
    }

    #[test]
    fn test_status_client_custom() {
        let client = StatusClient::new("https://status.ghe.internal/api/v2");
        assert_eq!(client.api_base, "https://status.ghe.internal/api/v2");
    }

    #[test]
    fn test_status_serialization() {
        let status = GitHubStatus {
            status: "operational".to_string(),
            components: vec![],
            incidents: vec![],
            fetched_at: Utc::now(),
            source: "https://status.github.com/api/v2".to_string(),
        };

        let json = serde_json::to_string(&status).unwrap();
        let parsed: GitHubStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.status, "operational");
        assert!(parsed.incidents.is_empty());
    }

    #[test]
    fn test_incident_serialization() {
        let incident = Incident {
            title: "Test incident".to_string(),
            status: "investigating".to_string(),
            impact: "major".to_string(),
            started_at: None,
            url: "https://status.github.com/incidents/123".to_string(),
        };

        let json = serde_json::to_string(&incident).unwrap();
        let parsed: Incident = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.title, "Test incident");
        assert_eq!(parsed.status, "investigating");
    }
}

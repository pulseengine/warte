//! GitHub status API integration — real status from status.github.com.
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
    /// Active incidents.
    pub incidents: Vec<Incident>,
    /// When the status was fetched.
    pub fetched_at: DateTime<Utc>,
    /// Source URL.
    pub source: String,
}

/// A GitHub status incident.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incident {
    pub title: String,
    pub status: String,
    pub started_at: Option<DateTime<Utc>>,
    pub url: String,
}

/// GitHub status API response (from status.github.com/api/v2/status.json).
#[derive(Debug, Deserialize)]
struct StatusApiResponse {
    status: StatusApiStatus,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct StatusApiStatus {
    indicator: String,
    description: String,
}

/// GitHub incidents API response (from status.github.com/api/v2/incidents.json).
#[derive(Debug, Deserialize)]
struct IncidentsApiResponse {
    incidents: Vec<IncidentApi>,
}

#[derive(Debug, Deserialize)]
struct IncidentApi {
    name: String,
    status: String,
    created_at: String,
    shortlink: String,
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
        Self::new("https://status.github.com/api/v2")
    }

    /// Create a new status client for a custom status page (e.g., GHE).
    pub fn new(api_base: &str) -> Self {
        Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
            cache: Arc::new(RwLock::new(None)),
            cache_ttl: Duration::minutes(5),
        }
    }

    /// Get the current GitHub status (cached).
    pub async fn get_status(&self) -> anyhow::Result<GitHubStatus> {
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

    /// Fetch fresh status from the API.
    async fn fetch_status(&self) -> anyhow::Result<GitHubStatus> {
        // Fetch overall status
        let status_url = format!("{}/status.json", self.api_base);
        let status_resp: StatusApiResponse =
            self.client.get(&status_url).send().await?.json().await?;

        let overall = match status_resp.status.indicator.as_str() {
            "none" => "operational",
            "minor" => "degraded",
            "major" => "outage",
            "critical" => "outage",
            _ => "unknown",
        };

        // Fetch active incidents
        let incidents_url = format!("{}/incidents.json", self.api_base);
        let incidents_resp: IncidentsApiResponse = self
            .client
            .get(&incidents_url)
            .send()
            .await?
            .json()
            .await
            .unwrap_or(IncidentsApiResponse { incidents: vec![] });

        let incidents: Vec<Incident> = incidents_resp
            .incidents
            .iter()
            .map(|inc| Incident {
                title: inc.name.clone(),
                status: inc.status.clone(),
                started_at: DateTime::parse_from_rfc3339(&inc.created_at)
                    .ok()
                    .map(|d| d.with_timezone(&Utc)),
                url: inc.shortlink.clone(),
            })
            .collect();

        Ok(GitHubStatus {
            status: overall.to_string(),
            incidents,
            fetched_at: Utc::now(),
            source: self.api_base.clone(),
        })
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
        assert_eq!(client.api_base, "https://status.github.com/api/v2");
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
            started_at: None,
            url: "https://status.github.com/incidents/123".to_string(),
        };

        let json = serde_json::to_string(&incident).unwrap();
        let parsed: Incident = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.title, "Test incident");
        assert_eq!(parsed.status, "investigating");
    }
}

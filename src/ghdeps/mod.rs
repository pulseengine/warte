//! GitHub dependency graph API integration — auto-discover dependencies.
//!
//! REQ-CRIT-003: The critical path computer SHALL auto-discover cross-repo
//! dependencies from the GitHub dependency graph API
//! (GET /repos/{owner}/{repo}/dependency-graph/sbom) in addition to
//! rivet externals.
//!
//! The integration:
//! 1. Queries the GitHub dependency graph API for each repo
//! 2. Parses the SBOM (SPDX JSON) to extract dependencies on other pulseengine repos
//! 3. Feeds the graph into CriticalPathComputer
//! 4. Caches the discovered graph (5-minute TTL)
//! 5. Falls back to rivet externals and manual declaration

use crate::critpath::{CriticalPathComputer, RepoDependency, RepoNode};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Cached dependencies with timestamp.
type DepsCache = Arc<RwLock<Option<(Vec<RepoDependency>, DateTime<Utc>)>>>;
use tokio::sync::RwLock;

/// GitHub dependency graph SBOM response (SPDX JSON format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpdxSbom {
    #[serde(rename = "spdxVersion")]
    pub spdx_version: String,
    #[serde(default)]
    pub packages: Vec<SpdxPackage>,
    #[serde(default)]
    pub relationships: Vec<SpdxRelationship>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpdxPackage {
    #[serde(rename = "SPDXID")]
    pub spdx_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "externalRefs", default)]
    pub external_refs: Vec<SpdxExternalRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpdxExternalRef {
    #[serde(rename = "referenceCategory")]
    pub reference_category: String,
    #[serde(rename = "referenceLocator")]
    pub reference_locator: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpdxRelationship {
    #[serde(rename = "spdxElementId")]
    pub spdx_element_id: String,
    #[serde(rename = "relationshipType")]
    pub relationship_type: String,
    #[serde(rename = "relatedSpdxElement")]
    pub related_spdx_element: String,
}

/// GitHub dependency graph integration.
pub struct GitHubDepGraphIntegration {
    /// GitHub API base URL.
    api_base: String,
    /// Organization name.
    org: String,
    /// GitHub token.
    token: String,
    /// HTTP client.
    client: reqwest::Client,
    /// Cached dependency graph.
    cache: DepsCache,
    /// Cache TTL (default: 5 minutes).
    cache_ttl: Duration,
}

impl GitHubDepGraphIntegration {
    /// Create a new GitHub dependency graph integration.
    pub fn new(org: &str, token: &str) -> Self {
        Self::with_api_base("https://api.github.com", org, token)
    }

    /// Create with a custom API base (for GHE).
    pub fn with_api_base(api_base: &str, org: &str, token: &str) -> Self {
        Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            org: org.to_string(),
            token: token.to_string(),
            client: reqwest::Client::new(),
            cache: Arc::new(RwLock::new(None)),
            cache_ttl: Duration::minutes(5),
        }
    }

    /// Discover dependencies from all repos.
    pub async fn discover(&self, repos: &[String]) -> anyhow::Result<Vec<RepoDependency>> {
        // Check cache
        {
            let cache = self.cache.read().await;
            if let Some((deps, cached_at)) = cache.as_ref() {
                if Utc::now() - *cached_at < self.cache_ttl {
                    return Ok(deps.clone());
                }
            }
        }

        // Discover from each repo
        let mut all_deps = Vec::new();
        for repo in repos {
            match self.discover_repo(repo).await {
                Ok(deps) => all_deps.extend(deps),
                Err(e) => {
                    tracing::warn!(
                        repo = %repo,
                        error = %e,
                        "failed to discover dependencies from GitHub"
                    );
                }
            }
        }

        // Update cache
        {
            let mut cache = self.cache.write().await;
            *cache = Some((all_deps.clone(), Utc::now()));
        }

        Ok(all_deps)
    }

    /// Discover dependencies from a single repo.
    async fn discover_repo(&self, repo: &str) -> anyhow::Result<Vec<RepoDependency>> {
        let url = format!(
            "{}/repos/{}/{}/dependency-graph/sbom",
            self.api_base, self.org, repo
        );

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/spdx+json")
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("GitHub API returned {}", response.status());
        }

        let sbom: SpdxSbom = response.json().await?;

        // Extract dependencies on other pulseengine repos
        let deps: Vec<RepoDependency> = sbom
            .packages
            .iter()
            .filter_map(|pkg| {
                // Look for external refs pointing to other pulseengine repos
                let dep_repo = pkg.external_refs.iter().find_map(|ext| {
                    if ext
                        .reference_locator
                        .contains(&format!("github.com/{}/", self.org))
                    {
                        // Extract repo name from URL
                        ext.reference_locator.rsplit('/').next().map(String::from)
                    } else {
                        None
                    }
                })?;

                Some(RepoDependency {
                    repo: repo.to_string(),
                    depends_on: vec![dep_repo],
                })
            })
            .collect();

        Ok(deps)
    }

    /// Build a CriticalPathComputer from discovered dependencies + repo nodes.
    pub async fn build_computer(
        &self,
        repos: &[String],
        nodes: Vec<RepoNode>,
        manual_deps: Vec<RepoDependency>,
    ) -> anyhow::Result<CriticalPathComputer> {
        let mut computer = CriticalPathComputer::new();

        // Add manual dependencies first (fallback)
        for dep in manual_deps {
            computer.add_dependency(dep);
        }

        // Add discovered dependencies
        match self.discover(repos).await {
            Ok(discovered) => {
                for dep in discovered {
                    computer.add_dependency(dep);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "GitHub dependency graph discovery failed, using manual deps only");
            }
        }

        // Add repo nodes
        for node in nodes {
            computer.add_node(node);
        }

        Ok(computer)
    }

    /// Invalidate the cache.
    pub async fn invalidate(&self) {
        let mut cache = self.cache.write().await;
        *cache = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spdx_sbom_deserialization() {
        let json = r#"{
            "spdxVersion": "SPDX-2.3",
            "packages": [
                {
                    "SPDXID": "SPDXRef-Package-1",
                    "name": "relay",
                    "externalRefs": [
                        {
                            "referenceCategory": "PACKAGE-MANAGER",
                            "referenceLocator": "https://github.com/pulseengine/relay"
                        }
                    ]
                }
            ],
            "relationships": []
        }"#;

        let sbom: SpdxSbom = serde_json::from_str(json).unwrap();
        assert_eq!(sbom.spdx_version, "SPDX-2.3");
        assert_eq!(sbom.packages.len(), 1);
        assert_eq!(sbom.packages[0].name, "relay");
    }

    #[test]
    fn test_integration_creation() {
        let integration = GitHubDepGraphIntegration::new("pulseengine", "fake-token");
        assert_eq!(integration.org, "pulseengine");
        assert_eq!(integration.api_base, "https://api.github.com");
        assert_eq!(integration.cache_ttl, Duration::minutes(5));
    }

    #[test]
    fn test_integration_ghe() {
        let integration = GitHubDepGraphIntegration::with_api_base(
            "https://ghe.internal/api/v3",
            "pulseengine",
            "fake-token",
        );
        assert_eq!(integration.api_base, "https://ghe.internal/api/v3");
    }
}

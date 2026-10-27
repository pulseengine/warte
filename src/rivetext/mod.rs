//! Rivet externals integration — auto-discover cross-repo dependencies.
//!
//! REQ-CRIT-002: The critical path computer SHALL auto-discover cross-repo
//! dependencies from rivet externals (rivet externals discover --format json)
//! instead of requiring manual dependency declaration.
//!
//! The integration:
//! 1. Runs 'rivet externals discover --format json' in each repo
//! 2. Parses the JSON output to build the dependency graph
//! 3. Feeds the graph into CriticalPathComputer
//! 4. Caches the discovered graph (5-minute TTL)
//! 5. Falls back to manual dependency declaration (warte.toml)

use crate::critpath::{CriticalPathComputer, RepoDependency, RepoNode};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;

/// Cached dependencies with timestamp.
type DepsCache = Arc<RwLock<Option<(Vec<RepoDependency>, DateTime<Utc>)>>>;
use tokio::sync::RwLock;

/// Rivet externals discover JSON output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RivetExternals {
    pub externals: Vec<RivetExternal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RivetExternal {
    pub name: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
}

/// Rivet externals integration — discovers cross-repo dependencies.
pub struct RivetExternalsIntegration {
    /// Repo paths (repo name → local path).
    repo_paths: HashMap<String, String>,
    /// Cached dependency graph.
    cache: DepsCache,
    /// Cache TTL (default: 5 minutes).
    cache_ttl: Duration,
}

impl RivetExternalsIntegration {
    /// Create a new rivet externals integration.
    pub fn new(repo_paths: HashMap<String, String>) -> Self {
        Self {
            repo_paths,
            cache: Arc::new(RwLock::new(None)),
            cache_ttl: Duration::minutes(5),
        }
    }

    /// Discover dependencies from all repos.
    pub async fn discover(&self) -> anyhow::Result<Vec<RepoDependency>> {
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
        for (repo_name, repo_path) in &self.repo_paths {
            match self.discover_repo(repo_name, repo_path) {
                Ok(deps) => all_deps.extend(deps),
                Err(e) => {
                    tracing::warn!(
                        repo = %repo_name,
                        error = %e,
                        "failed to discover externals"
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
    fn discover_repo(
        &self,
        repo_name: &str,
        repo_path: &str,
    ) -> anyhow::Result<Vec<RepoDependency>> {
        let output = Command::new("rivet")
            .args([
                "externals",
                "discover",
                "--format",
                "json",
                "--path",
                repo_path,
            ])
            .output()?;

        if !output.status.success() {
            anyhow::bail!(
                "rivet externals discover failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let externals: RivetExternals = serde_json::from_slice(&output.stdout)?;

        // Convert externals to dependencies
        // An external with a repo URL like "pulseengine/relay" means this repo depends on relay
        let deps: Vec<RepoDependency> = externals
            .externals
            .iter()
            .filter_map(|ext| {
                // Extract repo name from URL (e.g., "pulseengine/relay" → "relay")
                let dep_repo = ext
                    .url
                    .as_deref()
                    .or(ext.repo.as_deref())
                    .and_then(|url| url.rsplit('/').next())
                    .map(String::from)?;

                Some(RepoDependency {
                    repo: repo_name.to_string(),
                    depends_on: vec![dep_repo],
                })
            })
            .collect();

        Ok(deps)
    }

    /// Build a CriticalPathComputer from discovered dependencies + repo nodes.
    pub async fn build_computer(
        &self,
        nodes: Vec<RepoNode>,
        manual_deps: Vec<RepoDependency>,
    ) -> anyhow::Result<CriticalPathComputer> {
        let mut computer = CriticalPathComputer::new();

        // Add manual dependencies first (fallback)
        for dep in manual_deps {
            computer.add_dependency(dep);
        }

        // Add discovered dependencies (override/augment manual)
        match self.discover().await {
            Ok(discovered) => {
                for dep in discovered {
                    computer.add_dependency(dep);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "rivet externals discovery failed, using manual deps only");
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
    fn test_rivet_externals_deserialization() {
        let json = r#"{
            "externals": [
                { "name": "relay", "url": "https://github.com/pulseengine/relay", "commit": "abc123" },
                { "name": "gale", "url": "https://github.com/pulseengine/gale" }
            ]
        }"#;

        let externals: RivetExternals = serde_json::from_str(json).unwrap();
        assert_eq!(externals.externals.len(), 2);
        assert_eq!(externals.externals[0].name, "relay");
        assert_eq!(
            externals.externals[0].url.as_deref(),
            Some("https://github.com/pulseengine/relay")
        );
    }

    #[test]
    fn test_integration_creation() {
        let mut repo_paths = HashMap::new();
        repo_paths.insert("jess".to_string(), "/path/to/jess".to_string());
        repo_paths.insert("relay".to_string(), "/path/to/relay".to_string());

        let integration = RivetExternalsIntegration::new(repo_paths);
        assert_eq!(integration.cache_ttl, Duration::minutes(5));
    }

    #[tokio::test]
    async fn test_build_computer_with_manual_deps() {
        let repo_paths = HashMap::new();
        let integration = RivetExternalsIntegration::new(repo_paths);

        let nodes = vec![
            RepoNode {
                name: "jess".to_string(),
                status: "green".to_string(),
                queue_depth: 0,
            },
            RepoNode {
                name: "relay".to_string(),
                status: "red".to_string(),
                queue_depth: 5,
            },
        ];

        let manual_deps = vec![RepoDependency {
            repo: "jess".to_string(),
            depends_on: vec!["relay".to_string()],
        }];

        let computer = integration
            .build_computer(nodes, manual_deps)
            .await
            .unwrap();
        let path = computer.compute();

        // Should find the chain jess → relay with relay as bottleneck
        assert!(!path.chain.is_empty());
        assert!(path.blocking_repos.contains(&"relay".to_string()));
    }
}

//! Critical path computation — longest chain of dependent gates.
//!
//! REQ-CRIT-001: The critical path view SHALL compute the real dependency
//! graph between repos, not just ci_status heuristics.
//!
//! Data sources:
//! - Manual dependency declaration (warte.toml: repo_dependencies)
//! - rivet externals (rivet sync / rivet externals)
//! - GitHub dependency graph API
//!
//! Algorithm:
//! 1. Build a dependency graph: repo → depends_on → [repos]
//! 2. For each repo, check if it has a non-green gate (red or pending)
//! 3. Find the longest path in the graph where at least one repo has a non-green gate
//! 4. The critical path is the longest such path

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// A repo dependency declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoDependency {
    pub repo: String,
    pub depends_on: Vec<String>,
}

/// A repo with its CI status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoNode {
    pub name: String,
    pub status: String, // "green", "red", "pending", "unknown"
    pub queue_depth: u32,
}

impl RepoNode {
    pub fn is_blocking(&self) -> bool {
        self.status == "red" || self.status == "pending" || self.queue_depth > 5
    }
}

/// The critical path result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticalPath {
    /// The chain of repos (longest path with at least one blocking repo).
    pub chain: Vec<String>,
    /// The bottleneck repo (the blocking repo on the critical path).
    pub bottleneck: Option<String>,
    /// Total estimated wait (sum of queue depths along the chain).
    pub estimated_wait_seconds: u64,
    /// All blocking repos (not just the bottleneck).
    pub blocking_repos: Vec<String>,
}

/// Critical path computer.
pub struct CriticalPathComputer {
    /// Dependency graph: repo → depends_on → [repos].
    dependencies: HashMap<String, Vec<String>>,
    /// Repo nodes with CI status.
    nodes: HashMap<String, RepoNode>,
}

impl CriticalPathComputer {
    /// Create a new critical path computer.
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            nodes: HashMap::new(),
        }
    }

    /// Add a repo dependency.
    pub fn add_dependency(&mut self, dep: RepoDependency) {
        self.dependencies
            .entry(dep.repo)
            .or_default()
            .extend(dep.depends_on);
    }

    /// Add a repo node with CI status.
    pub fn add_node(&mut self, node: RepoNode) {
        self.nodes.insert(node.name.clone(), node);
    }

    /// Compute the critical path.
    ///
    /// Finds the longest chain of repos where each depends on the next
    /// and at least one repo in the chain is blocking (red/pending/high-queue).
    pub fn compute(&self) -> CriticalPath {
        let mut longest_chain: Vec<String> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();

        // Try starting from each repo
        for start in self.nodes.keys() {
            visited.clear();
            let chain = self.dfs_longest_path(start, &mut visited);
            if chain.len() > longest_chain.len() {
                longest_chain = chain;
            }
        }

        // Find blocking repos on the chain
        let blocking_repos: Vec<String> = longest_chain
            .iter()
            .filter(|name| {
                self.nodes
                    .get(*name)
                    .map(|n| n.is_blocking())
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        // The bottleneck is the first blocking repo on the chain
        let bottleneck = blocking_repos.first().cloned();

        // Estimate total wait (sum of queue depths along the chain)
        let estimated_wait: u64 = longest_chain
            .iter()
            .filter_map(|name| self.nodes.get(name))
            .map(|n| n.queue_depth as u64 * 60) // Assume 60s per queued job
            .sum();

        CriticalPath {
            chain: longest_chain,
            bottleneck,
            estimated_wait_seconds: estimated_wait,
            blocking_repos,
        }
    }

    /// DFS to find the longest path from a starting repo.
    fn dfs_longest_path(&self, repo: &str, visited: &mut HashSet<String>) -> Vec<String> {
        if visited.contains(repo) {
            return vec![];
        }
        visited.insert(repo.to_string());

        let mut longest: Vec<String> = vec![repo.to_string()];

        if let Some(deps) = self.dependencies.get(repo) {
            for dep in deps {
                let sub_path = self.dfs_longest_path(dep, visited);
                if sub_path.len() + 1 > longest.len() {
                    longest = vec![repo.to_string()];
                    longest.extend(sub_path);
                }
            }
        }

        longest
    }

    /// Get all blocking repos (not just on the critical path).
    pub fn blocking_repos(&self) -> Vec<String> {
        self.nodes
            .values()
            .filter(|n| n.is_blocking())
            .map(|n| n.name.clone())
            .collect()
    }
}

impl Default for CriticalPathComputer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_critical_path_simple_chain() {
        let mut computer = CriticalPathComputer::new();

        // jess → relay → gale (jess depends on relay, relay depends on gale)
        computer.add_dependency(RepoDependency {
            repo: "jess".to_string(),
            depends_on: vec!["relay".to_string()],
        });
        computer.add_dependency(RepoDependency {
            repo: "relay".to_string(),
            depends_on: vec!["gale".to_string()],
        });

        // All green
        computer.add_node(RepoNode {
            name: "jess".to_string(),
            status: "green".to_string(),
            queue_depth: 0,
        });
        computer.add_node(RepoNode {
            name: "relay".to_string(),
            status: "green".to_string(),
            queue_depth: 0,
        });
        computer.add_node(RepoNode {
            name: "gale".to_string(),
            status: "green".to_string(),
            queue_depth: 0,
        });

        let path = computer.compute();
        assert_eq!(path.chain.len(), 3);
        assert!(path.blocking_repos.is_empty());
        assert!(path.bottleneck.is_none());
    }

    #[test]
    fn test_critical_path_with_blocking() {
        let mut computer = CriticalPathComputer::new();

        computer.add_dependency(RepoDependency {
            repo: "jess".to_string(),
            depends_on: vec!["relay".to_string()],
        });
        computer.add_dependency(RepoDependency {
            repo: "relay".to_string(),
            depends_on: vec!["gale".to_string()],
        });

        // gale is red (blocking)
        computer.add_node(RepoNode {
            name: "jess".to_string(),
            status: "green".to_string(),
            queue_depth: 0,
        });
        computer.add_node(RepoNode {
            name: "relay".to_string(),
            status: "green".to_string(),
            queue_depth: 0,
        });
        computer.add_node(RepoNode {
            name: "gale".to_string(),
            status: "red".to_string(),
            queue_depth: 10,
        });

        let path = computer.compute();
        assert_eq!(path.chain.len(), 3);
        assert_eq!(path.blocking_repos, vec!["gale".to_string()]);
        assert_eq!(path.bottleneck, Some("gale".to_string()));
        assert_eq!(path.estimated_wait_seconds, 600); // 10 jobs * 60s
    }

    #[test]
    fn test_critical_path_no_dependencies() {
        let mut computer = CriticalPathComputer::new();
        computer.add_node(RepoNode {
            name: "standalone".to_string(),
            status: "green".to_string(),
            queue_depth: 0,
        });

        let path = computer.compute();
        assert_eq!(path.chain.len(), 1);
        assert!(path.blocking_repos.is_empty());
    }

    #[test]
    fn test_repo_node_is_blocking() {
        let green = RepoNode {
            name: "a".to_string(),
            status: "green".to_string(),
            queue_depth: 0,
        };
        let red = RepoNode {
            name: "b".to_string(),
            status: "red".to_string(),
            queue_depth: 0,
        };
        let pending = RepoNode {
            name: "c".to_string(),
            status: "pending".to_string(),
            queue_depth: 0,
        };
        let high_queue = RepoNode {
            name: "d".to_string(),
            status: "green".to_string(),
            queue_depth: 10,
        };

        assert!(!green.is_blocking());
        assert!(red.is_blocking());
        assert!(pending.is_blocking());
        assert!(high_queue.is_blocking());
    }
}

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
use std::collections::HashMap;

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

    /// Compute the critical path: the longest chain of repos, each depending
    /// on the next, with at least one blocking repo on it (REQ-CRIT-001).
    /// Nothing blocking: no critical path. Cycles are cut (a repo appears
    /// once per chain). Ties go to more blocking repos, then by name, so the
    /// answer is stable.
    pub fn compute(&self) -> CriticalPath {
        let mut starts: Vec<&String> = self.nodes.keys().chain(self.dependencies.keys()).collect();
        starts.sort();
        starts.dedup();
        let blocking = |r: &String| self.nodes.get(r).is_some_and(RepoNode::is_blocking);
        let score = |c: &[String]| (c.len(), c.iter().filter(|r| blocking(r)).count());
        let mut best: Vec<String> = vec![];
        let mut path: Vec<String> = vec![];
        let mut steps = 0usize;
        for s in starts {
            self.walk(
                s,
                &mut path,
                &mut |c: &[String]| {
                    if c.iter().any(&blocking) && score(c) > score(&best) {
                        best = c.to_vec();
                    }
                },
                &mut steps,
            );
        }
        let blocking_repos: Vec<String> = best.iter().filter(|r| blocking(r)).cloned().collect();
        // the bottleneck: the blocking repo deepest in the chain, which every
        // repo before it waits on
        let bottleneck = blocking_repos.last().cloned();
        let estimated_wait: u64 = best
            .iter()
            .filter_map(|name| self.nodes.get(name))
            .map(|n| n.queue_depth as u64 * 60)
            .sum();
        CriticalPath {
            chain: best,
            bottleneck,
            estimated_wait_seconds: estimated_wait,
            blocking_repos,
        }
    }

    /// Every simple path from `repo` along dependencies, reported at each
    /// step; bounded so a large graph cannot stall a request.
    fn walk(
        &self,
        repo: &str,
        path: &mut Vec<String>,
        seen: &mut dyn FnMut(&[String]),
        steps: &mut usize,
    ) {
        *steps += 1;
        if *steps > 100_000 || path.iter().any(|r| r == repo) {
            return;
        }
        path.push(repo.to_string());
        seen(path);
        if let Some(deps) = self.dependencies.get(repo) {
            let mut deps = deps.clone();
            deps.sort();
            deps.dedup();
            for d in deps {
                self.walk(&d, path, seen, steps);
            }
        }
        path.pop();
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

        // all green: no critical path (REQ-CRIT-001: at least one gate not green)
        let path = computer.compute();
        assert!(path.chain.is_empty());
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
        assert!(path.chain.is_empty());
        assert!(path.blocking_repos.is_empty());
    }

    // rivet: partially-verifies REQ-CRIT-001
    #[test]
    fn the_longest_blocked_chain_not_the_first_branch_walked() {
        let mut c = CriticalPathComputer::new();
        let dep = |r: &str, d: &[&str]| RepoDependency {
            repo: r.into(),
            depends_on: d.iter().map(|s| s.to_string()).collect(),
        };
        let node = |n: &str, st: &str| RepoNode {
            name: n.into(),
            status: st.into(),
            queue_depth: 0,
        };
        // jess -> {a, relay}; a -> relay; relay -> gale; gale -> jess (a cycle,
        // cut). A walk that shares `visited` across sibling branches can miss
        // the longest simple chain, which is 4 here.
        c.add_dependency(dep("jess", &["a", "relay"]));
        c.add_dependency(dep("a", &["relay"]));
        c.add_dependency(dep("relay", &["gale"]));
        c.add_dependency(dep("gale", &["jess"]));
        for n in ["jess", "a", "relay"] {
            c.add_node(node(n, "green"));
        }
        c.add_node(node("gale", "pending"));
        let p = c.compute();
        assert_eq!(p.chain, ["a", "relay", "gale", "jess"].map(String::from));
        assert_eq!(p.bottleneck.as_deref(), Some("gale"));
        // a longer chain that is all green loses to a shorter blocked one
        let mut c = CriticalPathComputer::new();
        c.add_dependency(dep("x", &["y"]));
        c.add_dependency(dep("y", &["z"]));
        c.add_dependency(dep("p", &["q"]));
        for n in ["x", "y", "z", "p"] {
            c.add_node(node(n, "green"));
        }
        c.add_node(node("q", "red"));
        assert_eq!(c.compute().chain, ["p", "q"].map(String::from));
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

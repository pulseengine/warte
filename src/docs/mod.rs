//! Embedded documentation — `warte docs <topic>` (like `varve docs`, `rivet docs`).

use std::collections::BTreeMap;

/// A documentation topic embedded at compile time.
struct Topic {
    name: &'static str,
    title: &'static str,
    content: &'static str,
}

/// All embedded topics, loaded via include_str! at compile time.
const TOPICS: &[Topic] = &[
    Topic {
        name: "architecture",
        title: "Architecture",
        content: include_str!("../../docs/topics/architecture.md"),
    },
    Topic {
        name: "agent-registration",
        title: "Agent registration",
        content: include_str!("../../docs/topics/agent-registration.md"),
    },
    Topic {
        name: "access-control",
        title: "Access control",
        content: include_str!("../../docs/topics/access-control.md"),
    },
    Topic {
        name: "artifact-distribution",
        title: "Artifact distribution",
        content: include_str!("../../docs/topics/artifact-distribution.md"),
    },
    Topic {
        name: "multi-instance-sync",
        title: "Multi-instance sync",
        content: include_str!("../../docs/topics/multi-instance-sync.md"),
    },
    Topic {
        name: "deployment",
        title: "Deployment",
        content: include_str!("../../docs/topics/deployment.md"),
    },
    Topic {
        name: "security-model",
        title: "Security model",
        content: include_str!("../../docs/topics/security-model.md"),
    },
    Topic {
        name: "cli-reference",
        title: "CLI reference",
        content: include_str!("../../docs/topics/cli-reference.md"),
    },
    Topic {
        name: "api-reference",
        title: "API reference",
        content: include_str!("../../docs/topics/api-reference.md"),
    },
    Topic {
        name: "migration",
        title: "Migration guide",
        content: include_str!("../../docs/topics/migration.md"),
    },
    Topic {
        name: "ground-station",
        title: "Ground station (absorbed smithy hub)",
        content: include_str!("../../docs/topics/ground-station.md"),
    },
];

/// Get all topics as a sorted map.
fn topics() -> BTreeMap<&'static str, &'static Topic> {
    TOPICS.iter().map(|t| (t.name, t)).collect()
}

/// List all available topics.
pub fn list_topics() -> Vec<(&'static str, &'static str)> {
    topics()
        .iter()
        .map(|(name, topic)| (*name, topic.title))
        .collect()
}

/// Get a topic by name. Returns None if not found.
pub fn get_topic(name: &str) -> Option<&'static str> {
    topics().get(name).map(|t| t.content)
}

/// Print the topic list (for `warte docs`).
pub fn print_topic_list() {
    println!("warte docs — embedded documentation\n");
    println!("Usage: warte docs <topic>\n");
    println!("Topics:");
    for (name, title) in list_topics() {
        println!("  {:<24} {}", name, title);
    }
    println!("\nChecks:");
    println!("  check --coverage    Assert every subcommand is documented");
}

/// Print a single topic (for `warte docs <topic>`).
pub fn print_topic(name: &str) -> bool {
    match get_topic(name) {
        Some(content) => {
            println!("{}", content);
            true
        }
        None => {
            eprintln!("unknown topic: '{}'", name);
            eprintln!("run `warte docs` to list available topics");
            false
        }
    }
}

/// Check documentation coverage — assert every CLI subcommand has a topic.
/// Returns the list of undocumented subcommands.
pub fn check_coverage() -> Vec<String> {
    // Every subcommand the CLI actually has, read from clap, so a new command
    // cannot be added without being documented.
    use clap::CommandFactory;
    let cli = crate::cli::Cli::command();
    let expected_docs: Vec<String> = cli
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .filter(|n| n != "help")
        .collect();

    // Topics that exist (broader than subcommands — includes architecture, security, etc.)
    let documented: Vec<&str> = list_topics().iter().map(|(name, _)| *name).collect();

    // For coverage, we check that key areas are documented
    let required_topics = [
        "architecture",
        "agent-registration",
        "access-control",
        "artifact-distribution",
        "multi-instance-sync",
        "deployment",
        "security-model",
        "cli-reference",
        "api-reference",
        "migration",
        "ground-station",
    ];

    let mut missing = Vec::new();
    for topic in required_topics {
        if !documented.contains(&topic) {
            missing.push(topic.to_string());
        }
    }

    // Also check that each subcommand is mentioned in the CLI reference
    let cli_ref = get_topic("cli-reference").unwrap_or("");
    for cmd in &expected_docs {
        if !cli_ref.contains(cmd.as_str()) {
            missing.push(cmd.clone());
        }
    }

    missing
}

/// Run the docs check (for `warte docs check --coverage`).
pub fn run_check() -> bool {
    let missing = check_coverage();
    if missing.is_empty() {
        println!("docs coverage: OK — all topics documented");
        true
    } else {
        eprintln!("docs coverage: MISSING topics:");
        for topic in &missing {
            eprintln!("  - {}", topic);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_topics_load() {
        for (name, _) in list_topics() {
            assert!(get_topic(name).is_some(), "topic '{}' should load", name);
        }
    }

    #[test]
    fn test_coverage_passes() {
        let missing = check_coverage();
        assert!(missing.is_empty(), "missing docs: {:?}", missing);
    }

    #[test]
    fn test_unknown_topic() {
        assert!(get_topic("nonexistent").is_none());
    }
}

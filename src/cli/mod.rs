//! CLI commands for warte.

use clap::{Parser, Subcommand};

/// Warte — the PulseEngine control tower.
#[derive(Parser)]
#[command(name = "warte")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(
    about = "The PulseEngine control tower — CI dashboard, agent orchestration, access control"
)]
#[command(long_about = None)]
pub struct Cli {
    /// Config file path
    #[arg(long, env = "WARTE_CONFIG", default_value = "warte.toml")]
    pub config: String,

    /// Output format
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the dashboard server
    Serve {
        /// Bind address (overrides config)
        #[arg(long)]
        bind: Option<String>,
    },

    /// Show factory health status
    Status,

    /// List registered agents
    Agents {
        /// Filter by health status
        #[arg(long)]
        health: Option<String>,
    },

    /// Show the weekly cron schedule
    Schedule,

    /// Open the dashboard in the browser
    Watch,

    /// Manage access rights
    Access {
        #[command(subcommand)]
        action: AccessCommands,
    },

    /// Migrate data from an existing system
    Migrate {
        /// Source system (smithy or temper)
        source: String,

        /// Path to the source data
        #[arg(long)]
        path: String,
    },

    /// Run the ground station: CCSDS telemetry from the smithy agent in, hub API out
    ///
    /// Same flags and data-dir layout as `smithy hub`, so it replaces it in place.
    Hub {
        /// HTTP listen address. Loopback by default: publish it through Caddy.
        #[arg(long, default_value = "127.0.0.1:8091")]
        http: String,
        /// UDP telemetry listen address. Required, so the exposure is a deliberate
        /// choice: bind the WireGuard interface address where possible.
        #[arg(long)]
        udp: String,
        /// Source address allowed to send telemetry (repeatable), e.g. the CI host's tunnel IP
        #[arg(long, required = true)]
        allow: Vec<std::net::IpAddr>,
        /// Hub state directory (hub-state.json + ledger/)
        #[arg(long, default_value = "/var/lib/smithy-hub")]
        data_dir: String,
        /// GitHub org the agent scans (qualifies repo names in /api/eta)
        #[arg(long, default_value = "pulseengine")]
        org: String,
    },

    /// Manage artifacts
    Artifact {
        #[command(subcommand)]
        action: ArtifactCommands,
    },

    /// Show embedded documentation
    Docs {
        /// Topic name (omit to list all topics)
        topic: Option<String>,

        /// Assert every subcommand is documented
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
pub enum AccessCommands {
    /// Grant access rights to an agent
    Grant {
        agent_id: String,
        /// Rights as resource:action pairs (comma-separated)
        rights: String,
    },
    /// Revoke access rights from an agent
    Revoke {
        agent_id: String,
        /// Rights as resource:action pairs (comma-separated)
        rights: String,
    },
    /// List access rights for an agent
    List { agent_id: String },
}

#[derive(Subcommand)]
pub enum ArtifactCommands {
    /// List available artifacts
    List,
    /// Upload a new artifact
    Upload {
        name: String,
        version: String,
        /// Path to the artifact file
        path: String,
    },
    /// Download an artifact
    Download {
        artifact_id: String,
        /// Output path
        #[arg(long, default_value = ".")]
        output: String,
    },
}

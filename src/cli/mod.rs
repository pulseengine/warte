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

    /// Block until a PR's or branch's CI changes, then print JSON and exit
    ///
    /// Exits 0 on a change, 3 when --timeout passes with none, 1 on error.
    /// Pass the printed cursor back as --since so nothing is missed between
    /// two calls. Meant to run as an agent's background command.
    #[command(group(clap::ArgGroup::new("target").required(true).args(["pr", "branch"])))]
    Watch {
        /// Watch CI runs (the only condition so far)
        #[arg(long, required = true)]
        ci: bool,
        /// Repository name without the org, e.g. spar
        #[arg(long)]
        repo: String,
        #[arg(long)]
        pr: Option<u32>,
        #[arg(long)]
        branch: Option<String>,
        /// Cursor from a previous answer
        #[arg(long)]
        since: Option<String>,
        /// Give up after this many seconds (exit 3)
        #[arg(long, default_value_t = 3600)]
        timeout: u64,
        /// warte's base URL, e.g. https://host/warte (default: the configured bind)
        #[arg(long, env = "WARTE_URL")]
        url: Option<String>,
        /// PEM CA certificate to trust for --url, e.g. a reverse proxy's
        /// local CA, in addition to the built-in roots
        #[arg(long, env = "WARTE_CA_CERT")]
        ca_cert: Option<std::path::PathBuf>,
    },

    /// Manage access rights
    Access {
        #[command(subcommand)]
        action: AccessCommands,
    },

    /// Migrate data from an existing system
    Migrate {
        /// Source system (temper; smithy hub data is read in place by `warte hub`)
        source: String,

        /// Path to the source data
        #[arg(long)]
        path: String,
    },

    /// The fleet model: read a spar instance of the Warte_Fleet types
    Fleet {
        #[command(subcommand)]
        action: FleetCommands,
    },

    /// What this machine is and runs: hardware, disks, agent CLIs, sessions
    Probe {
        #[command(subcommand)]
        action: ProbeCommands,
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
pub enum FleetCommands {
    /// Print the machines and boards a spar instance models; exit 1 when the
    /// model has problems (a board on no machine, a contradicting location)
    Show {
        /// `spar instance --format json` output
        instance: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
pub enum ProbeCommands {
    /// Collect once and print (read-only; nothing is sent)
    Report,
    /// On the ground station: mint a one-time enrollment token for a host
    /// (valid 15 minutes, used once; only its hash is stored)
    Token {
        /// The host name the machine's probe reports
        #[arg(long)]
        host: String,
    },
    /// On the machine: make a key here, enroll with the ground station, and
    /// keep key, certificate and CA in --state-dir
    Enroll {
        /// The token from `warte probe token`
        token: String,
        /// The ground station's base URL, e.g. https://<host>/warte
        #[arg(long, env = "WARTE_URL")]
        ground: String,
        /// Host name to enroll as (default: this machine's)
        #[arg(long)]
        host: Option<String>,
        #[arg(long, default_value = "/var/lib/warte-probe")]
        state_dir: std::path::PathBuf,
        /// PEM CA to trust for --ground (a reverse proxy's local CA)
        #[arg(long, env = "WARTE_CA_CERT")]
        ca_cert: Option<std::path::PathBuf>,
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

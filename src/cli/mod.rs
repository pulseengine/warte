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
    /// Config file path; it must exist when named [default: warte.toml in
    /// the working directory, if there is one]
    #[arg(long, env = "WARTE_CONFIG")]
    pub config: Option<std::path::PathBuf>,

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

    /// List registered agents, or remove one (`warte agents rm`)
    Agents {
        /// Filter by health status
        #[arg(long)]
        health: Option<String>,
        #[command(subcommand)]
        action: Option<AgentsCommands>,
    },

    /// This agent's client commands: GitHub tokens (REQ-GHTOKEN-004)
    Agent {
        #[command(subcommand)]
        action: AgentClientCommands,
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

    /// Operator tools for bearer tokens (run on the ground station host)
    Token {
        #[command(subcommand)]
        action: TokenCommands,
    },

    /// Manage access rights
    Access {
        #[command(subcommand)]
        action: AccessCommands,
    },

    /// Migrate data from an existing system
    Migrate {
        /// Source system (temper; the original hub data is read in place by `warte hub`)
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

    /// News from the tools' public feeds, newest first (contract news v1)
    News {
        /// Items published on or after this day (YYYY-MM-DD) or time (RFC 3339)
        #[arg(long)]
        since: Option<String>,
        /// Only this source id
        #[arg(long)]
        source: Option<String>,
        /// Only items whose title or excerpt contains this (case-insensitive)
        #[arg(long)]
        grep: Option<String>,
        /// Only items in this state: found, decided, landed, dismissed
        #[arg(long)]
        state: Option<String>,
        /// warte's base URL, e.g. https://host/warte (default: the configured bind)
        #[arg(long, env = "WARTE_URL")]
        url: Option<String>,
        /// PEM CA certificate to trust for --url, in addition to the built-in roots
        #[arg(long, env = "WARTE_CA_CERT")]
        ca_cert: Option<std::path::PathBuf>,
        /// Bearer token with news:write, needed when warte has [auth]
        #[arg(long, env = "WARTE_TOKEN", hide_env_values = true)]
        token: Option<String>,
    },

    /// The home link and app switcher file (contract nav v1)
    Nav {
        #[command(subcommand)]
        action: NavCommands,
    },

    /// The agent session list (contract sessions v1)
    Sessions {
        #[command(subcommand)]
        action: SessionsCommands,
    },

    /// Run the ground station: CCSDS telemetry from the fleet agent in, hub API out
    ///
    /// Same flags and data-dir layout as the original hub, so it replaces it in place.
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
        #[arg(long, default_value = "/var/lib/warte-hub")]
        data_dir: String,
        /// GitHub org the agent scans (qualifies repo names in /api/eta)
        #[arg(long, default_value = "pulseengine")]
        org: String,
        /// Name on the hub's page and status report
        #[arg(long, default_value = crate::hub::DEFAULT_BRAND)]
        brand: String,
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
pub enum NavCommands {
    /// Check a nav v1 file; exit 1 naming the file and the field when a rule
    /// fails
    Check {
        /// The nav v1 JSON file
        file: std::path::PathBuf,
        /// The app's own id, which must be in the file (as `[nav] self`)
        #[arg(long = "self", value_name = "ID")]
        self_id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum SessionsCommands {
    /// Check a sessions v1 file; exit 1 naming the file and the field when a
    /// rule fails
    Check {
        /// The sessions v1 JSON file
        file: std::path::PathBuf,
    },
    /// Each session's state: running, missing, paused or disabled (contract lifecycle v1)
    Status {
        #[command(flatten)]
        at: LifecycleArgs,
    },
    /// Start the missing sessions, or the one --id names, each in a detached
    /// zellij session; never stops or restarts a running one
    Start {
        #[command(flatten)]
        at: LifecycleArgs,
        /// Only this session id
        #[arg(long)]
        id: Option<String>,
        /// Why, kept in the event log (e.g. "login", "crashed at 09:10")
        #[arg(long)]
        reason: String,
        /// Print each start's argv and working directory as JSON lines; start nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Keep a session from being started until it is resumed
    Pause {
        #[command(flatten)]
        at: LifecycleArgs,
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// Let a paused session be started again
    Resume {
        #[command(flatten)]
        at: LifecycleArgs,
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// Forget warte's state for a session; refused while it runs; never
    /// touches its transcripts
    Rm {
        #[command(flatten)]
        at: LifecycleArgs,
        id: String,
        #[arg(long)]
        reason: String,
    },
}

/// Where the lifecycle commands read and keep things.
#[derive(clap::Args, Debug, Clone)]
pub struct LifecycleArgs {
    /// The sessions v1 file; checked before anything else
    #[arg(long)]
    pub config: std::path::PathBuf,
    /// warte's lifecycle state on this machine [default: ~/.local/state/warte/sessions]
    #[arg(long)]
    pub state: Option<std::path::PathBuf>,
    /// Claude Code's directory, for the last session id [default: ~/.claude]
    #[arg(long)]
    pub claude_dir: Option<std::path::PathBuf>,
}

#[derive(Subcommand)]
pub enum AgentsCommands {
    /// Remove one agent from this host's database (run on the ground station
    /// host; never over HTTP). Refused while it holds access policies, or
    /// data rows unless --with-data. Logged with the reason next to the
    /// database in agents-removed.jsonl.
    Rm {
        /// The agent id (the UUID `warte agents` lists)
        id: String,
        /// Why, 1 to 200 characters, kept in the log
        #[arg(long)]
        reason: String,
        /// Remove its data rows too
        #[arg(long)]
        with_data: bool,
    },
    /// Which repositories each agent's GitHub tokens reach (run on the
    /// ground station host; agents can never set this). REQ-GHTOKEN-002.
    Repos {
        #[command(subcommand)]
        action: ReposCommands,
    },
}

#[derive(Subcommand)]
pub enum ReposCommands {
    /// Every agent with repositories assigned
    List,
    /// One agent's repositories and grants
    Show {
        /// The agent name its warte token names
        name: String,
    },
    /// Replace an agent's repositories (1 to 10, owner/name)
    Set {
        name: String,
        #[arg(required = true)]
        repos: Vec<String>,
    },
    /// Remove an agent's repositories; its live tokens are revoked within a minute
    Clear { name: String },
    /// Grant a permission beyond the configured set (only actions:write)
    Grant { name: String, permission: String },
    /// Take a granted permission back
    Revoke { name: String, permission: String },
}

#[derive(Subcommand)]
pub enum AgentClientCommands {
    /// Print a GitHub token for this agent on stdout (for GH_TOKEN=$(...))
    GithubToken {
        #[command(flatten)]
        conn: AgentConn,
        /// Only these of the agent's repositories (owner/name; repeatable)
        #[arg(long = "repo")]
        repos: Vec<String>,
        /// Print the whole answer as JSON (token, expiry, repositories, permissions)
        #[arg(long)]
        json: bool,
    },
    /// git credential helper (`get` answers for https://github.com; `store`
    /// and `erase` do nothing). Configure: git config credential.https://github.com.helper '!warte agent git-credential'
    GitCredential {
        /// get, store or erase (git passes it)
        operation: String,
        #[command(flatten)]
        conn: AgentConn,
    },
}

/// How the client reaches warte and proves which agent it is.
#[derive(clap::Args)]
pub struct AgentConn {
    /// warte's base URL, e.g. https://host/warte
    #[arg(long, env = "WARTE_URL")]
    pub url: String,
    /// PEM CA certificate to trust for --url, in addition to the built-in roots
    #[arg(long, env = "WARTE_CA_CERT")]
    pub ca_cert: Option<std::path::PathBuf>,
    /// The agent's warte token
    #[arg(long, env = "WARTE_TOKEN", hide_env_values = true)]
    pub token: Option<String>,
    /// A file whose first line is the agent's warte token
    #[arg(long, env = "WARTE_TOKEN_FILE")]
    pub token_file: Option<std::path::PathBuf>,
}

#[derive(Subcommand)]
pub enum TokenCommands {
    /// Mint a bearer token for an agent; prints only the token on stdout.
    /// Reads (or creates, 0600) the signing seed the server uses.
    Mint {
        /// The agent id the token names
        agent_id: String,
        /// How long the token is valid, in hours (1..=8760)
        #[arg(long, default_value_t = 24)]
        valid_hours: i64,
        /// Let the token write as any agent, not only its own id (adds
        /// `agents:any`)
        #[arg(long)]
        any_agent: bool,
        /// Add a right beyond the default agent rights, e.g. costs:write
        /// (repeatable; an unknown right is an error)
        #[arg(long = "right", value_name = "RIGHT")]
        rights: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum ProbeCommands {
    /// Collect once and print (read-only; nothing is sent)
    Report,
    /// On this machine, no network I/O: each volume's free space and verdict,
    /// and what agents hold (scratchpads, target/, worktrees) with paths and
    /// sessions, as the probe last sized it (REQ-PRESSURE-002). Reads only;
    /// deletes nothing
    Pressure {
        /// `text` or `json`
        #[arg(long, default_value = "text")]
        format: String,
        /// Size the holders now instead of reading what the probe stored
        /// (slow when much is held)
        #[arg(long)]
        measure: bool,
        /// The probe's state directory (default: the platform's)
        #[arg(long, env = "WARTE_PROBE_STATE_DIR")]
        state_dir: Option<std::path::PathBuf>,
    },
    /// On an enrolled machine: collect once, sign with the enrolled key and
    /// send to the ground station (run it from launchd/systemd every minute)
    Send {
        /// The ground station's base URL, e.g. https://<host>/warte
        #[arg(long, env = "WARTE_URL")]
        ground: String,
        #[arg(long, default_value = "/var/lib/warte-probe")]
        state_dir: std::path::PathBuf,
        /// PEM CA to trust for --ground (a reverse proxy's local CA)
        #[arg(long, env = "WARTE_CA_CERT")]
        ca_cert: Option<std::path::PathBuf>,
        /// Also send a file another service here writes, signed, as
        /// `<name>=<path>` (e.g. `temper=/var/lib/temper/report.json`);
        /// repeatable (REQ-TEMPER-002)
        #[arg(long = "feed", value_parser = parse_feed)]
        feeds: Vec<(String, std::path::PathBuf)>,
    },
    /// As root, from a oneshot timer (hourly): read what only root can (SMART)
    /// and write it as JSON for the unprivileged probe. No network I/O
    /// (REQ-PROBE-012; see `warte docs probe-privileged`)
    Privileged {
        /// Where the probe reads it (the probe's `WARTE_PRIVILEGED_DIR`)
        #[arg(
            long,
            env = "WARTE_PRIVILEGED_DIR",
            default_value = "/var/lib/warte-privileged"
        )]
        out: std::path::PathBuf,
    },
    /// Offer the activity hook to each agent on this machine (Claude Code,
    /// Copilot CLI, Mistral Vibe): shows the exact change and writes it only
    /// after you confirm, backing each file up first (DD-PROBE-003)
    InstallHooks {
        /// Write without asking
        #[arg(long)]
        yes: bool,
    },
    /// Called by an agent's hooks (Claude Code, Copilot CLI, Mistral Vibe):
    /// reads the event on stdin and records only a verb ("editing",
    /// "waiting for you") for this agent. Never fails, prints nothing
    /// (REQ-PROBE-009)
    Hook {
        /// The event, when the hook's config names it rather than the payload
        /// (e.g. `agentStop`, `post_agent`)
        #[arg(long)]
        event: Option<String>,
    },
    /// On the ground station: refuse every certificate the host holds now,
    /// for reports and renewal; it needs a new token to enroll again
    Revoke {
        #[arg(long)]
        host: String,
    },
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

/// `<name>=<path>` for `probe send --feed`.
fn parse_feed(s: &str) -> Result<(String, std::path::PathBuf), String> {
    let (name, path) = s
        .split_once('=')
        .ok_or("expected <name>=<path>, e.g. temper=/var/lib/temper/report.json")?;
    if crate::probe::send::feed_apid(name).is_none() {
        return Err(format!("unknown feed {name:?} (known: temper)"));
    }
    if path.is_empty() {
        return Err("empty path".into());
    }
    Ok((name.to_string(), path.into()))
}

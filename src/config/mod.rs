//! Configuration for the warte control tower.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Server configuration, loaded from `warte.toml` or environment variables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Server bind address (default: 127.0.0.1:8080).
    #[serde(default = "default_bind")]
    pub bind: String,

    /// Database path (default: .warte/state.db).
    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,

    /// CA certificate path for mTLS (default: .warte/ca.pem).
    #[serde(default = "default_ca_path")]
    pub ca_cert_path: PathBuf,

    /// CA private key path for signing agent certificates (default: .warte/ca-key.pem).
    #[serde(default = "default_ca_key_path")]
    pub ca_key_path: PathBuf,

    /// Agent certificate validity in hours (default: 24).
    #[serde(default = "default_cert_validity_hours")]
    pub cert_validity_hours: u64,

    /// Access revocation propagation timeout in seconds (default: 5).
    #[serde(default = "default_revocation_timeout_secs")]
    pub revocation_timeout_secs: u64,

    /// Sync peer addresses (other warte instances).
    #[serde(default)]
    pub sync_peers: Vec<String>,

    /// Instance ID (auto-generated if not set).
    #[serde(default)]
    pub instance_id: Option<String>,

    /// Enable TLS (default: false for local dev, true for production).
    #[serde(default)]
    pub tls_enabled: bool,

    /// TLS certificate path (server cert).
    #[serde(default = "default_tls_cert_path")]
    pub tls_cert_path: PathBuf,

    /// TLS private key path (server key).
    #[serde(default = "default_tls_key_path")]
    pub tls_key_path: PathBuf,

    /// Rate limit: requests per minute per IP (default: 100).
    #[serde(default = "default_rate_limit_per_ip")]
    pub rate_limit_per_ip: u32,

    /// Rate limit: requests per minute per agent (default: 1000).
    #[serde(default = "default_rate_limit_per_agent")]
    pub rate_limit_per_agent: u32,

    /// GitHub's API as a CI source, for teams without a smithy agent (#81).
    /// Set by a `[github]` section or by `WARTE_GITHUB_ORG` + `WARTE_GITHUB_TOKEN`.
    #[serde(default)]
    pub github: Option<GithubConfig>,

    /// Ground station (absorbed smithy hub). When set, `warte serve` ingests the
    /// smithy agent's telemetry, mounts the hub API under `/ci`, and the Queue,
    /// Resources, Ask and Overview views read from it (DD-SMITHY-001).
    #[serde(default)]
    pub hub: Option<HubConfig>,
}

/// GitHub's REST API as a CI source (#81). The scan fills the same model as
/// the smithy agent's telemetry, so every view and agent API works on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubConfig {
    /// API base. `https://api.github.com`; GitHub Enterprise Cloud with data
    /// residency: `https://api.<tenant>.ghe.com`; GHES: `https://<host>/api/v3`.
    /// Set explicitly, never guessed from a web URL.
    #[serde(default = "default_github_api_url")]
    pub api_url: String,
    pub org: String,
    /// Prefer `WARTE_GITHUB_TOKEN` over writing it into the file.
    #[serde(default, skip_serializing)]
    pub token: Option<String>,
    /// Seconds between scans.
    #[serde(default = "default_github_interval_s")]
    pub interval_s: u64,
    /// Only repos pushed within this many days are scanned.
    #[serde(default = "default_github_active_days")]
    pub active_days: i64,
    /// At most this many runs have their jobs listed per scan.
    #[serde(default = "default_github_max_runs")]
    pub max_runs: usize,
    /// Extra PEM roots for the API, e.g. a TLS-intercepting proxy's CA.
    #[serde(default)]
    pub ca_bundle: Option<PathBuf>,
    /// GitHub status page API, or `off`.
    #[serde(default = "default_github_status_url")]
    pub status_url: String,
    /// Where the scanned state is kept when there is no `[hub]`.
    #[serde(default = "default_github_data_dir")]
    pub data_dir: String,
}

impl Default for GithubConfig {
    fn default() -> Self {
        GithubConfig {
            api_url: default_github_api_url(),
            org: String::new(),
            token: None,
            interval_s: default_github_interval_s(),
            active_days: default_github_active_days(),
            max_runs: default_github_max_runs(),
            ca_bundle: None,
            status_url: default_github_status_url(),
            data_dir: default_github_data_dir(),
        }
    }
}

fn default_github_api_url() -> String {
    "https://api.github.com".into()
}
fn default_github_interval_s() -> u64 {
    60
}
fn default_github_active_days() -> i64 {
    7
}
fn default_github_max_runs() -> usize {
    150
}
fn default_github_status_url() -> String {
    "https://www.githubstatus.com/api/v2".into()
}
fn default_github_data_dir() -> String {
    "/var/lib/warte/github".into()
}

/// `[hub]` in warte.toml — the same settings as `warte hub`'s flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubConfig {
    /// UDP telemetry listen address; bind the WireGuard interface address.
    pub udp: String,
    /// Source addresses allowed to send telemetry (the CI host's tunnel IP).
    pub allow: Vec<std::net::IpAddr>,
    /// Hub state directory (hub-state.json + ledger/).
    #[serde(default = "default_hub_data_dir")]
    pub data_dir: String,
    /// GitHub org the agent scans.
    #[serde(default = "default_hub_org")]
    pub org: String,
    /// The ground station probes its own machine every minute and shows it
    /// under Machines (REQ-PROBE-001).
    #[serde(default = "default_true")]
    pub probe_self: bool,
    /// A notional-cost report a workstation job copies here (REQ-COSTS-001);
    /// re-read when it changes. None: only `POST /api/costs`.
    #[serde(default)]
    pub costs_file: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            db_path: default_db_path(),
            ca_cert_path: default_ca_path(),
            ca_key_path: default_ca_key_path(),
            cert_validity_hours: default_cert_validity_hours(),
            revocation_timeout_secs: default_revocation_timeout_secs(),
            sync_peers: Vec::new(),
            instance_id: None,
            tls_enabled: false,
            tls_cert_path: default_tls_cert_path(),
            tls_key_path: default_tls_key_path(),
            rate_limit_per_ip: default_rate_limit_per_ip(),
            rate_limit_per_agent: default_rate_limit_per_agent(),
            github: None,
            hub: None,
        }
    }
}

impl Config {
    /// Load configuration from `warte.toml` in the current directory,
    /// falling back to defaults. Environment variables override file values.
    pub fn load() -> anyhow::Result<Self> {
        let mut config = Self::load_from_file().unwrap_or_default();
        config.apply_env_overrides();
        Ok(config)
    }

    fn load_from_file() -> anyhow::Result<Self> {
        let path = PathBuf::from("warte.toml");
        if !path.exists() {
            anyhow::bail!("no warte.toml found");
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(bind) = std::env::var("WARTE_BIND") {
            self.bind = bind;
        }
        if let Ok(db) = std::env::var("WARTE_DB_PATH") {
            self.db_path = PathBuf::from(db);
        }
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        if let (Some(org), Some(token)) = (env("WARTE_GITHUB_ORG"), env("WARTE_GITHUB_TOKEN")) {
            let g = self.github.get_or_insert_with(|| GithubConfig {
                org: org.clone(),
                ..Default::default()
            });
            g.org = org;
            g.token = Some(token);
        }
        if let Some(g) = self.github.as_mut() {
            if let Some(v) = env("WARTE_GITHUB_TOKEN") {
                g.token = Some(v);
            }
            if let Some(v) = env("WARTE_GITHUB_API_URL") {
                g.api_url = v;
            }
            if let Some(v) = env("WARTE_CA_BUNDLE") {
                g.ca_bundle = Some(v.into());
            }
            if let Some(v) = env("WARTE_GITHUB_STATUS_URL") {
                g.status_url = v;
            }
        }
        if let Ok(tls) = std::env::var("WARTE_TLS_ENABLED") {
            self.tls_enabled = tls == "true" || tls == "1";
        }
    }

    /// Get or generate the instance ID.
    pub fn instance_id(&self) -> String {
        self.instance_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string())
    }
}

fn default_hub_data_dir() -> String {
    "/var/lib/smithy-hub".to_string()
}
fn default_hub_org() -> String {
    "pulseengine".to_string()
}
fn default_bind() -> String {
    "127.0.0.1:8080".to_string()
}
fn default_db_path() -> PathBuf {
    PathBuf::from(".warte/state.db")
}
fn default_ca_path() -> PathBuf {
    PathBuf::from(".warte/ca.pem")
}
fn default_ca_key_path() -> PathBuf {
    PathBuf::from(".warte/ca-key.pem")
}
fn default_cert_validity_hours() -> u64 {
    24
}
fn default_revocation_timeout_secs() -> u64 {
    5
}
fn default_tls_cert_path() -> PathBuf {
    PathBuf::from(".warte/server-cert.pem")
}
fn default_tls_key_path() -> PathBuf {
    PathBuf::from(".warte/server-key.pem")
}
fn default_rate_limit_per_ip() -> u32 {
    100
}
fn default_rate_limit_per_agent() -> u32 {
    1000
}

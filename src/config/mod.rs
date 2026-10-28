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

    /// GitHub organization (for CI status, ETA estimation).
    #[serde(default)]
    pub github_org: Option<String>,

    /// GitHub token (for API access).
    #[serde(default)]
    pub github_token: Option<String>,

    /// Ground station (absorbed smithy hub). When set, `warte serve` ingests the
    /// smithy agent's telemetry, mounts the hub API under `/ci`, and the Queue,
    /// Resources, Ask and Overview views read from it (DD-SMITHY-001).
    #[serde(default)]
    pub hub: Option<HubConfig>,
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
            github_org: None,
            github_token: None,
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
        if let Ok(org) = std::env::var("WARTE_GITHUB_ORG") {
            self.github_org = Some(org);
        }
        if let Ok(token) = std::env::var("WARTE_GITHUB_TOKEN") {
            self.github_token = Some(token);
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

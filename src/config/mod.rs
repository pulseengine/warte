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

    /// File holding the bearer-token signing seed (64 hex digits, created with
    /// mode 0600 on first start; never commit it). Default: `token-seed` next
    /// to the CA key. Env: `WARTE_TOKEN_SEED_PATH`. Tokens survive restarts
    /// only because this file does.
    #[serde(default)]
    pub token_seed_path: Option<PathBuf>,

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

    /// The fleet model: a `spar instance --format json` of the Warte_Fleet
    /// types (REQ-FLEET-001). Env: `WARTE_FLEET_MODEL`.
    #[serde(default)]
    pub fleet_model: Option<PathBuf>,

    /// Scheduled jobs pushed by the hosts (REQ-JOBS-001). Env:
    /// `WARTE_JOBS_DIR`, `WARTE_JOBS_EXPECTED`.
    #[serde(default)]
    pub jobs: Option<JobsConfig>,

    /// temper's compliance report, forwarded by one machine's probe
    /// (REQ-TEMPER-002).
    #[serde(default)]
    pub temper: Option<TemperFeedConfig>,

    /// Which repo depends on which, for the Critical Path view
    /// (REQ-CRIT-001): `repo_dependencies = [{ repo = "jess", depends_on =
    /// ["relay", "gale"] }]`, at the top of the file, before any section.
    #[serde(default)]
    pub repo_dependencies: Vec<crate::critpath::RepoDependency>,

    /// GitHub's API as a CI source, for teams without a smithy agent (#81).
    /// Set by a `[github]` section or by `WARTE_GITHUB_ORG` + `WARTE_GITHUB_TOKEN`.
    #[serde(default)]
    pub github: Option<GithubConfig>,

    /// Ground station (absorbed smithy hub). When set, `warte serve` ingests the
    /// smithy agent's telemetry and the Queue,
    /// Resources, Ask and Overview views read from it (DD-SMITHY-001).
    #[serde(default)]
    pub hub: Option<HubConfig>,

    /// Machine history (REQ-MHIST-001). On whenever machines are shown; this
    /// section only tunes retention.
    #[serde(default)]
    pub history: Option<HistoryConfig>,

    /// When a volume counts as low or critical for `/api/pressure`
    /// (REQ-PRESSURE-001): `[pressure] low_gb = 20, critical_gb = 10`.
    #[serde(default)]
    pub pressure: crate::probe::pressure::Thresholds,

    /// Runner pools: names, kinds, OS, the inventory where GitHub will not
    /// list runners, and per-minute rates (REQ-RUNNERS-001..004):
    /// `[[runners.pool]] labels = ["linux-16core"], name = "...", count = 4`.
    #[serde(default)]
    pub runners: crate::hub::pools::RunnersConfig,
}

/// Machine history retention (REQ-MHIST-001).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HistoryConfig {
    /// Keep hourly min/avg/max this many days (default 90, at least 8).
    /// Per-minute points are always kept 7 days.
    #[serde(default)]
    pub keep_days: Option<u32>,
}

/// Where the hosts' job states land, and which must exist (REQ-JOBS-001).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JobsConfig {
    /// `<dir>/<host>/<job>.json`, as `pe-job` writes them.
    pub dir: PathBuf,
    /// The manifest: `<path relative to dir> <window_s>` per line.
    #[serde(default)]
    pub expected: Option<PathBuf>,
    /// Job host directory name -> machine name, e.g. `mini = "Mac.fritz.box"`.
    #[serde(default)]
    pub hosts: std::collections::BTreeMap<String, String>,
}

/// Where temper's report comes from (REQ-TEMPER-002).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemperFeedConfig {
    /// The enrolled probe host whose `temper` feed is accepted; any other
    /// probe's is refused.
    pub host: String,
    /// The report counts as stale when its last full sweep is older than
    /// this (default 26 h: a daily sweep plus slack).
    #[serde(default = "default_temper_stale_after_s")]
    pub stale_after_s: i64,
}

fn default_temper_stale_after_s() -> i64 {
    26 * 3600
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
    /// Stop the scan while GitHub reports this many requests or fewer left,
    /// so other users of the token keep headroom (REQ-GHSRC-003).
    #[serde(default = "default_github_rate_reserve")]
    pub rate_reserve: i64,
    /// Runner labels that mark a job self-hosted, for runner groups whose
    /// jobs carry no `self-hosted` label. `*` matches any run of characters;
    /// case-insensitive. Empty: only `self-hosted` does (REQ-GHSRC-005).
    #[serde(default)]
    pub self_hosted_labels: Vec<String>,
    /// Job ledger (REQ-RUNNERS-005): days back from today whose runs are
    /// read into the ledger for the monthly report. 0 turns it off.
    #[serde(default = "default_github_ledger_days")]
    pub ledger_days: u32,
    /// Seconds between ledger passes.
    #[serde(default = "default_github_ledger_interval_s")]
    pub ledger_interval_s: u64,
}

fn default_github_ledger_days() -> u32 {
    2
}

fn default_github_ledger_interval_s() -> u64 {
    600
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
            rate_reserve: default_github_rate_reserve(),
            self_hosted_labels: vec![],
            ledger_days: default_github_ledger_days(),
            ledger_interval_s: default_github_ledger_interval_s(),
        }
    }
}

fn default_github_api_url() -> String {
    "https://api.github.com".into()
}
fn default_github_interval_s() -> u64 {
    120
}
fn default_github_rate_reserve() -> i64 {
    150
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
    /// A `skill-usage.v1` report a workstation job copies here
    /// (REQ-SKILLS-001); re-read when it changes. None: only
    /// `POST /api/skill-usage`.
    #[serde(default)]
    pub skill_usage_file: Option<String>,
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
            token_seed_path: None,
            cert_validity_hours: default_cert_validity_hours(),
            revocation_timeout_secs: default_revocation_timeout_secs(),
            sync_peers: Vec::new(),
            instance_id: None,
            tls_enabled: false,
            tls_cert_path: default_tls_cert_path(),
            tls_key_path: default_tls_key_path(),
            rate_limit_per_ip: default_rate_limit_per_ip(),
            rate_limit_per_agent: default_rate_limit_per_agent(),
            fleet_model: None,
            jobs: None,
            temper: None,
            repo_dependencies: vec![],
            github: None,
            hub: None,
            history: None,
            pressure: Default::default(),
            runners: Default::default(),
        }
    }
}

impl Config {
    /// Load configuration from `warte.toml` in the current directory,
    /// falling back to defaults. Environment variables override file values.
    /// `warte.toml` in the working directory, then the environment. No file
    /// means defaults; a file that cannot be read or parsed is an error, never
    /// silently the defaults: that once ran the ground station without its
    /// hub, fleet model and jobs (2026-11-03).
    pub fn load() -> anyhow::Result<Self> {
        let mut config = Self::from_path(std::path::Path::new("warte.toml"))?.unwrap_or_default();
        config.apply_env_overrides();
        Ok(config)
    }

    /// The config in `path`: None if there is no such file.
    pub fn from_path(path: &std::path::Path) -> anyhow::Result<Option<Self>> {
        use anyhow::Context;
        if !path.exists() {
            return Ok(None);
        }
        let content =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("{} is not a valid warte config", path.display()))?;
        Ok(Some(config))
    }

    /// Where the token-signing seed lives.
    pub fn token_seed_file(&self) -> PathBuf {
        self.token_seed_path
            .clone()
            .unwrap_or_else(|| self.ca_key_path.with_file_name("token-seed"))
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(bind) = std::env::var("WARTE_BIND") {
            self.bind = bind;
        }
        if let Ok(db) = std::env::var("WARTE_DB_PATH") {
            self.db_path = PathBuf::from(db);
        }
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        if let Some(v) = env("WARTE_TOKEN_SEED_PATH") {
            self.token_seed_path = Some(v.into());
        }
        if let Some(v) = env("WARTE_FLEET_MODEL") {
            self.fleet_model = Some(v.into());
        }
        if let Some(v) = env("WARTE_JOBS_DIR") {
            self.jobs.get_or_insert_with(Default::default).dir = v.into();
        }
        if let Some(v) = env("WARTE_JOBS_EXPECTED") {
            if let Some(j) = self.jobs.as_mut() {
                j.expected = Some(v.into());
            }
        }
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
            if let Some(v) = env("WARTE_GITHUB_RATE_RESERVE").and_then(|v| v.parse().ok()) {
                g.rate_reserve = v;
            }
            if let Some(v) = env("WARTE_SELF_HOSTED_LABELS") {
                g.self_hosted_labels = v
                    .split(',')
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
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

#[cfg(test)]
mod load_tests {
    use super::*;

    // rivet: verifies REQ-DEPLOY-002
    #[test]
    fn a_config_that_does_not_parse_is_an_error_not_the_defaults() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("warte.toml");
        assert!(
            Config::from_path(&p).unwrap().is_none(),
            "no file: defaults"
        );
        // the 2026-11-03 case: a JSON object where TOML needs an inline table
        std::fs::write(
            &p,
            "bind = \"127.0.0.1:1\"\n[jobs]\ndir = \"j\"\nhosts = {\"mini\": \"Mac\"}\n",
        )
        .unwrap();
        let e = Config::from_path(&p).unwrap_err();
        assert!(
            format!("{e:#}").contains("not a valid warte config"),
            "{e:#}"
        );
        std::fs::write(
            &p,
            "bind = \"127.0.0.1:1\"\n[jobs]\ndir = \"j\"\nhosts = { \"mini\" = \"Mac\" }\n",
        )
        .unwrap();
        let c = Config::from_path(&p).unwrap().unwrap();
        assert_eq!(c.bind, "127.0.0.1:1");
        assert_eq!(c.jobs.unwrap().hosts["mini"], "Mac");
    }
}

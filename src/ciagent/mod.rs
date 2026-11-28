//! `warte ci-agent`: the fleet agent on a self-hosted GitHub Actions host.
//!
//!   warte ci-agent snapshot            collect once on the CI host and print it
//!   warte ci-agent agent --hub ADDR [--hub ADDR ...]   collect every minute and push to every hub
//!   warte ci-agent arch ...            print architecture.v1 from a spar instance and an inventory
//!
//! It collects runner and host state, keeps a ledger of finished jobs, and
//! sends both as CCSDS telemetry (`relay-ccsds`) to the ground station
//! (`warte hub`, see `warte docs ground-station`). It is read-only: it
//! observes the fleet and the existing automation and changes nothing.
//! Imported from the owners' private fleet tool; behaviour unchanged except
//! where `warte docs ci-agent` says so. Host paths come from `[ci_agent]`.
pub mod agent;
pub mod arch;
pub mod collect;
pub mod eta;
pub mod github;
pub mod incidents;
pub mod ledger;
pub mod model;
pub mod parse;
pub mod report;
pub mod telemetry;

use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

/// `[ci_agent]` in warte.toml: where the CI host keeps its runners and its
/// automation. Every key has a default; see `warte docs ci-agent`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct CiAgentConfig {
    /// Directory holding one directory per runner.
    pub runners_dir: String,
    /// A runner's user and directory name; `{id}` is its number.
    pub runner_name: String,
    /// A runner's systemd unit; `{runner}` is its name.
    pub runner_unit: String,
    /// Mount points `df` reports on.
    pub df_mounts: Vec<String>,
    /// Command printing a GitHub token, when `--token-cmd` is not given.
    pub token_cmd: String,
    /// systemd timers whose state is reported.
    pub timers: Vec<String>,
    /// Log file of the downtime dispatcher (last 8 lines are reported).
    pub dispatcher_log: String,
    /// systemd unit of the runner watchdog (its journal is read).
    pub watchdog_unit: String,
    /// systemd unit of the Bazel cache sweep (its journal is read).
    pub bazel_sweep_unit: String,
    /// Log file of the trace rollout (last 6 lines are reported).
    pub trace_rollout_log: String,
    /// `arch`: the machines whose agent sessions are read from the inventory.
    pub arch_hosts: Vec<String>,
    /// `arch`: the model path written to `source.model`.
    pub arch_model: String,
}

impl Default for CiAgentConfig {
    fn default() -> Self {
        CiAgentConfig {
            runners_dir: "/var/lib/runners".into(),
            runner_name: "runner{id}".into(),
            runner_unit: "github-runner@{runner}.service".into(),
            df_mounts: vec!["/".into(), "/var/lib/runners".into()],
            token_cmd: "/usr/local/sbin/ci-gh-token".into(),
            timers: vec![
                "ci-downtime.timer".into(),
                "ci-runner-watchdog.timer".into(),
                "ci-bazel-sweep.timer".into(),
                "ci-trace-rollout.timer".into(),
            ],
            dispatcher_log: "/var/log/ci-downtime.log".into(),
            watchdog_unit: "ci-runner-watchdog.service".into(),
            bazel_sweep_unit: "ci-bazel-sweep.service".into(),
            trace_rollout_log: "/var/log/ci-trace-rollout.log".into(),
            arch_hosts: vec![],
            arch_model: "architecture/security/security.aadl".into(),
        }
    }
}

impl CiAgentConfig {
    /// The patterns must hold their placeholder exactly once.
    pub fn check(&self) -> Result<(), String> {
        if self.runner_name.matches("{id}").count() != 1 {
            return Err(format!(
                "[ci_agent] runner_name {:?} must contain {{id}} once",
                self.runner_name
            ));
        }
        if self.runner_unit.matches("{runner}").count() != 1 {
            return Err(format!(
                "[ci_agent] runner_unit {:?} must contain {{runner}} once",
                self.runner_unit
            ));
        }
        Ok(())
    }

    /// Runner `id`'s user and directory name, e.g. `runner12`.
    pub fn runner(&self, id: u32) -> String {
        self.runner_name.replace("{id}", &id.to_string())
    }

    /// Runner `id`'s directory, e.g. `/var/lib/runners/runner12`.
    pub fn runner_dir(&self, id: u32) -> String {
        format!(
            "{}/{}",
            self.runners_dir.trim_end_matches('/'),
            self.runner(id)
        )
    }

    /// The `systemctl list-units` pattern for every runner unit, e.g. `github-runner@*`.
    pub fn unit_glob(&self) -> String {
        let (pre, _) = self
            .runner_unit
            .split_once("{runner}")
            .unwrap_or((&self.runner_unit, ""));
        format!("{pre}*")
    }

    /// The runner id of a unit name: `github-runner@runner12.service` -> 12.
    pub fn unit_id(&self, unit: &str) -> Option<u32> {
        let (upre, usuf) = self.runner_unit.split_once("{runner}")?;
        let name = unit.strip_prefix(upre)?.strip_suffix(usuf)?;
        let (npre, nsuf) = self.runner_name.split_once("{id}")?;
        name.strip_prefix(npre)?.strip_suffix(nsuf)?.parse().ok()
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Json,
    Text,
    Markdown,
}

#[derive(Args)]
pub struct Collect {
    /// GitHub organisation.
    #[arg(long, default_value = "pulseengine")]
    org: String,
    /// Command printing a GitHub token [default: [ci_agent] token_cmd].
    #[arg(long)]
    token_cmd: Option<String>,
    /// Arguments for --token-cmd. The default asks the helper for a token scoped
    /// down to Actions read + runners read: this tool only ever reads.
    #[arg(long, default_value = "--read-only", allow_hyphen_values = true)]
    token_args: String,
    /// GitHub runner names are "<prefix>-<id>"; defaults to the hostname.
    #[arg(long)]
    runner_prefix: Option<String>,
    /// Skip all GitHub API calls (local view only).
    #[arg(long)]
    no_github: bool,
    /// Seconds between queue scans (the queue scan is the expensive part).
    #[arg(long, default_value_t = 300)]
    queue_interval: i64,
    /// Only scan repos pushed within this many days.
    #[arg(long, default_value_t = 30)]
    active_days: i64,
    /// Cap on workflow runs whose jobs are listed per scan.
    #[arg(long, default_value_t = 120)]
    max_runs: usize,
}

/// The conveyor-belt ledger: finished jobs read back from GitHub per UTC day.
#[derive(Args)]
pub struct LedgerArgs {
    /// Keep the ledger here (systemd StateDirectory). Without it, no ledger runs.
    #[arg(long)]
    state_dir: Option<String>,
    /// Seconds between ledger passes.
    #[arg(long, default_value_t = 3600)]
    ledger_interval: u64,
    /// GET calls per ledger pass (the app token is shared with the dispatcher).
    #[arg(long, default_value_t = 600)]
    ledger_budget: u32,
    /// How many days back to backfill.
    #[arg(long, default_value_t = 30)]
    backfill_days: i64,
    /// Ledger records sent per 60 s frame (paces the backfill over the tunnel).
    #[arg(long, default_value_t = 150)]
    ledger_per_frame: usize,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Collect once and print.
    Snapshot {
        #[command(flatten)]
        c: Collect,
        #[arg(long, value_enum, default_value = "json")]
        format: Format,
    },
    /// Collect periodically and send CCSDS telemetry to the hub over UDP.
    Agent {
        #[command(flatten)]
        c: Collect,
        /// Hub telemetry address, e.g. 192.0.2.10:8092. Repeatable: every
        /// hub receives the same packets (parallel run of two ground stations).
        #[arg(long, required = true)]
        hub: Vec<String>,
        #[arg(long, default_value_t = 60)]
        interval: u64,
        /// Send one snapshot and exit.
        #[arg(long)]
        once: bool,
        #[command(flatten)]
        l: LedgerArgs,
    },
    /// Print warte's architecture.v1 from a spar instance and the inventory.
    Arch {
        /// spar instance JSON (`spar instance --format json`).
        #[arg(long)]
        instance: std::path::PathBuf,
        /// Inventory checkout (reads ansible/group_vars, host_vars and the Zellij layouts).
        #[arg(long)]
        repo: std::path::PathBuf,
        #[arg(long, default_value = "")]
        commit: String,
        #[arg(long, default_value = "")]
        spar_version: String,
        #[arg(long, default_value = "")]
        generated_at: String,
    },
}

fn opts(c: Collect, cfg: &CiAgentConfig) -> agent::Opts {
    agent::Opts {
        runner_prefix: c.runner_prefix.unwrap_or_else(collect::hostname),
        org: c.org,
        token_cmd: c.token_cmd.unwrap_or_else(|| cfg.token_cmd.clone()),
        token_args: c.token_args.split_whitespace().map(String::from).collect(),
        no_github: c.no_github,
        queue_interval: c.queue_interval,
        active_days: c.active_days,
        max_runs: c.max_runs,
        cfg: cfg.clone(),
    }
}

/// Run one `warte ci-agent` command. An error is printed by the caller,
/// which exits 1.
pub fn run(cmd: Cmd, cfg: &CiAgentConfig) -> Result<(), String> {
    cfg.check()?;
    match cmd {
        Cmd::Snapshot { c, format } => {
            let s = agent::snapshot(&opts(c, cfg), &mut None);
            match format {
                Format::Markdown => print!("{}", report::markdown(&s, collect::now(), None)),
                Format::Json => {
                    println!("{}", serde_json::to_string_pretty(&s).unwrap_or_default())
                }
                Format::Text => {
                    let busy = s.runners.iter().filter(|r| r.local_busy).count();
                    println!(
                        "{} runners, {} busy (local), load {:.1}/{} cpu, cpu psi {:.1}, mem psi {:.1}",
                        s.runners.len(),
                        busy,
                        s.machine.load[0],
                        s.machine.ncpu,
                        s.machine.psi.cpu.avg60,
                        s.machine.psi.memory.avg60
                    );
                    for r in &s.runners {
                        println!(
                            "  runner{:<3} {:<8} gh={:<8} {}",
                            r.id,
                            if r.local_busy { "BUSY" } else { "idle" },
                            r.gh_status.as_deref().unwrap_or("-"),
                            r.job.as_deref().unwrap_or("")
                        );
                    }
                    if let Some(q) = &s.queue {
                        println!(
                            "queue: {} jobs in {} runs{}",
                            q.jobs_queued,
                            q.runs_queued,
                            if q.truncated { " (lower bound)" } else { "" }
                        );
                    }
                    for e in &s.errors {
                        println!("  error: {e}");
                    }
                }
            }
            Ok(())
        }
        Cmd::Agent {
            c,
            hub,
            interval,
            once,
            l,
        } => agent::run(
            &opts(c, cfg),
            &hub,
            interval,
            once,
            l.state_dir.map(|dir| agent::LedgerOpts {
                dir,
                interval: l.ledger_interval,
                budget: l.ledger_budget,
                backfill_days: l.backfill_days,
                per_frame: l.ledger_per_frame,
            }),
        ),
        Cmd::Arch {
            instance,
            repo,
            commit,
            spar_version,
            generated_at,
        } => arch::build(&arch::Opts {
            instance: &instance,
            repo: &repo,
            commit: &commit,
            spar_version: &spar_version,
            generated_at: &generated_at,
            hosts: &cfg.arch_hosts,
            model: &cfg.arch_model,
        })
        .map(|doc| println!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_patterns_match_the_runner_units() {
        let c = CiAgentConfig::default();
        assert!(c.check().is_ok());
        assert_eq!(c.unit_glob(), "github-runner@*");
        assert_eq!(c.unit_id("github-runner@runner12.service"), Some(12));
        assert_eq!(c.unit_id("github-runner@runnerx.service"), None);
        assert_eq!(c.unit_id("other@runner12.service"), None);
        assert_eq!(c.runner(3), "runner3");
        assert_eq!(c.runner_dir(3), "/var/lib/runners/runner3");
    }

    #[test]
    fn custom_patterns_and_a_bad_one() {
        let c = CiAgentConfig {
            runners_dir: "/srv/ci/".into(),
            runner_name: "gh-{id}-x".into(),
            runner_unit: "actions@{runner}.service".into(),
            ..Default::default()
        };
        assert_eq!(c.unit_glob(), "actions@*");
        assert_eq!(c.unit_id("actions@gh-7-x.service"), Some(7));
        assert_eq!(c.runner_dir(7), "/srv/ci/gh-7-x");
        let bad = CiAgentConfig {
            runner_name: "runner".into(),
            ..Default::default()
        };
        assert!(bad.check().unwrap_err().contains("{id}"));
    }

    #[test]
    fn the_section_parses_with_defaults_for_missing_keys() {
        let c: CiAgentConfig = toml::from_str("runners_dir = \"/srv/ci\"").unwrap();
        assert_eq!(c.runners_dir, "/srv/ci");
        assert_eq!(c.runner_name, "runner{id}");
        assert!(toml::from_str::<CiAgentConfig>("nope = 1").is_err());
    }
}

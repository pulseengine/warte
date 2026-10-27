//! Data model shared by agent and hub. Everything here is an observation;
//! phase 1 is read-only and never acts on the fleet.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Snapshot {
    pub agent_version: String,
    pub host: String,
    pub taken_at: i64,
    pub machine: Machine,
    pub runners: Vec<Runner>,
    /// Latest queue scan (refreshed less often than the rest; see `scanned_at`).
    pub queue: Option<Queue>,
    pub automation: Automation,
    /// Collection problems, reported instead of silently producing a thinner snapshot.
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Machine {
    pub ncpu: u32,
    pub load: [f64; 3],
    pub mem_total_mb: u64,
    pub mem_avail_mb: u64,
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
    pub psi: Psi,
    pub disks: Vec<Disk>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
pub struct PsiLine {
    pub avg10: f64,
    pub avg60: f64,
    pub avg300: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Psi {
    pub cpu: PsiLine,
    pub memory: PsiLine,
    pub io: PsiLine,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Disk {
    pub mount: String,
    pub size_gb: f64,
    pub used_gb: f64,
    pub used_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Runner {
    pub id: u32,
    pub unit: String,
    pub active: bool,
    pub pid: u32,
    /// Ground truth from the host: a `Runner.Worker` process runs as this runner's user.
    /// GitHub's `busy` flag was measured lagging in BOTH directions, so this is the
    /// value to trust; `gh_busy` is shown alongside it, and disagreement is surfaced.
    pub local_busy: bool,
    pub job: Option<String>,
    pub job_started: Option<i64>,
    pub last_result: Option<String>,
    /// `relatime` / `noatime` of ~/.rustup inside the runner's own mount namespace.
    pub trace: Option<String>,
    pub gh_status: Option<String>,
    pub gh_busy: Option<bool>,
    /// Labels as GitHub reports them. Pools come from LABELS, not the runner class:
    /// runners 5 and 10 are `rust-cpu` *and* `light`.
    pub labels: Vec<String>,
    /// What the busy runner is working on, read from the job's own
    /// `_diag/Worker_*.log` on the host: no API calls, always current.
    pub ctx: Option<JobContext>,
    /// Resource use of this runner's own cgroup (the service and every job it runs).
    #[serde(default)]
    pub stats: Option<CgStats>,
}

/// One runner's cgroup counters. CPU is a RATE, so it needs two samples: it is
/// `None` on the first snapshot after the agent starts.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CgStats {
    /// Cores in use, averaged since the previous sample.
    pub cpu_cores: Option<f64>,
    pub mem_mb: u32,
    /// High-water mark since the service (re)started.
    pub mem_peak_mb: u32,
    /// The cgroup's `memory.max`; `None` means unlimited.
    pub mem_max_mb: Option<u32>,
    pub pids: u32,
    /// This cgroup's own pressure (avg60, %): time its tasks stalled.
    pub psi_cpu: f64,
    pub psi_mem: f64,
    /// `oom_kill` from memory.events: jobs killed for hitting the cap.
    pub oom_kills: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct JobContext {
    pub repo: String,
    pub workflow: String,
    pub run_id: u64,
    pub run_number: u32,
    pub run_attempt: u32,
    pub event: String,
    pub branch: String,
    pub base: String,
    pub sha: String,
    pub actor: String,
    pub pr: Option<u32>,
    pub pr_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Queue {
    pub scanned_at: i64,
    pub repos_scanned: u32,
    /// Workflow RUNS with something queued. A run fans out into many jobs —
    /// runs and jobs are different units and are never summed together.
    pub runs_queued: u32,
    /// Individual queued JOBS (what actually waits for a runner).
    pub jobs_queued: u32,
    /// Queued jobs per pool. A pool is the set of non-generic labels a job needs.
    pub pools: BTreeMap<String, PoolQueue>,
    pub top_runs: Vec<QueuedRun>,
    /// True if the scan hit its per-scan cap and the counts are a lower bound.
    pub truncated: bool,
    /// Jobs running on / waiting for GitHub-HOSTED runners (no `self-hosted`
    /// label), so the dashboard shows the whole picture, not just our box.
    #[serde(default)]
    pub hosted: Vec<HostedJob>,
    /// Hosted jobs seen before the list was capped for the wire.
    #[serde(default)]
    pub hosted_total: u32,
    /// Totals over ALL hosted jobs (the per-job list is capped).
    #[serde(default)]
    pub hosted_running: u32,
    #[serde(default)]
    pub hosted_queued: u32,
    /// ALL hosted jobs (not just the capped list) summed per repo + runs-on label.
    #[serde(default)]
    pub hosted_groups: Vec<HostedGroup>,
    /// Every queued job grouped per (run, pool): what `/api/eta` searches.
    #[serde(default)]
    pub queued: Vec<QueuedEntry>,
    /// True when EVERY queued run of every active repo had its jobs listed.
    /// Only then may "not in the list" be read as "not queued".
    #[serde(default)]
    pub queued_complete: bool,
    /// Realized waits (started_at - created_at) of recently started jobs, per pool.
    #[serde(default)]
    pub waits: BTreeMap<String, WaitStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct QueuedEntry {
    /// Repo name without the org.
    pub repo: String,
    pub run_id: u64,
    pub workflow: String,
    pub event: String,
    pub branch: String,
    pub pr: Option<u32>,
    pub pool: String,
    pub jobs: u32,
    /// created_at of the oldest queued job in this entry (unix seconds).
    pub since: i64,
}

/// Distribution of how long jobs in a pool ACTUALLY waited before starting.
/// Measured, not modelled: GitHub was measured NOT starting jobs in creation
/// order (20/55 inversions on 2026-10-22), so queue position predicts little.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct WaitStats {
    pub n: u32,
    pub p50: i64,
    pub p90: i64,
    pub max: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HostedGroup {
    pub repo: String,
    pub labels: String,
    pub running: u32,
    pub queued: u32,
    /// Oldest start/creation time in the group (unix seconds).
    pub since: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HostedJob {
    pub repo: String,
    pub workflow: String,
    pub job: String,
    /// `queued` or `in_progress`.
    pub status: String,
    /// GitHub's runner name once assigned (e.g. "GitHub Actions 12").
    pub runner: String,
    /// The runs-on labels, e.g. "ubuntu-latest".
    pub labels: String,
    /// started_at when running, created_at when queued (unix seconds).
    pub since: i64,
    pub run_id: u64,
    pub event: String,
    pub branch: String,
    pub pr: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct PoolQueue {
    pub jobs_queued: u32,
    pub oldest_s: i64,
    pub median_s: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueuedRun {
    pub repo: String,
    pub workflow: String,
    pub event: String,
    pub jobs_queued: u32,
    pub oldest_s: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Automation {
    pub dispatcher: Vec<String>,
    pub watchdog: Vec<String>,
    pub bazel_sweep: Vec<String>,
    pub trace_rollout: Vec<String>,
    pub timers: Vec<TimerState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TimerState {
    pub unit: String,
    pub active: bool,
    pub next: Option<String>,
}

/// One compact point per ingest, kept by the hub for trend lines.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Point {
    pub t: i64,
    pub runners: u32,
    pub online: u32,
    pub local_busy: u32,
    pub gh_busy: u32,
    pub jobs_queued: Option<u32>,
    pub load1: f64,
    pub psi_cpu: f64,
    pub psi_mem: f64,
    pub disk_runners_pct: Option<f64>,
    /// Per-runner cores and memory (MB), for the runner detail sparklines.
    #[serde(default)]
    pub runner_cpu: BTreeMap<u32, f64>,
    #[serde(default)]
    pub runner_mem: BTreeMap<u32, u32>,
    /// Queued jobs on OUR runners (all pools except hosted) and on GitHub-hosted.
    #[serde(default)]
    pub sh_queued: Option<u32>,
    #[serde(default)]
    pub hosted_queued: Option<u32>,
    #[serde(default)]
    pub hosted_running: Option<u32>,
    /// Longest current wait among self-hosted queued jobs (seconds).
    #[serde(default)]
    pub sh_oldest_s: Option<i64>,
    /// Measured p50 wait per pool (seconds) at this point in time.
    #[serde(default)]
    pub wait_p50: BTreeMap<String, i64>,
    /// Distinct PRs with at least one queued job.
    #[serde(default)]
    pub prs_waiting: Option<u32>,
}

impl Point {
    pub fn from_snapshot(s: &Snapshot) -> Point {
        Point {
            t: s.taken_at,
            runners: s.runners.len() as u32,
            online: s
                .runners
                .iter()
                .filter(|r| r.gh_status.as_deref() == Some("online"))
                .count() as u32,
            local_busy: s.runners.iter().filter(|r| r.local_busy).count() as u32,
            gh_busy: s.runners.iter().filter(|r| r.gh_busy == Some(true)).count() as u32,
            jobs_queued: s.queue.as_ref().map(|q| q.jobs_queued),
            load1: s.machine.load[0],
            psi_cpu: s.machine.psi.cpu.avg60,
            psi_mem: s.machine.psi.memory.avg60,
            disk_runners_pct: s
                .machine
                .disks
                .iter()
                .find(|d| d.mount == "/var/lib/runners")
                .map(|d| d.used_pct),
            runner_cpu: s
                .runners
                .iter()
                .filter_map(|r| {
                    Some((r.id, (r.stats.as_ref()?.cpu_cores? * 100.0).round() / 100.0))
                })
                .collect(),
            runner_mem: s
                .runners
                .iter()
                .filter_map(|r| Some((r.id, r.stats.as_ref()?.mem_mb)))
                .collect(),
            sh_queued: s.queue.as_ref().map(|q| {
                q.pools
                    .iter()
                    .filter(|(k, _)| k.as_str() != "hosted")
                    .map(|(_, p)| p.jobs_queued)
                    .sum()
            }),
            hosted_queued: s.queue.as_ref().map(|q| q.hosted_queued),
            hosted_running: s.queue.as_ref().map(|q| q.hosted_running),
            sh_oldest_s: s.queue.as_ref().map(|q| {
                q.pools
                    .iter()
                    .filter(|(k, _)| k.as_str() != "hosted")
                    .map(|(_, p)| p.oldest_s)
                    .max()
                    .unwrap_or(0)
            }),
            wait_p50: s
                .queue
                .as_ref()
                .map(|q| q.waits.iter().map(|(k, w)| (k.clone(), w.p50)).collect())
                .unwrap_or_default(),
            prs_waiting: s.queue.as_ref().filter(|q| q.queued_complete).map(|q| {
                let mut prs: Vec<(&str, u32)> = q
                    .queued
                    .iter()
                    .filter_map(|e| e.pr.map(|n| (e.repo.as_str(), n)))
                    .collect();
                prs.sort_unstable();
                prs.dedup();
                prs.len() as u32
            }),
        }
    }
}

/// One finished job, as GitHub recorded it: the conveyor belt's ledger line.
/// work = e - s, wait = s - c. `created_at` of a job behind `needs:` is set when
/// its parents finish (measured 2026-10-22), so `s - c` is waiting for a RUNNER.
/// Jobs that never ran (skipped, cancelled before start) have no runner and no `s`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct JobRecord {
    /// Ledger day: the UTC day the RUN was created (what GitHub's `created=` filter
    /// selects), so agent manifest and hub file agree even for runs crossing midnight.
    pub d: String,
    /// GitHub job id (unique per attempt).
    pub id: u64,
    pub run: u64,
    pub att: u32,
    /// Repo name without the org.
    pub repo: String,
    pub wf: String,
    pub job: String,
    pub pool: String,
    /// runner_name; empty when the job never got a runner.
    pub runner: String,
    pub ev: String,
    /// created_at / started_at / completed_at, unix seconds.
    pub c: i64,
    pub s: Option<i64>,
    pub e: Option<i64>,
    /// conclusion: success, failure, cancelled, skipped, ...
    pub r: String,
}

/// What the agent holds for one UTC day: lets the hub tell a complete day from
/// one with lost records, without trusting the transport.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DayManifest {
    pub day: String,
    pub n: u32,
    /// XOR of all job ids of the day.
    pub x: u64,
    /// The agent will not look at this day again (old enough, fully listed).
    pub closed: bool,
}

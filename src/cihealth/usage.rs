//! Runner usage for one repository over a window of ledger days (REQ-CI-009,
//! DD-CI-005): what the jig shadow pilot compares before and after.
//!
//! From the finished-job ledger only (no GitHub requests), for the jobs of
//! one repository whose run was created on a day from `from` to `until`:
//!
//! - runner-minutes per pool: the sum of `completed - started` over every
//!   attempt that got a runner (a re-run uses runner time too);
//! - runs by event, and minutes per run;
//! - pull-request branches, and minutes per branch: the ledger has no pull
//!   request number, so a branch stands for its pull request;
//! - the runner wait per pool, `started - created` over first attempts, at
//!   the 50th and 90th percentile.
//!
//! Runner names never leave this module: the answer is public, and pool names
//! already appear in public workflows.

use crate::hub::model::JobRecord;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The longest window, in days counted inclusively. The ledger reader spans
/// at most two calendar months, and 31 days never crosses more than one
/// month boundary.
pub const MAX_DAYS: i64 = 31;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Pool {
    pub pool: String,
    /// Finished jobs that got a runner, every attempt.
    pub jobs: usize,
    pub runner_minutes: f64,
    /// First attempts with a start time: the denominator of the waits.
    pub wait_samples: usize,
    pub wait_p50_s: Option<f64>,
    pub wait_p90_s: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Event {
    pub event: String,
    pub runs: usize,
    pub runner_minutes: f64,
    pub minutes_per_run: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Usage {
    pub repo: String,
    pub from: String,
    pub until: String,
    /// Ledger records of this repository in the window, before any filter.
    pub records: usize,
    pub runs: usize,
    pub runner_minutes: f64,
    pub minutes_per_run: Option<f64>,
    pub pr_branches: usize,
    pub minutes_per_pr_branch: Option<f64>,
    pub pools: Vec<Pool>,
    pub events: Vec<Event>,
}

/// Rejects anything but a repository name and an inclusive window of real
/// days, `from` not after `until`, at most [`MAX_DAYS`] long.
pub fn check(repo: &str, from: &str, until: &str) -> Result<(), String> {
    if repo.is_empty()
        || repo.len() > 100
        || !repo
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    {
        return Err("repo must be a repository name".into());
    }
    let day = |s: &str| {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .filter(|d| d.to_string() == s)
    };
    let (Some(f), Some(u)) = (day(from), day(until)) else {
        return Err("from and until must be days, YYYY-MM-DD".into());
    };
    let days = (u - f).num_days() + 1;
    if days < 1 {
        return Err("from must not be after until".into());
    }
    if days > MAX_DAYS {
        return Err(format!("the window is at most {MAX_DAYS} days"));
    }
    Ok(())
}

/// The value at the nearest rank, or `None` without samples.
fn percentile(v: &[f64], p: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut v = v.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v.get(i).copied()
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn per(minutes: f64, n: usize) -> Option<f64> {
    (n > 0).then(|| round1(minutes / n as f64))
}

/// The usage of `repo` over the ledger days `from..=until`.
pub fn usage(recs: &[JobRecord], repo: &str, from: &str, until: &str) -> Usage {
    let mine: Vec<&JobRecord> = recs
        .iter()
        .filter(|r| r.repo == repo && r.d.as_str() >= from && r.d.as_str() <= until)
        .collect();
    let mut pools: BTreeMap<&str, (usize, f64, Vec<f64>)> = BTreeMap::new();
    let mut events: BTreeMap<&str, (BTreeSet<u64>, f64)> = BTreeMap::new();
    let mut branches: BTreeMap<&str, f64> = BTreeMap::new();
    let mut runs: BTreeSet<u64> = BTreeSet::new();
    let mut total = 0.0;
    for r in &mine {
        runs.insert(r.run);
        let ev = events.entry(&r.ev).or_default();
        ev.0.insert(r.run);
        if r.pool.is_empty() {
            continue;
        }
        let p = pools.entry(&r.pool).or_default();
        if let (Some(s), Some(e)) = (r.s, r.e) {
            if e >= s && !r.runner.is_empty() {
                let m = (e - s) as f64 / 60.0;
                p.0 += 1;
                p.1 += m;
                ev.1 += m;
                total += m;
                if r.ev == "pull_request" {
                    if let Some(b) = r.br.as_deref().filter(|b| !b.is_empty()) {
                        *branches.entry(b).or_default() += m;
                    }
                }
            }
        }
        if r.att == 1 {
            if let Some(s) = r.s.filter(|s| *s >= r.c) {
                p.2.push((s - r.c) as f64);
            }
        }
    }
    let pr_minutes: f64 = branches.values().sum();
    Usage {
        repo: repo.into(),
        from: from.into(),
        until: until.into(),
        records: mine.len(),
        runs: runs.len(),
        runner_minutes: round1(total),
        minutes_per_run: per(total, runs.len()),
        pr_branches: branches.len(),
        minutes_per_pr_branch: per(pr_minutes, branches.len()),
        pools: pools
            .into_iter()
            .map(|(pool, (jobs, m, w))| Pool {
                pool: pool.into(),
                jobs,
                runner_minutes: round1(m),
                wait_samples: w.len(),
                wait_p50_s: percentile(&w, 0.5),
                wait_p90_s: percentile(&w, 0.9),
            })
            .collect(),
        events: events
            .into_iter()
            .map(|(event, (r, m))| Event {
                event: event.into(),
                runs: r.len(),
                runner_minutes: round1(m),
                minutes_per_run: per(m, r.len()),
            })
            .collect(),
    }
}

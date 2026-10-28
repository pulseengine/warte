//! Machine-readable queue and PR admission for agents (#55, REQ-QUEUE-001/002).
//!
//! Agents open small PRs faster than the fleet can run them. Before opening
//! one, an agent asks `GET /api/admission?repo=&kind=`; the answer comes from
//! the ground station's latest snapshot, and the thresholds live here,
//! versioned with warte, not in each agent.
//!
//! Stale or missing data never gates: warte does not hold an agent on numbers
//! it does not have.

use crate::hub::model::Snapshot;
use serde::Serialize;
use std::collections::BTreeSet;

/// A snapshot older than this is stale: the agent sends one a minute.
pub const STALE_AFTER_S: i64 = 5 * 60;
/// A fleet is saturated when every self-hosted runner is busy and at least
/// this many self-hosted jobs are already waiting.
pub const SATURATED_QUEUED_JOBS: u32 = 50;
/// A feature batches once its repo has this many runs queued.
pub const FEATURE_BATCH_QUEUED_RUNS: usize = 2;
/// A fix batches once its repo has this many runs queued.
pub const FIX_BATCH_QUEUED_RUNS: usize = 2;
/// When to ask again.
pub const RECHECK_BATCH_S: i64 = 600;
pub const RECHECK_HOLD_S: i64 = 1200;

const HOSTED_POOL: &str = "hosted";

#[derive(Serialize, Debug, PartialEq)]
pub struct SelfHosted {
    pub busy: usize,
    pub total: usize,
    pub queued_jobs: u32,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Hosted {
    pub queued_jobs: u32,
    pub oldest_wait_s: i64,
    pub median_wait_s: i64,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct RepoQueue {
    pub queued_runs: usize,
    pub running_runs: usize,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct AsOf {
    /// When the snapshot was taken on the CI host (unix seconds).
    pub taken_at: i64,
    /// Seconds since the ground station received it.
    pub age_s: i64,
    pub stale: bool,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct QueueView {
    pub self_hosted: SelfHosted,
    pub hosted: Hosted,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<RepoQueue>,
    pub as_of: AsOf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Docs,
    Fix,
    Feature,
    Security,
    RedMain,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "docs" => Kind::Docs,
            "fix" => Kind::Fix,
            "feature" => Kind::Feature,
            "security" => Kind::Security,
            "red-main" => Kind::RedMain,
            _ => return None,
        })
    }
}

#[derive(Serialize, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Open,
    Batch,
    Hold,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Admission {
    pub verdict: Verdict,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recheck_s: Option<i64>,
}

/// A repo name without the org, as the hub's `/api/eta` takes it. Only ever
/// compared with queue entries, never used as a path.
pub fn valid_repo(repo: &str) -> bool {
    !repo.is_empty()
        && repo.len() <= 100
        && repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

/// `received_at`: when the ground station got the snapshot; `now`: unix seconds.
pub fn queue(s: &Snapshot, received_at: i64, now: i64, repo: Option<&str>) -> QueueView {
    let busy = s
        .runners
        .iter()
        .filter(|r| r.local_busy || r.gh_busy == Some(true))
        .count();
    let (self_hosted_queued, hosted) = match &s.queue {
        Some(q) => {
            let self_hosted = q
                .pools
                .iter()
                .filter(|(pool, _)| pool.as_str() != HOSTED_POOL)
                .map(|(_, p)| p.jobs_queued)
                .sum();
            let hp = q.pools.get(HOSTED_POOL);
            (
                self_hosted,
                Hosted {
                    queued_jobs: q.hosted_queued,
                    oldest_wait_s: hp.map(|p| p.oldest_s).unwrap_or(0),
                    median_wait_s: hp.map(|p| p.median_s).unwrap_or(0),
                },
            )
        }
        None => (
            0,
            Hosted {
                queued_jobs: 0,
                oldest_wait_s: 0,
                median_wait_s: 0,
            },
        ),
    };
    let age = (now - received_at).max(0);
    QueueView {
        self_hosted: SelfHosted {
            busy,
            total: s.runners.len(),
            queued_jobs: self_hosted_queued,
        },
        hosted,
        repo: repo.map(|r| repo_queue(s, r)),
        as_of: AsOf {
            taken_at: s.taken_at,
            age_s: age,
            stale: age > STALE_AFTER_S,
        },
    }
}

fn repo_queue(s: &Snapshot, repo: &str) -> RepoQueue {
    let full = |r: &str| r == repo || r.rsplit('/').next() == Some(repo);
    let mut queued: BTreeSet<u64> = BTreeSet::new();
    let mut running: BTreeSet<u64> = BTreeSet::new();
    if let Some(q) = &s.queue {
        for e in q.queued.iter().filter(|e| full(&e.repo)) {
            queued.insert(e.run_id);
        }
        for h in q.hosted.iter().filter(|h| full(&h.repo)) {
            if h.status == "queued" {
                queued.insert(h.run_id);
            } else {
                running.insert(h.run_id);
            }
        }
    }
    for r in &s.runners {
        if let Some(c) = r.ctx.as_ref().filter(|c| full(&c.repo)) {
            if r.local_busy || r.gh_busy == Some(true) {
                running.insert(c.run_id);
            }
        }
    }
    RepoQueue {
        queued_runs: queued.len(),
        running_runs: running.len(),
    }
}

/// The admission verdict for opening a PR of `kind` in `repo`.
pub fn admit(s: &Snapshot, received_at: i64, now: i64, repo: &str, kind: Kind) -> Admission {
    let open = |reason: String| Admission {
        verdict: Verdict::Open,
        reason,
        recheck_s: None,
    };
    match kind {
        Kind::Security => return open("security fixes are never held".into()),
        Kind::RedMain => return open("fixing a red main is never held".into()),
        _ => {}
    }
    let q = queue(s, received_at, now, Some(repo));
    if q.as_of.stale {
        return open(format!(
            "queue data is stale ({}s old): not gating",
            q.as_of.age_s
        ));
    }
    let r = q.repo.as_ref().expect("repo requested");
    let saturated = q.self_hosted.total > 0
        && q.self_hosted.busy == q.self_hosted.total
        && q.self_hosted.queued_jobs >= SATURATED_QUEUED_JOBS;
    let batch = |reason: String| Admission {
        verdict: Verdict::Batch,
        reason,
        recheck_s: Some(RECHECK_BATCH_S),
    };
    match kind {
        Kind::Docs if r.queued_runs + r.running_runs > 0 => batch(format!(
            "{repo} has {} run(s) queued and {} running: fold docs into the next change",
            r.queued_runs, r.running_runs
        )),
        Kind::Docs => open(format!("{repo} has no CI in flight")),
        Kind::Fix if r.queued_runs >= FIX_BATCH_QUEUED_RUNS => batch(format!(
            "{repo} already has {} run(s) queued: batch the fix with them",
            r.queued_runs
        )),
        Kind::Fix => open(format!("{repo} has {} run(s) queued", r.queued_runs)),
        Kind::Feature if saturated => Admission {
            verdict: Verdict::Hold,
            reason: format!(
                "self-hosted fleet saturated: {}/{} runners busy, {} jobs queued",
                q.self_hosted.busy, q.self_hosted.total, q.self_hosted.queued_jobs
            ),
            recheck_s: Some(RECHECK_HOLD_S),
        },
        Kind::Feature if r.queued_runs >= FEATURE_BATCH_QUEUED_RUNS => batch(format!(
            "{repo} already has {} run(s) queued",
            r.queued_runs
        )),
        Kind::Feature => open(format!(
            "fleet has room: {}/{} runners busy, {} jobs queued",
            q.self_hosted.busy, q.self_hosted.total, q.self_hosted.queued_jobs
        )),
        Kind::Security | Kind::RedMain => unreachable!("handled above"),
    }
}

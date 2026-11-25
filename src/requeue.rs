//! Hosted jobs GitHub never gave a runner (REQ-QUEUE-003), phase 1 of
//! docs/plans/2026-11-25-requeue-not-acquired.md: classify and count.
//!
//! A job is `infra:not-acquired` when it finished `cancelled`, no runner took
//! it, it ran no step, and one of its check-run annotations says the job was
//! "not acquired by Runner". The first three come from the job listing; the
//! annotation costs one request, so it is read only for jobs that pass them.

use crate::hub::model::JobRecord;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// The signature a job record carries for this class.
pub const NOT_ACQUIRED: &str = "infra:not-acquired";
/// The annotation text GitHub writes for such a job.
const ANNOTATION: &str = "not acquired by Runner";
/// Annotation reads one ledger pass may spend.
pub const MAX_ANNOTATION_READS: usize = 20;

/// Whether a job from GitHub's job listing is cancelled, runner-less and
/// step-less: the conditions that make its annotations worth reading.
pub fn may_be_not_acquired(job: &Value) -> bool {
    let no_runner = job["runner_name"].as_str().is_none_or(str::is_empty)
        && job["runner_id"].as_u64().is_none_or(|id| id == 0);
    let no_step = job["steps"].as_array().is_none_or(|s| {
        s.iter()
            .all(|st| st["started_at"].is_null() && st["conclusion"] != "success")
    });
    job["status"] == "completed" && job["conclusion"] == "cancelled" && no_runner && no_step
}

/// Whether a check run's annotations (GitHub's array) say no runner acquired it.
pub fn says_not_acquired(annotations: &Value) -> bool {
    annotations.as_array().is_some_and(|a| {
        a.iter().any(|x| {
            x["message"]
                .as_str()
                .is_some_and(|m| m.contains(ANNOTATION))
        })
    })
}

/// Not-acquired jobs on one pool and UTC day.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PoolDay {
    pub pool: String,
    pub day: String,
    pub jobs: u32,
}

/// Not-acquired jobs per pool and day, newest day first, then by pool;
/// only `repo`'s when given.
pub fn counts(recs: &[JobRecord], repo: Option<&str>) -> Vec<PoolDay> {
    let mut by: BTreeMap<(String, String), u32> = BTreeMap::new();
    for r in recs
        .iter()
        .filter(|r| r.sig.as_deref() == Some(NOT_ACQUIRED))
        .filter(|r| repo.is_none_or(|x| r.repo == x))
    {
        *by.entry((r.d.clone(), r.pool.clone())).or_default() += 1;
    }
    let mut out: Vec<PoolDay> = by
        .into_iter()
        .map(|((day, pool), jobs)| PoolDay { pool, day, jobs })
        .collect();
    out.sort_by(|a, b| b.day.cmp(&a.day).then(a.pool.cmp(&b.pool)));
    out
}

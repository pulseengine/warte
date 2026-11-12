//! The job ledger for the GitHub source (REQ-RUNNERS-005): what the smithy
//! agent's ledger does, from GitHub's REST API, so the monthly report and the
//! runner minutes work at a site without an agent.
//!
//! A ledger day is the UTC day a RUN was created (GitHub's `created=` filter),
//! as on the agent. Each pass lists the runs of every scanned repo for each
//! day still open, reads the jobs of every finished run not read yet (all
//! attempts), and records each finished job once. The day's manifest (count
//! and XOR of the job ids recorded) goes to the hub with the records; it is
//! `closed` only once the day is over, every run created on it has finished
//! and been read, and no listing was cut short. Until then the report calls
//! the month preliminary.
//!
//! Requests: one listing per repo and open day, sent with the ETag of the
//! previous answer (a 304 does not count against the rate limit), plus one
//! job listing per finished run, once. Closed days are not listed again.

use super::{Client, GhError};
use crate::hub::model::{DayManifest, JobRecord};
use crate::hub::parse::{pool_with, rfc3339_utc};
use crate::hub::report::{day_of, day_start};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// A finished day closes this long after its end, so a run created just
/// before midnight has surely been listed.
pub const CLOSE_GRACE_S: i64 = 3600;
/// Days kept in the state file; older ones are dropped (the ledger keeps the
/// records, and the report reads at most a month back).
const KEEP_DAYS: i64 = 45;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DayState {
    /// Run id -> attempt read. A run re-run later is read again.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub runs: BTreeMap<u64, u32>,
    /// Job ids recorded on this day, while it is open (dropped once closed).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub ids: BTreeSet<u64>,
    pub n: u32,
    pub x: u64,
    pub closed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LedgerState {
    pub days: BTreeMap<String, DayState>,
}

impl LedgerState {
    /// The state file, or an empty state when it is missing or unreadable
    /// (the next pass reads the open days again; the ledger drops duplicates).
    pub fn load(path: &Path) -> LedgerState {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn manifests(&self) -> Vec<DayManifest> {
        self.days
            .iter()
            .map(|(d, s)| DayManifest {
                day: d.clone(),
                n: s.n,
                x: s.x,
                closed: s.closed,
            })
            .collect()
    }
}

/// One finished job as a ledger record; `None` for a job not finished.
pub fn record(
    day: &str,
    repo: &str,
    run: &Value,
    job: &Value,
    self_hosted: &[String],
) -> Option<JobRecord> {
    if job["status"] != "completed" {
        return None;
    }
    let t = |k: &str| job[k].as_str().and_then(rfc3339_utc);
    let labels: Vec<String> = job["labels"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| l.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
    Some(JobRecord {
        d: day.to_string(),
        id: job["id"].as_u64()?,
        run: run["id"].as_u64()?,
        att: job["run_attempt"]
            .as_u64()
            .or(run["run_attempt"].as_u64())
            .unwrap_or(1) as u32,
        repo: repo.to_string(),
        wf: s(&run["name"]),
        job: s(&job["name"]),
        pool: pool_with(&labels, self_hosted),
        runner: s(&job["runner_name"]),
        ev: s(&run["event"]),
        // as the agent: 0 when GitHub gave no creation time
        c: t("created_at").unwrap_or(0),
        s: t("started_at"),
        e: t("completed_at"),
        r: s(&job["conclusion"]),
        sig: None,
    })
}

/// Whether a record is a failure recent enough to read its log for a
/// shared incident: older ones could never join one on the overview.
pub fn wants_log(r: &JobRecord, now: i64) -> bool {
    (r.r == "failure" || r.r == "timed_out") && r.e.unwrap_or(r.c) > now - crate::incidents::KEEP_S
}

/// One ledger pass over `repos` for the days from `days` back to today.
/// Returns the new records and the manifest of every day it looked at.
/// `st` changes only when the pass succeeds, so a failed pass loses nothing:
/// the next one reads the same runs again.
pub async fn pass(
    c: &Client,
    org: &str,
    repos: &[String],
    state: &mut LedgerState,
    now: i64,
    days: u32,
    self_hosted: &[String],
) -> Result<(Vec<JobRecord>, Vec<DayManifest>), GhError> {
    c.begin_scan();
    let mut st = state.clone();
    let today = day_start(&day_of(now)).unwrap_or(now);
    let mut out = vec![];
    let mut looked = vec![];
    let mut logs = 0;
    for k in (0..=days as i64).rev() {
        let day = day_of(today - k * 86_400);
        if st.days.get(&day).is_some_and(|d| d.closed) {
            continue;
        }
        let mut ds = st.days.get(&day).cloned().unwrap_or_default();
        let mut pending = false;
        for repo in repos {
            let (runs, cut) = c
                .all_paged(
                    &format!("/repos/{org}/{repo}/actions/runs?created={day}&per_page=100"),
                    Some("workflow_runs"),
                )
                .await?;
            pending |= cut;
            for run in &runs {
                let Some(id) = run["id"].as_u64() else {
                    continue;
                };
                if run["status"] != "completed" {
                    pending = true;
                    continue;
                }
                let att = run["run_attempt"].as_u64().unwrap_or(1) as u32;
                if ds.runs.get(&id) == Some(&att) {
                    continue;
                }
                let (jobs, cut) = c
                    .all_paged(
                        &format!(
                            "/repos/{org}/{repo}/actions/runs/{id}/jobs?filter=all&per_page=100"
                        ),
                        Some("jobs"),
                    )
                    .await?;
                if cut {
                    pending = true;
                    continue;
                }
                for j in &jobs {
                    let Some(mut r) = record(&day, repo, run, j, self_hosted) else {
                        // a finished run whose job is not: read it again
                        pending = true;
                        continue;
                    };
                    // what the ledger would refuse never enters the manifest,
                    // or the day could never match
                    if crate::hub::belt::plausible(&r) && ds.ids.insert(r.id) {
                        // a recent failure's signature, from its log's end
                        // (REQ-CI-005); a log that cannot be read leaves none
                        if wants_log(&r, now) && logs < crate::incidents::MAX_LOGS_PER_PASS {
                            logs += 1;
                            if let Ok(t) = c.job_log_tail(org, repo, r.id).await {
                                r.sig = crate::incidents::of_log(&t).map(|s| s.encode());
                            }
                        }
                        ds.n += 1;
                        ds.x ^= r.id;
                        out.push(r);
                    }
                }
                ds.runs.insert(id, att);
            }
        }
        let over = day_start(&day).is_some_and(|s| now >= s + 86_400 + CLOSE_GRACE_S);
        if over && !pending {
            ds.closed = true;
            ds.runs.clear();
            ds.ids.clear();
        }
        looked.push(DayManifest {
            day: day.clone(),
            n: ds.n,
            x: ds.x,
            closed: ds.closed,
        });
        st.days.insert(day, ds);
    }
    let oldest = day_of(today - KEEP_DAYS * 86_400);
    st.days.retain(|d, _| *d >= oldest);
    *state = st;
    Ok((out, looked))
}

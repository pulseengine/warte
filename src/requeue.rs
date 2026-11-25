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

// ── Phase 2: the re-queue offer (REQ-QUEUE-004) ─────────────────────────

/// Backoff before each attempt, in minutes, after the ejection.
pub const BACKOFF_MIN: [i64; 3] = [30, 60, 120];
/// Offers per PR and UTC day.
pub const MAX_ATTEMPTS: u32 = 3;
/// An offer is made for an ejection at most this long ago.
pub const OFFER_FOR_S: i64 = 86_400;
/// PR heads one pass may resolve.
pub const MAX_HEAD_READS: usize = 10;

/// The PR number a merge-group branch names:
/// `gh-readonly-queue/<base>/pr-<N>-<sha>`.
pub fn pr_of(branch: &str) -> Option<u64> {
    let rest = branch.strip_prefix("gh-readonly-queue/")?;
    let last = rest.rsplit('/').next()?;
    let (n, _sha) = last.strip_prefix("pr-")?.split_once('-')?;
    n.parse().ok().filter(|n| *n > 0)
}

/// One finished merge-group run of a PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ejection {
    pub repo: String,
    pub pr: u64,
    pub run: u64,
    pub att: u32,
    /// When its last job finished.
    pub at: i64,
    /// The pool of its first not-acquired job, or of its first failed job.
    pub pool: String,
    /// Every job in it succeeded: the PR left the queue merged.
    pub passed: bool,
    /// It did not pass, and every job in it that did not succeed is
    /// `infra:not-acquired`.
    pub only_not_acquired: bool,
}

fn ok(r: &str) -> bool {
    matches!(r, "success" | "skipped" | "neutral")
}

/// The finished merge-group runs of `recs` that name a PR, oldest first.
pub fn ejections(recs: &[JobRecord]) -> Vec<Ejection> {
    let mut runs: BTreeMap<(String, u64, u32), Vec<&JobRecord>> = BTreeMap::new();
    for r in recs.iter().filter(|r| r.ev == "merge_group") {
        runs.entry((r.repo.clone(), r.run, r.att))
            .or_default()
            .push(r);
    }
    let mut out: Vec<Ejection> = runs
        .into_iter()
        .filter_map(|((repo, run, att), jobs)| {
            let pr = pr_of(jobs[0].br.as_deref()?)?;
            let bad: Vec<&&JobRecord> = jobs.iter().filter(|j| !ok(&j.r)).collect();
            let passed = bad.is_empty();
            let only = !passed && bad.iter().all(|j| j.sig.as_deref() == Some(NOT_ACQUIRED));
            let pool = bad
                .iter()
                .find(|j| j.sig.as_deref() == Some(NOT_ACQUIRED))
                .or(bad.first())
                .map(|j| j.pool.clone())
                .unwrap_or_default();
            Some(Ejection {
                repo,
                pr,
                run,
                att,
                at: jobs.iter().map(|j| j.e.unwrap_or(j.c)).max().unwrap_or(0),
                pool,
                passed,
                only_not_acquired: only,
            })
        })
        .collect();
    out.sort_by_key(|e| (e.at, e.run, e.att));
    out
}

/// What warte offers one repo's agent (seam B, `warte.requeue.v1`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Offer {
    pub pr: u64,
    /// The PR's head when warte resolved it after the ejection.
    pub head_sha: String,
    /// The ejecting merge-group run.
    pub run_id: u64,
    pub attempt: u32,
    pub of: u32,
    /// Not before this time (Unix seconds, UTC).
    pub earliest: i64,
    pub pool: String,
}

/// A PR whose third offer was ejected too, on one UTC day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exhausted {
    pub repo: String,
    pub pr: u64,
    pub day: String,
}

/// A repo whose offers stopped: a PR offered a re-queue was then ejected for
/// another reason. It holds until a person clears it after `at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    pub repo: String,
    pub pr: u64,
    pub run: u64,
    pub at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// (repo, offer)
    pub offers: Vec<(String, Offer)>,
    pub exhausted: Vec<Exhausted>,
    pub stopped: Vec<Stopped>,
}

impl Plan {
    /// One repo's offers, none while its offers are stopped.
    pub fn offers_for(&self, repo: &str) -> Vec<Offer> {
        if self.stopped.iter().any(|s| s.repo == repo) {
            return vec![];
        }
        self.offers
            .iter()
            .filter(|(r, _)| r == repo)
            .map(|(_, o)| o.clone())
            .collect()
    }
}

/// The offers, exhausted PRs and stopped repos for `ejs` (oldest first, as
/// [`ejections`] returns them). `heads` maps an ejecting run to the PR head
/// resolved for it; an ejection without one gets no offer. `cleared` maps a
/// repo to when a person last cleared its stop.
pub fn plan(
    ejs: &[Ejection],
    heads: &BTreeMap<u64, String>,
    cleared: &BTreeMap<String, i64>,
    now: i64,
) -> Plan {
    use crate::hub::report::day_of;
    let mut p = Plan::default();
    let mut by_pr: BTreeMap<(&str, u64), Vec<&Ejection>> = BTreeMap::new();
    for e in ejs {
        by_pr.entry((e.repo.as_str(), e.pr)).or_default().push(e);
    }
    for ((repo, pr), list) in by_pr {
        // the kill criterion: an ejection for another reason after one that
        // was offered, since the repo was last cleared
        let since = cleared.get(repo).copied().unwrap_or(i64::MIN);
        let mut offered = false;
        for e in list.iter().filter(|e| e.at > since) {
            if e.passed {
                offered = false;
            } else if e.only_not_acquired {
                offered = true;
            } else if offered && !p.stopped.iter().any(|s| s.repo == repo) {
                p.stopped.push(Stopped {
                    repo: repo.into(),
                    pr,
                    run: e.run,
                    at: e.at,
                });
            }
        }
        // an offer answers only the PR's latest ejection, and only one caused
        // by unacquired jobs alone
        let Some(last) = list.last().filter(|e| e.only_not_acquired) else {
            continue;
        };
        let day = day_of(last.at);
        let n = list
            .iter()
            .filter(|e| e.only_not_acquired && day_of(e.at) == day)
            .count() as u32;
        if n > MAX_ATTEMPTS {
            p.exhausted.push(Exhausted {
                repo: repo.into(),
                pr,
                day,
            });
            continue;
        }
        if now - last.at > OFFER_FOR_S {
            continue;
        }
        let Some(head) = heads.get(&last.run) else {
            continue;
        };
        p.offers.push((
            repo.into(),
            Offer {
                pr,
                head_sha: head.clone(),
                run_id: last.run,
                attempt: n,
                of: MAX_ATTEMPTS,
                earliest: last.at + BACKOFF_MIN[(n - 1) as usize] * 60,
                pool: last.pool.clone(),
            },
        ));
    }
    p.stopped.sort_by(|a, b| a.repo.cmp(&b.repo));
    p.stopped.dedup_by(|a, b| a.repo == b.repo);
    p
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Kept {
    /// Ejecting run -> the PR head resolved for it.
    #[serde(default)]
    heads: BTreeMap<u64, String>,
    /// Repo -> when a person last cleared its stopped offers.
    #[serde(default)]
    cleared: BTreeMap<String, i64>,
}

/// The re-queue state a hub keeps: the resolved heads and the clears, in
/// `requeue.json` in its data directory, and the latest plan in memory.
#[derive(Debug, Default)]
pub struct Board {
    path: Option<std::path::PathBuf>,
    inner: std::sync::Mutex<(Kept, Plan)>,
}

impl Board {
    /// The board kept in `dir`; empty when the file is missing or unreadable.
    pub fn open(dir: &str) -> Board {
        let path = std::path::Path::new(dir).join("requeue.json");
        let kept = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Board {
            path: Some(path),
            inner: std::sync::Mutex::new((kept, Plan::default())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (Kept, Plan)> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, k: &Kept) {
        let Some(path) = &self.path else { return };
        let tmp = path.with_extension("json.tmp");
        let ok = serde_json::to_vec(k).ok().is_some_and(|b| {
            std::fs::write(&tmp, b).is_ok() && std::fs::rename(&tmp, path).is_ok()
        });
        if !ok {
            tracing::warn!("requeue state not saved");
        }
    }

    /// Whether a PR head is known for this ejecting run.
    pub fn has_head(&self, run: u64) -> bool {
        self.lock().0.heads.contains_key(&run)
    }

    pub fn set_head(&self, run: u64, sha: &str) {
        let mut g = self.lock();
        g.0.heads.insert(run, sha.to_string());
        self.save(&g.0);
    }

    /// A person looked: offers for `repo` resume for ejections after `at`.
    pub fn clear(&self, repo: &str, at: i64) {
        let mut g = self.lock();
        g.0.cleared.insert(repo.to_string(), at);
        let ejs = std::mem::take(&mut g.1);
        // the stop lifts at once; the next pass recomputes the rest
        g.1 = Plan {
            stopped: ejs.stopped.into_iter().filter(|s| s.repo != repo).collect(),
            ..ejs
        };
        self.save(&g.0);
    }

    /// Recompute the plan from `ejs`; heads of runs no longer among them are
    /// dropped.
    pub fn update(&self, ejs: &[Ejection], now: i64) {
        let mut g = self.lock();
        let runs: std::collections::BTreeSet<u64> = ejs.iter().map(|e| e.run).collect();
        let before = g.0.heads.len();
        g.0.heads.retain(|r, _| runs.contains(r));
        g.1 = plan(ejs, &g.0.heads, &g.0.cleared, now);
        if g.0.heads.len() != before {
            self.save(&g.0);
        }
    }

    pub fn plan(&self) -> Plan {
        self.lock().1.clone()
    }
}

/// The plan as outage findings, so offers, exhausted retries and stopped
/// repos reach the owner as notices and pushes (REQ-QUEUE-004). An offer is
/// amber; an exhausted PR and a stopped repo are red.
pub fn detected(p: &Plan) -> Vec<crate::dashboard::fleet::Detected> {
    use crate::dashboard::fleet::Detected;
    let hm = |t: i64| {
        chrono::DateTime::from_timestamp(t, 0)
            .map(|d| d.format("%H:%M UTC").to_string())
            .unwrap_or_default()
    };
    let mut out: Vec<Detected> = p
        .offers
        .iter()
        .filter(|(r, _)| !p.stopped.iter().any(|s| &s.repo == r))
        .map(|(repo, o)| Detected {
            key: format!("requeue:{repo}#{}:{}", o.pr, o.run_id),
            sev: 1,
            text: format!(
                "{repo}#{} ejected from the merge queue: no {} runner acquired its job. Re-queue {}/{} after {}.",
                o.pr,
                o.pool,
                o.attempt,
                o.of,
                hm(o.earliest)
            ),
        })
        .collect();
    out.extend(p.exhausted.iter().map(|x| Detected {
        key: format!("requeue-exhausted:{}#{}:{}", x.repo, x.pr, x.day),
        sev: 2,
        text: format!(
            "{}#{} was ejected {} times on {} with no runner acquired: re-queue offers exhausted.",
            x.repo,
            x.pr,
            MAX_ATTEMPTS + 1,
            x.day
        ),
    }));
    out.extend(p.stopped.iter().map(|s| Detected {
        key: format!("requeue-stopped:{}", s.repo),
        sev: 2,
        text: format!(
            "Re-queue offers for {} stopped: {}#{} was ejected for another reason after an offer. Clear it on the Queue view once looked at.",
            s.repo, s.repo, s.pr
        ),
    }));
    out
}

/// The Queue view's panel: offers, exhausted PRs and stopped repos. With the
/// owner's CSRF token each stopped repo gets a Clear button.
pub fn view(p: &Plan, owner_csrf: Option<&str>) -> String {
    use crate::dashboard::fleet::esc;
    let hm = |t: i64| {
        chrono::DateTime::from_timestamp(t, 0)
            .map(|d| d.format("%H:%M UTC").to_string())
            .unwrap_or_default()
    };
    let mut rows = String::new();
    for (repo, o) in &p.offers {
        rows += &format!(
            "<tr><td>{}#{}</td><td>offer {}/{}</td><td>after {}</td><td>{}</td><td></td></tr>",
            esc(repo),
            o.pr,
            o.attempt,
            o.of,
            hm(o.earliest),
            esc(&o.pool)
        );
    }
    for x in &p.exhausted {
        rows += &format!(
            "<tr><td>{}#{}</td><td class=\"bad\">exhausted</td><td>{}</td><td></td><td></td></tr>",
            esc(&x.repo),
            x.pr,
            esc(&x.day)
        );
    }
    for s in &p.stopped {
        let button = owner_csrf
            .map(|_| {
                format!(
                    r#"<button data-requeue-repo="{}">Clear</button>"#,
                    esc(&s.repo)
                )
            })
            .unwrap_or_default();
        rows += &format!(
            "<tr><td>{}</td><td class=\"bad\">offers stopped</td><td>{}#{} ejected for another reason</td><td></td><td>{button}</td></tr>",
            esc(&s.repo),
            esc(&s.repo),
            s.pr
        );
    }
    let body = if rows.is_empty() {
        r#"<p class="empty">No merge-queue ejection caused only by unacquired hosted jobs in the last day.</p>"#.to_string()
    } else {
        format!(
            r#"<div class="scroll"><table class="data-table"><thead><tr><th>PR</th><th>State</th><th>When</th><th>Pool</th><th></th></tr></thead><tbody>{rows}</tbody></table></div>"#
        )
    };
    let script = match owner_csrf {
        Some(t) if !p.stopped.is_empty() => format!(
            r#"<script>
(function () {{
  document.querySelectorAll("button[data-requeue-repo]").forEach(function (b) {{
    b.addEventListener("click", function () {{
      fetch("api/requeue/clear", {{method: "POST", headers: {{"Content-Type": "application/json", "X-Warte-CSRF": "{}"}},
        body: JSON.stringify({{repo: b.dataset.requeueRepo}})}})
        .then(function (r) {{ if (r.ok) b.closest("tr").remove(); else r.text().then(function (t) {{ b.textContent = t; }}); }});
    }});
  }});
}})();
</script>"#,
            esc(t)
        ),
        _ => String::new(),
    };
    format!(
        r#"<section class="panel" id="requeue"><h3>Re-queue offers · merge-queue ejections with no runner</h3><p class="dim">The PR's agent re-queues it after the time shown, at most three times a day, once it has checked the ejection itself.</p>{body}{script}</section>"#
    )
}

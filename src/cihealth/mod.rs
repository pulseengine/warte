//! CI health trends: this week against the week before (REQ-PREDICT-005,
//! DD-PREDICT-004).
//!
//! From the finished-job ledger (the fleet agent's or the GitHub source's;
//! no extra GitHub requests), warte compares the last seven days with the
//! seven before:
//!
//! - the failure rate, per repository and overall;
//! - flaky jobs: a job that failed and then passed when its run was re-run;
//! - the queue wait (90th percentile, first attempts), per runner pool;
//! - how long a release run takes (median), over every workflow named
//!   release.
//!
//! A change is called out only when both weeks rest on enough data and the
//! change is large (see the constants): a trend shown is one worth acting
//! on, and "not enough data" is never a warning.

use crate::dashboard::fleet::esc;
use crate::hub::model::JobRecord;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub const WEEK_S: i64 = 7 * 86_400;
/// Each week needs at least this many finished jobs for a rate.
pub const MIN_JOBS: usize = 20;
/// A failure rate is called out at this rate or more, when it at least
/// doubled.
pub const FAIL_MIN: f64 = 0.10;
pub const FAIL_FACTOR: f64 = 2.0;
/// Flaky jobs are called out at this many in a week, when more than before.
pub const FLAKY_MIN: usize = 3;
/// A queue wait is called out at this many seconds or more, when it at
/// least doubled.
pub const WAIT_MIN_S: f64 = 300.0;
pub const WAIT_FACTOR: f64 = 2.0;
/// Each week needs at least this many waits, or release runs.
pub const MIN_SAMPLES: usize = 5;
/// A release run is called out when it got this much slower, by at least
/// [`RELEASE_MIN_GAIN_S`].
pub const RELEASE_FACTOR: f64 = 1.5;
pub const RELEASE_MIN_GAIN_S: f64 = 300.0;
/// How long a computed answer is kept.
pub const CACHE_S: i64 = 600;

/// One week of one scope.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Week {
    /// Finished jobs that succeeded or failed (cancelled and skipped are
    /// not counted).
    pub jobs: usize,
    pub failed: usize,
    pub flaky: usize,
}

impl Week {
    pub fn rate(&self) -> Option<f64> {
        (self.jobs > 0).then(|| self.failed as f64 / self.jobs as f64)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Scope {
    pub repo: String,
    pub this: Week,
    pub last: Week,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Wait {
    pub pool: String,
    /// 90th percentile in seconds, when the week has enough samples.
    pub this_p90_s: Option<f64>,
    pub last_p90_s: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Warning {
    /// A repository, a pool, or `all`.
    pub scope: String,
    /// `failure_rate`, `flaky`, `queue_wait` or `release_duration`.
    pub what: &'static str,
    pub this: f64,
    pub last: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Health {
    pub generated_at: i64,
    pub all: Scope,
    pub repos: Vec<Scope>,
    pub waits: Vec<Wait>,
    pub release_this_p50_s: Option<f64>,
    pub release_last_p50_s: Option<f64>,
    pub warnings: Vec<Warning>,
}

/// One attempt of a job: (attempt, failed, created).
type Attempt = (u32, bool, i64);

fn failed(r: &str) -> bool {
    matches!(r, "failure" | "timed_out" | "startup_failure")
}

fn counted(r: &str) -> bool {
    r == "success" || failed(r)
}

/// 0 for this week, 1 for the week before, none for older or newer.
fn week_of(t: i64, now: i64) -> Option<usize> {
    let age = now - t;
    if age < 0 {
        None
    } else if age < WEEK_S {
        Some(0)
    } else if age < 2 * WEEK_S {
        Some(1)
    } else {
        None
    }
}

fn percentile(mut v: Vec<f64>, p: f64) -> Option<f64> {
    if v.len() < MIN_SAMPLES {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v.get(i).copied()
}

fn pct(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

fn mins(s: f64) -> String {
    if s < 90.0 {
        format!("{s:.0} s")
    } else {
        format!("{:.0} min", s / 60.0)
    }
}

/// The trends from the ledger records of the last two weeks.
pub fn health(recs: &[JobRecord], now: i64) -> Health {
    let mut repos: BTreeMap<&str, [Week; 2]> = BTreeMap::new();
    let mut all = [Week::default(), Week::default()];
    // (repo, run, job) -> attempts as (attempt, failed?, created)
    let mut tries: BTreeMap<(&str, u64, &str), Vec<Attempt>> = BTreeMap::new();
    let mut waits: BTreeMap<&str, [Vec<f64>; 2]> = BTreeMap::new();
    // (repo, run, attempt) -> (first start, last end), release workflows
    let mut releases: BTreeMap<(&str, u64, u32), (i64, i64, bool)> = BTreeMap::new();

    for r in recs {
        if counted(&r.r) {
            if let Some(w) = week_of(r.c, now) {
                for wk in [&mut repos.entry(&r.repo).or_default()[w], &mut all[w]] {
                    wk.jobs += 1;
                    wk.failed += failed(&r.r) as usize;
                }
            }
            tries
                .entry((&r.repo, r.run, &r.job))
                .or_default()
                .push((r.att, failed(&r.r), r.c));
        }
        if r.att == 1 {
            if let (Some(s), Some(w)) = (r.s, week_of(r.c, now)) {
                if s >= r.c && !r.pool.is_empty() {
                    waits.entry(&r.pool).or_default()[w].push((s - r.c) as f64);
                }
            }
        }
        if r.wf.to_ascii_lowercase().contains("release") {
            if let (Some(s), Some(e)) = (r.s, r.e) {
                let x = releases
                    .entry((&r.repo, r.run, r.att))
                    .or_insert((s, e, true));
                x.0 = x.0.min(s);
                x.1 = x.1.max(e);
                x.2 &= r.r == "success" || r.r == "skipped";
            }
        }
    }

    // flaky: an attempt failed and a later one passed; counted in the week
    // of the passing attempt
    for ((repo, _, _), mut at) in tries {
        at.sort();
        for (i, (_, f, c)) in at.iter().enumerate() {
            if !*f && at[..i].iter().any(|(_, pf, _)| *pf) {
                if let Some(w) = week_of(*c, now) {
                    repos.entry(repo).or_default()[w].flaky += 1;
                    all[w].flaky += 1;
                }
                break;
            }
        }
    }

    let mut rel: [Vec<f64>; 2] = [vec![], vec![]];
    for (s, e, ok) in releases.values() {
        if *ok {
            if let Some(w) = week_of(*s, now) {
                rel[w].push((e - s) as f64);
            }
        }
    }
    let [rel_this, rel_last] = rel;
    let release_this_p50_s = percentile(rel_this, 0.5);
    let release_last_p50_s = percentile(rel_last, 0.5);

    let scope = |repo: &str, w: &[Week; 2]| Scope {
        repo: repo.into(),
        this: w[0].clone(),
        last: w[1].clone(),
    };
    let all = scope("all", &all);
    let repos: Vec<Scope> = repos.iter().map(|(r, w)| scope(r, w)).collect();
    let waits: Vec<Wait> = waits
        .into_iter()
        .map(|(pool, [a, b])| Wait {
            pool: pool.into(),
            this_p90_s: percentile(a, 0.9),
            last_p90_s: percentile(b, 0.9),
        })
        .collect();

    let mut warnings = Vec::new();
    for s in std::iter::once(&all).chain(repos.iter()) {
        let who = if s.repo == "all" {
            "all repositories".to_string()
        } else {
            s.repo.clone()
        };
        if s.this.jobs >= MIN_JOBS && s.last.jobs >= MIN_JOBS {
            let (a, b) = (s.this.rate().unwrap_or(0.0), s.last.rate().unwrap_or(0.0));
            if a >= FAIL_MIN && a >= FAIL_FACTOR * b {
                warnings.push(Warning {
                    scope: s.repo.clone(),
                    what: "failure_rate",
                    this: a,
                    last: b,
                    text: format!(
                        "{who}: {} of jobs failed this week ({} of {}), {} the week before",
                        pct(a),
                        s.this.failed,
                        s.this.jobs,
                        pct(b)
                    ),
                });
            }
        }
        if s.this.flaky >= FLAKY_MIN && s.this.flaky > s.last.flaky {
            warnings.push(Warning {
                scope: s.repo.clone(),
                what: "flaky",
                this: s.this.flaky as f64,
                last: s.last.flaky as f64,
                text: format!(
                    "{who}: {} jobs failed and then passed on a re-run this week, {} the week before",
                    s.this.flaky, s.last.flaky
                ),
            });
        }
    }
    for w in &waits {
        if let (Some(a), Some(b)) = (w.this_p90_s, w.last_p90_s) {
            if a >= WAIT_MIN_S && a >= WAIT_FACTOR * b {
                warnings.push(Warning {
                    scope: w.pool.clone(),
                    what: "queue_wait",
                    this: a,
                    last: b,
                    text: format!(
                        "pool {}: 1 job in 10 waited {} or more for a runner this week, {} the week before",
                        w.pool,
                        mins(a),
                        mins(b)
                    ),
                });
            }
        }
    }
    if let (Some(a), Some(b)) = (release_this_p50_s, release_last_p50_s) {
        if a >= RELEASE_FACTOR * b && a - b >= RELEASE_MIN_GAIN_S {
            warnings.push(Warning {
                scope: "all".into(),
                what: "release_duration",
                this: a,
                last: b,
                text: format!(
                    "a release run took {} this week (median), {} the week before",
                    mins(a),
                    mins(b)
                ),
            });
        }
    }

    Health {
        generated_at: now,
        all,
        repos,
        waits,
        release_this_p50_s,
        release_last_p50_s,
        warnings,
    }
}

/// The overview's CI health panel.
pub fn panel(h: &Health) -> String {
    let line = match (h.all.this.rate(), h.all.last.rate()) {
        (Some(a), Some(b)) => format!(
            "{} jobs this week, {} failed ({}); {} jobs and {} the week before. {} flaky this week.",
            h.all.this.jobs,
            h.all.this.failed,
            pct(a),
            h.all.last.jobs,
            pct(b),
            h.all.this.flaky
        ),
        (Some(a), None) => format!(
            "{} jobs this week, {} failed ({}); none the week before.",
            h.all.this.jobs,
            h.all.this.failed,
            pct(a)
        ),
        _ => "No finished jobs in the ledger this week.".into(),
    };
    let body = if h.warnings.is_empty() {
        "<p>No trend worth acting on.</p>".to_string()
    } else {
        let li: String = h
            .warnings
            .iter()
            .map(|w| format!("<li>{}</li>", esc(&w.text)))
            .collect();
        format!("<ul class=\"trendlist\">{li}</ul>")
    };
    format!(
        r#"<section class="panel cihealth"><h3>CI health</h3><p class="note">This week against the week before, from the finished-job ledger. {}</p>{body}</section>"#,
        esc(&line)
    )
}

/// The latest answer, recomputed at most every [`CACHE_S`] seconds: the
/// overview refreshes often and the ledger is read from files.
#[derive(Default)]
pub struct Cache(Mutex<Option<Arc<Health>>>);

impl Cache {
    pub async fn get(&self, hub: &crate::hub::Hub, now: i64) -> Arc<Health> {
        if let Some(h) = self.0.lock().ok().and_then(|g| g.clone()) {
            if now - h.generated_at < CACHE_S {
                return h;
            }
        }
        let hub = hub.clone();
        let from = crate::hub::report::day_of(now - 2 * WEEK_S - 86_400);
        let until = crate::hub::report::day_of(now + 86_400);
        let recs = tokio::task::spawn_blocking(move || hub.ledger_days(&from, &until))
            .await
            .unwrap_or_default();
        let h = Arc::new(health(&recs, now));
        if let Ok(mut g) = self.0.lock() {
            *g = Some(h.clone());
        }
        h
    }
}

//! GitHub's REST API as a CI source, for teams without a fleet agent (#81).
//!
//! The scan fills the ground station's model (`hub::model::Snapshot`) the way
//! the fleet agent does, so every view and `/api/eta`, `/api/queue`,
//! `/api/admission` and `/api/watch` work on it unchanged. It is the agent's
//! queue scan (the original hub) without the host view:
//! busy/idle comes from GitHub's runner API alone.
//!
//! The API base is configured, never derived: `https://api.github.com`,
//! `https://api.<tenant>.ghe.com` (Enterprise Cloud with data residency) or
//! `https://<host>/api/v3` (GHES). Read-only: only GETs are issued.

pub mod ledger;

use crate::config::GithubConfig;
use crate::hub::model::{
    HostedGroup, HostedJob, JobContext, PoolQueue, Queue, QueuedEntry, QueuedRun, Runner, Snapshot,
    WaitStats,
};
use crate::hub::parse::{age_stats, pool_with, rfc3339_utc};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Hosted jobs kept per snapshot, as the agent keeps them.
const HOSTED_CAP: usize = 64;
/// Pages followed per listing; the scan stays bounded on a huge org.
const MAX_PAGES: usize = 10;

#[derive(Debug, PartialEq)]
pub enum GhError {
    /// 401, or a 403 that is not a rate limit: the token is wrong or lacks a
    /// scope. Retrying does not help; say so.
    Auth(u16, String),
    /// Primary or secondary rate limit; wait until this unix time.
    RateLimited {
        until: i64,
    },
    Other(String),
}

impl std::fmt::Display for GhError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GhError::Auth(code, msg) => write!(
                f,
                "GitHub API refused the token ({code}): {msg}. Check WARTE_GITHUB_TOKEN and its scopes (Actions: read; self-hosted runners: read)."
            ),
            GhError::RateLimited { until } => write!(f, "GitHub API rate limit; next scan after {until}"),
            GhError::Other(m) => write!(f, "{m}"),
        }
    }
}

/// PEM roots from a file, for a TLS-intercepting proxy's CA. A file with no
/// certificate is an error: reqwest would accept the empty bundle silently.
pub fn extra_roots(path: &std::path::Path) -> Result<Vec<reqwest::Certificate>, String> {
    let bad = |e: &dyn std::fmt::Display| format!("CA bundle {}: {e}", path.display());
    let pem = std::fs::read(path).map_err(|e| bad(&e))?;
    let certs = reqwest::Certificate::from_pem_bundle(&pem).map_err(|e| bad(&e))?;
    if certs.is_empty() {
        return Err(bad(&"no PEM certificate in the file"));
    }
    Ok(certs)
}

/// A 200 answer kept for a conditional request (REQ-GHSRC-003).
struct Cached {
    etag: String,
    body: Value,
    next: Option<String>,
    /// The scan that last used it; older entries are dropped.
    scan: u64,
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
    /// Requests sent.
    pub calls: AtomicU32,
    /// Of those, answered 304 from the ETag cache (free against the primary
    /// rate limit).
    pub not_modified: AtomicU32,
    reserve: i64,
    /// Label patterns that mark a job self-hosted (REQ-GHSRC-005).
    self_hosted: Vec<String>,
    /// `x-ratelimit-remaining` and `-reset` of the last answer; `i64::MAX`
    /// until GitHub has said.
    remaining: AtomicI64,
    reset: AtomicI64,
    scan: AtomicU64,
    cache: Mutex<HashMap<String, Cached>>,
}

/// The `rel="next"` URL of a `Link` header.
fn next_link(link: Option<&str>) -> Option<String> {
    link?.split(',').find_map(|part| {
        let (url, rel) = part.split_once(';')?;
        rel.contains("rel=\"next\"").then(|| {
            url.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string()
        })
    })
}

impl Client {
    pub fn new(cfg: &GithubConfig) -> Result<Client, String> {
        let token = cfg
            .token
            .clone()
            .filter(|t| !t.is_empty())
            .ok_or("no GitHub token: set WARTE_GITHUB_TOKEN")?;
        let mut b = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("warte/", env!("CARGO_PKG_VERSION")));
        if let Some(p) = &cfg.ca_bundle {
            for c in extra_roots(p)? {
                b = b.add_root_certificate(c);
            }
        }
        Ok(Client {
            http: b.build().map_err(|e| e.to_string())?,
            base: cfg.api_url.trim_end_matches('/').to_string(),
            token,
            calls: Default::default(),
            not_modified: Default::default(),
            reserve: cfg.rate_reserve,
            self_hosted: cfg.self_hosted_labels.clone(),
            remaining: AtomicI64::new(i64::MAX),
            reset: AtomicI64::new(0),
            scan: AtomicU64::new(0),
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Start a scan: cached answers not used by the previous scan go (run
    /// and job listings change with every scan; the cache stays bounded by
    /// what two scans ask for).
    pub fn begin_scan(&self) {
        let n = self.scan.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut c) = self.cache.lock() {
            c.retain(|_, e| e.scan + 1 >= n);
        }
    }

    /// Requests GitHub says are left in this window, if it has said.
    pub fn remaining(&self) -> Option<i64> {
        Some(self.remaining.load(Ordering::Relaxed)).filter(|r| *r != i64::MAX)
    }

    /// One GET of an absolute URL: the JSON body and the next page, if any.
    ///
    /// A cached ETag goes out as `If-None-Match`; a 304 reuses the cached
    /// body. No request is sent while GitHub reports `rate_reserve` or fewer
    /// requests left before its reset (REQ-GHSRC-003).
    async fn get_url(&self, url: &str) -> Result<(Value, Option<String>), GhError> {
        let now = crate::hub::now();
        let reset = self.reset.load(Ordering::Relaxed);
        if self.remaining.load(Ordering::Relaxed) <= self.reserve && now < reset {
            return Err(GhError::RateLimited { until: reset });
        }
        let etag = self
            .cache
            .lock()
            .ok()
            .and_then(|c| c.get(url).map(|e| e.etag.clone()));
        self.calls.fetch_add(1, Ordering::Relaxed);
        let mut req = self
            .http
            .get(url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(e) = &etag {
            req = req.header("If-None-Match", e);
        }
        let r = req
            .send()
            .await
            .map_err(|e| GhError::Other(format!("GET {url}: {e}")))?;
        let status = r.status().as_u16();
        let h = |k: &str| {
            r.headers()
                .get(k)
                .and_then(|v| v.to_str().ok())
                .map(String::from)
        };
        if let Some(v) = h("x-ratelimit-remaining").and_then(|v| v.parse().ok()) {
            self.remaining.store(v, Ordering::Relaxed);
        }
        if let Some(v) = h("x-ratelimit-reset").and_then(|v| v.parse().ok()) {
            self.reset.store(v, Ordering::Relaxed);
        }
        let scan = self.scan.load(Ordering::Relaxed);
        if status == 304 {
            if let Some(e) = self.cache.lock().ok().as_mut().and_then(|c| c.get_mut(url)) {
                self.not_modified.fetch_add(1, Ordering::Relaxed);
                e.scan = scan;
                return Ok((e.body.clone(), e.next.clone()));
            }
            return Err(GhError::Other(format!(
                "GET {url}: HTTP 304 without a cached answer"
            )));
        }
        if status == 403 || status == 429 {
            // rate limits are 403/429 with remaining 0 or a retry-after
            // remembered, so no request goes out before the reset even when
            // the scan loop wakes earlier to report the limit
            let until = if let Some(s) = h("retry-after").and_then(|v| v.parse::<i64>().ok()) {
                Some(now + s)
            } else if h("x-ratelimit-remaining").as_deref() == Some("0") {
                Some(
                    h("x-ratelimit-reset")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(now + 60),
                )
            } else {
                None
            };
            if let Some(until) = until {
                self.remaining.store(0, Ordering::Relaxed);
                self.reset.store(until, Ordering::Relaxed);
                return Err(GhError::RateLimited { until });
            }
        }
        if status == 401 || status == 403 {
            let msg = r
                .json::<Value>()
                .await
                .ok()
                .and_then(|v| v["message"].as_str().map(String::from))
                .unwrap_or_default();
            return Err(GhError::Auth(status, msg));
        }
        if !(200..300).contains(&status) {
            return Err(GhError::Other(format!("GET {url}: HTTP {status}")));
        }
        let next = next_link(h("link").as_deref());
        let new_etag = h("etag");
        let v = r
            .json::<Value>()
            .await
            .map_err(|e| GhError::Other(format!("GET {url}: {e}")))?;
        if let (Some(etag), Ok(mut c)) = (new_etag, self.cache.lock()) {
            c.insert(
                url.to_string(),
                Cached {
                    etag,
                    body: v.clone(),
                    next: next.clone(),
                    scan,
                },
            );
        }
        Ok((v, next))
    }

    /// The end of one job's log (at most [`crate::incidents::LOG_TAIL`]
    /// bytes), for its error signature only (REQ-CI-005). GitHub answers
    /// with a redirect to a signed download URL; the token is not sent to
    /// that other host.
    pub async fn job_log_tail(&self, org: &str, repo: &str, id: u64) -> Result<String, GhError> {
        let now = crate::hub::now();
        let reset = self.reset.load(Ordering::Relaxed);
        if self.remaining.load(Ordering::Relaxed) <= self.reserve && now < reset {
            return Err(GhError::RateLimited { until: reset });
        }
        let url = format!("{}/repos/{org}/{repo}/actions/jobs/{id}/logs", self.base);
        self.calls.fetch_add(1, Ordering::Relaxed);
        let mut r = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| GhError::Other(format!("GET job log: {e}")))?;
        if !r.status().is_success() {
            return Err(GhError::Other(format!(
                "GET job log: HTTP {}",
                r.status().as_u16()
            )));
        }
        let cap = crate::incidents::LOG_TAIL;
        let mut tail: Vec<u8> = Vec::new();
        while let Some(chunk) = r
            .chunk()
            .await
            .map_err(|e| GhError::Other(format!("GET job log: {e}")))?
        {
            tail.extend_from_slice(&chunk);
            if tail.len() > 2 * cap {
                tail.drain(..tail.len() - cap);
            }
        }
        if tail.len() > cap {
            tail.drain(..tail.len() - cap);
        }
        Ok(String::from_utf8_lossy(&tail).into_owned())
    }

    async fn get(&self, path: &str) -> Result<Value, GhError> {
        Ok(self.get_url(&format!("{}{path}", self.base)).await?.0)
    }

    /// Every item of a paged listing, following `Link: rel="next"` up to
    /// [`MAX_PAGES`]. `key` names the array in an object body; `None` for a
    /// bare array.
    async fn all(&self, path: &str, key: Option<&str>) -> Result<Vec<Value>, GhError> {
        Ok(self.all_paged(path, key).await?.0)
    }

    /// [`Client::all`], and whether a further page was left unread.
    async fn all_paged(
        &self,
        path: &str,
        key: Option<&str>,
    ) -> Result<(Vec<Value>, bool), GhError> {
        let mut out = vec![];
        let mut url = Some(format!("{}{path}", self.base));
        for _ in 0..MAX_PAGES {
            let Some(u) = url.take() else { break };
            let (v, next) = self.get_url(&u).await?;
            let items = match key {
                Some(k) => v[k].as_array().cloned(),
                None => v.as_array().cloned(),
            };
            out.extend(items.unwrap_or_default());
            url = next;
        }
        Ok((out, url.is_some()))
    }

    /// Org runners as GitHub sees them. There is no host view here, so
    /// GitHub's busy flag is the only one.
    pub async fn runners(&self, org: &str) -> Result<Vec<Runner>, GhError> {
        let rs = self
            .all(
                &format!("/orgs/{org}/actions/runners?per_page=100"),
                Some("runners"),
            )
            .await?;
        Ok(rs
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let busy = r["busy"].as_bool().unwrap_or(false);
                Runner {
                    id: i as u32 + 1,
                    unit: r["name"].as_str().unwrap_or_default().to_string(),
                    active: r["status"] == "online",
                    local_busy: busy,
                    gh_status: r["status"].as_str().map(String::from),
                    gh_busy: Some(busy),
                    labels: r["labels"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|l| l["name"].as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    ..Default::default()
                }
            })
            .collect())
    }

    /// Non-archived repos pushed within `active_days`.
    pub async fn active_repos(
        &self,
        org: &str,
        active_days: i64,
        now: i64,
    ) -> Result<Vec<String>, GhError> {
        let rs = self
            .all(
                &format!("/orgs/{org}/repos?per_page=100&type=all&sort=pushed"),
                None,
            )
            .await?;
        Ok(rs
            .iter()
            .filter(|r| !r["archived"].as_bool().unwrap_or(false))
            .filter(|r| {
                let pushed = r["pushed_at"].as_str().and_then(rfc3339_utc).unwrap_or(0);
                now - pushed <= active_days * 86400
            })
            .filter_map(|r| r["name"].as_str().map(String::from))
            .collect())
    }

    /// The agent's queue scan: queued runs of every repo first, then running
    /// ones; jobs listed for at most `max_runs` runs. `truncated` and
    /// `queued_complete` say when the result is a lower bound.
    ///
    /// Also returns the self-hosted runners seen busy through their running
    /// jobs, for when the runner list cannot be read (REQ-GHSRC-004).
    pub async fn queue(
        &self,
        org: &str,
        active_days: i64,
        max_runs: usize,
        now: i64,
    ) -> Result<(Queue, Vec<Runner>), GhError> {
        let repos = self.active_repos(org, active_days, now).await?;
        let mut runs: Vec<(String, Value, bool)> = vec![];
        for status in ["queued", "in_progress"] {
            for repo in &repos {
                let v = self
                    .get(&format!(
                        "/repos/{org}/{repo}/actions/runs?status={status}&per_page=30"
                    ))
                    .await?;
                for run in v["workflow_runs"].as_array().cloned().unwrap_or_default() {
                    runs.push((repo.clone(), run, status == "queued"));
                }
            }
        }
        let n_queued_runs = runs.iter().filter(|r| r.2).count();
        let truncated = runs.len() > max_runs;
        let queued_complete = n_queued_runs <= max_runs;
        let mut ages: BTreeMap<String, Vec<i64>> = BTreeMap::new();
        let mut waited: BTreeMap<String, Vec<i64>> = BTreeMap::new();
        let mut top: Vec<QueuedRun> = vec![];
        let mut hosted: Vec<HostedJob> = vec![];
        let mut queued: Vec<QueuedEntry> = vec![];
        let mut busy: Vec<Runner> = vec![];
        for (repo, run, _) in runs.into_iter().take(max_runs) {
            let id = run["id"].as_u64().unwrap_or(0);
            let jobs = self
                .all(
                    &format!("/repos/{org}/{repo}/actions/runs/{id}/jobs?per_page=100"),
                    Some("jobs"),
                )
                .await?;
            let s = |k: &str| run[k].as_str().unwrap_or_default().to_string();
            let pr = run["pull_requests"][0]["number"].as_u64().map(|n| n as u32);
            let mut per_pool: BTreeMap<String, (u32, i64)> = BTreeMap::new();
            let (mut n, mut oldest) = (0u32, 0i64);
            for j in &jobs {
                let status = j["status"].as_str().unwrap_or_default();
                let labels: Vec<String> = j["labels"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|l| l.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let pool = pool_with(&labels, &self.self_hosted);
                let created = j["created_at"].as_str().and_then(rfc3339_utc);
                let started = j["started_at"].as_str().and_then(rfc3339_utc);
                if status == "in_progress" {
                    if let (Some(c), Some(st)) = (created, started) {
                        if now - st <= 3 * 3600 && st >= c {
                            waited.entry(pool.clone()).or_default().push(st - c);
                        }
                    }
                }
                if (status == "queued" || status == "in_progress")
                    && crate::hub::pools::is_hosted(&pool)
                {
                    let ts = if status == "in_progress" {
                        started
                    } else {
                        created
                    };
                    hosted.push(HostedJob {
                        repo: format!("{org}/{repo}"),
                        workflow: s("name"),
                        job: j["name"].as_str().unwrap_or_default().to_string(),
                        status: status.to_string(),
                        runner: j["runner_name"].as_str().unwrap_or_default().to_string(),
                        labels: labels.join(","),
                        since: ts.unwrap_or(now),
                        run_id: id,
                        event: s("event"),
                        branch: s("head_branch"),
                        pr,
                    });
                }
                if status == "in_progress" && !crate::hub::pools::is_hosted(&pool) {
                    let runner = j["runner_name"].as_str().unwrap_or_default();
                    busy.push(Runner {
                        unit: if runner.is_empty() {
                            "self-hosted".into()
                        } else {
                            runner.to_string()
                        },
                        active: true,
                        local_busy: true,
                        gh_status: Some("online".into()),
                        gh_busy: Some(true),
                        // a job marked by a configured label counts as self-hosted
                        labels: if labels.iter().any(|l| l.eq_ignore_ascii_case("self-hosted")) {
                            labels.clone()
                        } else {
                            [labels.clone(), vec!["self-hosted".into()]].concat()
                        },
                        job: j["name"].as_str().map(String::from),
                        job_started: started,
                        ctx: Some(JobContext {
                            repo: format!("{org}/{repo}"),
                            workflow: s("name"),
                            run_id: id,
                            run_number: run["run_number"].as_u64().unwrap_or(0) as u32,
                            run_attempt: run["run_attempt"].as_u64().unwrap_or(1) as u32,
                            event: s("event"),
                            branch: s("head_branch"),
                            sha: s("head_sha"),
                            pr,
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                if status != "queued" {
                    continue;
                }
                let c = created.unwrap_or(now);
                let age = (now - c).max(0);
                ages.entry(pool.clone()).or_default().push(age);
                let e = per_pool.entry(pool).or_insert((0, c));
                e.0 += 1;
                e.1 = e.1.min(c);
                n += 1;
                oldest = oldest.max(age);
            }
            for (pool, (jobs, since)) in per_pool {
                queued.push(QueuedEntry {
                    repo: repo.clone(),
                    run_id: id,
                    workflow: s("name"),
                    event: s("event"),
                    branch: s("head_branch"),
                    pr,
                    pool,
                    jobs,
                    since,
                });
            }
            if n > 0 {
                top.push(QueuedRun {
                    repo: repo.clone(),
                    workflow: s("name"),
                    event: s("event"),
                    jobs_queued: n,
                    oldest_s: oldest,
                });
            }
        }
        let waits: BTreeMap<String, WaitStats> = waited
            .into_iter()
            .filter_map(|(k, mut v)| crate::hub::eta::wait_stats(&mut v).map(|w| (k, w)))
            .collect();
        let runs_queued = top.len() as u32;
        let jobs_queued = ages.values().map(|v| v.len() as u32).sum();
        let pools = ages
            .into_iter()
            .map(|(k, mut v)| {
                let (o, m) = age_stats(&mut v);
                (
                    k,
                    PoolQueue {
                        jobs_queued: v.len() as u32,
                        oldest_s: o,
                        median_s: m,
                    },
                )
            })
            .collect();
        top.sort_by(|a, b| {
            b.jobs_queued
                .cmp(&a.jobs_queued)
                .then(b.oldest_s.cmp(&a.oldest_s))
        });
        top.truncate(10);
        hosted.sort_by(|a, b| {
            (a.status != "in_progress")
                .cmp(&(b.status != "in_progress"))
                .then(a.since.cmp(&b.since))
        });
        let hosted_total = hosted.len() as u32;
        let hosted_running = hosted.iter().filter(|j| j.status == "in_progress").count() as u32;
        let mut groups: BTreeMap<(String, String), HostedGroup> = BTreeMap::new();
        for j in &hosted {
            let g = groups
                .entry((j.repo.clone(), j.labels.clone()))
                .or_insert_with(|| HostedGroup {
                    repo: j.repo.clone(),
                    labels: j.labels.clone(),
                    since: j.since,
                    ..Default::default()
                });
            if j.status == "in_progress" {
                g.running += 1
            } else {
                g.queued += 1
            }
            g.since = g.since.min(j.since);
        }
        let mut hosted_groups: Vec<HostedGroup> = groups.into_values().collect();
        hosted_groups.sort_by_key(|g| std::cmp::Reverse(g.running + g.queued));
        hosted.truncate(HOSTED_CAP);
        busy.sort_by(|a, b| a.unit.cmp(&b.unit));
        for (i, r) in busy.iter_mut().enumerate() {
            r.id = i as u32 + 1;
        }
        let q = Queue {
            scanned_at: now,
            repos_scanned: repos.len() as u32,
            runs_queued,
            jobs_queued,
            pools,
            top_runs: top,
            truncated,
            hosted,
            hosted_total,
            hosted_running,
            hosted_queued: hosted_total - hosted_running,
            hosted_groups,
            queued,
            queued_complete,
            waits,
        };
        Ok((q, busy))
    }
}

/// One scan as a ground-station snapshot. Errors land in `errors`, where the
/// Health and Outages views show them, instead of an empty-looking fleet.
pub async fn snapshot(c: &Client, cfg: &GithubConfig) -> Result<Snapshot, GhError> {
    c.begin_scan();
    let now = crate::hub::now();
    let mut errors = vec![];
    let listed = match c.runners(&cfg.org).await {
        Ok(r) => Some(r),
        // listing org runners needs an admin scope (403), or the endpoint is
        // hidden from ordinary members (404); the queue needs neither
        Err(GhError::Auth(code, _)) => {
            errors.push(format!(
                "runners not listed ({code}): the token cannot read the org's self-hosted runners; busy ones are shown from their running jobs, idle ones are unknown"
            ));
            None
        }
        Err(GhError::Other(m)) if m.ends_with("HTTP 404") => {
            errors.push(
                "runners not listed (404): the token cannot read the org's self-hosted runners; busy ones are shown from their running jobs, idle ones are unknown"
                    .into(),
            );
            None
        }
        Err(e) => return Err(e),
    };
    let (queue, busy) = c
        .queue(&cfg.org, cfg.active_days, cfg.max_runs, now)
        .await?;
    let runners_inferred = listed.is_none();
    Ok(Snapshot {
        agent_version: concat!("warte-github ", env!("CARGO_PKG_VERSION")).into(),
        host: format!("github:{}", cfg.org),
        taken_at: now,
        runners: listed.unwrap_or(busy),
        queue: Some(queue),
        errors,
        runners_inferred,
        ..Default::default()
    })
}

/// One ledger pass; a failure is logged and retried at the next pass (the
/// state keeps what was read, the ledger drops duplicates).
async fn ledger_pass(
    c: &Client,
    cfg: &GithubConfig,
    hub: &crate::hub::Hub,
    st: &mut ledger::LedgerState,
    path: &std::path::Path,
    now: i64,
) {
    let calls = c.calls.load(Ordering::Relaxed);
    let repos = match c.active_repos(&cfg.org, cfg.active_days, now).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "GitHub ledger pass: repos not listed");
            return;
        }
    };
    match ledger::pass(
        c,
        &cfg.org,
        &repos,
        st,
        now,
        cfg.ledger_days,
        &cfg.self_hosted_labels,
    )
    .await
    {
        Ok((recs, mans)) => {
            tracing::debug!(
                records = recs.len(),
                requests = c.calls.load(Ordering::Relaxed) - calls,
                "GitHub ledger pass"
            );
            hub.ingest_ledger(recs, mans);
        }
        Err(e) => tracing::warn!(error = %e, "GitHub ledger pass failed"),
    }
    if let Err(e) = st.save(path) {
        tracing::warn!(error = %e, "GitHub ledger state not saved");
    }
}

/// Scan every `interval_s` and hand each snapshot to the hub. A rate limit
/// waits for its reset; a refused token is reported once per scan and the
/// scan is retried at the normal interval, so fixing the token needs no
/// restart.
pub fn spawn(
    hub: crate::hub::Hub,
    cfg: GithubConfig,
    runners: crate::hub::pools::RunnersConfig,
) -> Result<(), String> {
    let client = Client::new(&cfg)?;
    // the ledger pass has its own client, so its ETags outlive the scans
    // between two passes (REQ-RUNNERS-005)
    let lclient = Client::new(&cfg)?;
    let lpath = std::path::Path::new(&cfg.data_dir).join("ghledger.json");
    let mut lstate = ledger::LedgerState::load(&lpath);
    hub.ingest_ledger(vec![], lstate.manifests());
    let mut last_ledger = i64::MIN;
    tokio::spawn(async move {
        loop {
            let (calls, cached) = (
                client.calls.load(Ordering::Relaxed),
                client.not_modified.load(Ordering::Relaxed),
            );
            let wait = match snapshot(&client, &cfg).await {
                Ok(mut s) => {
                    // REQ-RUNNERS-002: the config inventory fills what GitHub
                    // would not list
                    s.runners = crate::hub::pools::fill(
                        std::mem::take(&mut s.runners),
                        !s.runners_inferred,
                        &runners,
                        &cfg.self_hosted_labels,
                    );
                    tracing::debug!(
                        requests = client.calls.load(Ordering::Relaxed) - calls,
                        not_modified = client.not_modified.load(Ordering::Relaxed) - cached,
                        remaining = ?client.remaining(),
                        "GitHub scan"
                    );
                    hub.ingest_snapshot(s);
                    let now = crate::hub::now();
                    if cfg.ledger_days > 0 && now - last_ledger >= cfg.ledger_interval_s as i64 {
                        last_ledger = now;
                        ledger_pass(&lclient, &cfg, &hub, &mut lstate, &lpath, now).await;
                    }
                    Duration::from_secs(cfg.interval_s)
                }
                Err(e @ GhError::RateLimited { until }) => {
                    // shown as its own finding, not only as stale data; the
                    // client sends nothing until the reset, so waking at the
                    // normal interval to say so again costs no requests
                    tracing::warn!(until, "GitHub rate limit: pausing the scan");
                    hub.ingest_snapshot(Snapshot {
                        host: format!("github:{}", cfg.org),
                        taken_at: crate::hub::now(),
                        errors: vec![e.to_string()],
                        ..Default::default()
                    });
                    Duration::from_secs(
                        ((until - crate::hub::now()).max(1) as u64).min(cfg.interval_s.max(1)),
                    )
                }
                Err(e) => {
                    tracing::warn!(error = %e, "GitHub scan failed");
                    hub.ingest_snapshot(Snapshot {
                        host: format!("github:{}", cfg.org),
                        taken_at: crate::hub::now(),
                        errors: vec![e.to_string()],
                        ..Default::default()
                    });
                    Duration::from_secs(cfg.interval_s)
                }
            };
            tokio::time::sleep(wait).await;
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_page_comes_from_the_link_header() {
        let l = r#"<https://api.t.ghe.com/orgs/o/repos?page=2>; rel="next", <https://api.t.ghe.com/orgs/o/repos?page=5>; rel="last""#;
        assert_eq!(
            next_link(Some(l)).as_deref(),
            Some("https://api.t.ghe.com/orgs/o/repos?page=2")
        );
        assert_eq!(next_link(Some(r#"<x>; rel="prev""#)), None);
        assert_eq!(next_link(None), None);
    }
}

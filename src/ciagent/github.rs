//! Minimal read-only GitHub client. Auth reuses a GitHub App token helper
//! (`--token-cmd`, or `[ci_agent] token_cmd`); this tool only ever issues GETs.
use crate::ciagent::collect::run;
use crate::ciagent::model::{
    HostedGroup, HostedJob, PoolQueue, Queue, QueuedEntry, QueuedRun, WaitStats,
};
use crate::ciagent::parse;
use reqwest::blocking::{Client, Response};
use reqwest::redirect::Policy;
use serde_json::Value;
use std::collections::BTreeMap;

/// Hosted jobs carried per snapshot (the wire splits them into chunk packets).
pub const HOSTED_CAP: usize = 64;

const USER_AGENT: &str = "warte-ci-agent";

pub struct Gh {
    /// Re-minted on a 401: the helper hands out a cached token while it has more
    /// than 5 minutes left, and a long ledger pass can outlive that.
    token: std::cell::RefCell<String>,
    token_cmd: String,
    token_args: Vec<String>,
    agent: Client,
    /// The logs endpoint answers with a redirect that is read, not followed.
    no_follow: Client,
    /// `X-RateLimit-Remaining` of the last response. The app token is shared with
    /// the downtime dispatcher, whose correctness matters more than our ledger.
    pub rate_remaining: std::cell::Cell<i64>,
    /// GET calls made through this client.
    pub calls: std::cell::Cell<u32>,
}

pub struct GhRunner {
    pub name: String,
    pub status: String,
    pub busy: bool,
    pub labels: Vec<String>,
}

/// A failed call: an HTTP status of 400 or more, or a transport error. The
/// text names the URL and status as `<url>: status code <n>`.
enum CallErr {
    Status(u16, String),
    Other(String),
}

impl std::fmt::Display for CallErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallErr::Status(_, s) | CallErr::Other(s) => f.write_str(s),
        }
    }
}

/// Send a request; a status of 400 or more is an error, as before the import.
fn send(rb: reqwest::blocking::RequestBuilder) -> Result<Response, CallErr> {
    let r = rb.send().map_err(|e| CallErr::Other(e.to_string()))?;
    let code = r.status().as_u16();
    if code >= 400 {
        return Err(CallErr::Status(
            code,
            format!("{}: status code {code}", r.url()),
        ));
    }
    Ok(r)
}

fn rate_header(r: &Response) -> Option<i64> {
    r.headers()
        .get("X-RateLimit-Remaining")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

fn client(redirects: Policy) -> Result<Client, String> {
    // TLS roots come from the operating system's store (REQ-TLS-001)
    Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(redirects)
        .build()
        .map_err(|e| format!("http client: {e}"))
}

impl Gh {
    pub fn new(token_cmd: &str, token_args: &[String]) -> Result<Gh, String> {
        let args: Vec<&str> = token_args.iter().map(String::as_str).collect();
        let token = run(token_cmd, &args)?.trim().to_string();
        if token.is_empty() {
            return Err("token helper returned nothing".into());
        }
        Ok(Gh {
            token: std::cell::RefCell::new(token),
            token_cmd: token_cmd.to_string(),
            token_args: token_args.to_vec(),
            agent: client(Policy::limited(5))?,
            no_follow: client(Policy::none())?,
            rate_remaining: std::cell::Cell::new(i64::MAX),
            calls: std::cell::Cell::new(0),
        })
    }

    fn api(&self, c: &Client, path: &str) -> reqwest::blocking::RequestBuilder {
        c.get(format!("https://api.github.com{path}"))
            .header("Authorization", format!("Bearer {}", self.token.borrow()))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", USER_AGENT)
    }

    pub(crate) fn get(&self, path: &str) -> Result<Value, String> {
        let call = || {
            self.calls.set(self.calls.get() + 1);
            send(self.api(&self.agent, path))
        };
        let resp = match call() {
            Err(CallErr::Status(401, _)) => {
                // token expired mid-pass: mint a fresh one and retry once
                let args: Vec<&str> = self.token_args.iter().map(String::as_str).collect();
                let fresh = run(&self.token_cmd, &args)?.trim().to_string();
                *self.token.borrow_mut() = fresh;
                call()
            }
            Err(CallErr::Status(c, _)) if c >= 500 => {
                // GitHub 5xx are usually transient: one retry after a pause
                std::thread::sleep(std::time::Duration::from_secs(2));
                call()
            }
            r => r,
        }
        .map_err(|e| format!("GET {path}: {e}"))?;
        if let Some(n) = rate_header(&resp) {
            self.rate_remaining.set(n);
        }
        resp.json::<Value>().map_err(|e| format!("GET {path}: {e}"))
    }

    /// The last `max` bytes of a job's log, read to find its error signature
    /// and then dropped. GitHub answers the logs endpoint with a redirect to a
    /// short-lived blob URL; that URL is fetched without our token and with a
    /// Range for the tail. A server that ignores Range still gets cut to `max`.
    pub fn log_tail(&self, org: &str, repo: &str, job: u64, max: usize) -> Result<String, String> {
        use std::io::Read;
        let path = format!("/repos/{org}/{repo}/actions/jobs/{job}/logs");
        self.calls.set(self.calls.get() + 1);
        let resp =
            send(self.api(&self.no_follow, &path)).map_err(|e| format!("GET {path}: {e}"))?;
        if let Some(n) = rate_header(&resp) {
            self.rate_remaining.set(n);
        }
        let loc = match resp.status().as_u16() {
            300..=399 => resp
                .headers()
                .get("Location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| format!("GET {path}: redirect without Location"))?
                .to_string(),
            s => return Err(format!("GET {path}: status {s}, expected a redirect")),
        };
        let blob = send(
            self.agent
                .get(&loc)
                .header("Range", format!("bytes=-{max}"))
                .header("User-Agent", USER_AGENT),
        )
        .map_err(|e| format!("GET log blob: {e}"))?;
        // a full log (Range ignored) is read up to 64 MiB, keeping only its tail
        let mut buf = Vec::new();
        blob.take(64 << 20)
            .read_to_end(&mut buf)
            .map_err(|e| format!("read log blob: {e}"))?;
        let tail = &buf[buf.len().saturating_sub(max)..];
        Ok(String::from_utf8_lossy(tail).into_owned())
    }

    pub fn org_runners(&self, org: &str) -> Result<Vec<GhRunner>, String> {
        let mut out = vec![];
        for page in 1..=5 {
            let v = self.get(&format!(
                "/orgs/{org}/actions/runners?per_page=100&page={page}"
            ))?;
            let rs = v["runners"].as_array().cloned().unwrap_or_default();
            for r in &rs {
                out.push(GhRunner {
                    name: r["name"].as_str().unwrap_or("").to_string(),
                    status: r["status"].as_str().unwrap_or("").to_string(),
                    busy: r["busy"].as_bool().unwrap_or(false),
                    labels: r["labels"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|l| l["name"].as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                });
            }
            if rs.len() < 100 {
                break;
            }
        }
        Ok(out)
    }

    /// Non-archived repos pushed within `active_days`.
    pub fn active_repos(
        &self,
        org: &str,
        active_days: i64,
        now: i64,
    ) -> Result<Vec<String>, String> {
        let mut repos = vec![];
        for page in 1..=5 {
            let v = self.get(&format!(
                "/orgs/{org}/repos?per_page=100&type=all&sort=pushed&page={page}"
            ))?;
            let rs = v.as_array().cloned().unwrap_or_default();
            for r in &rs {
                let pushed = r["pushed_at"]
                    .as_str()
                    .and_then(parse::rfc3339_utc)
                    .unwrap_or(0);
                if !r["archived"].as_bool().unwrap_or(false) && now - pushed <= active_days * 86400
                {
                    repos.push(r["name"].as_str().unwrap_or("").to_string());
                }
            }
            if rs.len() < 100 {
                break;
            }
        }
        Ok(repos)
    }

    /// Queue scan over repos pushed within `active_days`, counting JOBS (not runs)
    /// per pool. Bounded by `max_runs` job listings so it stays well inside the
    /// app's rate limit; `truncated` says when the result is a lower bound.
    pub fn queue(
        &self,
        org: &str,
        active_days: i64,
        max_runs: usize,
        now: i64,
    ) -> Result<Queue, String> {
        let repos = self.active_repos(org, active_days, now)?;
        let mut ages: BTreeMap<String, Vec<i64>> = BTreeMap::new();
        let mut top: Vec<QueuedRun> = vec![];
        let mut hosted: Vec<HostedJob> = vec![];
        let mut queued: Vec<QueuedEntry> = vec![];
        let mut waited: BTreeMap<String, Vec<i64>> = BTreeMap::new();
        // Pass 1 lists QUEUED runs of every repo before any in_progress run, so the
        // job budget is spent where "is PR X queued?" is answered. In_progress runs
        // (which can still hold queued jobs, and give us realized waits) come after.
        let mut runs: Vec<(String, Value, bool)> = vec![];
        for status in ["queued", "in_progress"] {
            for repo in &repos {
                let v = self.get(&format!(
                    "/repos/{org}/{repo}/actions/runs?status={status}&per_page=30"
                ))?;
                for run in v["workflow_runs"].as_array().cloned().unwrap_or_default() {
                    runs.push((repo.clone(), run, status == "queued"));
                }
            }
        }
        let n_queued_runs = runs.iter().filter(|r| r.2).count();
        let truncated = runs.len() > max_runs;
        let queued_complete = n_queued_runs <= max_runs;
        for (repo, run, _) in runs.into_iter().take(max_runs) {
            let id = run["id"].as_i64().unwrap_or(0);
            let jobs = self.get(&format!(
                "/repos/{org}/{repo}/actions/runs/{id}/jobs?per_page=100"
            ))?;
            let mut per_pool: BTreeMap<String, (u32, i64)> = BTreeMap::new();
            let mut n = 0u32;
            let mut oldest = 0i64;
            for j in jobs["jobs"].as_array().cloned().unwrap_or_default() {
                let status = j["status"].as_str().unwrap_or("");
                let labels: Vec<String> = j["labels"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|l| l.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let pool = parse::pool_of(&labels);
                let created = j["created_at"].as_str().and_then(parse::rfc3339_utc);
                let started = j["started_at"].as_str().and_then(parse::rfc3339_utc);
                if status == "in_progress" {
                    // realized wait of a job that started within the last 3 hours
                    if let (Some(c), Some(s)) = (created, started) {
                        if now - s <= 3 * 3600 && s >= c {
                            waited.entry(pool.clone()).or_default().push(s - c);
                        }
                    }
                }
                if (status == "queued" || status == "in_progress") && parse::is_hosted_pool(&pool) {
                    let ts = if status == "in_progress" {
                        "started_at"
                    } else {
                        "created_at"
                    };
                    hosted.push(HostedJob {
                        repo: format!("{org}/{repo}"),
                        workflow: run["name"].as_str().unwrap_or("").to_string(),
                        job: j["name"].as_str().unwrap_or("").to_string(),
                        status: status.to_string(),
                        runner: j["runner_name"].as_str().unwrap_or("").to_string(),
                        labels: labels.join(","),
                        since: j[ts].as_str().and_then(parse::rfc3339_utc).unwrap_or(now),
                        run_id: id as u64,
                        event: run["event"].as_str().unwrap_or("").to_string(),
                        branch: run["head_branch"].as_str().unwrap_or("").to_string(),
                        pr: run["pull_requests"][0]["number"].as_u64().map(|n| n as u32),
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
                    run_id: id as u64,
                    workflow: run["name"].as_str().unwrap_or("").to_string(),
                    event: run["event"].as_str().unwrap_or("").to_string(),
                    branch: run["head_branch"].as_str().unwrap_or("").to_string(),
                    pr: run["pull_requests"][0]["number"].as_u64().map(|n| n as u32),
                    pool,
                    jobs,
                    since,
                });
            }
            if n > 0 {
                top.push(QueuedRun {
                    repo: repo.clone(),
                    workflow: run["name"].as_str().unwrap_or("").to_string(),
                    event: run["event"].as_str().unwrap_or("").to_string(),
                    jobs_queued: n,
                    oldest_s: oldest,
                });
            }
        }
        let waits = waited
            .into_iter()
            .filter_map(|(k, mut v)| crate::ciagent::eta::wait_stats(&mut v).map(|w| (k, w)))
            .collect::<BTreeMap<String, WaitStats>>();
        let runs_queued = top.len() as u32;
        let jobs_queued = ages.values().map(|v| v.len() as u32).sum();
        let pools = ages
            .into_iter()
            .map(|(k, mut v)| {
                let (o, m) = parse::age_stats(&mut v);
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
        // running first, then longest-waiting/running; the cap keeps the wire bounded
        hosted.sort_by(|a, b| {
            (a.status != "in_progress")
                .cmp(&(b.status != "in_progress"))
                .then(a.since.cmp(&b.since))
        });
        let hosted_total = hosted.len() as u32;
        let hosted_running = hosted.iter().filter(|j| j.status == "in_progress").count() as u32;
        let hosted_queued = hosted_total - hosted_running;
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
        Ok(Queue {
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
            hosted_queued,
            hosted_groups,
            queued,
            queued_complete,
            waits,
        })
    }
}

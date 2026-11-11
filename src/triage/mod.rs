//! Issue triage queue (REQ-TRIAGE-001..002): a fast first reply to issues
//! people open, without warte ever writing to GitHub.
//!
//! warte polls the organisation's public event feed every `interval_s`
//! (conditional requests, no token, so only public repositories are seen)
//! and queues each issue a person opens or reopens. A scheduled run outside
//! warte reads the queue, replies and labels as a GitHub App bot, and marks
//! the issue done here. warte only detects and hands out, at most `per_hour`
//! issues an hour; the rest wait and are counted.
//!
//! Issue text is untrusted: the queue carries the title only, cut short, and
//! the run reads the rest from GitHub itself, as data.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

/// Issues remembered as seen, so a re-listed event is not queued again.
const SEEN_MAX: usize = 5000;
/// Done items kept for the record.
const DONE_MAX: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TriageConfig {
    pub org: String,
    #[serde(default = "default_api_url")]
    pub api_url: String,
    #[serde(default = "default_interval_s")]
    pub interval_s: u64,
    /// Issues handed out per hour, at most.
    #[serde(default = "default_per_hour")]
    pub per_hour: u32,
    /// An issue whose body contains one of these was written by an agent
    /// (they file under the owner's account): not queued.
    #[serde(default = "default_markers")]
    pub agent_markers: Vec<String>,
    /// Logins never queued (besides every `[bot]` and non-User account).
    #[serde(default)]
    pub skip_logins: Vec<String>,
    #[serde(default)]
    pub data_dir: Option<String>,
}

fn default_api_url() -> String {
    "https://api.github.com".into()
}
fn default_interval_s() -> u64 {
    600
}
fn default_per_hour() -> u32 {
    10
}
fn default_markers() -> Vec<String> {
    vec![
        "Generated with [Claude Code]".into(),
        "Claude-Session:".into(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Item {
    pub repo: String,
    pub number: u64,
    /// Untrusted: shown escaped, never acted on.
    pub title: String,
    pub author: String,
    pub url: String,
    /// `opened` or `reopened`.
    pub action: String,
    pub queued_at: i64,
    /// Set when the run marked it done.
    pub done_at: Option<i64>,
    /// What the run did: `replied`, `asked`, `skipped`.
    pub outcome: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    etag: Option<String>,
    seen: VecDeque<String>,
    queue: Vec<Item>,
    done: VecDeque<Item>,
    /// When the last poll answered, and how.
    polled_at: i64,
    poll_error: Option<String>,
}

pub struct Triage {
    cfg: TriageConfig,
    path: PathBuf,
    st: Mutex<State>,
}

fn key(repo: &str, n: u64) -> String {
    format!("{repo}#{n}")
}

fn plain(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// A repository or login name as GitHub allows them.
fn name_ok(s: &str) -> bool {
    (1..=100).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// The queue entry for one event, or why not. Pure, for the tests.
pub fn from_event(e: &Value, cfg: &TriageConfig, now: i64) -> Option<Item> {
    if e["type"] != "IssuesEvent" {
        return None;
    }
    let action = e["payload"]["action"].as_str()?;
    if action != "opened" && action != "reopened" {
        return None;
    }
    let issue = &e["payload"]["issue"];
    if !issue["pull_request"].is_null() {
        return None;
    }
    // people only: the actor who opened it, and the issue's author
    let actor = e["actor"]["login"].as_str()?;
    let author = issue["user"]["login"].as_str()?;
    for who in [actor, author] {
        if who.ends_with("[bot]") || cfg.skip_logins.iter().any(|l| l.eq_ignore_ascii_case(who)) {
            return None;
        }
    }
    if issue["user"]["type"].as_str().unwrap_or("User") != "User" {
        return None;
    }
    let body = issue["body"].as_str().unwrap_or_default();
    if cfg
        .agent_markers
        .iter()
        .any(|m| !m.is_empty() && body.contains(m.as_str()))
    {
        return None;
    }
    let full = e["repo"]["name"].as_str()?;
    let (org, repo) = full.split_once('/')?;
    if !org.eq_ignore_ascii_case(&cfg.org) || !name_ok(repo) || !name_ok(author) {
        return None;
    }
    let number = issue["number"].as_u64()?;
    Some(Item {
        repo: repo.to_string(),
        number,
        title: plain(issue["title"].as_str().unwrap_or_default(), 120),
        author: author.to_string(),
        url: format!("https://github.com/{org}/{repo}/issues/{number}"),
        action: action.to_string(),
        queued_at: now,
        done_at: None,
        outcome: None,
    })
}

/// What the run is handed: the oldest queued issues, as many as the hour
/// still allows, and how many wait beyond that.
#[derive(Debug, Serialize)]
pub struct Offer {
    pub ready: Vec<Item>,
    pub waiting: usize,
    pub budget_left: u32,
    pub per_hour: u32,
    pub polled_at: i64,
    pub poll_error: Option<String>,
    pub recent: Vec<Item>,
}

impl Triage {
    pub fn open(data_dir: &str, cfg: TriageConfig) -> Result<Triage, String> {
        std::fs::create_dir_all(data_dir).map_err(|e| format!("{data_dir}: {e}"))?;
        let path = PathBuf::from(data_dir).join("triage.json");
        let st = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Ok(Triage {
            cfg,
            path,
            st: Mutex::new(st),
        })
    }

    fn save(&self, st: &State) {
        let tmp = self.path.with_extension("json.tmp");
        if let Ok(b) = serde_json::to_vec(st) {
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queue what the events hold that was not seen before. Returns how many
    /// were queued.
    pub fn ingest(&self, events: &[Value], now: i64) -> usize {
        let mut st = self.lock();
        let mut n = 0;
        // the feed is newest first: queue oldest first
        for e in events.iter().rev() {
            let Some(item) = from_event(e, &self.cfg, now) else {
                continue;
            };
            // a reopened issue is triaged again: its key carries the action
            let k = format!(
                "{}:{}",
                key(&item.repo, item.number),
                e["id"].as_str().unwrap_or("")
            );
            if st.seen.contains(&k) {
                continue;
            }
            st.seen.push_back(k);
            if st
                .queue
                .iter()
                .any(|q| q.repo == item.repo && q.number == item.number)
            {
                continue;
            }
            st.queue.push(item);
            n += 1;
        }
        while st.seen.len() > SEEN_MAX {
            st.seen.pop_front();
        }
        self.save(&st);
        n
    }

    pub fn offer(&self, now: i64) -> Offer {
        let st = self.lock();
        let used = st
            .done
            .iter()
            .filter(|d| d.done_at.is_some_and(|t| now - t < 3600))
            .count() as u32;
        let left = self.cfg.per_hour.saturating_sub(used);
        let ready: Vec<Item> = st.queue.iter().take(left as usize).cloned().collect();
        Offer {
            waiting: st.queue.len() - ready.len(),
            ready,
            budget_left: left,
            per_hour: self.cfg.per_hour,
            polled_at: st.polled_at,
            poll_error: st.poll_error.clone(),
            recent: st.done.iter().rev().take(20).cloned().collect(),
        }
    }

    /// The run is done with an issue. `Err` when it is not queued.
    pub fn done(&self, repo: &str, number: u64, outcome: &str, now: i64) -> Result<(), String> {
        if !["replied", "asked", "skipped"].contains(&outcome) {
            return Err("outcome is replied, asked or skipped".into());
        }
        let mut st = self.lock();
        let i = st
            .queue
            .iter()
            .position(|q| q.repo == repo && q.number == number)
            .ok_or("that issue is not queued")?;
        let mut it = st.queue.remove(i);
        it.done_at = Some(now);
        it.outcome = Some(outcome.to_string());
        st.done.push_back(it);
        while st.done.len() > DONE_MAX {
            st.done.pop_front();
        }
        self.save(&st);
        Ok(())
    }

    fn polled(&self, now: i64, etag: Option<String>, err: Option<String>) {
        let mut st = self.lock();
        st.polled_at = now;
        st.poll_error = err;
        if etag.is_some() {
            st.etag = etag;
        }
        self.save(&st);
    }

    fn etag(&self) -> Option<String> {
        self.lock().etag.clone()
    }

    /// Repository#number of everything queued, for tests and the record.
    pub fn queued(&self) -> BTreeSet<String> {
        self.lock()
            .queue
            .iter()
            .map(|q| key(&q.repo, q.number))
            .collect()
    }

    /// One poll of the org's public events, sent with the last ETag.
    pub async fn poll(&self, http: &reqwest::Client, now: i64) -> Result<usize, String> {
        let url = format!(
            "{}/orgs/{}/events?per_page=100",
            self.cfg.api_url.trim_end_matches('/'),
            self.cfg.org
        );
        let mut req = http
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(e) = self.etag() {
            req = req.header("If-None-Match", e);
        }
        let r = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let why = format!("GitHub not reached: {}", e.without_url());
                self.polled(now, None, Some(why.clone()));
                return Err(why);
            }
        };
        let status = r.status().as_u16();
        let etag = r
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        if status == 304 {
            self.polled(now, None, None);
            return Ok(0);
        }
        if !(200..300).contains(&status) {
            let why = format!("GitHub answered {status}");
            self.polled(now, None, Some(why.clone()));
            return Err(why);
        }
        let events: Vec<Value> = r.json().await.unwrap_or_default();
        let n = self.ingest(&events, now);
        self.polled(now, etag, None);
        Ok(n)
    }
}

/// Poll every `interval_s`.
pub fn spawn(t: std::sync::Arc<Triage>) {
    tokio::spawn(async move {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("warte/", env!("CARGO_PKG_VERSION"), " triage"))
            .build()
            .unwrap_or_default();
        loop {
            if let Err(e) = t.poll(&http, crate::hub::now()).await {
                tracing::warn!(error = %e, "triage poll failed");
            }
            tokio::time::sleep(Duration::from_secs(t.cfg.interval_s.max(60))).await;
        }
    });
}

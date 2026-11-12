//! Shared incidents (REQ-CI-005, DD-CI-002): failed CI jobs in two or more
//! repositories whose failing step carries the same error signature, within
//! an hour of each other, shown as one incident with one cause.
//!
//! A signature is built from fixed patterns only (a resolver failure, a
//! connection timeout or refusal, a server error, each for one host), never
//! from raw log text: no path, secret or log line reaches the read API. A
//! host is shown by name only when it is a well-known public dependency or a
//! name the dependency canaries watch; any other host groups the same way
//! but is shown as "a host", so no internal name is published.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::dashboard::fleet::Detected;
use crate::hub::model::JobRecord;

/// Failures this close to each other join one incident.
pub const WINDOW_S: i64 = 3600;
/// An incident stays on the overview this long after its last failure.
pub const KEEP_S: i64 = 86_400;
/// An incident spans at least this many repositories.
pub const MIN_REPOS: usize = 2;
/// How often the store recomputes from the ledger.
pub const EVERY_S: u64 = 60;
/// How much of a job log's end is read for its signature.
pub const LOG_TAIL: usize = 256 * 1024;
/// At most this many job logs per GitHub ledger pass.
pub const MAX_LOGS_PER_PASS: usize = 30;

/// Public dependencies shown by name (a suffix match: `azure.archive.ubuntu.com`
/// is `archive.ubuntu.com`'s).
pub const PUBLIC_HOSTS: &[&str] = &[
    "archive.ubuntu.com",
    "security.ubuntu.com",
    "ports.ubuntu.com",
    "ppa.launchpadcontent.net",
    "deb.debian.org",
    "registry.npmjs.org",
    "crates.io",
    "pypi.org",
    "files.pythonhosted.org",
    "proxy.golang.org",
    "github.com",
    "githubusercontent.com",
    "ghcr.io",
    "docker.io",
    "quay.io",
    "dl-cdn.alpinelinux.org",
    "packages.microsoft.com",
    "nodejs.org",
    "static.rust-lang.org",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Resolver,
    Timeout,
    Refused,
    ServerError,
}

const KINDS: [Kind; 4] = [
    Kind::Resolver,
    Kind::Timeout,
    Kind::Refused,
    Kind::ServerError,
];

impl Kind {
    fn code(self) -> &'static str {
        match self {
            Kind::Resolver => "resolver",
            Kind::Timeout => "timeout",
            Kind::Refused => "refused",
            Kind::ServerError => "server_error",
        }
    }
}

/// One normalised error: what went wrong, and with which host.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Signature {
    pub kind: Kind,
    pub host: String,
}

impl Signature {
    /// The form a job record carries: `kind:host`.
    pub fn encode(&self) -> String {
        format!("{}:{}", self.kind.code(), self.host)
    }

    /// A record's signature, or `None` for anything not built from the fixed
    /// patterns (an agent can send any string; only these get through).
    pub fn parse(s: &str) -> Option<Signature> {
        let (k, host) = s.split_once(':')?;
        let kind = KINDS.into_iter().find(|x| x.code() == k)?;
        let host = host_name(host)?;
        Some(Signature { kind, host })
    }

    /// Plain words, naming the host only when `named` allows it.
    pub fn words(&self, named: bool) -> String {
        let h = if named { self.host.as_str() } else { "a host" };
        match self.kind {
            Kind::Resolver => format!("{h} did not resolve"),
            Kind::Timeout => format!("connections to {h} timed out"),
            Kind::Refused => format!("{h} refused connections"),
            Kind::ServerError => format!("{h} answered with server errors"),
        }
    }
}

/// A DNS name as a signature may carry it: lowercase, with a dot, at most
/// 253 characters. An address is not one: IPv4 ends in an all-digit label,
/// IPv6 holds colons.
fn host_name(s: &str) -> Option<String> {
    let s = s.trim_end_matches('.').to_ascii_lowercase();
    let ok = s.len() <= 253
        && s.contains('.')
        && s.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && !l.starts_with('-')
                && !l.ends_with('-')
        })
        // a top-level label is never all digits
        && !s.rsplit('.').next().unwrap_or("").chars().all(|c| c.is_ascii_digit());
    ok.then_some(s)
}

/// The host right after `marker` in `line`, up to the first character a
/// host cannot hold.
fn host_after(line: &str, marker: &str) -> Option<String> {
    let i = line.find(marker)? + marker.len();
    let rest = line[i..].trim_start_matches(['\'', '"', '`', ' ']);
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '.'))
        .unwrap_or(rest.len());
    host_name(&rest[..end])
}

/// The host of the first `http://` or `https://` URL in `line`.
fn url_host(line: &str) -> Option<String> {
    let i = line.find("://")?;
    if !line[..i].ends_with("http") && !line[..i].ends_with("https") {
        return None;
    }
    host_after(&line[i..], "://")
}

/// The signature one log line carries, if it matches a fixed pattern.
pub fn of_line(line: &str) -> Option<Signature> {
    let l = line.to_ascii_lowercase();
    let sig = |kind, host: Option<String>| host.map(|host| Signature { kind, host });
    // resolver failures, as apt, curl/git/cargo, npm and Go print them
    for m in [
        "temporary failure resolving ",
        "could not resolve host: ",
        "could not resolve host ",
        "getaddrinfo enotfound ",
        "getaddrinfo eai_again ",
    ] {
        if let Some(s) = sig(Kind::Resolver, host_after(&l, m)) {
            return Some(s);
        }
    }
    if l.contains("no such host") || l.contains("name or service not known") {
        if let Some(s) = sig(
            Kind::Resolver,
            host_after(&l, "lookup ").or_else(|| url_host(&l)),
        ) {
            return Some(s);
        }
    }
    let target = || {
        host_after(&l, "failed to connect to ")
            .or_else(|| host_after(&l, "could not connect to "))
            .or_else(|| host_after(&l, "connect to "))
            .or_else(|| url_host(&l))
    };
    if l.contains("timed out") || l.contains("etimedout") {
        if let Some(s) = sig(Kind::Timeout, target()) {
            return Some(s);
        }
    }
    if l.contains("connection refused") || l.contains("econnrefused") {
        if let Some(s) = sig(Kind::Refused, target()) {
            return Some(s);
        }
    }
    let server_error = [
        "500 internal server error",
        "502 bad gateway",
        "503 service unavailable",
        "504 gateway timeout",
        "returned error: 500",
        "returned error: 502",
        "returned error: 503",
        "returned error: 504",
    ];
    if server_error.iter().any(|p| l.contains(p)) {
        return sig(Kind::ServerError, url_host(&l));
    }
    None
}

/// A failed job's signature from the end of its log: the last line that
/// matches, being the one closest to the failure. Nothing of the log is kept.
pub fn of_log(log: &str) -> Option<Signature> {
    log.lines().rev().find_map(of_line)
}

/// Whether `host` may be shown by name: a public dependency or a name in
/// `watched` (the dependency canaries' names).
pub fn named(host: &str, watched: &[String]) -> bool {
    let under = |base: &str| host == base || host.ends_with(&format!(".{base}"));
    PUBLIC_HOSTS.iter().any(|b| under(b)) || watched.iter().any(|w| host == w)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Incident {
    pub kind: Kind,
    /// The host, when it may be shown; `None` otherwise.
    pub host: Option<String>,
    /// The signature in words.
    pub what: String,
    pub repos: Vec<String>,
    pub jobs: usize,
    pub first: i64,
    pub last: i64,
    /// A failure within [`WINDOW_S`] of now.
    pub ongoing: bool,
    #[serde(skip)]
    pub sig: Signature,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Report {
    pub generated_at: i64,
    pub window_s: i64,
    pub min_repos: usize,
    pub incidents: Vec<Incident>,
}

fn failed(r: &JobRecord) -> bool {
    r.r == "failure" || r.r == "timed_out"
}

/// Failed jobs grouped by signature, split wherever two failures are more
/// than [`WINDOW_S`] apart; groups spanning [`MIN_REPOS`] repositories whose
/// last failure is within [`KEEP_S`] of `now`. Ongoing first, then newest.
pub fn group(recs: &[JobRecord], now: i64, watched: &[String]) -> Report {
    let mut fails: Vec<(Signature, i64, &str)> = recs
        .iter()
        .filter(|r| failed(r))
        .filter_map(|r| {
            let at = r.e.unwrap_or(r.c);
            let sig = Signature::parse(r.sig.as_deref()?)?;
            (at > now - KEEP_S - WINDOW_S && at <= now + 300).then_some((sig, at, r.repo.as_str()))
        })
        .collect();
    fails.sort();
    let mut out = Vec::new();
    let mut i = 0;
    while i < fails.len() {
        let mut j = i + 1;
        while j < fails.len() && fails[j].0 == fails[i].0 && fails[j].1 - fails[j - 1].1 <= WINDOW_S
        {
            j += 1;
        }
        let run = &fails[i..j];
        let repos: BTreeSet<&str> = run.iter().map(|f| f.2).collect();
        let (first, last) = (run[0].1, run[run.len() - 1].1);
        if repos.len() >= MIN_REPOS && last >= now - KEEP_S {
            let sig = run[0].0.clone();
            let show = named(&sig.host, watched);
            out.push(Incident {
                kind: sig.kind,
                host: show.then(|| sig.host.clone()),
                what: sig.words(show),
                repos: repos.into_iter().map(String::from).collect(),
                jobs: run.len(),
                first,
                last,
                ongoing: now - last <= WINDOW_S,
                sig,
            });
        }
        i = j;
    }
    out.sort_by(|a, b| b.ongoing.cmp(&a.ongoing).then(b.last.cmp(&a.last)));
    Report {
        generated_at: now,
        window_s: WINDOW_S,
        min_repos: MIN_REPOS,
        incidents: out,
    }
}

fn hhmm(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_default()
}

fn repos_words(r: &[String]) -> String {
    match r {
        [a, b] => format!("{a} and {b}"),
        _ => r.join(", "),
    }
}

/// One line per incident: the cause, where, and since when.
pub fn text(i: &Incident) -> String {
    let span = if i.ongoing {
        format!("failing since {} UTC", hhmm(i.first))
    } else {
        format!("from {} to {} UTC", hhmm(i.first), hhmm(i.last))
    };
    format!(
        "{}: {} failed jobs in {} repositories ({}), {span}. One shared cause, not {} separate failures",
        i.what,
        i.jobs,
        i.repos.len(),
        repos_words(&i.repos),
        i.repos.len()
    )
}

/// Ongoing incidents notify; past ones stay on the panel only.
pub fn detected(r: &Report) -> Vec<Detected> {
    r.incidents
        .iter()
        .filter(|i| i.ongoing)
        .map(|i| Detected {
            key: format!(
                "shared-incident:{}:{}",
                i.kind.code(),
                i.host.as_deref().unwrap_or("unnamed")
            ),
            sev: 2,
            text: text(i),
        })
        .collect()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The "Shared incidents" panel; empty when there is none to show.
pub fn panel(r: &Report) -> String {
    if r.incidents.is_empty() {
        return String::new();
    }
    let li: String = r
        .incidents
        .iter()
        .map(|i| {
            format!(
                r#"<li class="{}">{}</li>"#,
                if i.ongoing { "bad" } else { "dim" },
                esc(&text(i))
            )
        })
        .collect();
    format!(
        r#"<section class="panel shared"><h3>Shared incidents</h3><p class="note">Failed jobs in two or more repositories with the same error within an hour of each other, over the last day.</p><ul class="trendlist">{li}</ul></section>"#
    )
}

/// The latest report, recomputed every [`EVERY_S`] from the hub's ledger.
pub struct Incidents {
    watched: Vec<String>,
    latest: Mutex<Option<Arc<Report>>>,
}

impl Incidents {
    pub fn new(watched: Vec<String>) -> Incidents {
        Incidents {
            watched,
            latest: Mutex::new(None),
        }
    }

    pub fn latest(&self) -> Option<Arc<Report>> {
        self.latest.lock().ok().and_then(|g| g.clone())
    }

    /// Recompute from the ledger now.
    pub async fn refresh(&self, hub: &crate::hub::Hub, now: i64) -> Arc<Report> {
        let hub = hub.clone();
        let from = crate::hub::report::day_of(now - KEEP_S - WINDOW_S);
        let until = crate::hub::report::day_of(now + 86_400);
        let recs = tokio::task::spawn_blocking(move || hub.ledger_days(&from, &until))
            .await
            .unwrap_or_default();
        let r = Arc::new(group(&recs, now, &self.watched));
        if let Ok(mut g) = self.latest.lock() {
            *g = Some(r.clone());
        }
        r
    }
}

pub fn spawn(store: Arc<Incidents>, hub: crate::hub::Hub) {
    tokio::spawn(async move {
        loop {
            store.refresh(&hub, crate::hub::now()).await;
            tokio::time::sleep(std::time::Duration::from_secs(EVERY_S)).await;
        }
    });
}

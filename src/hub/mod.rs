//! Ground station absorbed from the original hub (DD-SMITHY-001, REQ-SMITHY-001..003).
//!
//! The fleet agent on the CI host collects a snapshot and dials OUT to us with
//! CCSDS space packets over UDP (relay-ccsds), so the CI host exposes nothing.
//! We accept telemetry only from allow-listed source addresses, keep the latest
//! snapshot plus per-minute history in `hub-state.json` and every finished job
//! in the per-day JSONL ledger, and serve the hub's read-only API unchanged
//! (`web/openapi.json` is the contract; REQ-SMITHY-002).
//!
//! `model`, `telemetry`, `eta`, `belt`, `report` and `parse` come from
//! the original hub, with only
//! their `crate::` paths rewritten. They are kept whole, agent-side items
//! included, so the wire format and on-disk formats stay byte-compatible with
//! the agent and with an existing `/var/lib/warte-hub`. Only the HTTP layer is
//! new: the original hub served it with tiny_http, here it is an axum router.
#![allow(dead_code)]

pub mod belt;
pub mod eta;
pub mod model;
pub mod parse;
pub mod pools;
pub mod report;
pub mod telemetry;

use axum::extract::{RawQuery, Request, State};
use axum::http::{header, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use model::{DayManifest, JobRecord, Point, Snapshot};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use telemetry::{apid_name, Assembler};

/// One point per completed frame (~1/min) for 7 days.
const HISTORY_MAX: usize = 7 * 24 * 60;
pub const INDEX: &str = include_str!("web/index.html");
pub const OPENAPI: &str = include_str!("web/openapi.json");

/// Lock, or end the process if a panic poisoned the lock. the original hub served
/// HTTP on one thread, so any panic there ended the process and systemd
/// restarted it with state from disk. Under axum a panic only ends one task;
/// carrying on with a poisoned lock would leave ingest dead and every request
/// failing, so restore the original hub's behaviour: exit and let systemd restart us.
fn lk<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|_| {
        tracing::error!("hub state lock poisoned by a panic; exiting for a clean restart");
        std::process::exit(70)
    })
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `repo`, `pr`, `branch` from a query string, validated. Err = 400 message.
#[allow(clippy::type_complexity)]
fn eta_query(query: &str) -> Result<(String, Option<u32>, Option<String>), &'static str> {
    let get = |k: &str| {
        query.split('&').find_map(|kv| {
            kv.split_once('=')
                .filter(|(a, _)| *a == k)
                .map(|(_, v)| v.to_string())
        })
    };
    let repo = get("repo").ok_or("repo is required")?;
    if repo.is_empty()
        || repo.len() > 100
        || !repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err("repo must be a repository name without the org, e.g. spar");
    }
    let pr = match get("pr") {
        Some(v) => Some(
            v.parse::<u32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or("pr must be a positive integer")?,
        ),
        None => None,
    };
    let branch = match get("branch") {
        Some(v) => {
            let v = pct_decode(&v).ok_or("branch is not valid percent-encoding")?;
            if v.is_empty() || v.len() > 200 || v.chars().any(|c| c.is_control()) {
                return Err("branch is invalid");
            }
            Some(v)
        }
        None => None,
    };
    if pr.is_none() && branch.is_none() {
        return Err("give pr or branch");
    }
    Ok((repo, pr, branch))
}

/// `month=YYYY-MM`, default: the current UTC month.
fn month_query(query: &str) -> Result<String, &'static str> {
    match query.split('&').find_map(|kv| kv.strip_prefix("month=")) {
        None => Ok(report::day_of(now())[..7].to_string()),
        Some(m)
            if m.len() == 7
                && m.as_bytes()[4] == b'-'
                && m.bytes()
                    .enumerate()
                    .all(|(i, b)| i == 4 || b.is_ascii_digit()) =>
        {
            Ok(m.to_string())
        }
        Some(_) => Err("month must be YYYY-MM"),
    }
}

fn pct_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                out.push(
                    u8::from_str_radix(std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?, 16).ok()?,
                );
                i += 3
            }
            b'+' => {
                out.push(b' ');
                i += 1
            }
            c => {
                out.push(c);
                i += 1
            }
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn eta_query_validation() {
        assert_eq!(
            eta_query("repo=spar&pr=511"),
            Ok(("spar".into(), Some(511), None))
        );
        assert_eq!(
            eta_query("repo=spar&branch=fix%2Fsom-count"),
            Ok(("spar".into(), None, Some("fix/som-count".into())))
        );
        assert!(eta_query("repo=../etc&pr=1").is_err());
        assert!(
            eta_query("repo=pulseengine/spar&pr=1").is_err(),
            "org must not be included"
        );
        assert!(eta_query("repo=spar&pr=-1").is_err());
        assert!(eta_query("repo=spar&pr=0").is_err());
        assert!(eta_query("repo=spar").is_err(), "needs pr or branch");
        assert!(
            eta_query("repo=spar&branch=a%0Ab").is_err(),
            "no control characters"
        );
        assert!(eta_query("repo=spar&branch=%zz").is_err());
        assert_eq!(month_query("month=2026-10"), Ok("2026-10".into()));
        assert!(
            month_query("month=2026-1").is_err()
                && month_query("month=../../x").is_err()
                && month_query("month=2026/10").is_err()
        );
        assert_eq!(month_query("").unwrap().len(), 7);
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Persisted {
    latest: Option<Snapshot>,
    received_at: i64,
    history: VecDeque<Point>,
}

#[derive(Default)]
struct Shared {
    p: Persisted,
    asm: Assembler,
    /// Datagrams from addresses not on the allow-list.
    foreign: u64,
    frames: u64,
}

/// One telemetry stream (APID) as the hub counted it since it started.
#[derive(Serialize)]
pub struct LinkRow {
    pub apid: String,
    pub stream: String,
    pub received: u64,
    pub lost: u64,
    pub restarts: u64,
    pub last_seen: i64,
}

#[derive(Serialize)]
struct StateView<'a> {
    hub_now: i64,
    received_at: i64,
    snapshot: &'a Option<Snapshot>,
    link: Vec<LinkRow>,
    rejected: u64,
    foreign: u64,
    frames: u64,
}

fn save(path: &str, p: &Persisted) {
    let tmp = format!("{path}.tmp");
    if let Ok(b) = serde_json::to_vec(p) {
        if std::fs::write(&tmp, b).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// the original hub refused to start without an allow-list before touching anything.
pub fn check_allow(allow: &[IpAddr]) -> Result<(), String> {
    if allow.is_empty() {
        return Err("refusing to start: no --allow source address for telemetry".into());
    }
    Ok(())
}

/// Hub state shared by the UDP ingest thread and the HTTP handlers.
#[derive(Clone)]
pub struct Hub {
    shared: Arc<Mutex<Shared>>,
    // the ledger has its own lock: building a month report reads files and must
    // not stall telemetry ingest
    ledger: Arc<Mutex<belt::HubLedger>>,
    org: Arc<str>,
    state_path: Arc<str>,
    /// Counts complete snapshots; long-polls (`/api/watch`) wait on it.
    tick: Arc<tokio::sync::watch::Sender<u64>>,
    /// Held `/api/watch` requests on this hub.
    watch_slots: Arc<crate::watch::Slots>,
    /// Where each snapshot's numbers go over time (REQ-MHIST-001).
    history: Arc<std::sync::OnceLock<crate::history::History>>,
    /// `[runners]` and the self-hosted label patterns, for the report's
    /// names, kinds and notional cost (REQ-RUNNERS-004).
    runners: Arc<std::sync::OnceLock<(pools::RunnersConfig, Vec<String>)>>,
}

impl Hub {
    /// Open (or create) a hub data directory. The layout is the original hub's:
    /// `hub-state.json` plus `ledger/`, so an existing `/var/lib/warte-hub`
    /// is read as-is.
    pub fn open(data_dir: &str, org: &str) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir).map_err(|e| format!("{data_dir}: {e}"))?;
        let path = format!("{data_dir}/hub-state.json");
        let restored: Persisted = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Ok(Hub {
            shared: Arc::new(Mutex::new(Shared {
                p: restored,
                ..Default::default()
            })),
            ledger: Arc::new(Mutex::new(belt::HubLedger::open(data_dir)?)),
            org: org.into(),
            state_path: path.into(),
            tick: Arc::new(tokio::sync::watch::channel(0).0),
            watch_slots: Default::default(),
            history: Default::default(),
            runners: Default::default(),
        })
    }

    /// Send every snapshot's numbers to `h` from now on. The first time a
    /// store sees this hub, the points the hub already holds are imported.
    /// Name, classify and price pools in reports from `[runners]`.
    pub fn set_runners(&self, cfg: pools::RunnersConfig, self_hosted: Vec<String>) {
        let _ = self.runners.set((cfg, self_hosted));
    }

    pub fn attach_history(&self, h: crate::history::History) {
        if h.meta("hub_imported").is_none() {
            let (host, pts) = {
                let g = lk(&self.shared);
                let host = g.p.latest.as_ref().map(|s| s.host.clone());
                (host, g.p.history.iter().cloned().collect::<Vec<_>>())
            };
            if let Some(host) = host {
                for p in &pts {
                    h.record_waiting(&host, p.t, &crate::history::project::from_point(p));
                }
                h.flush();
                h.set_meta("hub_imported", now());
            }
        }
        let _ = self.history.set(h);
    }

    fn to_history(&self, host: &str, p: &Point) {
        if let Some(h) = self.history.get() {
            h.record(
                host,
                crate::history::project::minute(p.t, now()),
                &crate::history::project::from_point(p),
            );
        }
    }

    /// Bind the telemetry socket and ingest on a dedicated thread.
    /// Returns the bound address (useful with port 0).
    pub fn spawn_ingest(&self, udp: &str, allow: Vec<IpAddr>) -> Result<SocketAddr, String> {
        check_allow(&allow)?;
        let sock = UdpSocket::bind(udp).map_err(|e| format!("bind udp {udp}: {e}"))?;
        let local = sock.local_addr().map_err(|e| format!("udp {udp}: {e}"))?;
        let hub = self.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            loop {
                let Ok((n, src)) = sock.recv_from(&mut buf) else {
                    continue;
                };
                hub.ingest(&buf[..n], src.ip(), &allow);
            }
        });
        Ok(local)
    }

    /// One datagram. Split out of the socket loop so tests can drive it.
    pub fn ingest(&self, datagram: &[u8], src: IpAddr, allow: &[IpAddr]) {
        let mut g = lk(&self.shared);
        if !allow.contains(&src) {
            g.foreign += 1;
            // log the first few, so a wrong --allow (e.g. the router NATs the
            // tunnel) is diagnosable instead of a silently empty dashboard
            if g.foreign <= 10 {
                tracing::warn!(%src, so_far = g.foreign, "telemetry from non-allowed source");
            }
            return;
        }
        let t = now();
        let snap = g.asm.ingest(datagram, t);
        if !g.asm.ledger_in.is_empty() {
            let recs = std::mem::take(&mut g.asm.ledger_in);
            lk(&self.ledger).ingest(recs);
        }
        if let Some(snap) = snap {
            let point = Point::from_snapshot(&snap);
            let host = snap.host.clone();
            g.p.history.push_back(point.clone());
            while g.p.history.len() > HISTORY_MAX {
                g.p.history.pop_front();
            }
            g.p.latest = Some(snap);
            g.p.received_at = t;
            g.frames += 1;
            if g.frames.is_multiple_of(5) {
                save(&self.state_path, &g.p);
            }
            drop(g);
            self.to_history(&host, &point);
            self.tick.send_modify(|n| *n += 1);
        }
    }

    /// A whole snapshot from a source other than the CCSDS link (the GitHub
    /// API scan, #81): stored and announced exactly like an assembled one.
    pub fn ingest_snapshot(&self, snap: Snapshot) {
        let t = now();
        let point = Point::from_snapshot(&snap);
        let host = snap.host.clone();
        let mut g = lk(&self.shared);
        g.p.history.push_back(point.clone());
        while g.p.history.len() > HISTORY_MAX {
            g.p.history.pop_front();
        }
        g.p.latest = Some(snap);
        g.p.received_at = t;
        g.frames += 1;
        if g.frames.is_multiple_of(5) {
            save(&self.state_path, &g.p);
        }
        drop(g);
        self.to_history(&host, &point);
        self.tick.send_modify(|n| *n += 1);
    }

    /// Finished-job records and day manifests from the GitHub source's
    /// ledger pass (REQ-RUNNERS-005), the way the agent's packets bring them.
    pub fn ingest_ledger(&self, recs: Vec<JobRecord>, manifests: Vec<DayManifest>) {
        if !recs.is_empty() {
            lk(&self.ledger).ingest(recs);
        }
        let mut g = lk(&self.shared);
        for m in manifests {
            g.asm.manifests.insert(m.day.clone(), m);
        }
    }

    /// Latest snapshot and when it was received (unix seconds), for the
    /// dashboard. `None` until the first complete frame arrives.
    pub fn snapshot(&self) -> Option<(Snapshot, i64)> {
        let g = lk(&self.shared);
        g.p.latest.clone().map(|s| (s, g.p.received_at))
    }

    /// The telemetry link per stream, as `/api/state` reports it.
    pub fn link(&self) -> Vec<LinkRow> {
        link_rows(&lk(&self.shared))
    }

    /// Wakes on every complete snapshot.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.tick.subscribe()
    }

    /// Held `/api/watch` requests, per client and in total.
    pub fn watch_slots(&self) -> &crate::watch::Slots {
        &self.watch_slots
    }

    /// The GitHub org repo names are relative to.
    pub fn org(&self) -> &str {
        &self.org
    }

    /// Per-minute history points taken at or after `since` (unix seconds).
    pub fn history_since(&self, since: i64) -> Vec<model::Point> {
        let g = lk(&self.shared);
        g.p.history
            .iter()
            .filter(|p| p.t >= since)
            .cloned()
            .collect()
    }

    /// The same answer `/api/eta` gives; `None` before the first snapshot.
    pub fn eta(&self, repo: &str, pr: Option<u32>, branch: Option<&str>) -> Option<eta::Answer> {
        let g = lk(&self.shared);
        g.p.latest
            .as_ref()
            .map(|s| eta::answer(s, &self.org, repo, pr, branch, now()))
    }

    /// The `/api/eta` answer for a raw query string, as an HTTP response:
    /// served by `warte hub` and at warte's own `/api/eta` (REQ-SMITHY-005).
    pub fn eta_response(&self, query: &str) -> Response {
        let g = lk(&self.shared);
        match (&g.p.latest, eta_query(query)) {
            (None, _) => text(503, "no snapshot received yet"),
            (_, Err(e)) => text(400, e),
            (Some(s), Ok((repo, pr, branch))) => json(&eta::answer(
                s,
                &self.org,
                &repo,
                pr,
                branch.as_deref(),
                now(),
            )),
        }
    }

    /// The hub's HTTP API, same paths and responses as the original hub.
    pub fn router(&self) -> Router {
        Router::new()
            .route("/api/state", get(api_state))
            .route("/api/history", get(api_history))
            .route("/status.md", get(status_md))
            .route("/api/eta", get(api_eta))
            .route("/report.md", get(report_md))
            .route("/api/report", get(api_report))
            .route(
                "/api/openapi.json",
                get(|| async { body(200, "application/json", OPENAPI.into()) }),
            )
            .route("/", get(index))
            .route("/index.html", get(index))
            .route("/healthz", get(|| async { text(200, "ok") }))
            .fallback(|| async { text(404, "not found") })
            .layer(middleware::from_fn(get_only))
            .with_state(self.clone())
    }
}

/// the original hub answered only GET; HEAD and every other method got its 404.
/// axum would answer HEAD from the GET route and other methods with 405.
async fn get_only(req: Request, next: Next) -> Response {
    if req.method() != Method::GET {
        return text(404, "not found");
    }
    next.run(req).await
}

fn body(code: u16, ctype: &'static str, b: Vec<u8>) -> Response {
    (
        StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        [
            (header::CONTENT_TYPE, ctype),
            (header::CACHE_CONTROL, "no-store"),
        ],
        b,
    )
        .into_response()
}
fn text(code: u16, s: &str) -> Response {
    body(code, "text/plain; charset=utf-8", s.as_bytes().to_vec())
}
fn json<T: Serialize>(v: &T) -> Response {
    body(
        200,
        "application/json",
        serde_json::to_vec(v).unwrap_or_default(),
    )
}
fn markdown(s: String) -> Response {
    body(200, "text/markdown; charset=utf-8", s.into_bytes())
}

/// The name the hub's page and status report carry; `warte hub --brand`.
static BRAND: std::sync::OnceLock<String> = std::sync::OnceLock::new();
/// The brand when none is set.
pub const DEFAULT_BRAND: &str = "warte";

/// Set the brand once, before serving; later calls are ignored.
pub fn set_brand(b: &str) {
    let _ = BRAND.set(b.to_string());
}

pub fn brand() -> &'static str {
    BRAND.get().map(String::as_str).unwrap_or(DEFAULT_BRAND)
}

/// The page with its brand in place of `__BRAND__`, HTML-escaped.
pub fn index_html() -> String {
    let b = brand()
        .to_uppercase()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    INDEX.replace("__BRAND__", &b)
}

async fn index() -> Response {
    body(200, "text/html; charset=utf-8", index_html().into_bytes())
}

fn link_rows(g: &Shared) -> Vec<LinkRow> {
    g.asm
        .link
        .iter()
        .map(|(a, l)| LinkRow {
            apid: format!("0x{a:03x}"),
            stream: apid_name(*a),
            received: l.received,
            lost: l.lost,
            restarts: l.restarts,
            last_seen: l.last_seen,
        })
        .collect()
}

async fn api_state(State(h): State<Hub>) -> Response {
    h.state_response()
}

async fn api_history(State(h): State<Hub>, RawQuery(q): RawQuery) -> Response {
    h.history_response(&q.unwrap_or_default())
}

async fn status_md(State(h): State<Hub>, RawQuery(q): RawQuery) -> Response {
    h.status_md_response(&q.unwrap_or_default())
}

/// `hours=H` of a query, 1 to 168, default 24.
pub fn hours_query(query: &str) -> i64 {
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("hours="))
        .and_then(|v| v.parse().ok())
        .unwrap_or(24)
        .clamp(1, 168)
}

/// The ground station's answers as HTTP responses, served by `warte hub` and
/// under warte's own API (REQ-SMITHY-006), so both say the same.
impl Hub {
    /// Latest snapshot with the telemetry link's counters.
    pub fn state_response(&self) -> Response {
        let g = lk(&self.shared);
        let link = link_rows(&g);
        json(&StateView {
            hub_now: now(),
            received_at: g.p.received_at,
            snapshot: &g.p.latest,
            link,
            rejected: g.asm.rejected,
            foreign: g.foreign,
            frames: g.frames,
        })
    }

    /// Per-minute fleet points of the last `hours=` (default 24, at most 168).
    pub fn history_response(&self, query: &str) -> Response {
        let since = now() - hours_query(query) * 3600;
        let g = lk(&self.shared);
        let pts: Vec<&Point> = g.p.history.iter().filter(|p| p.t >= since).collect();
        json(&pts)
    }

    /// Fleet status as Markdown, or one PR's answer with `repo=` and `pr=`/`branch=`.
    pub fn status_md_response(&self, query: &str) -> Response {
        let g = lk(&self.shared);
        match (&g.p.latest, query.contains("repo=")) {
            (None, _) => text(503, "no snapshot received yet"),
            (Some(s), true) => match eta_query(query) {
                Ok((repo, pr, branch)) => markdown(report::eta_markdown(&eta::answer(
                    s,
                    &self.org,
                    &repo,
                    pr,
                    branch.as_deref(),
                    now(),
                ))),
                Err(e) => text(400, e),
            },
            (Some(s), false) => markdown(report::markdown(
                s,
                now(),
                Some(g.asm.link.values().map(|l| l.lost).sum()),
            )),
        }
    }

    /// The month's conveyor-belt report (`month=YYYY-MM`, default this
    /// month), as JSON or Markdown. Reads the month's ledger off the async workers.
    pub async fn report_response(&self, query: Option<String>, md: bool) -> Response {
        match month_report_blocking(self.clone(), query).await {
            Ok(b) if md => markdown(belt::markdown(&b)),
            Ok(b) => json(&b),
            Err(e) => text(400, e),
        }
    }

    /// Ledger records of the days `from..=until` (`YYYY-MM-DD`), for the
    /// briefing's CI facts (REQ-BRIEF-001). Reads files: call off the async
    /// workers for long ranges.
    pub fn ledger_days(&self, from: &str, until: &str) -> Vec<JobRecord> {
        let l = lk(&self.ledger);
        let mut months: Vec<String> = vec![from[..7.min(from.len())].to_string()];
        if until.get(..7) != from.get(..7) {
            months.push(until[..7.min(until.len())].to_string());
        }
        months
            .iter()
            .flat_map(|m| l.month(m))
            .filter(|r| r.d.as_str() >= from && r.d.as_str() <= until)
            .collect()
    }

    /// The month's report as data, for the dashboard.
    pub async fn month_report(&self, month: &str) -> Result<belt::BeltReport, &'static str> {
        month_report_blocking(self.clone(), Some(format!("month={month}"))).await
    }
}

async fn api_eta(State(h): State<Hub>, RawQuery(q): RawQuery) -> Response {
    h.eta_response(&q.unwrap_or_default())
}

fn month_report(h: &Hub, query: &str) -> Result<belt::BeltReport, &'static str> {
    let month = month_query(query)?;
    let (manifests, labels) = {
        let g = lk(&h.shared);
        let labels =
            g.p.latest
                .as_ref()
                .map(|s| {
                    s.runners
                        .iter()
                        .map(|r| {
                            // the agent names its runners <host>-<id>; the
                            // GitHub source has GitHub's runner names
                            let name = if s.host.starts_with("github:") {
                                r.unit.clone()
                            } else {
                                format!("{}-{}", s.host, r.id)
                            };
                            (name, r.labels.clone())
                        })
                        .collect()
                })
                .unwrap_or_default();
        (g.asm.manifests.clone(), labels)
    };
    let (recs, stored) = {
        let l = lk(&h.ledger);
        (l.month(&month), l.stored.clone())
    };
    let r = belt::report(&month, &recs, &stored, &manifests, &labels);
    Ok(match h.runners.get() {
        Some((cfg, sh)) => r.priced(cfg, sh),
        None => r.priced(&Default::default(), &[]),
    })
}

/// Building a month report reads every day file of the month; keep that off
/// the async workers so it cannot stall other requests.
async fn month_report_blocking(
    h: Hub,
    q: Option<String>,
) -> Result<belt::BeltReport, &'static str> {
    tokio::task::spawn_blocking(move || month_report(&h, &q.unwrap_or_default()))
        .await
        .unwrap_or(Err("report failed"))
}

async fn report_md(State(h): State<Hub>, RawQuery(q): RawQuery) -> Response {
    h.report_response(q, true).await
}

async fn api_report(State(h): State<Hub>, RawQuery(q): RawQuery) -> Response {
    h.report_response(q, false).await
}

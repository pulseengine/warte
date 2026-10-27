//! Ground station absorbed from smithy hub (DD-SMITHY-001, REQ-SMITHY-001..003).
//!
//! The smithy agent on the CI host collects a snapshot and dials OUT to us with
//! CCSDS space packets over UDP (relay-ccsds), so the CI host exposes nothing.
//! We accept telemetry only from allow-listed source addresses, keep the latest
//! snapshot plus per-minute history in `hub-state.json` and every finished job
//! in the per-day JSONL ledger, and serve the hub's read-only API unchanged
//! (`web/openapi.json` is the contract; REQ-SMITHY-002).
//!
//! `model`, `telemetry`, `eta`, `belt`, `report` and `parse` come from
//! pulseengine/smithy `tools/smithy` at e087dae (see `UPSTREAM`), with only
//! their `crate::` paths rewritten. They are kept whole, agent-side items
//! included, so the wire format and on-disk formats stay byte-compatible with
//! the agent and with an existing `/var/lib/smithy-hub`. Only the HTTP layer is
//! new: smithy served it with tiny_http, here it is an axum router.
#![allow(dead_code)]

pub mod belt;
pub mod eta;
pub mod model;
pub mod parse;
pub mod report;
pub mod telemetry;

use axum::extract::{RawQuery, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use model::{Point, Snapshot};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::net::{IpAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use telemetry::{apid_name, Assembler};

/// The smithy commit these modules and `web/` were taken from.
pub const UPSTREAM: &str =
    "pulseengine/smithy@e087dae2423fd55b1cfee605fabbff7a6106921b tools/smithy";

/// One point per completed frame (~1/min) for 7 days.
const HISTORY_MAX: usize = 7 * 24 * 60;
pub const INDEX: &str = include_str!("web/index.html");
pub const OPENAPI: &str = include_str!("web/openapi.json");

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

#[derive(Serialize)]
struct LinkRow {
    apid: String,
    stream: String,
    received: u64,
    lost: u64,
    restarts: u64,
    last_seen: i64,
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

/// Hub state shared by the UDP ingest thread and the HTTP handlers.
#[derive(Clone)]
pub struct Hub {
    shared: Arc<Mutex<Shared>>,
    // the ledger has its own lock: building a month report reads files and must
    // not stall telemetry ingest
    ledger: Arc<Mutex<belt::HubLedger>>,
    org: Arc<str>,
    state_path: Arc<str>,
}

impl Hub {
    /// Open (or create) a hub data directory. The layout is smithy hub's:
    /// `hub-state.json` plus `ledger/`, so an existing `/var/lib/smithy-hub`
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
        })
    }

    /// Bind the telemetry socket and ingest on a dedicated thread.
    pub fn spawn_ingest(&self, udp: &str, allow: Vec<IpAddr>) -> Result<(), String> {
        if allow.is_empty() {
            return Err("refusing to start: no --allow source address for telemetry".into());
        }
        let sock = UdpSocket::bind(udp).map_err(|e| format!("bind udp {udp}: {e}"))?;
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
        Ok(())
    }

    /// One datagram. Split out of the socket loop so tests can drive it.
    pub fn ingest(&self, datagram: &[u8], src: IpAddr, allow: &[IpAddr]) {
        let mut g = self.shared.lock().unwrap();
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
            self.ledger.lock().unwrap().ingest(recs);
        }
        if let Some(snap) = snap {
            g.p.history.push_back(Point::from_snapshot(&snap));
            while g.p.history.len() > HISTORY_MAX {
                g.p.history.pop_front();
            }
            g.p.latest = Some(snap);
            g.p.received_at = t;
            g.frames += 1;
            if g.frames.is_multiple_of(5) {
                save(&self.state_path, &g.p);
            }
        }
    }

    /// The hub's HTTP API, same paths and responses as smithy hub.
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
            .with_state(self.clone())
    }
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

async fn index() -> Response {
    body(200, "text/html; charset=utf-8", INDEX.into())
}

async fn api_state(State(h): State<Hub>) -> Response {
    let g = h.shared.lock().unwrap();
    let link = g
        .asm
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
        .collect();
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

async fn api_history(State(h): State<Hub>, RawQuery(q): RawQuery) -> Response {
    let query = q.unwrap_or_default();
    let hours: i64 = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("hours="))
        .and_then(|v| v.parse().ok())
        .unwrap_or(24)
        .clamp(1, 168);
    let since = now() - hours * 3600;
    let g = h.shared.lock().unwrap();
    let pts: Vec<&Point> = g.p.history.iter().filter(|p| p.t >= since).collect();
    json(&pts)
}

async fn status_md(State(h): State<Hub>, RawQuery(q): RawQuery) -> Response {
    let query = q.unwrap_or_default();
    let g = h.shared.lock().unwrap();
    match (&g.p.latest, query.contains("repo=")) {
        (None, _) => text(503, "no snapshot received yet"),
        (Some(s), true) => match eta_query(&query) {
            Ok((repo, pr, branch)) => markdown(report::eta_markdown(&eta::answer(
                s,
                &h.org,
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

async fn api_eta(State(h): State<Hub>, RawQuery(q): RawQuery) -> Response {
    let query = q.unwrap_or_default();
    let g = h.shared.lock().unwrap();
    match (&g.p.latest, eta_query(&query)) {
        (None, _) => text(503, "no snapshot received yet"),
        (_, Err(e)) => text(400, e),
        (Some(s), Ok((repo, pr, branch))) => {
            json(&eta::answer(s, &h.org, &repo, pr, branch.as_deref(), now()))
        }
    }
}

fn month_report(h: &Hub, query: &str) -> Result<belt::BeltReport, &'static str> {
    let month = month_query(query)?;
    let (manifests, labels) = {
        let g = h.shared.lock().unwrap();
        let labels =
            g.p.latest
                .as_ref()
                .map(|s| {
                    s.runners
                        .iter()
                        .map(|r| (format!("{}-{}", s.host, r.id), r.labels.clone()))
                        .collect()
                })
                .unwrap_or_default();
        (g.asm.manifests.clone(), labels)
    };
    let (recs, stored) = {
        let l = h.ledger.lock().unwrap();
        (l.month(&month), l.stored.clone())
    };
    Ok(belt::report(&month, &recs, &stored, &manifests, &labels))
}

async fn report_md(State(h): State<Hub>, RawQuery(q): RawQuery) -> Response {
    match month_report(&h, &q.unwrap_or_default()) {
        Ok(b) => markdown(belt::markdown(&b)),
        Err(e) => text(400, e),
    }
}

async fn api_report(State(h): State<Hub>, RawQuery(q): RawQuery) -> Response {
    match month_report(&h, &q.unwrap_or_default()) {
        Ok(b) => json(&b),
        Err(e) => text(400, e),
    }
}

//! External canaries: do the names CI depends on resolve, and through which
//! resolvers (REQ-PREDICT-003, DD-PREDICT-002)?
//!
//! Every few minutes warte asks each configured resolver for each configured
//! name (an A query over UDP, written here: no DNS library), and the
//! system's own resolver too. A failure is reported only after two failing
//! rounds in a row. Then warte names the likely cause from which resolvers
//! still answer:
//!
//! - a name fails through some resolvers and resolves through others: a
//!   resolver problem, not the dependency (the 2026-11-11 incident);
//! - a name fails through every resolver: the dependency's DNS;
//! - one resolver fails for every name: that resolver;
//! - every resolver fails for every name: the network.
//!
//! Resolvers are shown by their label only: their addresses are
//! infrastructure and the read API is unauthenticated (#46). Outcomes are a
//! fixed set of words, never raw error text.

use crate::dashboard::fleet::{esc, Detected};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// The shortest interval between rounds.
pub const MIN_EVERY_S: u64 = 60;
/// How long one query may take.
pub const TIMEOUT: Duration = Duration::from_secs(3);
/// A failure is reported after this many failing rounds in a row: one lost
/// packet is not an incident.
pub const REPORT_AFTER: u32 = 2;
/// The label of the system's own resolver.
pub const SYSTEM: &str = "system";

fn default_every_s() -> u64 {
    300
}
fn default_true() -> bool {
    true
}

/// `[canaries]` in warte.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanariesConfig {
    /// The names to resolve: the hosts your CI downloads from.
    pub names: Vec<String>,
    /// Resolvers to ask directly, each with a label that is shown and an
    /// address that is not.
    #[serde(default)]
    pub resolver: Vec<ResolverConfig>,
    /// Also resolve through the system's resolver, labelled `system`.
    #[serde(default = "default_true")]
    pub system: bool,
    /// Seconds between rounds (default 300, at least 60).
    #[serde(default = "default_every_s")]
    pub every_s: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolverConfig {
    pub label: String,
    /// An IP address, with or without a port (53 by default).
    pub address: String,
}

/// How a name is resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum Via {
    System,
    Udp(SocketAddr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resolver {
    pub label: String,
    pub via: Via,
}

fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 253
        && n.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && !l.starts_with('-')
                && !l.ends_with('-')
        })
}

impl CanariesConfig {
    /// The resolvers to ask, or why the section is wrong.
    pub fn resolvers(&self) -> Result<Vec<Resolver>, String> {
        if self.names.is_empty() {
            return Err("[canaries] names is empty".into());
        }
        for n in &self.names {
            if !valid_name(n) {
                return Err(format!("[canaries] {n:?} is not a host name"));
            }
        }
        if self.every_s < MIN_EVERY_S {
            return Err(format!("[canaries] every_s must be at least {MIN_EVERY_S}"));
        }
        let mut out = Vec::new();
        if self.system {
            out.push(Resolver {
                label: SYSTEM.into(),
                via: Via::System,
            });
        }
        for r in &self.resolver {
            let label = r.label.trim();
            if label.is_empty() || label == SYSTEM {
                return Err(format!(
                    "[[canaries.resolver]] needs a label other than {SYSTEM:?}"
                ));
            }
            if out.iter().any(|o| o.label == label) {
                return Err(format!(
                    "[[canaries.resolver]] label {label:?} is used twice"
                ));
            }
            let addr = r
                .address
                .parse::<SocketAddr>()
                .or_else(|_| {
                    r.address
                        .parse::<IpAddr>()
                        .map(|ip| SocketAddr::new(ip, 53))
                })
                .map_err(|_| {
                    format!("[[canaries.resolver]] {label:?}: the address is not an IP address")
                })?;
            out.push(Resolver {
                label: label.into(),
                via: Via::Udp(addr),
            });
        }
        if out.is_empty() {
            return Err(
                "[canaries] has no resolver: add [[canaries.resolver]] or keep system".into(),
            );
        }
        Ok(out)
    }
}

/// What one query came to: a fixed set, never raw error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    TimedOut,
    ServerFailure,
    NoSuchName,
    Refused,
    NoAddress,
    Failed,
}

impl Outcome {
    pub fn words(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::TimedOut => "timed out",
            Outcome::ServerFailure => "server failure",
            Outcome::NoSuchName => "no such name",
            Outcome::Refused => "refused",
            Outcome::NoAddress => "no address",
            Outcome::Failed => "failed",
        }
    }
}

/// A DNS query for the A record of `name`, recursion desired.
pub fn query(id: u16, name: &str) -> Vec<u8> {
    let mut q = Vec::with_capacity(18 + name.len());
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.trim_end_matches('.').split('.') {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 1, 0, 1]);
    q
}

/// What a reply to query `id` says, or none when it is not a reply to it.
pub fn answer(id: u16, reply: &[u8]) -> Option<Outcome> {
    if reply.len() < 12 || reply[0..2] != id.to_be_bytes() || reply[2] & 0x80 == 0 {
        return None;
    }
    let answers = u16::from_be_bytes([reply[6], reply[7]]);
    Some(match reply[3] & 0x0f {
        0 if answers > 0 => Outcome::Ok,
        0 => Outcome::NoAddress,
        2 => Outcome::ServerFailure,
        3 => Outcome::NoSuchName,
        5 => Outcome::Refused,
        _ => Outcome::Failed,
    })
}

async fn ask_udp(addr: SocketAddr, name: &str, timeout: Duration) -> Outcome {
    let local = if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let Ok(sock) = tokio::net::UdpSocket::bind(local).await else {
        return Outcome::Failed;
    };
    let id: u16 = rand::random();
    let q = query(id, name);
    let exchange = async {
        sock.send_to(&q, addr).await.ok()?;
        let mut buf = [0u8; 1500];
        loop {
            let (n, from) = sock.recv_from(&mut buf).await.ok()?;
            if from != addr {
                continue;
            }
            if let Some(o) = answer(id, &buf[..n]) {
                return Some(o);
            }
        }
    };
    match tokio::time::timeout(timeout, exchange).await {
        Ok(Some(o)) => o,
        Ok(None) => Outcome::Failed,
        Err(_) => Outcome::TimedOut,
    }
}

async fn ask_system(name: &str, timeout: Duration) -> Outcome {
    match tokio::time::timeout(timeout, tokio::net::lookup_host((name, 443))).await {
        Err(_) => Outcome::TimedOut,
        Ok(Err(_)) => Outcome::Failed,
        Ok(Ok(mut addrs)) => {
            if addrs.next().is_some() {
                Outcome::Ok
            } else {
                Outcome::NoAddress
            }
        }
    }
}

/// Ask `via` for `name`: the outcome and how long it took in ms.
pub async fn ask(via: &Via, name: &str, timeout: Duration) -> (Outcome, u64) {
    let start = Instant::now();
    let o = match via {
        Via::System => ask_system(name, timeout).await,
        Via::Udp(a) => ask_udp(*a, name, timeout).await,
    };
    (o, start.elapsed().as_millis() as u64)
}

/// One name through one resolver, in the latest round.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Check {
    pub name: String,
    pub resolver: String,
    pub outcome: Outcome,
    pub ms: u64,
    /// Failing rounds in a row, this one included; 0 when it resolved.
    pub failing_rounds: u32,
    /// When the current run of failures began.
    pub failing_since: Option<i64>,
}

/// The latest round.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Report {
    pub at: i64,
    pub every_s: u64,
    pub names: Vec<String>,
    pub resolvers: Vec<String>,
    pub report_after: u32,
    pub checks: Vec<Check>,
}

/// The next report from a round's results, carrying failure runs on from
/// the previous one.
pub fn next(
    prev: Option<&Report>,
    at: i64,
    every_s: u64,
    names: &[String],
    resolvers: &[String],
    results: Vec<(String, String, Outcome, u64)>,
) -> Report {
    let before: HashMap<(&str, &str), &Check> = prev
        .map(|p| {
            p.checks
                .iter()
                .map(|c| ((c.name.as_str(), c.resolver.as_str()), c))
                .collect()
        })
        .unwrap_or_default();
    let checks = results
        .into_iter()
        .map(|(name, resolver, outcome, ms)| {
            let was = before.get(&(name.as_str(), resolver.as_str()));
            let (failing_rounds, failing_since) = if outcome == Outcome::Ok {
                (0, None)
            } else {
                match was.filter(|w| w.failing_rounds > 0) {
                    Some(w) => (w.failing_rounds + 1, w.failing_since),
                    None => (1, Some(at)),
                }
            };
            Check {
                name,
                resolver,
                outcome,
                ms,
                failing_rounds,
                failing_since,
            }
        })
        .collect();
    Report {
        at,
        every_s,
        names: names.to_vec(),
        resolvers: resolvers.to_vec(),
        report_after: REPORT_AFTER,
        checks,
    }
}

fn reported(c: &Check) -> bool {
    c.outcome != Outcome::Ok && c.failing_rounds >= REPORT_AFTER
}

fn clock(t: Option<i64>) -> String {
    t.and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default()
}

fn list(v: &[&str]) -> String {
    v.join(", ")
}

/// The likely causes, as outage findings (alerts notify).
pub fn judge(r: &Report, now: i64) -> Vec<Detected> {
    let mut out = Vec::new();
    // the round itself stopped
    if now - r.at > 3 * r.every_s as i64 {
        out.push(Detected {
            key: "canary-stale".into(),
            sev: 1,
            text: format!(
                "the dependency canaries last ran {}: warte's canary task has stopped",
                clock(Some(r.at))
            ),
        });
        return out;
    }
    let of = |name: &str, res: &str| {
        r.checks
            .iter()
            .find(|c| c.name == name && c.resolver == res)
    };
    let earliest = |cs: &[&Check]| cs.iter().filter_map(|c| c.failing_since).min();
    let outcomes = |cs: &[&Check]| {
        let mut w: Vec<&str> = cs.iter().map(|c| c.outcome.words()).collect();
        w.sort();
        w.dedup();
        list(&w)
    };

    // every resolver fails for every name: the network
    let all: Vec<&Check> = r.checks.iter().collect();
    if !all.is_empty() && all.iter().all(|c| reported(c)) {
        out.push(Detected {
            key: "canary-all".into(),
            sev: 2,
            text: format!(
                "no name resolves through any resolver ({}) since {}: warte's network or DNS is down",
                outcomes(&all),
                clock(earliest(&all))
            ),
        });
        return out;
    }

    // one resolver fails for every name (of two or more): that resolver
    let mut wide: Vec<&str> = Vec::new();
    if r.names.len() >= 2 {
        for res in &r.resolvers {
            let cs: Vec<&Check> = r.names.iter().filter_map(|n| of(n, res)).collect();
            if cs.len() == r.names.len() && cs.iter().all(|c| reported(c)) {
                wide.push(res);
                out.push(Detected {
                    key: format!("canary-resolver:{res}"),
                    sev: 1,
                    text: format!(
                        "resolver {res} answers none of the {} names ({}) since {}: that resolver is failing, not the dependencies",
                        cs.len(),
                        outcomes(&cs),
                        clock(earliest(&cs))
                    ),
                });
            }
        }
    }

    // per name, among the other resolvers
    for name in &r.names {
        let cs: Vec<&Check> = r
            .resolvers
            .iter()
            .filter(|res| !wide.contains(&res.as_str()))
            .filter_map(|res| of(name, res))
            .collect();
        let failing: Vec<&Check> = cs.iter().copied().filter(|c| reported(c)).collect();
        if failing.is_empty() {
            continue;
        }
        let working: Vec<&str> = cs
            .iter()
            .filter(|c| c.outcome == Outcome::Ok)
            .map(|c| c.resolver.as_str())
            .collect();
        let via: Vec<&str> = failing.iter().map(|c| c.resolver.as_str()).collect();
        let text = if working.is_empty() {
            format!(
                "{name} does not resolve through {} ({}) since {}: the dependency's DNS is failing",
                if via.len() > 1 {
                    format!("any of {}", list(&via))
                } else {
                    list(&via)
                },
                outcomes(&failing),
                clock(earliest(&failing))
            )
        } else {
            format!(
                "{name} fails through {} ({}) since {} but resolves through {}: a resolver problem, not {name}",
                list(&via),
                outcomes(&failing),
                clock(earliest(&failing)),
                list(&working)
            )
        };
        out.push(Detected {
            key: format!("canary:{name}"),
            sev: if working.is_empty() { 2 } else { 1 },
            text,
        });
    }
    out
}

/// The latest round, shared with the server.
#[derive(Default)]
pub struct Canaries {
    latest: RwLock<Option<Report>>,
}

impl Canaries {
    pub fn latest(&self) -> Option<Report> {
        self.latest.read().ok().and_then(|g| g.clone())
    }

    /// Store a round's results.
    pub fn record(
        &self,
        at: i64,
        every_s: u64,
        names: &[String],
        resolvers: &[String],
        results: Vec<(String, String, Outcome, u64)>,
    ) {
        if let Ok(mut g) = self.latest.write() {
            let r = next(g.as_ref(), at, every_s, names, resolvers, results);
            *g = Some(r);
        }
    }
}

/// Ask every resolver for every name, all at once.
pub async fn round(
    names: &[String],
    resolvers: &[Resolver],
    timeout: Duration,
) -> Vec<(String, String, Outcome, u64)> {
    let asks = resolvers.iter().flat_map(|r| {
        names.iter().map(move |n| async move {
            let (o, ms) = ask(&r.via, n, timeout).await;
            (n.clone(), r.label.clone(), o, ms)
        })
    });
    futures::future::join_all(asks).await
}

/// Run a round every `every_s` seconds, for as long as warte runs.
pub fn spawn(store: Arc<Canaries>, cfg: CanariesConfig, resolvers: Vec<Resolver>) {
    let labels: Vec<String> = resolvers.iter().map(|r| r.label.clone()).collect();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(cfg.every_s));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let results = round(&cfg.names, &resolvers, TIMEOUT).await;
            store.record(crate::hub::now(), cfg.every_s, &cfg.names, &labels, results);
        }
    });
}

/// The overview's Dependencies panel: one row per name, one column per
/// resolver.
pub fn panel(r: Option<&Report>) -> String {
    let Some(r) = r else {
        return String::new();
    };
    let head: String = r
        .resolvers
        .iter()
        .map(|res| format!("<th>{}</th>", esc(res)))
        .collect();
    let mut rows = String::new();
    for name in &r.names {
        let mut cells = String::new();
        let mut bad = false;
        for res in &r.resolvers {
            let cell = match r
                .checks
                .iter()
                .find(|c| &c.name == name && &c.resolver == res)
            {
                Some(c) if c.outcome == Outcome::Ok => {
                    format!(r#"<td>ok <span class="dim">{} ms</span></td>"#, c.ms)
                }
                Some(c) => {
                    bad |= reported(c);
                    format!(
                        r#"<td class="{}">{}<br><span class="dim">since {}</span></td>"#,
                        if reported(c) { "bad" } else { "dim" },
                        c.outcome.words(),
                        esc(&clock(c.failing_since))
                    )
                }
                None => "<td class=\"dim\">—</td>".into(),
            };
            cells += &cell;
        }
        rows += &format!(
            r#"<tr class="{}"><td class="mono">{}</td>{cells}</tr>"#,
            if bad { "warnrow" } else { "" },
            esc(name)
        );
    }
    format!(
        r#"<section class="panel canaries"><h3>Dependencies</h3><p class="note">Do the names CI downloads from resolve? Asked {} through each resolver; a failure counts after {} rounds in a row. Last round {}.</p><table><thead><tr><th>Name</th>{head}</tr></thead><tbody>{rows}</tbody></table></section>"#,
        match r.every_s / 60 {
            1 => "every minute".to_string(),
            m => format!("every {m} minutes"),
        },
        r.report_after,
        esc(&clock(Some(r.at)))
    )
}

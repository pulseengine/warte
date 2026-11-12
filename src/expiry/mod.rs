//! Expiry dates: what runs out, and when (REQ-PREDICT-004, DD-PREDICT-003).
//!
//! warte collects end dates from four places:
//!
//! - its own certificates: the CA it signs probe certificates with, and its
//!   TLS certificate when it serves TLS itself;
//! - TLS endpoints in `[[expiry.tls]]`: every certificate the endpoint
//!   sends (the leaf and its intermediates), read without trusting them, so
//!   a proxy's internal CA is checked too;
//! - PEM files in `[[expiry.pem]]`, for a root the endpoint does not send;
//! - dates in `[[expiry.date]]`, for what warte cannot read (signing keys,
//!   GitHub App keys);
//!
//! and from the bearer tokens it sees: each job's token, by the job's name,
//! the last time it was used.
//!
//! An end date within `amber_days` (30) is an amber outage finding, within
//! `red_days` (7) or past a red one, so the alerts notify. Endpoints and
//! files are shown by their label only: addresses and paths are
//! infrastructure and the read API is unauthenticated (#46). What could not
//! be read is said in a fixed set of words, never raw error text.

use crate::dashboard::fleet::{esc, Detected};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// How long one endpoint may take to connect and hand over its certificates.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// Items further out than this are counted on the panel, not listed.
pub const PANEL_DAYS: f64 = 90.0;

fn default_every_s() -> u64 {
    6 * 3600
}
fn default_amber_days() -> f64 {
    30.0
}
fn default_red_days() -> f64 {
    7.0
}

/// `[expiry]` in warte.toml. Without it warte still watches its own
/// certificates and the tokens it sees.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpiryConfig {
    #[serde(default)]
    pub tls: Vec<TlsTarget>,
    #[serde(default)]
    pub pem: Vec<PemFile>,
    #[serde(default)]
    pub date: Vec<Manual>,
    /// Seconds between checks of endpoints and files (default 6 hours).
    #[serde(default = "default_every_s")]
    pub every_s: u64,
    #[serde(default = "default_amber_days")]
    pub amber_days: f64,
    #[serde(default = "default_red_days")]
    pub red_days: f64,
}

impl Default for ExpiryConfig {
    fn default() -> Self {
        Self {
            tls: vec![],
            pem: vec![],
            date: vec![],
            every_s: default_every_s(),
            amber_days: default_amber_days(),
            red_days: default_red_days(),
        }
    }
}

/// A TLS endpoint: `address` is `host:port` and is never shown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsTarget {
    pub label: String,
    pub address: String,
    /// The name sent in the handshake (default: the host of `address`).
    #[serde(default)]
    pub server_name: Option<String>,
}

/// A PEM file of certificates: `path` is never shown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PemFile {
    pub label: String,
    pub path: PathBuf,
}

/// An end date warte cannot read for itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manual {
    pub what: String,
    /// `YYYY-MM-DD`; the end of that day, UTC.
    pub on: String,
}

fn day_end(on: &str) -> Option<i64> {
    chrono::NaiveDate::parse_from_str(on, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(23, 59, 59))
        .map(|t| t.and_utc().timestamp())
}

fn host_of(address: &str) -> Option<&str> {
    let (host, port) = address.rsplit_once(':')?;
    port.parse::<u16>().ok()?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    (!host.is_empty()).then_some(host)
}

impl ExpiryConfig {
    /// Why the section is wrong, if it is.
    pub fn check(&self) -> Result<(), String> {
        if self.every_s < 600 {
            return Err("[expiry] every_s must be at least 600".into());
        }
        if !(self.red_days >= 0.0 && self.amber_days >= self.red_days) {
            return Err("[expiry] needs 0 <= red_days <= amber_days".into());
        }
        let mut labels: Vec<&str> = Vec::new();
        for t in &self.tls {
            if t.label.trim().is_empty() {
                return Err("[[expiry.tls]] needs a label".into());
            }
            if host_of(&t.address).is_none() {
                return Err(format!(
                    "[[expiry.tls]] {:?}: the address must be host:port",
                    t.label
                ));
            }
            labels.push(&t.label);
        }
        for p in &self.pem {
            if p.label.trim().is_empty() {
                return Err("[[expiry.pem]] needs a label".into());
            }
            labels.push(&p.label);
        }
        for d in &self.date {
            if d.what.trim().is_empty() {
                return Err("[[expiry.date]] needs what".into());
            }
            if day_end(&d.on).is_none() {
                return Err(format!(
                    "[[expiry.date]] {:?}: on must be YYYY-MM-DD",
                    d.what
                ));
            }
            labels.push(&d.what);
        }
        let mut sorted = labels.clone();
        sorted.sort();
        if let Some(w) = sorted.windows(2).find(|w| w[0] == w[1]) {
            return Err(format!("[expiry] {:?} is used twice", w[0]));
        }
        Ok(())
    }
}

/// Where an end date comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// warte's own certificates.
    Own,
    Tls,
    Pem,
    Date,
    Token,
}

/// Why an end date could not be read: a fixed set, never raw error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Problem {
    Unreachable,
    TimedOut,
    HandshakeFailed,
    Unreadable,
    NoCertificate,
}

impl Problem {
    pub fn words(self) -> &'static str {
        match self {
            Problem::Unreachable => "unreachable",
            Problem::TimedOut => "timed out",
            Problem::HandshakeFailed => "TLS handshake failed",
            Problem::Unreadable => "could not be read",
            Problem::NoCertificate => "holds no certificate",
        }
    }
}

/// One thing that runs out.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    pub kind: Kind,
    /// The label, the date's `what`, or the job's name.
    pub what: String,
    /// Which certificate: `leaf`, `intermediate 1`, `root`, ...
    pub part: Option<String>,
    pub expires_at: Option<i64>,
    pub problem: Option<Problem>,
}

/// The certificates in `ders`, in order, with their end dates. A
/// self-signed certificate is the `root`; otherwise the first is the `leaf`
/// and the rest are intermediates.
pub fn certificates(kind: Kind, what: &str, ders: &[Vec<u8>]) -> Vec<Item> {
    if ders.is_empty() {
        return vec![Item {
            kind,
            what: what.into(),
            part: None,
            expires_at: None,
            problem: Some(Problem::NoCertificate),
        }];
    }
    ders.iter()
        .enumerate()
        .map(|(i, der)| match x509_parser::parse_x509_certificate(der) {
            Ok((_, c)) => {
                let root = c.tbs_certificate.subject.as_raw() == c.tbs_certificate.issuer.as_raw();
                let part = if root {
                    "root".to_string()
                } else if i == 0 {
                    "leaf".to_string()
                } else {
                    format!("intermediate {i}")
                };
                Item {
                    kind,
                    what: what.into(),
                    part: Some(part),
                    expires_at: Some(c.validity().not_after.timestamp()),
                    problem: None,
                }
            }
            Err(_) => Item {
                kind,
                what: what.into(),
                part: Some(format!("certificate {}", i + 1)),
                expires_at: None,
                problem: Some(Problem::Unreadable),
            },
        })
        .collect()
}

/// The certificates in PEM text.
pub fn pem_certificates(text: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    for p in x509_parser::pem::Pem::iter_from_buffer(text) {
        let p = p.ok()?;
        if p.label == "CERTIFICATE" {
            out.push(p.contents);
        }
    }
    Some(out)
}

fn read_pem(kind: Kind, what: &str, path: &std::path::Path) -> Vec<Item> {
    match std::fs::read(path).ok().and_then(|b| pem_certificates(&b)) {
        Some(ders) => certificates(kind, what, &ders),
        None => vec![Item {
            kind,
            what: what.into(),
            part: None,
            expires_at: None,
            problem: Some(Problem::Unreadable),
        }],
    }
}

/// Accepts any certificate: warte reads end dates here, it does not trust.
#[derive(Debug)]
struct ReadOnly(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for ReadOnly {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The certificates a TLS endpoint sends, or why there are none.
pub async fn fetch(t: &TlsTarget, timeout: Duration) -> Result<Vec<Vec<u8>>, Problem> {
    let name = t
        .server_name
        .clone()
        .or_else(|| host_of(&t.address).map(String::from))
        .ok_or(Problem::Unreachable)?;
    let name =
        rustls::pki_types::ServerName::try_from(name).map_err(|_| Problem::HandshakeFailed)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|_| Problem::HandshakeFailed)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(ReadOnly(provider)))
        .with_no_client_auth();
    let exchange = async {
        let tcp = tokio::net::TcpStream::connect(&t.address)
            .await
            .map_err(|_| Problem::Unreachable)?;
        let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg))
            .connect(name, tcp)
            .await
            .map_err(|_| Problem::HandshakeFailed)?;
        Ok(tls
            .get_ref()
            .1
            .peer_certificates()
            .unwrap_or_default()
            .iter()
            .map(|c| c.as_ref().to_vec())
            .collect())
    };
    tokio::time::timeout(timeout, exchange)
        .await
        .unwrap_or(Err(Problem::TimedOut))
}

/// warte's own certificate files, read at each check.
#[derive(Debug, Clone, Default)]
pub struct Own {
    pub ca: Option<PathBuf>,
    pub tls: Option<PathBuf>,
}

/// Check warte's own certificates, the endpoints and the files.
pub async fn check(cfg: &ExpiryConfig, own: &Own, timeout: Duration) -> Vec<Item> {
    let mut out = Vec::new();
    if let Some(p) = &own.ca {
        out.extend(read_pem(Kind::Own, "warte CA", p));
    }
    if let Some(p) = &own.tls {
        out.extend(read_pem(Kind::Own, "warte TLS", p));
    }
    let fetched = futures::future::join_all(cfg.tls.iter().map(|t| fetch(t, timeout))).await;
    for (t, r) in cfg.tls.iter().zip(fetched) {
        match r {
            Ok(ders) => out.extend(certificates(Kind::Tls, &t.label, &ders)),
            Err(p) => out.push(Item {
                kind: Kind::Tls,
                what: t.label.clone(),
                part: None,
                expires_at: None,
                problem: Some(p),
            }),
        }
    }
    for p in &cfg.pem {
        out.extend(read_pem(Kind::Pem, &p.label, &p.path));
    }
    out
}

/// Everything that runs out, soonest first.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Report {
    /// When endpoints and files were last checked (0 before the first check).
    pub checked_at: i64,
    pub amber_days: f64,
    pub red_days: f64,
    pub items: Vec<Item>,
}

/// The report from the last check, the configured dates and the tokens
/// seen.
pub fn report(
    cfg: &ExpiryConfig,
    checked_at: i64,
    checked: &[Item],
    tokens: &[(String, i64)],
) -> Report {
    let mut items = checked.to_vec();
    items.extend(cfg.date.iter().map(|d| Item {
        kind: Kind::Date,
        what: d.what.clone(),
        part: None,
        expires_at: day_end(&d.on),
        problem: None,
    }));
    items.extend(tokens.iter().map(|(who, at)| Item {
        kind: Kind::Token,
        what: who.clone(),
        part: None,
        expires_at: Some(*at),
        problem: None,
    }));
    // problems first, then soonest; ties by name, so the order is stable
    items.sort_by(|a, b| {
        (a.expires_at.unwrap_or(i64::MIN), &a.what, &a.part).cmp(&(
            b.expires_at.unwrap_or(i64::MIN),
            &b.what,
            &b.part,
        ))
    });
    Report {
        checked_at,
        amber_days: cfg.amber_days,
        red_days: cfg.red_days,
        items,
    }
}

fn days_left(at: i64, now: i64) -> f64 {
    (at - now) as f64 / 86_400.0
}

fn date(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn name(i: &Item) -> String {
    let what = match i.kind {
        Kind::Token => format!("the token for {}", i.what),
        _ => i.what.clone(),
    };
    match &i.part {
        Some(p) => format!("{what} ({p} certificate)"),
        None => what,
    }
}

fn when(at: i64, now: i64) -> String {
    let d = days_left(at, now);
    if d < 0.0 {
        format!("expired on {}", date(at))
    } else if d < 1.0 {
        format!("expires {} (within a day)", date(at))
    } else {
        format!("expires {} (in {} days)", date(at), d.floor() as i64)
    }
}

fn kind_key(k: Kind) -> &'static str {
    match k {
        Kind::Own => "own",
        Kind::Tls => "tls",
        Kind::Pem => "pem",
        Kind::Date => "date",
        Kind::Token => "token",
    }
}

/// End dates within `amber_days`, as outage findings (alerts notify).
pub fn detected(r: &Report, now: i64) -> Vec<Detected> {
    r.items
        .iter()
        .filter_map(|i| {
            let at = i.expires_at?;
            let d = days_left(at, now);
            (d <= r.amber_days).then(|| Detected {
                // stable while it lasts: no dates or counts in the key
                key: format!(
                    "expiry:{}:{}:{}",
                    kind_key(i.kind),
                    i.what,
                    i.part.as_deref().unwrap_or("")
                ),
                sev: if d <= r.red_days { 2 } else { 1 },
                text: format!("{} {}", name(i), when(at, now)),
            })
        })
        .collect()
}

/// The overview's Expiring panel: the next [`PANEL_DAYS`] and anything
/// that could not be read.
pub fn panel(r: &Report, now: i64) -> String {
    let shown: Vec<&Item> = r
        .items
        .iter()
        .filter(|i| {
            i.problem.is_some()
                || i.expires_at
                    .is_some_and(|at| days_left(at, now) <= PANEL_DAYS)
        })
        .collect();
    let later = r.items.len() - shown.len();
    let mut rows = String::new();
    for i in &shown {
        let (cls, end) = match (i.expires_at, i.problem) {
            (Some(at), _) => {
                let d = days_left(at, now);
                let cls = if d <= r.red_days {
                    "badrow"
                } else if d <= r.amber_days {
                    "warnrow"
                } else {
                    ""
                };
                (cls, esc(&when(at, now)))
            }
            (None, Some(p)) => ("", format!(r#"<span class="dim">{}</span>"#, p.words())),
            (None, None) => ("", String::new()),
        };
        rows += &format!(
            r#"<tr class="{cls}"><td>{}</td><td>{end}</td></tr>"#,
            esc(&name(i))
        );
    }
    let note = format!(
        "Certificates, keys and tokens, amber within {:.0} days and red within {:.0}.{}",
        r.amber_days,
        r.red_days,
        if later > 0 {
            format!(" {later} more run out later than {PANEL_DAYS:.0} days.")
        } else {
            String::new()
        }
    );
    let body = if shown.is_empty() {
        format!("<p>Nothing runs out within {PANEL_DAYS:.0} days.</p>")
    } else {
        format!(
            r#"<table><thead><tr><th>What</th><th>End</th></tr></thead><tbody>{rows}</tbody></table>"#
        )
    };
    format!(
        r#"<section class="panel expiring"><h3>Expiring</h3><p class="note">{note}</p>{body}</section>"#
    )
}

/// The latest check, shared with the server.
pub struct Expiry {
    pub cfg: ExpiryConfig,
    pub own: Own,
    checked: RwLock<(i64, Vec<Item>)>,
}

impl Expiry {
    pub fn new(cfg: ExpiryConfig, own: Own) -> Self {
        Self {
            cfg,
            own,
            checked: RwLock::new((0, Vec::new())),
        }
    }

    pub fn set(&self, at: i64, items: Vec<Item>) {
        if let Ok(mut g) = self.checked.write() {
            *g = (at, items);
        }
    }

    /// The report now, with the tokens seen.
    pub fn report(&self, tokens: &[(String, i64)]) -> Report {
        let (at, items) = self.checked.read().map(|g| g.clone()).unwrap_or_default();
        report(&self.cfg, at, &items, tokens)
    }
}

/// Check every `every_s` seconds, the first at once.
pub fn spawn(e: Arc<Expiry>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(e.cfg.every_s));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let items = check(&e.cfg, &e.own, TIMEOUT).await;
            e.set(crate::hub::now(), items);
        }
    });
}

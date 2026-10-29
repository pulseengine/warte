//! Costs: what agent chats and subagents cost, as notional list prices
//! (REQ-COSTS-001).
//!
//! A workstation job extracts token counts from its agent transcripts, prices
//! them with a versioned list-price table and pushes one report, whole. Two
//! ways in, the same checks on both:
//!
//! - `[hub] costs_file`: a file the job copies to the ground station over ssh
//!   (the channel scheduled jobs already use). warte re-reads it when its
//!   modification time changes; that time is the report's `received_at`.
//! - `POST /api/costs`, behind warte's bearer-token auth (REQ-API-002). Usable
//!   once warte issues tokens to agents (#17).
//!
//! The latest report is kept next to the hub state and served to the Costs
//! view and `GET /api/costs`.
//!
//! The figures are NOTIONAL: list prices applied to token counts. They are
//! not a bill.
//!
//! The read API is unauthenticated (warte#46), so the report may hold numbers,
//! model ids, repository names and categories, never free text. Every struct
//! denies unknown fields, and every string is checked against a short
//! character class. A report that carries a prompt or a task description is
//! rejected, not stored.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The label every view of these figures carries.
pub const LABEL: &str = "notional list price, not billed";
/// The report schema this build accepts.
pub const SCHEMA: u32 = 1;
/// The largest report accepted, in bytes.
pub const MAX_BODY: usize = 8 * 1024 * 1024;
/// A report older than this is shown as stale (the job runs nightly).
pub const STALE_S: i64 = 26 * 3600;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tokens {
    pub input: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
    pub output: u64,
    pub requests: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub id: String,
    pub date: String,
    pub machine: String,
    pub repo: String,
    pub model: String,
    pub tokens: Tokens,
    pub usd: f64,
    pub usd_main: f64,
    pub usd_subagents: f64,
    pub usd_advisor: f64,
    pub n_subagents: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Subagent {
    pub id: String,
    pub session: String,
    pub date: String,
    pub agent_type: String,
    pub model: String,
    pub parent_model: String,
    /// down / same / up / unknown: the subagent's tier against its caller's.
    pub route: String,
    pub category: String,
    pub linked: bool,
    pub tokens: Tokens,
    pub usd: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct RepoUsd {
    pub repo: String,
    pub usd: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ModelUsd {
    pub model: String,
    pub usd: f64,
    pub tokens: Tokens,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct CategoryUsd {
    pub category: String,
    pub usd: f64,
    pub subagents: u64,
}

/// Subagents that ran on a cheaper tier than the context that called them.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Cheaper {
    pub calls_down: u64,
    pub calls_same: u64,
    pub calls_up: u64,
    pub calls_unknown: u64,
    pub call_share: Option<f64>,
    pub usd_subagents_down: f64,
    pub usd_subagents: f64,
    pub usd_share: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub usd: f64,
    pub usd_main: f64,
    pub usd_subagents: f64,
    pub usd_advisor: f64,
    pub sessions: u64,
    pub by_repo: Vec<RepoUsd>,
    pub by_model: Vec<ModelUsd>,
    pub by_category: Vec<CategoryUsd>,
    pub cheaper_tier: Cheaper,
    pub unpriced_requests: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Day {
    pub day: String,
    pub usd: f64,
    pub usd_main: f64,
    pub usd_subagents: f64,
    pub usd_advisor: f64,
    pub usd_subagents_down: f64,
}

/// USD per million tokens: input, 5m cache write, 1h cache write, cache read,
/// output (fast mode: input, output).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Prices {
    pub version: String,
    pub source: String,
    pub per_mtok: BTreeMap<String, Vec<f64>>,
    pub long_prompt: u64,
    pub per_mtok_long: BTreeMap<String, Vec<f64>>,
    pub per_mtok_fast: BTreeMap<String, Vec<f64>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Classifier {
    pub version: u32,
    pub categories: Vec<String>,
}

/// One report, as the extractor pushes it.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct CostReport {
    pub schema: u32,
    pub kind: String,
    pub notional: bool,
    pub label: String,
    pub generated_at: String,
    pub prices: Prices,
    pub classifier: Classifier,
    pub counts: BTreeMap<String, u64>,
    pub unpriced_models: BTreeMap<String, u64>,
    /// `all`, `last_30d` and one per month (`YYYY-MM`).
    pub windows: BTreeMap<String, Window>,
    pub daily: Vec<Day>,
    pub sessions: Vec<Session>,
    pub subagents: Vec<Subagent>,
}

/// A name: model id, repository, category, machine, agent type. No spaces, no
/// markup, at most 64 characters (a task description has spaces).
fn name_ok(s: &str) -> bool {
    s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-()".contains(c))
}

fn id_ok(s: &str) -> bool {
    (1..=16).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric())
}

fn date_ok(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() || s == "unknown"
}

fn window_ok(s: &str) -> bool {
    s == "all"
        || s == "last_30d"
        || (s.len() == 7
            && chrono::NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d").is_ok())
}

fn usd_ok(x: f64) -> bool {
    x.is_finite() && x >= 0.0
}

fn share_ok(x: Option<f64>) -> bool {
    x.is_none_or(|x| x.is_finite() && (0.0..=1.0).contains(&x))
}

impl CostReport {
    /// Check what serde cannot: every string is a name, an id or a date, and
    /// every amount is finite and non-negative. Returns why not.
    pub fn validate(&self) -> Result<(), String> {
        // Names the field, never the value: the reason is shown on the public
        // read API, and the value may be exactly the text that must not be.
        let bad = |what: &str, _v: &str| Err(format!("{what} is not allowed"));
        if self.schema != SCHEMA {
            return Err(format!(
                "schema {} (this warte accepts {SCHEMA})",
                self.schema
            ));
        }
        if self.kind != "claude-code-costs" || !self.notional || self.label != LABEL {
            return Err(format!(
                "a costs report is kind claude-code-costs, notional true, label {LABEL:?}"
            ));
        }
        if chrono::DateTime::parse_from_rfc3339(&self.generated_at).is_err() {
            return bad("generated_at", &self.generated_at);
        }
        let p = &self.prices;
        if !name_ok(&p.version) {
            return bad("prices.version", &p.version);
        }
        if !p.source.starts_with("https://")
            || p.source.len() > 200
            || p.source
                .chars()
                .any(|c| c.is_whitespace() || "<>\"'".contains(c))
        {
            return bad("prices.source", &p.source);
        }
        for table in [&p.per_mtok, &p.per_mtok_long, &p.per_mtok_fast] {
            for (m, v) in table {
                if !name_ok(m) || v.len() > 5 || !v.iter().all(|x| usd_ok(*x)) {
                    return bad("a price row", m);
                }
            }
        }
        for c in &self.classifier.categories {
            if !name_ok(c) {
                return bad("a category", c);
            }
        }
        for k in self.counts.keys().chain(self.unpriced_models.keys()) {
            if !name_ok(k) {
                return bad("a count name", k);
            }
        }
        for (k, w) in &self.windows {
            if !window_ok(k) {
                return bad("a window", k);
            }
            let amounts = [w.usd, w.usd_main, w.usd_subagents, w.usd_advisor];
            let ch = &w.cheaper_tier;
            if !amounts.iter().all(|x| usd_ok(*x))
                || !usd_ok(ch.usd_subagents)
                || !usd_ok(ch.usd_subagents_down)
                || !share_ok(ch.call_share)
                || !share_ok(ch.usd_share)
            {
                return Err(format!("window {k}: an amount is negative or not finite"));
            }
            for r in &w.by_repo {
                if !name_ok(&r.repo) || !usd_ok(r.usd) {
                    return bad("a repository", &r.repo);
                }
            }
            for m in &w.by_model {
                if !name_ok(&m.model) || !usd_ok(m.usd) {
                    return bad("a model", &m.model);
                }
            }
            for c in &w.by_category {
                if !name_ok(&c.category) || !usd_ok(c.usd) {
                    return bad("a category", &c.category);
                }
            }
        }
        for d in &self.daily {
            let amounts = [
                d.usd,
                d.usd_main,
                d.usd_subagents,
                d.usd_advisor,
                d.usd_subagents_down,
            ];
            if !date_ok(&d.day) || !amounts.iter().all(|x| usd_ok(*x)) {
                return bad("a day", &d.day);
            }
        }
        for s in &self.sessions {
            let amounts = [s.usd, s.usd_main, s.usd_subagents, s.usd_advisor];
            if !id_ok(&s.id) || !date_ok(&s.date) {
                return bad("a session id or date", &s.id);
            }
            for v in [&s.machine, &s.repo, &s.model] {
                if !name_ok(v) {
                    return bad("a session field", v);
                }
            }
            if !amounts.iter().all(|x| usd_ok(*x)) {
                return bad("a session amount", &s.id);
            }
        }
        for a in &self.subagents {
            if !id_ok(&a.id) || !id_ok(&a.session) || !date_ok(&a.date) || !usd_ok(a.usd) {
                return bad("a subagent id, date or amount", &a.id);
            }
            for v in [
                &a.agent_type,
                &a.model,
                &a.parent_model,
                &a.route,
                &a.category,
            ] {
                if !name_ok(v) {
                    return bad("a subagent field", v);
                }
            }
        }
        Ok(())
    }
}

/// The stored report and when it arrived.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Stored {
    pub received_at: i64,
    pub report: CostReport,
}

#[derive(Default)]
struct State {
    latest: Option<Stored>,
    /// Why the last report read from `costs_file` was not accepted.
    rejected: Option<String>,
    /// Modification time of `costs_file` when it was last read.
    seen: Option<std::time::SystemTime>,
}

/// The latest report, persisted next to the hub state as `costs.json`.
pub struct Costs {
    state: std::sync::Mutex<State>,
    path: std::path::PathBuf,
    source: Option<std::path::PathBuf>,
}

/// Read, parse and check one report file.
pub fn read_report(path: &std::path::Path) -> Result<CostReport, String> {
    let b = std::fs::read(path).map_err(|e| format!("cannot read the report: {}", e.kind()))?;
    if b.len() > MAX_BODY {
        return Err(format!("the report is larger than {MAX_BODY} bytes"));
    }
    // serde's message can quote the input (a field name, a string value): say
    // only where and what kind of error.
    let r: CostReport = serde_json::from_slice(&b).map_err(|e| {
        format!(
            "not a costs report ({:?} error at line {} column {})",
            e.classify(),
            e.line(),
            e.column()
        )
    })?;
    r.validate()?;
    Ok(r)
}

impl Costs {
    /// `data_dir`: where warte keeps its copy. `source`: the `[hub] costs_file`
    /// a workstation job writes, if configured.
    pub fn open(data_dir: &str, source: Option<&str>) -> Self {
        let path = std::path::Path::new(data_dir).join("costs.json");
        let latest = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        Costs {
            state: std::sync::Mutex::new(State {
                latest,
                ..Default::default()
            }),
            path,
            source: source.map(std::path::PathBuf::from),
        }
    }

    /// Re-read `costs_file` when its modification time changed. A report that
    /// fails the checks replaces nothing it would show: the view says why.
    fn refresh(&self, st: &mut State) {
        let Some(src) = &self.source else { return };
        let mtime = match std::fs::metadata(src).and_then(|m| m.modified()) {
            Ok(t) => t,
            // Nothing pushed yet: the quiet empty state.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            // A denial (permissions, a unit sandbox hiding the path) must not
            // look like an empty result (#80). Name the error kind only.
            Err(e) => {
                let why = format!("cannot read the report: {}", e.kind());
                if st.rejected.as_deref() != Some(why.as_str()) {
                    tracing::warn!(%why, "costs_file rejected");
                }
                // A report already held (a POST, or an earlier read) is kept;
                // `seen` is cleared so the file is re-read once it is readable.
                st.seen = None;
                st.rejected = Some(why);
                return;
            }
        };
        if st.seen == Some(mtime) {
            return;
        }
        st.seen = Some(mtime);
        let at = mtime
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        if st.latest.as_ref().is_some_and(|l| l.received_at > at) {
            return; // a newer report arrived by POST
        }
        match read_report(src) {
            Ok(report) => {
                st.latest = Some(Stored {
                    received_at: at,
                    report,
                });
                st.rejected = None;
            }
            Err(why) => {
                tracing::warn!(%why, "costs_file rejected");
                st.latest = None;
                st.rejected = Some(why);
            }
        }
    }

    /// Replace the stored report. Errors when it cannot be persisted, so the
    /// pushing job sees a failure instead of a report that vanishes on restart.
    pub fn record(&self, report: CostReport, received_at: i64) -> std::io::Result<()> {
        let stored = Stored {
            received_at,
            report,
        };
        let b = serde_json::to_vec(&stored).map_err(std::io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, b)?;
        std::fs::rename(&tmp, &self.path)?;
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.latest = Some(stored);
        st.rejected = None;
        Ok(())
    }

    pub fn latest(&self) -> Option<Stored> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.refresh(&mut st);
        st.latest.clone()
    }

    /// Why the last report from `costs_file` was rejected, if it was.
    pub fn rejected(&self) -> Option<String> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.refresh(&mut st);
        st.rejected.clone()
    }
}

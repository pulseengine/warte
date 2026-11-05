//! Skills: which agent skills are actually used (REQ-SKILLS-001).
//!
//! A workstation job counts skill invocations in its local agent transcripts
//! and pushes one `skill-usage.v1` report, whole. Same design as Costs
//! (REQ-COSTS-001), two ways in with the same checks on both:
//!
//! - `[hub] skill_usage_file`: a file the job copies to the ground station
//!   over ssh. warte re-reads it when its modification time changes; that
//!   time is the report's `received_at`.
//! - `POST /api/skill-usage`, behind bearer-token auth and the `skills:write`
//!   right (REQ-API-004).
//!
//! The latest report is kept per `source` (an operator-chosen label, never a
//! hostname) next to the hub state and served to the Skills view and
//! `GET /api/skill-usage`.
//!
//! The read API is unauthenticated (warte#46), so a report holds counts and
//! names, never free text. Every struct denies unknown fields and every
//! string is checked against a strict character class. A report that carries
//! a prompt or a task description is rejected, not stored, and the reason
//! never quotes the input.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The report schema this build accepts.
pub const SCHEMA: &str = "skill-usage.v1";
/// The largest report accepted, in bytes.
pub const MAX_BODY: usize = 1024 * 1024;
/// A report older than this is shown as stale (the job runs nightly).
pub const STALE_S: i64 = 26 * 3600;
/// The most entries in any list of a report.
pub const MAX_LIST: usize = 1000;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub from: String,
    pub to: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Invocations {
    pub model: u64,
    pub user_slash: u64,
    pub subagent: u64,
}

impl Invocations {
    pub fn total(&self) -> u64 {
        self.model
            .saturating_add(self.user_slash)
            .saturating_add(self.subagent)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RepoUse {
    pub repo: String,
    pub invocations: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub name: String,
    pub plugin: String,
    pub invocations: Invocations,
    pub sessions: u64,
    pub skill_md_reads: u64,
    pub repos: Vec<RepoUse>,
}

/// One report, as the extractor pushes it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkillReport {
    pub schema: String,
    pub source: String,
    pub generated_at: String,
    pub window: Window,
    pub sessions_total: u64,
    pub sessions_with_skill: u64,
    pub installed: Vec<String>,
    pub skills: Vec<Skill>,
}

/// A source label: lowercase letters, digits and `-`, 1 to 32.
fn source_ok(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A skill, plugin or repository name: 1 to 64 of `[a-z0-9._:-]`. A task
/// description has spaces and capitals.
fn name_ok(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._:-".contains(c))
}

fn date(s: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

impl SkillReport {
    /// Check what serde cannot. The reason names the field, never the value:
    /// it is shown on the public read API, and the value may be exactly the
    /// text that must not be.
    pub fn validate(&self) -> Result<(), String> {
        let bad = |what: &str| Err(format!("{what} is not allowed"));
        if self.schema != SCHEMA {
            return Err(format!("the schema is not {SCHEMA}"));
        }
        if !source_ok(&self.source) {
            return bad("source");
        }
        if chrono::DateTime::parse_from_rfc3339(&self.generated_at).is_err() {
            return bad("generated_at");
        }
        match (date(&self.window.from), date(&self.window.to)) {
            (Some(f), Some(t)) if f <= t => {}
            _ => return bad("window"),
        }
        if self.sessions_with_skill > self.sessions_total {
            return bad("sessions_with_skill");
        }
        if self.installed.len() > MAX_LIST || self.skills.len() > MAX_LIST {
            return bad("a list of this length");
        }
        if !self.installed.iter().all(|n| name_ok(n)) {
            return bad("an installed name");
        }
        let mut seen = std::collections::BTreeSet::new();
        for s in &self.skills {
            if !name_ok(&s.name) {
                return bad("a skill name");
            }
            if !name_ok(&s.plugin) {
                return bad("a plugin name");
            }
            if !seen.insert((&s.plugin, &s.name)) {
                return bad("a skill listed twice");
            }
            if s.repos.len() > MAX_LIST {
                return bad("a list of this length");
            }
            if !s.repos.iter().all(|r| name_ok(&r.repo)) {
                return bad("a repository name");
            }
        }
        Ok(())
    }
}

/// Parse and check one report. serde's message can quote the input (a field
/// name, a string value): say only where and what kind of error.
pub fn parse(b: &[u8]) -> Result<SkillReport, String> {
    if b.len() > MAX_BODY {
        return Err(format!("the report is larger than {MAX_BODY} bytes"));
    }
    let r: SkillReport = serde_json::from_slice(b).map_err(|e| {
        format!(
            "not a skill-usage report ({:?} error at line {} column {})",
            e.classify(),
            e.line(),
            e.column()
        )
    })?;
    r.validate()?;
    Ok(r)
}

/// Read, parse and check one report file.
pub fn read_report(path: &std::path::Path) -> Result<SkillReport, String> {
    let b = std::fs::read(path).map_err(|e| format!("cannot read the report: {}", e.kind()))?;
    parse(&b)
}

/// The stored report and when it arrived.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Stored {
    pub received_at: i64,
    pub report: SkillReport,
}

#[derive(Default)]
struct State {
    /// Reports received by POST, one per source; persisted.
    posted: BTreeMap<String, Stored>,
    /// The report read from `skill_usage_file`, if it passed the checks.
    from_file: Option<Stored>,
    /// Why the last read of `skill_usage_file` was not accepted.
    rejected: Option<String>,
    /// Modification time of `skill_usage_file` when it was last read.
    seen: Option<std::time::SystemTime>,
}

/// The latest report per source, persisted next to the hub state as
/// `skill-usage.json`.
pub struct Skills {
    state: std::sync::Mutex<State>,
    path: std::path::PathBuf,
    source: Option<std::path::PathBuf>,
}

impl Skills {
    /// `data_dir`: where warte keeps its copy. `source`: the `[hub]
    /// skill_usage_file` a workstation job writes, if configured.
    pub fn open(data_dir: &str, source: Option<&str>) -> Self {
        let path = std::path::Path::new(data_dir).join("skill-usage.json");
        let posted = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Skills {
            state: std::sync::Mutex::new(State {
                posted,
                ..Default::default()
            }),
            path,
            source: source.map(std::path::PathBuf::from),
        }
    }

    /// Re-read `skill_usage_file` when its modification time changed. A
    /// report that fails the checks shows nothing of itself: the view says why.
    fn refresh(&self, st: &mut State) {
        let Some(src) = &self.source else { return };
        let mtime = match std::fs::metadata(src).and_then(|m| m.modified()) {
            Ok(t) => t,
            // Nothing pushed yet: the quiet empty state.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            // A denial must not look like an empty result (#80). Name the
            // error kind only.
            Err(e) => {
                let why = format!("cannot read the report: {}", e.kind());
                if st.rejected.as_deref() != Some(why.as_str()) {
                    tracing::warn!(%why, "skill_usage_file rejected");
                }
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
        match read_report(src) {
            Ok(report) => {
                st.from_file = Some(Stored {
                    received_at: at,
                    report,
                });
                st.rejected = None;
            }
            Err(why) => {
                tracing::warn!(%why, "skill_usage_file rejected");
                st.from_file = None;
                st.rejected = Some(why);
            }
        }
    }

    /// Replace the stored report of its source. Errors when it cannot be
    /// persisted, so the pushing job sees a failure instead of a report that
    /// vanishes on restart.
    pub fn record(&self, report: SkillReport, received_at: i64) -> std::io::Result<()> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = st.posted.clone();
        next.insert(
            report.source.clone(),
            Stored {
                received_at,
                report,
            },
        );
        let b = serde_json::to_vec(&next).map_err(std::io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, b)?;
        std::fs::rename(&tmp, &self.path)?;
        st.posted = next;
        Ok(())
    }

    /// The latest report of every source, by source. Where a source arrived
    /// both by file and by POST, the newer one.
    pub fn all(&self) -> Vec<Stored> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.refresh(&mut st);
        let mut m = st.posted.clone();
        if let Some(f) = &st.from_file {
            let newer = m
                .get(&f.report.source)
                .is_none_or(|p| p.received_at <= f.received_at);
            if newer {
                m.insert(f.report.source.clone(), f.clone());
            }
        }
        m.into_values().collect()
    }

    /// Why the last report from `skill_usage_file` was rejected, if it was.
    pub fn rejected(&self) -> Option<String> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.refresh(&mut st);
        st.rejected.clone()
    }
}

/// One skill summed over all sources, with its per-source parts.
#[derive(Debug, Clone)]
pub struct SkillTotal {
    pub plugin: String,
    pub name: String,
    pub invocations: Invocations,
    pub sessions: u64,
    pub skill_md_reads: u64,
    /// `(source, counts)`, by source.
    pub by_source: Vec<(String, Skill)>,
}

/// What the Skills view shows, computed from the stored reports.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    /// Most invoked first.
    pub skills: Vec<SkillTotal>,
    /// Installed on some source and invoked on none, by name.
    pub never_invoked: Vec<String>,
    pub sessions_total: u64,
    pub sessions_with_skill: u64,
}

pub fn summarize(all: &[Stored]) -> Summary {
    let mut by: BTreeMap<(String, String), SkillTotal> = BTreeMap::new();
    let mut s = Summary::default();
    let mut installed = std::collections::BTreeSet::new();
    for st in all {
        let r = &st.report;
        s.sessions_total = s.sessions_total.saturating_add(r.sessions_total);
        s.sessions_with_skill = s.sessions_with_skill.saturating_add(r.sessions_with_skill);
        installed.extend(r.installed.iter().cloned());
        for k in &r.skills {
            let t = by
                .entry((k.plugin.clone(), k.name.clone()))
                .or_insert_with(|| SkillTotal {
                    plugin: k.plugin.clone(),
                    name: k.name.clone(),
                    invocations: Invocations::default(),
                    sessions: 0,
                    skill_md_reads: 0,
                    by_source: Vec::new(),
                });
            t.invocations.model = t.invocations.model.saturating_add(k.invocations.model);
            t.invocations.user_slash = t
                .invocations
                .user_slash
                .saturating_add(k.invocations.user_slash);
            t.invocations.subagent = t
                .invocations
                .subagent
                .saturating_add(k.invocations.subagent);
            t.sessions = t.sessions.saturating_add(k.sessions);
            t.skill_md_reads = t.skill_md_reads.saturating_add(k.skill_md_reads);
            t.by_source.push((r.source.clone(), k.clone()));
        }
    }
    let used: std::collections::BTreeSet<&str> = by
        .values()
        .filter(|t| t.invocations.total() > 0)
        .map(|t| t.name.as_str())
        .collect();
    s.never_invoked = installed
        .iter()
        .filter(|n| !used.contains(n.as_str()))
        .cloned()
        .collect();
    s.skills = by.into_values().collect();
    s.skills
        .sort_by_key(|t| std::cmp::Reverse(t.invocations.total()));
    s
}

//! temper's compliance report, `temper-report.v1` (REQ-TEMPER-002).
//!
//! temper (the GitHub App on its own host) writes this file; that host's
//! enrolled probe sends it, signed, to the ground station. No free text: every
//! string is an enum, an identifier, an RFC 3339 time or hex, and anything
//! else rejects the whole report. warte's read API is unauthenticated (#46),
//! so nothing a repo author controls may pass through as prose.

use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "temper-report.v1";
/// Upper bounds temper's writer also enforces; beyond them the whole report
/// is rejected and nothing is stored.
pub const MAX_BYTES: usize = 1 << 20;
pub const MAX_REPOS: usize = 500;
pub const MAX_CHECKS: usize = 40;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: String,
    pub generated_at: String,
    pub temper: TemperBuild,
    pub org: String,
    pub source: Source,
    pub summary: Summary,
    pub repos: Vec<Repo>,
    pub org_checks: Vec<Check>,
    pub reviews: Reviews,
    pub tasks: Tasks,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TemperBuild {
    pub version: String,
    pub commit: String,
    pub config_sha256: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub trigger: Trigger,
    pub last_sweep_at: Option<String>,
    pub api: Api,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Trigger {
    Webhook,
    Sweep,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Api {
    pub calls_last_hour: u64,
    pub remaining: u64,
    pub limit: u64,
    pub reset_at: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub repos: u64,
    pub compliant: u64,
    pub noncompliant: u64,
    pub unknown: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Repo {
    pub name: String,
    pub checked_at: String,
    pub default_branch: String,
    pub checks: Vec<Check>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub id: CheckId,
    /// `None` when `code` is `unknown` or `error_*`.
    pub ok: Option<bool>,
    pub severity: Severity,
    pub code: Code,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckId {
    #[serde(rename = "protection.present")]
    ProtectionPresent,
    #[serde(rename = "protection.enforce_admins")]
    ProtectionEnforceAdmins,
    #[serde(rename = "protection.required_reviews")]
    ProtectionRequiredReviews,
    #[serde(rename = "protection.required_checks")]
    ProtectionRequiredChecks,
    #[serde(rename = "protection.signed_commits")]
    ProtectionSignedCommits,
    #[serde(rename = "protection.linear_history")]
    ProtectionLinearHistory,
    #[serde(rename = "ruleset.tag_v")]
    RulesetTagV,
    #[serde(rename = "merge.settings")]
    MergeSettings,
    #[serde(rename = "merge.auto_merge")]
    MergeAutoMerge,
    #[serde(rename = "merge.relaxation_open")]
    MergeRelaxationOpen,
    #[serde(rename = "dependabot.config")]
    DependabotConfig,
    #[serde(rename = "dependabot.config_valid")]
    DependabotConfigValid,
    #[serde(rename = "labels.synced")]
    LabelsSynced,
    #[serde(rename = "templates.present")]
    TemplatesPresent,
    #[serde(rename = "codeowners.present")]
    CodeownersPresent,
    #[serde(rename = "secret_scanning")]
    SecretScanning,
    #[serde(rename = "standards.drift")]
    StandardsDrift,
}

impl CheckId {
    /// The id as temper writes it.
    pub fn as_str(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    Ok,
    Missing,
    WeakerThanStandard,
    StricterThanStandard,
    DivergentDeclared,
    DivergentUndeclared,
    NotApplicable,
    ErrorApi,
    ErrorPermission,
    Unknown,
}

impl Code {
    pub fn as_str(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default()
    }
    fn undecided(&self) -> bool {
        matches!(self, Code::Unknown | Code::ErrorApi | Code::ErrorPermission)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Reviews {
    pub window_days: u64,
    pub posted: u64,
    pub suppressed_no_findings: u64,
    pub findings: u64,
    pub dropped_unquoted: u64,
    pub rivet_oracle_runs: u64,
    pub errors: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tasks {
    pub pending: u64,
    pub in_progress: u64,
    pub failed: u64,
    pub oldest_pending_age_s: u64,
    pub stale_in_progress: u64,
}

fn ident(s: &str) -> bool {
    (1..=100).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// RFC 3339 → unix seconds.
pub fn time(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp())
}

fn semver(s: &str) -> bool {
    let core = s.split(['-', '+']).next().unwrap_or("");
    s.len() <= 64
        && core.split('.').count() == 3
        && core
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
}

/// Parse and check a report. The reason names only where and what kind of
/// error, never the input (it may be shown on an unauthenticated page).
pub fn parse(body: &[u8]) -> Result<Report, String> {
    if body.len() > MAX_BYTES {
        return Err(format!("larger than {MAX_BYTES} bytes"));
    }
    let r: Report = serde_json::from_slice(body).map_err(|e| {
        format!(
            "not a {SCHEMA} report ({:?} error at line {} column {})",
            e.classify(),
            e.line(),
            e.column()
        )
    })?;
    r.validate()?;
    Ok(r)
}

impl Report {
    pub fn validate(&self) -> Result<(), String> {
        let bad = |what: &str| Err(format!("invalid {what}"));
        if self.schema != SCHEMA {
            return bad("schema");
        }
        if time(&self.generated_at).is_none() {
            return bad("generated_at");
        }
        if !semver(&self.temper.version) {
            return bad("temper.version");
        }
        if !hex(&self.temper.commit, 40) {
            return bad("temper.commit");
        }
        if !hex(&self.temper.config_sha256, 64) {
            return bad("temper.config_sha256");
        }
        if !ident(&self.org) {
            return bad("org");
        }
        if let Some(t) = &self.source.last_sweep_at {
            if time(t).is_none() {
                return bad("source.last_sweep_at");
            }
        }
        if time(&self.source.api.reset_at).is_none() {
            return bad("source.api.reset_at");
        }
        if self.repos.len() > MAX_REPOS {
            return bad("repos (too many)");
        }
        if self.org_checks.len() > MAX_CHECKS {
            return bad("org_checks (too many)");
        }
        let check = |c: &Check| {
            // `ok` is null exactly when the result is undecided
            c.ok.is_none() == c.code.undecided()
        };
        if !self.org_checks.iter().all(check) {
            return bad("org_checks (ok does not match code)");
        }
        for (i, r) in self.repos.iter().enumerate() {
            if !ident(&r.name) || !ident(&r.default_branch) || time(&r.checked_at).is_none() {
                return Err(format!("invalid repos[{i}]"));
            }
            if r.checks.len() > MAX_CHECKS {
                return Err(format!("invalid repos[{i}].checks (too many)"));
            }
            if !r.checks.iter().all(check) {
                return Err(format!(
                    "invalid repos[{i}].checks (ok does not match code)"
                ));
            }
        }
        Ok(())
    }

    /// Failed checks across repos and the org, worst first.
    pub fn failures(&self) -> Vec<(&str, &Check)> {
        let mut v: Vec<(&str, &Check)> = self
            .org_checks
            .iter()
            .map(|c| (self.org.as_str(), c))
            .chain(
                self.repos
                    .iter()
                    .flat_map(|r| r.checks.iter().map(move |c| (r.name.as_str(), c))),
            )
            .filter(|(_, c)| c.ok == Some(false))
            .collect();
        v.sort_by(|a, b| (a.1.severity, a.0, a.1.id).cmp(&(b.1.severity, b.0, b.1.id)));
        v
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A valid report, as temper's draft schema describes it.
    pub fn sample() -> serde_json::Value {
        serde_json::json!({
            "schema": "temper-report.v1",
            "generated_at": "2026-11-04T06:00:00Z",
            "temper": {"version": "2.4.0", "commit": "a".repeat(40), "config_sha256": "b".repeat(64)},
            "org": "pulseengine",
            "source": {"trigger": "webhook", "last_sweep_at": "2026-11-04T01:00:00Z",
                       "api": {"calls_last_hour": 12, "remaining": 4980, "limit": 5000, "reset_at": "2026-11-04T07:00:00Z"}},
            "summary": {"repos": 2, "compliant": 1, "noncompliant": 1, "unknown": 0},
            "repos": [
                {"name": "warte", "checked_at": "2026-11-04T05:59:00Z", "default_branch": "main",
                 "checks": [{"id": "protection.present", "ok": true, "severity": "critical", "code": "ok"}]},
                {"name": "spar", "checked_at": "2026-11-04T05:00:00Z", "default_branch": "main",
                 "checks": [
                    {"id": "protection.signed_commits", "ok": false, "severity": "high", "code": "missing"},
                    {"id": "dependabot.config", "ok": null, "severity": "medium", "code": "error_permission"}
                 ]}
            ],
            "org_checks": [{"id": "standards.drift", "ok": false, "severity": "medium", "code": "divergent_undeclared"}],
            "reviews": {"window_days": 7, "posted": 3, "suppressed_no_findings": 5, "findings": 4,
                        "dropped_unquoted": 1, "rivet_oracle_runs": 2, "errors": 0},
            "tasks": {"pending": 0, "in_progress": 0, "failed": 1, "oldest_pending_age_s": 0, "stale_in_progress": 0}
        })
    }

    fn try_with(f: impl FnOnce(&mut serde_json::Value)) -> Result<Report, String> {
        let mut v = sample();
        f(&mut v);
        parse(&serde_json::to_vec(&v).unwrap())
    }

    // rivet: verifies REQ-TEMPER-002
    #[test]
    fn the_draft_report_parses_and_failures_come_worst_first() {
        let r = parse(&serde_json::to_vec(&sample()).unwrap()).unwrap();
        let f = r.failures();
        assert_eq!(f.len(), 2, "an undecided check is not a failure");
        assert_eq!(
            (f[0].0, f[0].1.id.as_str().as_str()),
            ("spar", "protection.signed_commits")
        );
        assert_eq!(f[1].1.code.as_str(), "divergent_undeclared");
    }

    // rivet: verifies REQ-TEMPER-002
    #[test]
    fn anything_but_enums_identifiers_times_and_hex_is_refused() {
        type Edit = fn(&mut serde_json::Value);
        let cases: Vec<(&str, Edit)> = vec![
            ("unknown field", |v| v["note"] = "hello".into()),
            ("unknown nested field", |v| {
                v["repos"][0]["title"] = "x".into()
            }),
            ("schema", |v| v["schema"] = "temper-report.v2".into()),
            ("free text name", |v| {
                v["repos"][0]["name"] = "<script>alert(1)</script>".into()
            }),
            ("branch with space", |v| {
                v["repos"][0]["default_branch"] = "my branch".into()
            }),
            ("unknown check id", |v| {
                v["repos"][0]["checks"][0]["id"] = "protection.whatever".into()
            }),
            ("unknown code", |v| {
                v["repos"][0]["checks"][0]["code"] = "fine".into()
            }),
            ("unknown severity", |v| {
                v["org_checks"][0]["severity"] = "urgent".into()
            }),
            ("ok with an error code", |v| {
                v["repos"][1]["checks"][1]["ok"] = true.into()
            }),
            ("null ok with a decided code", |v| {
                v["repos"][0]["checks"][0]["ok"] = serde_json::Value::Null
            }),
            ("bad time", |v| v["generated_at"] = "yesterday".into()),
            ("short commit", |v| v["temper"]["commit"] = "abc".into()),
            ("version text", |v| {
                v["temper"]["version"] = "latest and greatest".into()
            }),
            ("negative count", |v| v["tasks"]["failed"] = (-1).into()),
            ("trigger", |v| v["source"]["trigger"] = "manual".into()),
        ];
        for (what, edit) in cases {
            let e = try_with(edit).expect_err(what);
            assert!(
                !e.contains("script") && !e.contains("hello"),
                "{what}: reason quotes the input: {e}"
            );
        }
        // caps
        assert!(try_with(|v| {
            let r = v["repos"][0].clone();
            v["repos"] = serde_json::Value::Array(vec![r; MAX_REPOS + 1]);
        })
        .is_err());
        assert!(try_with(|v| {
            let c = v["repos"][0]["checks"][0].clone();
            v["repos"][0]["checks"] = serde_json::Value::Array(vec![c; MAX_CHECKS + 1]);
        })
        .is_err());
        assert!(parse(&vec![b' '; MAX_BYTES + 1]).is_err());
        // a null last_sweep_at (never swept yet) is fine
        assert!(try_with(|v| v["source"]["last_sweep_at"] = serde_json::Value::Null).is_ok());
    }
}

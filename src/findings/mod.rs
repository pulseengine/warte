//! Security findings from hosts (REQ-FINDINGS-001, warte#114). Detectors on a
//! host (a baseline-drift check, AIDE, Falco) write `security-findings.v1`;
//! the host's enrolled probe sends it as the signed `security` feed. warte
//! stores the latest set per host, shows the open ones, lets the owner accept
//! a known state into the baseline, and raises high findings and a silent
//! detector on the glance. warte detects nothing itself.
//!
//! Metadata only: the schema has no field for file contents or secret values,
//! and unknown fields are refused.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

pub const SCHEMA: &str = "security-findings.v1";
pub const MAX_FINDINGS: usize = 500;
/// A host whose last findings report is older than this is a silent detector.
pub const SILENT_S: i64 = 36 * 3600;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    /// drift, aide, falco or other.
    pub source: String,
    pub rule: String,
    pub severity: Severity,
    /// A path, a key fingerprint, a port: what the finding is about.
    pub subject: String,
    pub first_seen: String,
    pub last_seen: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: String,
    pub generated_at: String,
    pub findings: Vec<Finding>,
}

fn short(s: &str, max: usize) -> bool {
    !s.is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}

/// Parse and check a findings report. An error names the field.
pub fn parse(data: &[u8]) -> Result<Report, String> {
    let r: Report = serde_json::from_slice(data).map_err(|e| {
        let m = e.to_string();
        format!("schema: {}", m.split('`').next().unwrap_or(&m).trim_end())
    })?;
    if r.schema != SCHEMA {
        return Err(format!("schema: must be {SCHEMA}"));
    }
    if r.findings.len() > MAX_FINDINGS {
        return Err(format!("findings: at most {MAX_FINDINGS}; summarize"));
    }
    for (i, f) in r.findings.iter().enumerate() {
        if !["drift", "aide", "falco", "other"].contains(&f.source.as_str()) {
            return Err(format!("findings[{i}].source: drift, aide, falco or other"));
        }
        if !short(&f.rule, 80) {
            return Err(format!("findings[{i}].rule: 1 to 80 characters"));
        }
        if !short(&f.subject, 200) {
            return Err(format!("findings[{i}].subject: 1 to 200 characters"));
        }
        for (k, v) in [("first_seen", &f.first_seen), ("last_seen", &f.last_seen)] {
            if chrono::DateTime::parse_from_rfc3339(v).is_err() {
                return Err(format!("findings[{i}].{k}: not RFC 3339"));
            }
        }
    }
    Ok(r)
}

/// A stored finding row: host, source, rule, subject, severity, first and
/// last seen, count.
type Row = (String, String, String, String, String, String, String, i64);

/// One open finding of one host.
#[derive(Debug, Clone, Serialize)]
pub struct Open {
    pub host: String,
    #[serde(flatten)]
    pub finding: Finding,
}

/// The findings store.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

fn db(e: sqlx::Error) -> String {
    format!("database error: {e}")
}

impl Store {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Replace `host`'s findings with `r`, sent at `at`. Refused (false) when
    /// `at` is not newer than the host's last report: a replay.
    pub async fn record(&self, host: &str, at: i64, now: i64, r: &Report) -> Result<bool, String> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let last: Option<(i64,)> = sqlx::query_as("SELECT at FROM security_reports WHERE host = ?")
            .bind(host)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
        if last.is_some_and(|(l,)| at <= l) {
            return Ok(false);
        }
        sqlx::query("DELETE FROM security_findings WHERE host = ?")
            .bind(host)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        for f in &r.findings {
            sqlx::query(
                "INSERT OR REPLACE INTO security_findings (host, source, rule, subject, severity, first_seen, last_seen, count) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(host)
            .bind(&f.source)
            .bind(&f.rule)
            .bind(&f.subject)
            .bind(serde_json::to_value(f.severity).ok().and_then(|v| v.as_str().map(String::from)))
            .bind(&f.first_seen)
            .bind(&f.last_seen)
            .bind(i64::try_from(f.count).unwrap_or(i64::MAX))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        }
        sqlx::query(
            "INSERT INTO security_reports (host, at, received_at) VALUES (?, ?, ?) ON CONFLICT(host) DO UPDATE SET at = excluded.at, received_at = excluded.received_at",
        )
        .bind(host)
        .bind(at)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(true)
    }

    /// Open findings (not in the baseline), highest severity, then newest first.
    pub async fn open(&self) -> Vec<Open> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT f.host, f.source, f.rule, f.subject, f.severity, f.first_seen, f.last_seen, f.count
             FROM security_findings f LEFT JOIN security_baseline b
               ON b.host = f.host AND b.source = f.source AND b.rule = f.rule AND b.subject = f.subject
             WHERE b.host IS NULL",
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        let mut out: Vec<Open> = rows
            .into_iter()
            .filter_map(
                |(host, source, rule, subject, sev, first_seen, last_seen, count)| {
                    Some(Open {
                        host,
                        finding: Finding {
                            source,
                            rule,
                            subject,
                            severity: serde_json::from_value(serde_json::Value::from(sev)).ok()?,
                            first_seen,
                            last_seen,
                            count: u64::try_from(count).unwrap_or(0),
                        },
                    })
                },
            )
            .collect();
        out.sort_by(|a, b| {
            (b.finding.severity, &b.finding.last_seen)
                .cmp(&(a.finding.severity, &a.finding.last_seen))
        });
        out
    }

    /// Hosts whose detector went silent: last report older than [`SILENT_S`].
    pub async fn silent(&self, now: i64) -> Vec<String> {
        sqlx::query_as::<_, (String,)>(
            "SELECT host FROM security_reports WHERE received_at < ? ORDER BY host",
        )
        .bind(now - SILENT_S)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(h,)| h)
        .collect()
    }

    /// Accept one finding into the host's baseline; false if it is not open.
    pub async fn accept(
        &self,
        host: &str,
        source: &str,
        rule: &str,
        subject: &str,
        by: &str,
    ) -> Result<bool, String> {
        let exists: Option<(String,)> = sqlx::query_as(
            "SELECT host FROM security_findings WHERE host = ? AND source = ? AND rule = ? AND subject = ?",
        )
        .bind(host)
        .bind(source)
        .bind(rule)
        .bind(subject)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        if exists.is_none() {
            return Ok(false);
        }
        sqlx::query(
            "INSERT OR REPLACE INTO security_baseline (host, source, rule, subject, accepted_by, accepted_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(host)
        .bind(source)
        .bind(rule)
        .bind(subject)
        .bind(by)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(true)
    }
}

/// The Security view: open findings per host, highest first, with an
/// accept button; silent detectors on top.
pub fn view(open: &[Open], silent: &[String], csrf: Option<&str>) -> String {
    let esc = crate::dashboard::fleet::esc;
    let silent_note = if silent.is_empty() {
        String::new()
    } else {
        format!(
            r#"<p class="warn">No findings report for over 36 hours from: {}. The detector may have stopped.</p>"#,
            esc(&silent.join(", "))
        )
    };
    let rows: String = open
        .iter()
        .map(|o| {
            let f = &o.finding;
            let sev = serde_json::to_value(f.severity).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
            let accept = csrf
                .map(|_| {
                    format!(
                        r#"<button type="button" class="btn-small" data-host="{}" data-source="{}" data-rule="{}" data-subject="{}">Accept</button>"#,
                        esc(&o.host),
                        esc(&f.source),
                        esc(&f.rule),
                        esc(&f.subject)
                    )
                })
                .unwrap_or_default();
            format!(
                r#"<tr class="{}"><td>{}</td><td>{}</td><td>{}</td><td>{} · {}</td><td><code>{}</code></td><td>{} ×{}</td><td>{accept}</td></tr>"#,
                if f.severity >= Severity::High { "warnrow" } else { "" },
                esc(&o.host),
                esc(&sev),
                esc(&f.source),
                esc(&f.rule),
                esc(&f.first_seen),
                esc(&f.subject),
                esc(&f.last_seen),
                f.count,
            )
        })
        .collect();
    let body = if open.is_empty() {
        r#"<p class="empty">No open findings.</p>"#.to_string()
    } else {
        format!(
            r#"<div class="scroll"><table class="data-table"><thead><tr><th>Host</th><th>Severity</th><th>Source</th><th>Rule · first seen</th><th>Subject</th><th>Last seen</th><th></th></tr></thead><tbody>{rows}</tbody></table></div>"#
        )
    };
    let script = csrf
        .map(|t| {
            format!(
                r#"<script>
(function () {{
  var root = document.getElementById("findings");
  if (!root) return;
  root.querySelectorAll("button[data-host]").forEach(function (b) {{
    b.addEventListener("click", function () {{
      fetch("api/security/accept", {{method: "POST", headers: {{"Content-Type": "application/json", "X-Warte-CSRF": "{}"}},
        body: JSON.stringify({{host: b.dataset.host, source: b.dataset.source, rule: b.dataset.rule, subject: b.dataset.subject}})}})
        .then(function (r) {{ if (r.ok) b.closest("tr").remove(); else r.text().then(function (t) {{ b.textContent = t; }}); }});
    }});
  }});
}})();
</script>"#,
                esc(t)
            )
        })
        .unwrap_or_default();
    format!(
        r#"<div class="overview findings" id="findings"><h2>Security findings</h2><p>Reported by each host's detectors through its probe. Accepting a finding adds it to that host's baseline, so it stops alerting.</p>{silent_note}{body}{script}</div>"#
    )
}

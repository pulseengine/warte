//! Reports from producers (REQ-REPORTS-001, warte#87, seam
//! pulseengine-warte/reports): `POST /api/reports/<kind>` with a token holding
//! `reports:write:<kind>`, checked against the kind's schema string and the
//! fields warte renders, latest wins. The views render; warte computes
//! nothing and calls no API for them.
//!
//! Kinds: `seam-check` (schema seam-check.v1), the Seams view (#76).

use serde_json::Value;
use sqlx::SqlitePool;

/// The largest report accepted.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

/// A report kind: its URL name, the schema string it must carry, and how
/// many days until the view flags it stale.
#[derive(Debug, Clone, Copy)]
pub struct Kind {
    pub name: &'static str,
    pub schema: &'static str,
    pub stale_days: i64,
}

pub const SEAM_CHECK: Kind = Kind {
    name: "seam-check",
    schema: "seam-check.v1",
    stale_days: 8,
};

pub const KINDS: [Kind; 1] = [SEAM_CHECK];

/// The right a producer's token needs to post `kind`.
pub fn write_right(kind: &Kind) -> String {
    format!("reports:write:{}", kind.name)
}

/// Check a report of `kind`: its schema string and the fields warte renders.
/// An error names the field, never a value.
pub fn check(kind: &Kind, body: &[u8]) -> Result<Value, String> {
    if body.len() > MAX_BYTES {
        return Err(format!("the report is larger than {MAX_BYTES} bytes"));
    }
    let v: Value =
        serde_json::from_slice(body).map_err(|_| "the report is not JSON".to_string())?;
    if v["schema"] != kind.schema {
        return Err(format!("schema: must be {}", kind.schema));
    }
    if kind.name == SEAM_CHECK.name {
        if v["now"].as_str().is_none() {
            return Err("now: missing".into());
        }
        let seams = v["seams"].as_array().ok_or("seams: not a list")?;
        for (i, s) in seams.iter().enumerate() {
            if !s["source"]["repo"].is_string() {
                return Err(format!("seams[{i}].source.repo: missing"));
            }
            for k in ["violations", "warnings"] {
                if !s[k].is_array() {
                    return Err(format!("seams[{i}].{k}: not a list"));
                }
            }
        }
        for k in ["issues_without_block", "errors"] {
            if !v[k].is_array() {
                return Err(format!("{k}: not a list"));
            }
        }
        if !v["exit"].is_i64() {
            return Err("exit: not a number".into());
        }
    }
    Ok(v)
}

/// The latest reports, in warte's database.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn put(&self, kind: &Kind, v: &Value) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO reports (kind, body, received_at) VALUES (?, ?, ?)
             ON CONFLICT(kind) DO UPDATE SET body = excluded.body, received_at = excluded.received_at",
        )
        .bind(kind.name)
        .bind(v.to_string())
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| format!("database error: {e}"))
    }

    /// The latest report of `kind` and when it arrived (unix seconds).
    pub async fn get(&self, kind: &Kind) -> Option<(Value, i64)> {
        let (body, at): (String, String) =
            sqlx::query_as("SELECT body, received_at FROM reports WHERE kind = ?")
                .bind(kind.name)
                .fetch_optional(&self.pool)
                .await
                .ok()??;
        let at = chrono::DateTime::parse_from_rfc3339(&at).ok()?.timestamp();
        Some((serde_json::from_str(&body).ok()?, at))
    }
}

/// The Seams view (#76): every seam with its state, violations first, then
/// warnings, then the passing ones; issues labelled seam without a block;
/// the checker's own errors. Stale after the kind's days.
pub fn seams_view(report: Option<&(Value, i64)>, now: i64) -> String {
    let esc = crate::dashboard::fleet::esc;
    let Some((v, at)) = report else {
        return r#"<div class="overview"><h2>Seams</h2><p class="empty">No seam-check report yet: the weekly seam-check workflow posts it (<code>POST /api/reports/seam-check</code>).</p></div>"#.into();
    };
    let age_days = (now - at) / 86_400;
    let stale = age_days > SEAM_CHECK.stale_days;
    let mut seams: Vec<&Value> = v["seams"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let rank = |s: &Value| {
        if s["violations"].as_array().is_some_and(|a| !a.is_empty()) {
            0
        } else if s["warnings"].as_array().is_some_and(|a| !a.is_empty()) {
            1
        } else {
            2
        }
    };
    seams.sort_by_key(|s| (rank(s), s["id"].as_str().unwrap_or("").to_string()));
    let msgs = |s: &Value, k: &str| -> String {
        s[k].as_array()
            .map(|a| {
                a.iter()
                    .map(|m| {
                        format!(
                            "<li>{}: {}</li>",
                            esc(m["rule"].as_str().unwrap_or("")),
                            esc(m["message"].as_str().unwrap_or(""))
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let rows: String = seams
        .iter()
        .map(|s| {
            let src = &s["source"];
            let link = format!(
                r#"<a href="{}">{}#{}</a>"#,
                esc(src["url"].as_str().unwrap_or("")),
                esc(src["repo"].as_str().unwrap_or("")),
                src["issue"].as_u64().unwrap_or(0)
            );
            let consumers: Vec<String> = s["consumers"]
                .as_array()
                .map(|c| c.iter().filter_map(|x| x["repo"].as_str().map(esc)).collect())
                .unwrap_or_default();
            let questions: String = s["open_questions"]
                .as_array()
                .map(|q| {
                    q.iter()
                        .map(|x| format!("<li>{} <span class=\"dim\">({})</span></li>", esc(x["q"].as_str().unwrap_or("")), esc(x["owner"].as_str().unwrap_or(""))))
                        .collect()
                })
                .unwrap_or_default();
            format!(
                r#"<tr class="{}"><td><b>{}</b><br>{link}</td><td>{}</td><td>{}</td><td>{}</td><td><ul>{}{}</ul>{}</td></tr>"#,
                match rank(s) {
                    0 => "warnrow",
                    1 => "amberrow",
                    _ => "",
                },
                esc(s["id"].as_str().unwrap_or("(no id)")),
                esc(s["status"].as_str().unwrap_or("")),
                esc(s["provider"]["repo"].as_str().unwrap_or("")),
                consumers.join(", "),
                msgs(s, "violations"),
                msgs(s, "warnings"),
                if questions.is_empty() { String::new() } else { format!("<p>Open questions:</p><ul>{questions}</ul>") },
            )
        })
        .collect();
    let errors: String = v["errors"]
        .as_array()
        .map(|e| {
            e.iter()
                .filter_map(Value::as_str)
                .map(|x| format!("<li>{}</li>", esc(x)))
                .collect()
        })
        .unwrap_or_default();
    format!(
        r#"<div class="overview seams"><h2>Seams</h2><p>From seam-check, generated {} ({} day(s) ago){}; exit {}.</p>{}<div class="scroll"><table class="data-table"><thead><tr><th>Seam</th><th>Status</th><th>Provider</th><th>Consumers</th><th>Findings</th></tr></thead><tbody>{rows}</tbody></table></div></div>"#,
        esc(v["now"].as_str().unwrap_or("")),
        age_days,
        if stale {
            r#" <span class="pill warn">stale</span>"#
        } else {
            ""
        },
        v["exit"].as_i64().unwrap_or(-1),
        if errors.is_empty() {
            String::new()
        } else {
            format!("<p>The checker reported:</p><ul>{errors}</ul>")
        },
    )
}

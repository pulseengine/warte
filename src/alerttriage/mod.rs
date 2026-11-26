//! Agent triage of alerts before they page (REQ-ALERT-006, warte#219,
//! `warte.alert-triage.v1`).
//!
//! Every opened or escalated alert is stored with the context a triage agent
//! reads: the other open findings, recent CI runs naming the same repo or
//! pool, and this key's earlier verdicts. The agent runs outside warte (warte
//! never holds a model credential): a deployment timer polls
//! `GET /api/alerts/triage/pending` and starts a headless run only when the
//! list is non-empty; the run posts one verdict per alert. Each poll is the
//! runner's heartbeat. The owner marks alerts Real or Noise on the Queue view.
//!
//! In shadow mode (the default) paging is unchanged and verdicts are only
//! stored and measured. In gate mode a non-red alert is held until the agent
//! calls it real (it pages) or noise (it stays in the notice digest only).
//! Red alerts and escalations always page; an alert with no verdict after
//! [`HOLD_S`] pages; and while the runner has not polled for
//! [`RUNNER_STALE_S`], everything pages as before.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;

pub const SCHEMA: &str = "warte.alert-triage.v1";
/// A non-red alert without a verdict pages after this long (gate mode).
pub const HOLD_S: i64 = 1800;
/// A runner silent this long no longer holds anything back (gate mode).
pub const RUNNER_STALE_S: i64 = 1800;
/// The switch to gate mode is judged only on this many owner-labelled alerts,
/// with at least [`MIN_REAL`] the owner called real.
pub const MIN_LABELLED: u64 = 30;
pub const MIN_REAL: u64 = 5;
/// The most alerts one poll returns, and the longest reason kept.
pub const MAX_PENDING: i64 = 50;
pub const MAX_REASON: usize = 200;
/// Pending alerts older than this are not offered any more.
pub const PENDING_FOR_S: i64 = 86_400;

/// `[alert_triage]` in warte.toml.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriageConfig {
    #[serde(default)]
    pub mode: Mode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Shadow,
    Gate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Real,
    Noise,
}

impl Verdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Real => "real",
            Verdict::Noise => "noise",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "real" => Some(Verdict::Real),
            "noise" => Some(Verdict::Noise),
            _ => None,
        }
    }
}

/// What gate mode does with one alert's push now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    Now,
    Hold,
    Drop,
}

/// Push, hold or drop an opened alert. Pure. Shadow mode, red alerts and
/// escalations always push; a real verdict pushes, a noise verdict drops;
/// without a verdict it holds until [`HOLD_S`], and never while the runner
/// is stale or has never polled.
pub fn decide(
    mode: Mode,
    sev: u8,
    escalated: bool,
    verdict: Option<Verdict>,
    opened_at: i64,
    now: i64,
    last_poll: Option<i64>,
) -> Push {
    if mode == Mode::Shadow || sev >= 2 || escalated {
        return Push::Now;
    }
    match verdict {
        Some(Verdict::Real) => Push::Now,
        Some(Verdict::Noise) => Push::Drop,
        None => {
            let runner_stale = last_poll.is_none_or(|p| now - p >= RUNNER_STALE_S);
            if runner_stale || now - opened_at >= HOLD_S {
                Push::Now
            } else {
                Push::Hold
            }
        }
    }
}

/// One stored alert with its verdicts.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub id: String,
    pub key: String,
    pub sev: u8,
    pub text: String,
    pub opened_at: i64,
    pub verdict: Option<Verdict>,
    pub reason: Option<String>,
    pub owner: Option<Verdict>,
}

/// Agreement between the agent and the owner, with denominators.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Measures {
    /// Alerts with both an agent verdict and an owner signal.
    pub labelled: u64,
    pub agree: u64,
    /// The agent said noise, the owner said real: the critical number.
    pub missed_real: u64,
    pub owner_real: u64,
    pub owner_noise: u64,
    /// Agent noise among owner noise.
    pub noise_caught: u64,
    /// Alerts with a verdict and no owner signal, and without a verdict.
    pub unlabelled: u64,
    pub no_verdict: u64,
    /// The keys the agent got wrong, most frequent first.
    pub wrong_keys: Vec<(String, u64)>,
}

impl Measures {
    /// Enough owner labels to judge the switch to gate mode.
    pub fn judgeable(&self) -> bool {
        self.labelled >= MIN_LABELLED && self.owner_real >= MIN_REAL
    }
}

pub fn measures(rows: &[Row]) -> Measures {
    let mut m = Measures::default();
    let mut wrong: std::collections::BTreeMap<String, u64> = Default::default();
    for r in rows {
        match (r.verdict, r.owner) {
            (None, _) => m.no_verdict += 1,
            (Some(_), None) => m.unlabelled += 1,
            (Some(a), Some(o)) => {
                m.labelled += 1;
                match o {
                    Verdict::Real => m.owner_real += 1,
                    Verdict::Noise => m.owner_noise += 1,
                }
                if a == o {
                    m.agree += 1;
                    if o == Verdict::Noise {
                        m.noise_caught += 1;
                    }
                } else {
                    *wrong.entry(r.key.clone()).or_default() += 1;
                    if o == Verdict::Real {
                        m.missed_real += 1;
                    }
                }
            }
        }
    }
    let mut w: Vec<(String, u64)> = wrong.into_iter().collect();
    w.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    m.wrong_keys = w;
    m
}

/// The recent runs naming the alert's repo or pool, newest first, at most
/// ten. No runner names and no host part of an error signature.
pub fn runs_for(recs: &[crate::hub::model::JobRecord], key: &str, text: &str) -> Vec<Value> {
    let names = |s: &str| !s.is_empty() && (key.contains(s) || text.contains(s));
    let mut v: Vec<&crate::hub::model::JobRecord> = recs
        .iter()
        .filter(|r| names(&r.repo) || names(&r.pool))
        .collect();
    v.sort_by_key(|r| std::cmp::Reverse(r.c));
    v.into_iter()
        .take(10)
        .map(|r| {
            json!({
                "repo": r.repo,
                "workflow": r.wf,
                "job": r.job,
                "pool": r.pool,
                "conclusion": r.r,
                "created_at": r.c,
                "signature": r.sig.as_deref().map(|s| s.split(':').next().unwrap_or_default()),
            })
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum VerdictError {
    NotFound,
    Already,
}

pub struct Store {
    pool: SqlitePool,
}

fn row(r: RawRow) -> Row {
    Row {
        id: r.0,
        key: r.1,
        sev: r.2.clamp(0, 2) as u8,
        text: r.3,
        opened_at: r.4,
        verdict: r.5.as_deref().and_then(Verdict::parse),
        reason: r.6,
        owner: r.7.as_deref().and_then(Verdict::parse),
    }
}

type RawRow = (
    String,
    String,
    i64,
    String,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
);

impl Store {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Store an opened or escalated alert; a repeated id is ignored.
    pub async fn record(
        &self,
        id: &str,
        key: &str,
        sev: u8,
        text: &str,
        opened_at: i64,
        context: &Value,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT OR IGNORE INTO alert_triage (id, key, sev, text, opened_at, context) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(key)
        .bind(sev as i64)
        .bind(text)
        .bind(opened_at)
        .bind(context.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Alerts without a verdict from the last day, oldest first, with context.
    pub async fn pending(&self, now: i64) -> Vec<Value> {
        let rows: Vec<(String, String, i64, String, i64, String)> = sqlx::query_as(
            "SELECT id, key, sev, text, opened_at, context FROM alert_triage WHERE verdict IS NULL AND opened_at >= ? ORDER BY opened_at LIMIT ?",
        )
        .bind(now - PENDING_FOR_S)
        .bind(MAX_PENDING)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        rows.into_iter()
            .map(|(id, key, sev, text, at, ctx)| {
                json!({
                    "id": id,
                    "key": key,
                    "sev": sev,
                    "text": text,
                    "opened_at": at,
                    "context": serde_json::from_str::<Value>(&ctx).unwrap_or(Value::Null),
                })
            })
            .collect()
    }

    pub async fn verdict(
        &self,
        id: &str,
        v: Verdict,
        reason: &str,
        model: &str,
        now: i64,
    ) -> Result<(), VerdictError> {
        let r = sqlx::query(
            "UPDATE alert_triage SET verdict = ?, reason = ?, model = ?, verdict_at = ? WHERE id = ? AND verdict IS NULL",
        )
        .bind(v.as_str())
        .bind(reason)
        .bind(model)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|_| VerdictError::NotFound)?;
        if r.rows_affected() == 1 {
            return Ok(());
        }
        let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM alert_triage WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten();
        Err(if exists.is_some() {
            VerdictError::Already
        } else {
            VerdictError::NotFound
        })
    }

    /// The owner's signal; the owner may change it.
    pub async fn owner(&self, id: &str, v: Verdict, by: &str, now: i64) -> bool {
        sqlx::query("UPDATE alert_triage SET owner = ?, owner_by = ?, owner_at = ? WHERE id = ?")
            .bind(v.as_str())
            .bind(by)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await
            .is_ok_and(|r| r.rows_affected() == 1)
    }

    pub async fn verdict_of(&self, id: &str) -> Option<Verdict> {
        let v: Option<(Option<String>,)> =
            sqlx::query_as("SELECT verdict FROM alert_triage WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .ok()
                .flatten();
        v.and_then(|(s,)| s).as_deref().and_then(Verdict::parse)
    }

    pub async fn polled(&self, now: i64) {
        let _ = sqlx::query(
            "INSERT INTO alert_triage_runner (id, last_poll) VALUES (1, ?) ON CONFLICT(id) DO UPDATE SET last_poll = excluded.last_poll",
        )
        .bind(now)
        .execute(&self.pool)
        .await;
    }

    pub async fn last_poll(&self) -> Option<i64> {
        sqlx::query_as::<_, (i64,)>("SELECT last_poll FROM alert_triage_runner WHERE id = 1")
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten()
            .map(|(t,)| t)
    }

    /// Every stored alert since `since`, newest first.
    pub async fn rows(&self, since: i64) -> Vec<Row> {
        sqlx::query_as::<_, RawRow>(
            "SELECT id, key, sev, text, opened_at, verdict, reason, owner FROM alert_triage WHERE opened_at >= ? ORDER BY opened_at DESC",
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(row)
        .collect()
    }

    /// This key's earlier alerts and their verdicts, at most ten.
    pub async fn history(&self, key: &str, since: i64) -> Vec<Value> {
        sqlx::query_as::<_, RawRow>(
            "SELECT id, key, sev, text, opened_at, verdict, reason, owner FROM alert_triage WHERE key = ? AND opened_at >= ? ORDER BY opened_at DESC LIMIT 10",
        )
        .bind(key)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(row)
        .map(|r| {
            json!({
                "opened_at": r.opened_at,
                "verdict": r.verdict,
                "owner": r.owner,
            })
        })
        .collect()
    }
}

/// The finding key and severity of a recorded alert notice, and whether it
/// escalated; `None` for a clear or the summary notice.
pub fn opened(n: &Value) -> Option<(String, u8, bool)> {
    let word = n["tags"].get(1).and_then(Value::as_str)?;
    let sev = match word {
        "down" => 2,
        "degraded" => 1,
        _ => return None,
    };
    let key = n["body"]
        .as_str()?
        .rsplit_once("\n\nfinding: ")
        .map(|(_, k)| k.to_string())?;
    if key == "summary" {
        return None;
    }
    let escalated = n["title"]
        .as_str()
        .is_some_and(|t| t.starts_with("Now down: "));
    Some((key, sev, escalated))
}

/// The cleared finding key of a notice, if it is a clear.
pub fn cleared(n: &Value) -> Option<String> {
    (n["tags"].get(1).and_then(Value::as_str) == Some("cleared"))
        .then(|| {
            n["body"]
                .as_str()?
                .rsplit_once("\n\nfinding: ")
                .map(|(_, k)| k.to_string())
        })
        .flatten()
}

/// The Queue view's panel: recent alerts with the agent's verdict, the
/// owner's one-tap signal, and the measures.
pub fn view(
    rows: &[Row],
    mode: Mode,
    last_poll: Option<i64>,
    now: i64,
    owner_csrf: Option<&str>,
) -> String {
    use crate::dashboard::fleet::esc;
    let m = measures(rows);
    let pct = |a: u64, b: u64| {
        (a * 100)
            .checked_div(b)
            .map_or_else(|| "–".to_string(), |p| format!("{p}% ({a}/{b})"))
    };
    let runner = match last_poll {
        Some(p) if now - p < RUNNER_STALE_S => format!("runner polled {} min ago", (now - p) / 60),
        Some(p) => format!("runner silent for {} min", (now - p) / 60),
        None => "runner never polled".to_string(),
    };
    let wrong = if m.wrong_keys.is_empty() {
        String::new()
    } else {
        format!(
            "<p class=\"dim\">Wrong on: {}</p>",
            m.wrong_keys
                .iter()
                .take(10)
                .map(|(k, n)| format!("{} ×{n}", esc(k)))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let missed_class = if m.missed_real > 0 { " bad" } else { "" };
    let measures = format!(
        r#"<div class="kpis"><div class="kpi{missed_class}"><b>{}</b> missed real <span class="dim">of {} owner-real</span></div><div class="kpi"><b>{}</b> noise caught</div><div class="kpi"><b>{}</b> agreement</div><div class="kpi"><b>{}</b> unlabelled · <b>{}</b> without a verdict</div></div>{wrong}<p class="dim">{}: judged at ≥ {MIN_LABELLED} owner-labelled with ≥ {MIN_REAL} owner-real ({}). {}.</p>"#,
        m.missed_real,
        m.owner_real,
        pct(m.noise_caught, m.owner_noise),
        pct(m.agree, m.labelled),
        m.unlabelled,
        m.no_verdict,
        if mode == Mode::Gate {
            "Gate mode"
        } else {
            "Shadow mode, every alert pages"
        },
        if m.judgeable() {
            "enough labels"
        } else {
            "not yet"
        },
        runner,
    );
    let mut trs = String::new();
    for r in rows.iter().take(20) {
        let agent = match (&r.verdict, &r.reason) {
            (Some(v), reason) => format!(
                "{} <span class=\"dim\">{}</span>",
                v.as_str(),
                esc(reason.as_deref().unwrap_or_default())
            ),
            (None, _) => "<span class=\"dim\">pending</span>".to_string(),
        };
        let owner = match (owner_csrf, r.owner) {
            (Some(_), cur) => {
                let b = |v: Verdict, label: &str| {
                    let on = if cur == Some(v) {
                        " aria-pressed=\"true\""
                    } else {
                        ""
                    };
                    format!(
                        r#"<button data-triage-id="{}" data-triage-verdict="{}"{on}>{label}</button>"#,
                        esc(&r.id),
                        v.as_str()
                    )
                };
                format!("{}{}", b(Verdict::Real, "Real"), b(Verdict::Noise, "Noise"))
            }
            (None, Some(v)) => v.as_str().to_string(),
            (None, None) => String::new(),
        };
        let when = chrono::DateTime::from_timestamp(r.opened_at, 0)
            .map(|d| d.format("%d %b %H:%M").to_string())
            .unwrap_or_default();
        let sev = if r.sev >= 2 { "red" } else { "amber" };
        trs += &format!(
            "<tr><td>{when}</td><td>{sev}</td><td>{}</td><td>{agent}</td><td>{owner}</td></tr>",
            esc(&r.text)
        );
    }
    let table = if trs.is_empty() {
        r#"<p class="empty">No alerts in the last 14 days.</p>"#.to_string()
    } else {
        format!(
            r#"<div class="scroll"><table class="data-table"><thead><tr><th>Opened</th><th></th><th>Alert</th><th>Agent</th><th>Owner</th></tr></thead><tbody>{trs}</tbody></table></div>"#
        )
    };
    let script = match owner_csrf {
        Some(t) if !rows.is_empty() => format!(
            r#"<script>
(function () {{
  document.querySelectorAll("button[data-triage-id]").forEach(function (b) {{
    b.addEventListener("click", function () {{
      fetch("api/alerts/owner-verdict", {{method: "POST", headers: {{"Content-Type": "application/json", "X-Warte-CSRF": "{}"}},
        body: JSON.stringify({{id: b.dataset.triageId, verdict: b.dataset.triageVerdict}})}})
        .then(function (r) {{ if (r.ok) {{ b.parentNode.querySelectorAll("button").forEach(function (x) {{ x.removeAttribute("aria-pressed"); }}); b.setAttribute("aria-pressed", "true"); }} else r.text().then(function (t) {{ b.textContent = t; }}); }});
    }});
  }});
}})();
</script>"#,
            esc(t)
        ),
        _ => String::new(),
    };
    format!(
        r#"<section class="panel" id="alert-triage"><h3>Alert triage · the agent's verdict on each alert</h3>{measures}{table}{script}</section>"#
    )
}

struct Held {
    n: Value,
    id: String,
    key: String,
    sev: u8,
    escalated: bool,
    at: i64,
}

/// Gate mode's held alerts and the keys dropped as noise, across ticks.
#[derive(Default)]
pub struct Gate {
    held: Vec<Held>,
    noise: std::collections::HashSet<String>,
}

impl Gate {
    /// The notices to push this tick, from this tick's notices and the held
    /// ones. A clear of a held or noise-dropped alert is not pushed either.
    pub async fn step(&mut self, store: &Store, ns: &[Value], now: i64) -> Vec<Value> {
        let last = store.last_poll().await;
        let mut out = Vec::new();
        for n in ns {
            if let Some((key, sev, escalated)) = opened(n) {
                let id = n["id"].as_str().unwrap_or_default().to_string();
                self.held.retain(|h| h.key != key);
                self.noise.remove(&key);
                let v = store.verdict_of(&id).await;
                match decide(Mode::Gate, sev, escalated, v, now, now, last) {
                    Push::Now => out.push(n.clone()),
                    Push::Hold => self.held.push(Held {
                        n: n.clone(),
                        id,
                        key,
                        sev,
                        escalated,
                        at: now,
                    }),
                    Push::Drop => {
                        self.noise.insert(key);
                    }
                }
            } else if let Some(key) = cleared(n) {
                let was_held = self.held.iter().any(|h| h.key == key);
                self.held.retain(|h| h.key != key);
                if !was_held && !self.noise.remove(&key) {
                    out.push(n.clone());
                }
            } else {
                out.push(n.clone());
            }
        }
        let mut keep = Vec::new();
        for h in std::mem::take(&mut self.held) {
            let v = store.verdict_of(&h.id).await;
            match decide(Mode::Gate, h.sev, h.escalated, v, h.at, now, last) {
                Push::Now => out.push(h.n),
                Push::Hold => keep.push(h),
                Push::Drop => {
                    self.noise.insert(h.key);
                }
            }
        }
        self.held = keep;
        out
    }
}

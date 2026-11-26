//! Outage alerts (REQ-ALERT-001): what the Outages view detects reaches the
//! owner without anyone opening the view.
//!
//! Every minute the server evaluates the same findings the view renders (the
//! fleet's, and GitHub's status) and records a factory notice (`notice.v1`,
//! schema version 2) under its own agent, `warte-alerts`, when a finding
//! opens, escalates or clears. The owner's `notice.py digest` / `review` read
//! every agent's notices from warte, so the attention budget applies there.
//!
//! A red finding opens after two consecutive evaluations and clears after
//! three without it. An amber one waits longer both ways (five to open,
//! fifteen to clear), so a value hovering at its threshold does not flap.
//! A finding that stays open never notifies again unless its severity
//! rises. One evaluation records at most [`MAX_PER_TICK`] notices; the rest
//! are folded into one summary, so a fan-out cannot flood the owner.

use crate::dashboard::fleet::Detected;

pub mod house;
pub mod ntfy;
use std::collections::HashMap;

/// Consecutive evaluations a red finding must be present before it opens.
pub const OPEN_AFTER: u32 = 2;
/// Consecutive evaluations an open red finding must be absent before it clears.
pub const CLEAR_AFTER: u32 = 3;
/// The same for amber (degraded) findings: slower both ways.
pub const AMBER_OPEN_AFTER: u32 = 5;
pub const AMBER_CLEAR_AFTER: u32 = 15;
/// The most notices one evaluation records; the rest become one summary.
pub const MAX_PER_TICK: usize = 5;

/// Evaluations a finding of this severity must be present before it opens.
pub fn open_after(sev: u8) -> u32 {
    if sev >= 2 {
        OPEN_AFTER
    } else {
        AMBER_OPEN_AFTER
    }
}

/// Evaluations an open finding of this severity must be absent to clear.
pub fn clear_after(sev: u8) -> u32 {
    if sev >= 2 {
        CLEAR_AFTER
    } else {
        AMBER_CLEAR_AFTER
    }
}
/// The agent name alerts are recorded under.
pub const AGENT: &str = "warte-alerts";

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Opened(Detected),
    Escalated(Detected),
    Cleared { key: String, text: String },
}

#[derive(Debug, Default)]
struct Track {
    sev: u8,
    text: String,
    seen: u32,
    missing: u32,
    open: bool,
}

/// Debounced open/escalate/clear per finding key. Pure: no clock, no I/O.
#[derive(Debug, Default)]
pub struct Tracker {
    t: HashMap<String, Track>,
}

impl Tracker {
    pub fn step(&mut self, found: &[Detected]) -> Vec<Event> {
        let mut out = Vec::new();
        for d in found {
            let tr = self.t.entry(d.key.clone()).or_default();
            tr.missing = 0;
            tr.seen = tr.seen.saturating_add(1);
            let rose = tr.open && d.sev > tr.sev;
            tr.text = d.text.clone();
            if !tr.open && tr.seen >= open_after(d.sev) {
                tr.open = true;
                tr.sev = d.sev;
                out.push(Event::Opened(d.clone()));
            } else if rose {
                tr.sev = d.sev;
                out.push(Event::Escalated(d.clone()));
            } else if !tr.open {
                tr.sev = d.sev;
            }
        }
        let present: std::collections::HashSet<&str> =
            found.iter().map(|d| d.key.as_str()).collect();
        let mut gone = Vec::new();
        for (k, tr) in self
            .t
            .iter_mut()
            .filter(|(k, _)| !present.contains(k.as_str()))
        {
            tr.missing += 1;
            tr.seen = 0;
            if !tr.open || tr.missing >= clear_after(tr.sev) {
                if tr.open {
                    out.push(Event::Cleared {
                        key: k.clone(),
                        text: tr.text.clone(),
                    });
                }
                gone.push(k.clone());
            }
        }
        for k in gone {
            self.t.remove(&k);
        }
        out
    }
}

/// The factory notice for an event (notice.v1, schema version 2).
pub fn notice(
    e: &Event,
    now: chrono::DateTime<chrono::Utc>,
    link: Option<&str>,
) -> serde_json::Value {
    let (kind, word, key, text) = match e {
        Event::Opened(d) => (
            if d.sev == 2 { "blocked" } else { "info" },
            if d.sev == 2 { "down" } else { "degraded" },
            &d.key,
            &d.text,
        ),
        Event::Escalated(d) => ("blocked", "down", &d.key, &d.text),
        Event::Cleared { key, text } => ("info", "cleared", key, text),
    };
    let title: String = match e {
        Event::Cleared { .. } => format!("Cleared: {text}"),
        Event::Escalated(_) => format!("Now down: {text}"),
        Event::Opened(_) => text.clone(),
    }
    .chars()
    .take(200)
    .collect();
    let mut n = serde_json::json!({
        "schema_version": 2,
        "id": format!("warte-alert-{}", uuid::Uuid::now_v7()),
        "kind": kind,
        "revision": 1,
        "created_at": now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "agent": AGENT,
        "flow": "warte-outages",
        "title": title,
        "body": format!("{text}\n\nfinding: {key}"),
        "tags": ["outage", word],
    });
    if let Some(l) = link {
        n["links"] = serde_json::json!([l]);
    }
    n
}

/// The notices for one evaluation's events, worst first, at most
/// [`MAX_PER_TICK`]: beyond that the remainder is one summary notice.
pub fn notices(
    events: &[Event],
    now: chrono::DateTime<chrono::Utc>,
    link: Option<&str>,
) -> Vec<serde_json::Value> {
    let rank = |e: &Event| match e {
        Event::Escalated(_) => 0,
        Event::Opened(d) if d.sev >= 2 => 1,
        Event::Opened(_) => 2,
        Event::Cleared { .. } => 3,
    };
    let mut ev: Vec<&Event> = events.iter().collect();
    ev.sort_by_key(|e| rank(e));
    if ev.len() <= MAX_PER_TICK {
        return ev.into_iter().map(|e| notice(e, now, link)).collect();
    }
    let keep = MAX_PER_TICK - 1;
    let mut out: Vec<_> = ev[..keep].iter().map(|e| notice(e, now, link)).collect();
    let rest = &ev[keep..];
    let opened = rest
        .iter()
        .filter(|e| !matches!(e, Event::Cleared { .. }))
        .count();
    let cleared = rest.len() - opened;
    let sample: Vec<String> = rest
        .iter()
        .take(10)
        .map(|e| match e {
            Event::Opened(d) | Event::Escalated(d) => d.text.clone(),
            Event::Cleared { text, .. } => format!("Cleared: {text}"),
        })
        .collect();
    let mut n = notice(
        &Event::Opened(Detected {
            key: "summary".into(),
            sev: 1,
            text: format!(
                "{} more outage change(s): {opened} opened, {cleared} cleared.",
                rest.len()
            ),
        }),
        now,
        link,
    );
    n["body"] = serde_json::json!(format!(
        "{}\n\n{}{}",
        n["title"].as_str().unwrap_or_default(),
        sample.join("\n"),
        if rest.len() > sample.len() {
            "\n…"
        } else {
            ""
        }
    ));
    out.push(n);
    out
}

/// Run the evaluation loop: every minute, record notices for the events.
pub fn spawn(state: crate::server::AppState) {
    tokio::spawn(async move {
        let mut tracker = Tracker::default();
        let http = reqwest::Client::new();
        let disk = state.config.disk_levels();
        let mut house = house::Reader::default();
        let mode = state
            .config
            .alert_triage
            .as_ref()
            .map(|t| t.mode)
            .unwrap_or_default();
        let mut gate = crate::alerttriage::Gate::default();
        let mut id: Option<String> = None;
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let mut found = state
                .hub
                .as_ref()
                .and_then(|h| crate::dashboard::fleet::detected_with(h, &disk))
                .map(|(l, _)| l)
                .unwrap_or_default();
            if let Ok(st) = state.status.get_status().await {
                found.extend(crate::dashboard::fleet::github_detected(&st));
            }
            // red machine findings and late feeds notify too (REQ-TEMPER-003)
            found.extend(crate::dashboard::fleet::host_detected(
                &state,
                crate::hub::now(),
            ));
            // re-queue offers, exhausted retries, stopped repos (REQ-QUEUE-004)
            if let Some(h) = &state.hub {
                found.extend(crate::requeue::detected(&h.requeue.plan()));
            }
            // the house controller's alarms (REQ-ALERT-005)
            if let Some(cfg) = &state.config.house {
                let fresh = house::fetch(&http, cfg).await;
                let now = chrono::Utc::now();
                let ws = house.take(fresh, now.timestamp());
                found.extend(house::detected(
                    ws,
                    now.timestamp_millis(),
                    cfg.silent_hold_min,
                ));
            }
            let events = tracker.step(&found);
            // the alerts agent heartbeats every tick, so it never shows as unhealthy
            let id = match &id {
                Some(id) => id.clone(),
                None => match agent_id(&state).await {
                    Ok(v) => {
                        id = Some(v.clone());
                        v
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "alerts agent unavailable");
                        continue;
                    }
                },
            };
            let _ = state.agents.heartbeat(&id).await;
            let ns = notices(&events, chrono::Utc::now(), None);
            for n in &ns {
                if let Err(err) = record(&state, &id, n).await {
                    tracing::warn!(error = %err, "alert notice not recorded");
                }
            }
            // every opened alert is stored for the triage agent (REQ-ALERT-006)
            let now = crate::hub::now();
            let triage = crate::alerttriage::Store::new(state.db.pool().clone());
            for n in &ns {
                if let Some((key, sev, _)) = crate::alerttriage::opened(n) {
                    let text = n["title"].as_str().unwrap_or_default();
                    let ctx = triage_context(&state, &triage, &key, text, &found, now).await;
                    let id = n["id"].as_str().unwrap_or_default();
                    if let Err(err) = triage.record(id, &key, sev, text, now, &ctx).await {
                        tracing::warn!(error = %err, "alert not stored for triage");
                    }
                }
            }
            // the same notices, pushed (REQ-ALERT-003); in gate mode only what
            // the triage verdicts and the fail-open rules let through
            let push = match mode {
                crate::alerttriage::Mode::Shadow => ns,
                crate::alerttriage::Mode::Gate => gate.step(&triage, &ns, now).await,
            };
            if let (Some(cfg), false) = (&state.config.notify, push.is_empty()) {
                ntfy::deliver(&http, cfg, &push).await;
            }
        }
    });
}

/// What the triage agent reads with an alert: the other open findings, recent
/// runs naming the same repo or pool, and this key's last week of verdicts.
async fn triage_context(
    state: &crate::server::AppState,
    store: &crate::alerttriage::Store,
    key: &str,
    text: &str,
    found: &[Detected],
    now: i64,
) -> serde_json::Value {
    use crate::hub::report::day_of;
    let open: Vec<serde_json::Value> = found
        .iter()
        .filter(|d| d.key != key)
        .take(20)
        .map(|d| serde_json::json!({"key": d.key, "sev": d.sev, "text": d.text}))
        .collect();
    let runs = state
        .hub
        .as_ref()
        .map(|h| {
            let recs = h.ledger_days(&day_of(now - 86_400), &day_of(now));
            crate::alerttriage::runs_for(&recs, key, text)
        })
        .unwrap_or_default();
    let history = store.history(key, now - 7 * 86_400).await;
    serde_json::json!({"open": open, "runs": runs, "history": history})
}

async fn agent_id(state: &crate::server::AppState) -> anyhow::Result<String> {
    if let Some(a) = state
        .agents
        .list()
        .await?
        .into_iter()
        .find(|a| a.name == AGENT)
    {
        return Ok(a.id);
    }
    Ok(state
        .agents
        .register(crate::agent::RegisterRequest {
            name: AGENT.into(),
            repo: None,
            identity_type: "notice".into(),
            public_key: String::new(),
        })
        .await?
        .agent_id)
}

async fn record(
    state: &crate::server::AppState,
    agent_id: &str,
    n: &serde_json::Value,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO agent_data (id, agent_id, data_type, content, captured_at, instance_id) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(agent_id)
    .bind("notice.v1")
    .bind(serde_json::to_string(n)?)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(state.config.instance_id())
    .execute(state.db.pool())
    .await?;
    Ok(())
}

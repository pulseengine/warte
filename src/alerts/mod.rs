//! Outage alerts (REQ-ALERT-001): what the Outages view detects reaches the
//! owner without anyone opening the view.
//!
//! Every minute the server evaluates the same findings the view renders (the
//! fleet's, and GitHub's status) and records a factory notice (`notice.v1`,
//! schema version 2) under its own agent, `warte-alerts`, when a finding
//! opens, escalates or clears. The owner's `notice.py digest` / `review` read
//! every agent's notices from warte, so the attention budget applies there.
//!
//! A finding opens only after it is seen on two consecutive evaluations and
//! clears only after three without it, so a single flapping snapshot does
//! not notify. A finding that stays open never notifies again unless its
//! severity rises.

use crate::dashboard::fleet::Detected;
use std::collections::HashMap;

/// Consecutive evaluations a finding must be present before it opens.
pub const OPEN_AFTER: u32 = 2;
/// Consecutive evaluations a finding must be absent before it clears.
pub const CLEAR_AFTER: u32 = 3;
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
            if !tr.open && tr.seen >= OPEN_AFTER {
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
            if tr.missing >= CLEAR_AFTER || !tr.open {
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

/// Run the evaluation loop: every minute, record notices for the events.
pub fn spawn(state: crate::server::AppState) {
    tokio::spawn(async move {
        let mut tracker = Tracker::default();
        let mut id: Option<String> = None;
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let mut found = state
                .hub
                .as_ref()
                .and_then(crate::dashboard::fleet::detected)
                .map(|(l, _)| l)
                .unwrap_or_default();
            if let Ok(st) = state.status.get_status().await {
                found.extend(crate::dashboard::fleet::github_detected(&st));
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
            for e in &events {
                let n = notice(e, chrono::Utc::now(), None);
                if let Err(err) = record(&state, &id, &n).await {
                    tracing::warn!(error = %err, "alert notice not recorded");
                }
            }
        }
    });
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

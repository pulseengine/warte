//! Operator duties (REQ-OPS-001..003): what warte expects from whoever runs
//! it, and whether it is being done.
//!
//! warte judges every duty from its own data, never from the operator's
//! word: a report's age, a probe's last report, who wrote the latest
//! briefing. `docs/topics/operator.md` describes each duty and
//! `deploy/operator/` holds templates for it; a test keeps the three in step.

use serde::Serialize;

/// How long a CI source may go without a snapshot before it is late.
pub const CI_LATE_S: i64 = 15 * 60;
/// How long the model job has to replace the plain briefing.
pub const BRIEFING_GRACE_S: i64 = 6 * 3600;
/// How long a triage hand-out may wait before the job counts as late.
pub const TRIAGE_LATE_S: i64 = 2 * 3600;

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Met,
    Late,
    NotSetUp,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            State::Met => "met",
            State::Late => "late",
            State::NotSetUp => "not set up",
        }
    }
}

/// A duty as the operator document names it.
pub struct Spec {
    /// Stable key, also the document's section anchor.
    pub key: &'static str,
    pub title: &'static str,
    /// The token right the duty's job needs, if any.
    pub right: Option<&'static str>,
    pub every: &'static str,
}

/// Every duty, in the order the document and the view list them.
pub const DUTIES: [Spec; 7] = [
    Spec {
        key: "ci-source",
        title: "Feed the CI source",
        right: None,
        every: "continuously",
    },
    Spec {
        key: "probes",
        title: "Run a probe on each machine",
        right: None,
        every: "every minute",
    },
    Spec {
        key: "costs",
        title: "Send the costs report",
        right: Some("costs:write"),
        every: "nightly",
    },
    Spec {
        key: "skills",
        title: "Send the skill-usage report",
        right: Some("skills:write"),
        every: "nightly",
    },
    Spec {
        key: "briefing",
        title: "Run the briefing job",
        right: Some("briefing:write"),
        every: "every two days",
    },
    Spec {
        key: "triage",
        title: "Run the triage job",
        right: Some("triage:write"),
        every: "within the hour",
    },
    Spec {
        key: "login",
        title: "Provide a login",
        right: None,
        every: "once",
    },
];

/// One duty's verdict.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Duty {
    pub key: &'static str,
    pub title: &'static str,
    pub right: Option<&'static str>,
    pub every: &'static str,
    pub state: State,
    /// Since when it is late, where known (unix seconds).
    pub since: Option<i64>,
    pub detail: String,
}

/// What warte knows about each duty. `None` at the outer level: that part
/// of warte is not configured at all.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub now: i64,
    /// The CI source's last snapshot, if a source is configured.
    pub ci: Option<Option<i64>>,
    /// Each machine's last report, if the ground station keeps machines.
    pub probes: Option<Vec<(String, i64)>>,
    /// The costs report's arrival and data lag in days.
    pub costs: Option<Option<(i64, Option<i64>)>>,
    /// The latest skill-usage report's arrival.
    pub skills: Option<Option<i64>>,
    /// The latest briefing: its source (`model` or `template`) and when it
    /// was written.
    pub briefing: Option<Option<(String, i64)>>,
    /// The oldest hand-out still waiting for the job, when it was queued.
    pub triage: Option<Option<i64>>,
    pub login: bool,
}

fn ago(now: i64, t: i64) -> String {
    let s = (now - t).max(0);
    if s < 3600 {
        format!("{} min ago", s / 60)
    } else if s < 2 * 86_400 {
        format!("{} h ago", s / 3600)
    } else {
        format!("{} days ago", s / 86_400)
    }
}

/// The verdict for every duty.
pub fn judge(i: &Inputs) -> Vec<Duty> {
    let now = i.now;
    DUTIES
        .iter()
        .map(|s| {
            let (state, since, detail) = match s.key {
                "ci-source" => match i.ci {
                    None => (State::NotSetUp, None, "no CI source configured ([hub] or [github])".into()),
                    Some(None) => (State::Late, None, "configured, but no snapshot has arrived".into()),
                    Some(Some(t)) if now - t > CI_LATE_S => {
                        (State::Late, Some(t), format!("last snapshot {}", ago(now, t)))
                    }
                    Some(Some(t)) => (State::Met, None, format!("last snapshot {}", ago(now, t))),
                },
                "probes" => match &i.probes {
                    None => (State::NotSetUp, None, "no ground station configured".into()),
                    Some(v) if v.is_empty() => {
                        (State::NotSetUp, None, "no machine has reported".into())
                    }
                    Some(v) => {
                        let mut quiet: Vec<&(String, i64)> = v
                            .iter()
                            .filter(|(_, t)| now - t > crate::probe::MACHINE_STALE_S)
                            .collect();
                        quiet.sort_by_key(|(_, t)| *t);
                        if quiet.is_empty() {
                            (State::Met, None, format!("{} machine(s) reporting", v.len()))
                        } else {
                            let names: Vec<&str> = quiet.iter().map(|(h, _)| h.as_str()).collect();
                            (
                                State::Late,
                                Some(quiet[0].1),
                                format!(
                                    "{} of {} machine(s) not reporting: {}",
                                    quiet.len(),
                                    v.len(),
                                    names.join(", ")
                                ),
                            )
                        }
                    }
                },
                "costs" => match i.costs {
                    None => (State::NotSetUp, None, "no ground station configured".into()),
                    Some(None) => (State::NotSetUp, None, "no report has arrived".into()),
                    Some(Some((t, lag))) => {
                        if now - t > crate::costs::STALE_S {
                            (State::Late, Some(t), format!("last report {}", ago(now, t)))
                        } else if let Some(d) = lag {
                            (
                                State::Late,
                                Some(t),
                                format!("the report arrived {}, but its newest data is {d} day(s) old: its input may have stopped", ago(now, t)),
                            )
                        } else {
                            (State::Met, None, format!("last report {}", ago(now, t)))
                        }
                    }
                },
                "skills" => match i.skills {
                    None => (State::NotSetUp, None, "no ground station configured".into()),
                    Some(None) => (State::NotSetUp, None, "no report has arrived".into()),
                    Some(Some(t)) if now - t > crate::skills::STALE_S => {
                        (State::Late, Some(t), format!("last report {}", ago(now, t)))
                    }
                    Some(Some(t)) => (State::Met, None, format!("last report {}", ago(now, t))),
                },
                "briefing" => match &i.briefing {
                    None => (State::NotSetUp, None, "no [briefing] configured".into()),
                    Some(None) => (State::Late, None, "no briefing written yet".into()),
                    Some(Some((src, t))) if src == "model" => (
                        State::Met,
                        None,
                        format!("the latest briefing is the model's, written {}", ago(now, *t)),
                    ),
                    Some(Some((_, t))) if now - t <= BRIEFING_GRACE_S => (
                        State::Met,
                        None,
                        format!("the plain version is up since {}; the model job has until {} h after", ago(now, *t), BRIEFING_GRACE_S / 3600),
                    ),
                    Some(Some((_, t))) => (
                        State::Late,
                        Some(*t),
                        "the latest briefing is still the plain version: the model job did not post one that fits the facts".into(),
                    ),
                },
                "triage" => match i.triage {
                    None => (State::NotSetUp, None, "no [triage] configured".into()),
                    Some(Some(t)) if now - t > TRIAGE_LATE_S => (
                        State::Late,
                        Some(t),
                        format!("an issue handed out {} is not marked done", ago(now, t)),
                    ),
                    Some(_) => (State::Met, None, "nothing waits beyond its time".into()),
                },
                _ => {
                    if i.login {
                        (State::Met, None, "a login provider is configured".into())
                    } else {
                        (
                            State::NotSetUp,
                            None,
                            "no [auth]: personal data is open to anyone who can reach warte".into(),
                        )
                    }
                }
            };
            Duty {
                key: s.key,
                title: s.title,
                right: s.right,
                every: s.every,
                state,
                since,
                detail,
            }
        })
        .collect()
}

/// What warte knows, read from the running server.
pub fn inputs(state: &crate::server::AppState, now: i64) -> Inputs {
    Inputs {
        now,
        ci: state.hub.as_ref().map(|h| h.snapshot().map(|(_, t)| t)),
        probes: state.machines.as_ref().map(|m| {
            m.list()
                .into_iter()
                .map(|s| (s.report.host, s.received_at))
                .collect()
        }),
        costs: state.costs.as_ref().map(|c| {
            c.latest()
                .map(|s| (s.received_at, s.report.data_lag_days()))
        }),
        skills: state
            .skills
            .as_ref()
            .map(|s| s.all().iter().map(|x| x.received_at).max()),
        briefing: state
            .briefing
            .as_ref()
            .map(|b| b.latest().map(|x| (x.source, x.written_at))),
        triage: state
            .triage
            .as_ref()
            .map(|t| t.offer(now).ready.iter().map(|i| i.queued_at).min()),
        login: state.login.is_some(),
    }
}

/// The Health view's panel.
pub fn panel(duties: &[Duty]) -> String {
    let mut rows = String::new();
    for d in duties {
        let badge = match d.state {
            State::Met => "green",
            State::Late => "red",
            State::NotSetUp => "gray",
        };
        rows += &format!(
            r#"<tr><td>{}</td><td><span class="badge {badge}">{}</span></td><td>{}</td><td class="mono">{}</td><td>{}</td></tr>"#,
            crate::dashboard::fleet::esc(d.title),
            d.state.word(),
            d.every,
            d.right.unwrap_or(""),
            crate::dashboard::fleet::esc(&d.detail),
        );
    }
    format!(
        r#"<section class="panel operator"><h3>Operator duties</h3><p class="note">What warte expects from whoever runs it, judged from warte's own data. Each duty and its template: <span class="mono">docs/topics/operator.md</span>, <span class="mono">deploy/operator/</span>.</p><table><thead><tr><th>Duty</th><th>State</th><th>How often</th><th>Token right</th><th>Detail</th></tr></thead><tbody>{rows}</tbody></table></section>"#
    )
}

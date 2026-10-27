//! Dashboard sections fed by the ground station (DD-SMITHY-001).
//!
//! These replace views that read warte's own `ci_jobs`, `ci_status` and
//! `wait_history` tables. Everything shown comes from the hub's latest snapshot
//! or its `/api/eta` answer, so the dashboard and the hub API never disagree.
//! Repo, branch, workflow and job names are written by repo authors: they are
//! HTML-escaped, never trusted.

use crate::hub::eta::Answer;
use crate::hub::model::Snapshot;
use crate::hub::{now, Hub};

/// A snapshot older than this is shown as stale (the agent sends one a minute).
const STALE_AFTER_S: i64 = 5 * 60;

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn dur(s: i64) -> String {
    match s.max(0) {
        s if s < 90 => format!("{s}s"),
        s if s < 5400 => format!("{}m", s / 60),
        s => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

/// Why there is nothing to show, as an HTML paragraph.
fn unavailable(hub: Option<&Hub>) -> String {
    match hub {
        None => r#"<p class="empty">Ground station not configured. Add a <code>[hub]</code> section to warte.toml (see <code>warte docs ground-station</code>).</p>"#.into(),
        Some(_) => r#"<p class="empty">Ground station is up, but no telemetry has arrived from the smithy agent yet.</p>"#.into(),
    }
}

/// Freshness line from the snapshot's real age, not the render time.
pub fn freshness(received_at: i64) -> String {
    let age = now() - received_at;
    let (class, word) = if age <= STALE_AFTER_S {
        ("green", "fresh")
    } else {
        ("red", "stale")
    };
    format!(
        r#"<p class="freshness">Fleet snapshot {} old <span class="freshness-badge {class}">{word}</span></p>"#,
        dur(age)
    )
}

fn snap(hub: Option<&Hub>) -> Result<(Snapshot, i64), String> {
    hub.and_then(Hub::snapshot).ok_or_else(|| unavailable(hub))
}

/// Queue view: per-pool queued jobs, and every queued run.
pub fn queue(hub: Option<&Hub>) -> String {
    let (s, at) = match snap(hub) {
        Ok(v) => v,
        Err(why) => return why,
    };
    let Some(q) = &s.queue else {
        return format!(
            r#"<p class="empty">No queue scan in the latest snapshot.</p>{}"#,
            freshness(at)
        );
    };
    let busy: usize = s
        .runners
        .iter()
        .filter(|r| r.gh_busy == Some(true) || r.local_busy)
        .count();
    let mut cards = format!(
        r#"<div class="stat-card"><span class="stat-value">{busy}/{}</span><span class="stat-label">self-hosted runners busy</span><span class="stat-detail">{} jobs queued in {} runs{}</span></div>"#,
        s.runners.len(),
        q.jobs_queued,
        q.runs_queued,
        if q.truncated { " (lower bound)" } else { "" },
    );
    for (pool, p) in &q.pools {
        let class = match p.jobs_queued {
            0 => "green",
            1..=10 => "yellow",
            _ => "red",
        };
        cards += &format!(
            r#"<div class="stat-card"><span class="stat-value">{}</span><span class="stat-label">{}</span><span class="stat-detail">oldest {}, median {}</span><span class="badge {class}">queued</span></div>"#,
            p.jobs_queued,
            esc(pool),
            dur(p.oldest_s),
            dur(p.median_s),
        );
    }
    let waiting = if q.queued.is_empty() {
        r#"<p class="empty">No jobs waiting in queue.</p>"#.to_string()
    } else {
        let t = now();
        let rows: String = q
            .queued
            .iter()
            .map(|e| {
                let what = match e.pr {
                    Some(n) => format!("#{n}"),
                    None => esc(&e.branch),
                };
                format!(
                    "<tr><td><code>{}</code></td><td>{what}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    esc(&e.repo),
                    esc(&e.workflow),
                    esc(&e.pool),
                    e.jobs,
                    dur(t - e.since),
                )
            })
            .collect();
        format!(
            r#"<table class="data-table"><thead><tr><th>Repo</th><th>PR / branch</th><th>Workflow</th><th>Pool</th><th>Jobs</th><th>Waiting</th></tr></thead><tbody>{rows}</tbody></table>"#
        )
    };
    format!(
        r#"<div class="stat-cards">{cards}</div><h3>Waiting Jobs</h3>{waiting}{}"#,
        freshness(at)
    )
}

/// Critical-path inputs: repos with the most queued jobs, and measured pool waits.
pub fn bottlenecks(hub: Option<&Hub>) -> String {
    let (s, at) = match snap(hub) {
        Ok(v) => v,
        Err(why) => return why,
    };
    let Some(q) = &s.queue else {
        return format!(
            r#"<p class="empty">No queue scan in the latest snapshot.</p>{}"#,
            freshness(at)
        );
    };
    let mut per_repo: std::collections::BTreeMap<&str, u32> = Default::default();
    for e in &q.queued {
        *per_repo.entry(e.repo.as_str()).or_default() += e.jobs;
    }
    let mut repos: Vec<(&str, u32)> = per_repo.into_iter().filter(|(_, n)| *n > 5).collect();
    repos.sort_by_key(|r| std::cmp::Reverse(r.1));
    let cards = if repos.is_empty() {
        r#"<p class="empty">No repo has more than 5 jobs queued.</p>"#.to_string()
    } else {
        repos
            .iter()
            .map(|(r, n)| {
                format!(
                    r#"<div class="stat-card"><span class="stat-label">{}</span><span class="badge yellow">{n} jobs queued</span></div>"#,
                    esc(r)
                )
            })
            .collect()
    };
    let waits = if q.waits.is_empty() {
        r#"<p class="empty">No measured waits in the latest snapshot.</p>"#.to_string()
    } else {
        let rows: String = q
            .waits
            .iter()
            .map(|(pool, w)| {
                format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    esc(pool),
                    w.n,
                    dur(w.p50),
                    dur(w.p90),
                    dur(w.max)
                )
            })
            .collect();
        format!(
            r#"<table class="data-table"><thead><tr><th>Runner pool</th><th>Recent starts</th><th>Wait p50</th><th>p90</th><th>max</th></tr></thead><tbody>{rows}</tbody></table>"#
        )
    };
    format!(
        r#"<h3>Bottlenecks</h3><div class="stat-cards">{cards}</div><h3>Measured pool waits</h3>{waits}{}"#,
        freshness(at)
    )
}

/// Ask view: the hub's ETA answer for one PR or branch.
pub fn ask(hub: Option<&Hub>, repo: &str, pr: Option<u32>, branch: Option<&str>) -> String {
    let title = match (pr, branch) {
        (Some(n), _) => format!("{} PR #{n}", esc(repo)),
        (None, Some(b)) => format!("{} @ {}", esc(repo), esc(b)),
        (None, None) => esc(repo),
    };
    if repo.is_empty() || (pr.is_none() && branch.is_none()) {
        return r#"<div class="ask-result"><p class="empty">Give a repo (without the org) and a PR number or branch.</p></div>"#.into();
    }
    let Some(hub) = hub else {
        return format!(
            r#"<div class="ask-result"><h3>ETA for {title}</h3>{}</div>"#,
            unavailable(None)
        );
    };
    let Some(a) = hub.eta(repo, pr, branch) else {
        return format!(
            r#"<div class="ask-result"><h3>ETA for {title}</h3>{}</div>"#,
            unavailable(Some(hub))
        );
    };
    format!(
        r#"<div class="ask-result"><h3>ETA for {title}</h3>{}<p class="basis">{}</p></div>"#,
        answer_html(&a),
        esc(a.scope)
    )
}

fn answer_html(a: &Answer) -> String {
    let class = match a.status {
        "running" | "not_queued" => "green",
        "queued" | "running+queued" => "yellow",
        _ => "gray",
    };
    let mut out = format!(
        r#"<p class="estimate">Status: <span class="badge {class}">{}</span></p>"#,
        esc(a.status)
    );
    for r in &a.running {
        out += &format!(
            "<p>Running <code>{}</code> on {} for {}</p>",
            esc(&r.job),
            esc(&r.on),
            r.for_s.map(dur).unwrap_or_else(|| "an unknown time".into())
        );
    }
    for q in &a.queued {
        let est = match &q.estimate {
            Some(e) => {
                let range = match e.start_in_high_s {
                    Some(h) => format!("starts in {}–{}", dur(e.start_in_low_s), dur(h)),
                    // waited longer than 90% of recent jobs: no honest upper bound
                    None => format!(
                        "starts in {} or later, no upper bound",
                        dur(e.start_in_low_s)
                    ),
                };
                format!(
                    "{range}{} ({})",
                    if e.overdue {
                        ", overdue: probably stuck or starved"
                    } else {
                        ""
                    },
                    esc(&e.basis)
                )
            }
            None => "no estimate: too few recent starts in this pool".into(),
        };
        out += &format!(
            "<p>Queued in <code>{}</code> for {}: {est}</p>",
            esc(&q.pool),
            dur(q.waited_s)
        );
    }
    if a.status == "unknown" {
        out += r#"<p class="empty">The last queue scan was incomplete, so "not in the list" does not mean "not queued".</p>"#;
    }
    out
}

/// Health cards for the fleet: runners online/busy and jobs queued.
pub fn health_cards(hub: Option<&Hub>) -> String {
    match hub.and_then(Hub::snapshot) {
        None => r#"<div class="stat-card"><span class="stat-value">—</span><span class="stat-label">CI fleet</span><span class="stat-detail">no ground-station telemetry</span></div>"#.into(),
        Some((s, at)) => {
            let online = s.runners.iter().filter(|r| r.gh_status.as_deref() == Some("online")).count();
            let busy = s.runners.iter().filter(|r| r.gh_busy == Some(true) || r.local_busy).count();
            let queued = s.queue.as_ref().map(|q| q.jobs_queued).unwrap_or(0);
            let age = now() - at;
            format!(
                r#"<div class="stat-card"><span class="stat-value">{online}/{}</span><span class="stat-label">Runners online</span><span class="stat-detail">{busy} busy</span></div><div class="stat-card"><span class="stat-value">{queued}</span><span class="stat-label">Jobs queued</span><span class="stat-detail">snapshot {} old{}</span></div>"#,
                s.runners.len(),
                dur(age),
                if age > STALE_AFTER_S { " (stale)" } else { "" },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_author_controlled_names() {
        assert_eq!(
            esc(r#"<script>alert("x")</script>&'"#),
            "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;&amp;&#39;"
        );
    }

    #[test]
    fn says_why_there_is_nothing_to_show() {
        assert!(queue(None).contains("not configured"));
        assert!(ask(None, "warte", Some(1), None).contains("not configured"));
        assert!(ask(None, "", None, None).contains("Give a repo"));
    }
}

//! The Compliance view (REQ-TEMPER-002): temper's latest report, forwarded
//! by a probe. Only enums, identifiers and numbers reach the page; every
//! string is escaped anyway.

use super::fleet::esc;
use crate::temper::report::{Report, Severity};
use crate::temper::Stored;

fn age(s: i64) -> String {
    let s = s.max(0);
    if s < 5400 {
        format!("{} min", s / 60)
    } else if s < 2 * 86400 {
        format!("{:.1} h", s as f64 / 3600.0)
    } else {
        format!("{} d", s / 86400)
    }
}

fn sev(s: Severity) -> (&'static str, &'static str) {
    match s {
        Severity::Critical => ("red", "critical"),
        Severity::High => ("red", "high"),
        Severity::Medium => ("yellow", "medium"),
        Severity::Low => ("yellow", "low"),
        Severity::Info => ("", "info"),
    }
}

fn failures(r: &Report) -> String {
    let f = r.failures();
    if f.is_empty() {
        return r#"<p class="empty">Every decided check passes.</p>"#.into();
    }
    let rows: String = f
        .iter()
        .map(|(repo, c)| {
            let (cls, label) = sev(c.severity);
            format!(
                r#"<tr><td>{}</td><td class="mono">{}</td><td><span class="status-badge {cls}">{label}</span></td><td class="mono">{}</td></tr>"#,
                esc(repo),
                esc(&c.id.as_str()),
                esc(&c.code.as_str())
            )
        })
        .collect();
    format!(
        r#"<table class="data-table"><thead><tr><th>Repo</th><th>Check</th><th>Severity</th><th>Result</th></tr></thead><tbody>{rows}</tbody></table>"#
    )
}

pub fn view(configured: bool, latest: Option<&Stored>, stale_after_s: i64, now: i64) -> String {
    if !configured {
        return r#"<div class="overview"><h2>Compliance</h2><p class="empty">No temper feed configured: set <code>[temper] host = "&lt;probe host&gt;"</code> in warte.toml, and run that host's probe with <code>--feed temper=&lt;report path&gt;</code>.</p></div>"#.into();
    }
    let Some(s) = latest else {
        return r#"<div class="overview"><h2>Compliance</h2><p class="empty">No temper report received yet.</p></div>"#.into();
    };
    let r = &s.report;
    let sweep = r
        .source
        .last_sweep_at
        .as_deref()
        .and_then(crate::temper::report::time);
    let (pill, word) = match sweep {
        Some(t) if now - t <= stale_after_s => ("pill", format!("last sweep {} ago", age(now - t))),
        Some(t) => (
            "pill stale",
            format!("STALE · last sweep {} ago", age(now - t)),
        ),
        None => ("pill stale", "no full sweep yet".to_string()),
    };
    let undecided: usize = r
        .repos
        .iter()
        .flat_map(|x| &x.checks)
        .chain(&r.org_checks)
        .filter(|c| c.ok.is_none())
        .count();
    let t = &r.tasks;
    let tasks_hot = t.failed > 0 || t.stale_in_progress > 0;
    let rv = &r.reviews;
    let api = &r.source.api;
    format!(
        r#"<div class="overview"><h2>Compliance <span class="{pill}">{word}</span></h2><p class="lede">{org}: <b>{compliant}</b> of {repos} repos compliant, <b>{nonc}</b> not, {unk} unknown · {undecided} checks undecided (API or permission errors). From temper {ver} via the probe on {host}, received {rec} ago.</p><section class="panel"><h3>Failed checks · worst first</h3>{fail}</section><section class="panel"><h3>temper's own work</h3><div class="kpis"><div class="kpi{tcls}"><div class="l">Tasks</div><div class="v">{failed} failed</div><div class="s">{pend} pending · {stale} stuck in progress</div></div><div class="kpi"><div class="l">AI reviews · {win} d</div><div class="v">{posted}</div><div class="s">posted · {find} findings · {err} errors</div></div><div class="kpi"><div class="l">GitHub API (temper's App)</div><div class="v">{calls}/h</div><div class="s">{rem} of {lim} left</div></div></div></section></div>"#,
        org = esc(&r.org),
        compliant = r.summary.compliant,
        repos = r.summary.repos,
        nonc = r.summary.noncompliant,
        unk = r.summary.unknown,
        ver = esc(&r.temper.version),
        host = esc(&s.host),
        rec = age(now - s.received_at),
        fail = failures(r),
        tcls = if tasks_hot { " warn" } else { "" },
        failed = t.failed,
        pend = t.pending,
        stale = t.stale_in_progress,
        win = rv.window_days,
        posted = rv.posted,
        find = rv.findings,
        err = rv.errors,
        calls = api.calls_last_hour,
        rem = api.remaining,
        lim = api.limit,
    )
}

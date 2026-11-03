//! The Jobs view (REQ-JOBS-001): every scheduled job, worst first.

use super::fleet::esc;
use crate::jobs::Job;

fn ago(now: i64, t: Option<i64>) -> String {
    match t {
        None => "never".into(),
        Some(t) => {
            let s = (now - t).max(0);
            if s < 5400 {
                format!("{} min ago", s / 60)
            } else if s < 2 * 86400 {
                format!("{:.1} h ago", s as f64 / 3600.0)
            } else {
                format!("{} d ago", s / 86400)
            }
        }
    }
}

fn span(s: i64) -> String {
    let a = s.abs();
    if a < 5400 {
        format!("{} min", a / 60)
    } else if a < 2 * 86400 {
        format!("{:.1} h", a as f64 / 3600.0)
    } else {
        format!("{} d", a / 86400)
    }
}

pub fn view(jobs: Option<&[Job]>, now: i64) -> String {
    let Some(jobs) = jobs else {
        return r#"<div class="overview"><h2>Jobs</h2><p class="empty">No jobs directory configured ([jobs] dir in warte.toml, or WARTE_JOBS_DIR).</p></div>"#.into();
    };
    let bad = jobs.iter().filter(|j| j.bad()).count();
    let rows: String = jobs
        .iter()
        .map(|j| {
            let (cls, label) = match j.state.as_str() {
                "ok" => ("green", "ok"),
                "failing" => ("yellow", "last run failed"),
                "overdue" => ("red", "overdue"),
                _ => ("red", "missing"),
            };
            let due = match j.due_in_s {
                Some(d) if d >= 0 => format!("overdue in {}", span(d)),
                Some(d) => format!("overdue for {}", span(d)),
                None => "no success yet".into(),
            };
            let run = match (j.last_run, j.exit) {
                (Some(_), Some(e)) => format!("{} (exit {e})", ago(now, j.last_run)),
                _ => ago(now, j.last_run),
            };
            format!(
                r#"<tr class="job-{state}"><td>{host}</td><td>{name}</td><td><span class="status-badge {cls}">{label}</span></td><td>{ok}</td><td>{run}</td><td>every {win} · {due}</td></tr>"#,
                state = esc(&j.state),
                host = esc(&j.host),
                name = esc(&j.name),
                ok = ago(now, j.last_success),
                win = span(j.window_s),
            )
        })
        .collect();
    format!(
        r#"<div class="overview"><h2>Jobs</h2><p class="lede">{n} scheduled jobs across the hosts · <b>{bad}</b> overdue or missing. A job is overdue when no success came within its window, so a stopped timer or a host that is down shows up here.</p><table class="data-table"><thead><tr><th>Host</th><th>Job</th><th>State</th><th>Last success</th><th>Last run</th><th>Window</th></tr></thead><tbody>{rows}</tbody></table></div>"#,
        n = jobs.len()
    )
}

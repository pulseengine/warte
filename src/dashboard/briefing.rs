//! Briefing view (REQ-BRIEF-003): one line anyone understands, three plain
//! bullets, and a collapsed section with the numbers for an IT reader. Every
//! string from GitHub or a model run is escaped.

use super::fleet::esc;
use crate::briefing::{Briefing, Store};

fn usd(x: f64) -> String {
    format!("${x:.0}")
}

fn pretty(day: &str) -> String {
    chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map(|d| d.format("%-d %b").to_string())
        .unwrap_or_else(|_| day.to_string())
}

fn it_section(b: &Briefing) -> String {
    let f = &b.facts;
    let link = |repo: &str, kind: &str, n: u64| {
        format!(
            r#"<a href="https://github.com/{}/{}/{kind}/{n}">#{n}</a>"#,
            esc(&f.org),
            esc(repo)
        )
    };
    let rows: String = f
        .repos
        .iter()
        .map(|r| {
            let prs: Vec<String> = r.merged.iter().map(|i| link(&r.repo, "pull", i.number)).collect();
            let iss: Vec<String> = r
                .closed_issues
                .iter()
                .map(|i| link(&r.repo, "issues", i.number))
                .collect();
            format!(
                r#"<tr><td>{}</td><td class="n">{}</td><td>{}</td><td class="n">{}</td><td>{}</td><td class="mono">{}</td><td class="n">{}</td><td class="n">{}</td></tr>"#,
                esc(&r.repo),
                r.merged.len(),
                prs.join(" "),
                r.closed_issues.len(),
                iss.join(" "),
                esc(&r.releases.join(", ")),
                r.ci_runs,
                r.ci_failed_jobs
            )
        })
        .collect();
    let t = &f.totals;
    let mut out = format!(
        r#"<div class="scroll"><table class="data-table"><thead><tr><th>Repository</th><th class="n">Merged</th><th>Pull requests</th><th class="n">Closed</th><th>Issues</th><th>Releases</th><th class="n">CI runs</th><th class="n">Failed jobs</th></tr></thead><tbody>{rows}<tr><td><b>{} repositories</b></td><td class="n"><b>{}</b></td><td></td><td class="n"><b>{}</b></td><td></td><td class="n"><b>{}</b></td><td class="n"><b>{}</b></td><td class="n"><b>{}</b></td></tr></tbody></table></div>"#,
        t.repos, t.merged, t.closed_issues, t.releases, t.ci_runs, t.ci_failed_jobs
    );
    if !f.open_prs.is_empty() {
        let li: String = f
            .open_prs
            .iter()
            .map(|p| {
                format!(
                    "<li>{} {} · {} · open {} d</li>",
                    esc(&p.repo),
                    link(&p.repo, "pull", p.number),
                    esc(&p.title),
                    p.days_open
                )
            })
            .collect();
        out += &format!("<h4>Open for more than a day</h4><ul>{li}</ul>");
    }
    if let Some(tc) = &f.tokens {
        let ch = if tc.prev_week_usd > 0.0 {
            format!(" ({:+.0}%)", (tc.week_usd / tc.prev_week_usd - 1.0) * 100.0)
        } else {
            String::new()
        };
        out += &format!(
            r#"<h4>Token check</h4><p>Last 7 days {} against {} the week before{}. <span class="dim">{}</span></p>"#,
            usd(tc.week_usd),
            usd(tc.prev_week_usd),
            esc(&ch),
            esc(&tc.label)
        );
    }
    if !f.markers.is_empty() {
        let li: String = f
            .markers
            .iter()
            .map(|m| format!("<li><b>{}</b> {}</li>", esc(&m.date), esc(&m.text)))
            .collect();
        out += &format!("<h4>Changes in the setup</h4><ul>{li}</ul>");
    }
    if !f.gaps.is_empty() {
        let li: String = f
            .gaps
            .iter()
            .map(|g| format!("<li>{}</li>", esc(g)))
            .collect();
        out += &format!(
            r#"<div class="finding amber">Not everything could be read; the numbers are a lower bound:<ul>{li}</ul></div>"#
        );
    }
    out += &format!(
        r#"<div class="note">Public repositories of {} only, read from GitHub without a token; CI from the ground station's ledger. Text: {}{}.</div>"#,
        esc(&f.org),
        esc(&b.source),
        b.model
            .as_deref()
            .map(|m| format!(" ({})", esc(m)))
            .unwrap_or_default()
    );
    out
}

pub fn view(store: Option<&Store>, period: Option<&str>) -> String {
    let Some(s) = store else {
        return r#"<div class="overview"><h2>Briefing</h2><p class="empty">No briefing configured. Add a <code>[briefing]</code> section with the GitHub organisation.</p></div>"#.into();
    };
    let b = match period {
        Some(p) => s.get(p),
        None => s.latest(),
    };
    let Some(b) = b else {
        return r#"<div class="overview"><h2>Briefing</h2><p class="empty">The first briefing is written after the first two-day period closes.</p></div>"#.into();
    };
    let bullets: String = b
        .text
        .bullets
        .iter()
        .map(|x| format!("<li>{}</li>", esc(x)))
        .collect();
    let rejected = b
        .rejected
        .as_deref()
        .map(|r| {
            format!(
                r#"<div class="finding amber">The written text was not used: {}. This is the plain version made from the facts.</div>"#,
                esc(r)
            )
        })
        .unwrap_or_default();
    let others: String = s
        .periods()
        .iter()
        .take(15)
        .map(|p| {
            let cur = if *p == b.facts.from { " active" } else { "" };
            format!(
                r##"<button class="nav-btn{cur}" hx-get="api/dashboard/briefing-partial?from={p}" hx-target="#content" hx-swap="innerHTML" data-no-refresh>{}</button>"##,
                esc(&pretty(p))
            )
        })
        .collect();
    format!(
        r#"<div class="overview briefing"><h2>Briefing <span class="pill">{} – {}</span></h2>{rejected}<section class="panel"><p class="brief-line">{}</p><ul class="brief-bullets">{bullets}</ul><details><summary>For the IT reader</summary>{}</details></section><section class="panel"><h3>Earlier briefings</h3><div class="periods">{others}</div></section></div>"#,
        esc(&pretty(&b.facts.from)),
        esc(&pretty(&b.facts.until)),
        esc(&b.text.line),
        it_section(&b)
    )
}

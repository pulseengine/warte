//! Skills view (REQ-SKILLS-001): invocations per skill, summed across sources
//! and per source, the skills never invoked, the share of sessions that used
//! any skill, and the window and age of each report. Every name comes from a
//! workstation: escaped.

use super::fleet::{dur_pub as dur, esc};
use crate::skills::{summarize, Skills, Stored, STALE_S};

fn share(part: u64, whole: u64) -> String {
    if whole == 0 {
        "n/a".into()
    } else {
        format!("{:.0}%", part as f64 * 100.0 / whole as f64)
    }
}

fn table(head: &str, rows: String) -> String {
    format!(
        r#"<div class="scroll"><table class="data-table"><thead><tr>{head}</tr></thead><tbody>{rows}</tbody></table></div>"#
    )
}

fn report_row(s: &Stored, now: i64) -> String {
    let age = now - s.received_at;
    let (pill, word) = if age <= STALE_S {
        ("pill", "fresh")
    } else {
        ("pill stale", "stale")
    };
    let r = &s.report;
    format!(
        r#"<tr><td>{}</td><td>{} to {}</td><td><span class="{pill}">{word} · {} ago</span></td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td></tr>"#,
        esc(&r.source),
        esc(&r.window.from),
        esc(&r.window.to),
        dur(age),
        r.sessions_with_skill,
        r.sessions_total,
        share(r.sessions_with_skill, r.sessions_total),
    )
}

pub fn view(skills: Option<&Skills>, now: i64) -> String {
    let all = skills.map(Skills::all).unwrap_or_default();
    if all.is_empty() {
        let why = match skills.and_then(Skills::rejected) {
            Some(why) => format!(
                r#"<div class="finding red">The last report was rejected and is not shown: {}</div>"#,
                esc(&why)
            ),
            None => String::new(),
        };
        return format!(
            r#"<div class="overview"><h2>Skills</h2>{why}<p class="empty">No skill-usage report received yet. A workstation job sends one nightly.</p></div>"#
        );
    }
    let sum = summarize(&all);
    let newest = all.iter().map(|s| s.received_at).max().unwrap_or(0);
    let age = now - newest;
    let (pill, word) = if age <= STALE_S {
        ("pill", "fresh")
    } else {
        ("pill stale", "stale")
    };
    let rejected = match skills.and_then(Skills::rejected) {
        Some(why) => format!(
            r#"<div class="finding red">The last report from the file was rejected and is not shown: {}</div>"#,
            esc(&why)
        ),
        None => String::new(),
    };
    let kpis = format!(
        r#"<div class="kpis"><div class="kpi"><div class="l">Sessions using any skill</div><div class="v">{}</div><div class="s">{} of {} sessions</div></div><div class="kpi"><div class="l">Skills invoked</div><div class="v">{}</div><div class="s">across {} source(s)</div></div><div class="kpi"><div class="l">Never invoked</div><div class="v">{}</div><div class="s">installed, no use in the window</div></div></div>"#,
        share(sum.sessions_with_skill, sum.sessions_total),
        sum.sessions_with_skill,
        sum.sessions_total,
        sum.skills
            .iter()
            .filter(|t| t.invocations.total() > 0)
            .count(),
        all.len(),
        sum.never_invoked.len(),
    );
    let head = r#"<th>Skill</th><th>Plugin</th><th class="n">Model</th><th class="n">Slash</th><th class="n">Subagent</th><th class="n">Sessions</th><th class="n">SKILL.md reads</th>"#;
    let total_rows: String = sum
        .skills
        .iter()
        .map(|t| {
            format!(
                r#"<tr><td>{}</td><td class="dim">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td></tr>"#,
                esc(&t.name),
                esc(&t.plugin),
                t.invocations.model,
                t.invocations.user_slash,
                t.invocations.subagent,
                t.sessions,
                t.skill_md_reads
            )
        })
        .collect();
    let source_rows: String = sum
        .skills
        .iter()
        .flat_map(|t| t.by_source.iter())
        .map(|(src, k)| {
            format!(
                r#"<tr><td>{}</td><td>{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td></tr>"#,
                esc(src),
                esc(&k.name),
                k.invocations.model,
                k.invocations.user_slash,
                k.invocations.subagent,
                k.sessions,
                k.skill_md_reads
            )
        })
        .collect();
    let never = if sum.never_invoked.is_empty() {
        r#"<p class="empty">Every installed skill was invoked.</p>"#.to_string()
    } else {
        let items: String = sum
            .never_invoked
            .iter()
            .map(|n| format!(r#"<li class="mono">{}</li>"#, esc(n)))
            .collect();
        format!("<ul>{items}</ul>")
    };
    format!(
        r#"<div class="overview"><h2>Skills <span class="{pill}">{word} · {} ago</span></h2>{rejected}<section class="panel">{kpis}</section><section class="panel"><h3>Reports</h3>{}</section><section class="panel"><h3>Invocations per skill · all sources</h3>{}</section><section class="panel"><h3>Never invoked</h3>{never}<div class="note">Installed on some source, with no invocation in any report's window.</div></section><section class="panel"><h3>Invocations per source</h3>{}</section><div class="note">Counted from local agent transcripts by a workstation job. Model: the agent chose the skill; slash: the user did; subagent: a subagent did. Use measures what happened, not what was intended.</div></div>"#,
        dur(age),
        table(
            r#"<th>Source</th><th>Window</th><th>Report</th><th class="n">Sessions with a skill</th><th class="n">Sessions</th><th class="n">Share</th>"#,
            all.iter().map(|s| report_row(s, now)).collect()
        ),
        table(head, total_rows),
        table(
            r#"<th>Source</th><th>Skill</th><th class="n">Model</th><th class="n">Slash</th><th class="n">Subagent</th><th class="n">Sessions</th><th class="n">SKILL.md reads</th>"#,
            source_rows
        ),
    )
}

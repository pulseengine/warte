//! Costs view (REQ-COSTS-001): notional $ per day, top sessions, repos,
//! models and task categories, and the share of subagent work on a cheaper
//! tier. Every name in the report comes from a workstation: escaped.

use super::fleet::{dur_pub as dur, esc};
use crate::costs::{CostReport, Costs, Effort, Signals, Week, Window, LABEL, STALE_S};
use std::collections::BTreeMap;

fn usd(x: f64) -> String {
    if x >= 1000.0 {
        format!("${:.0}", x)
    } else if x >= 10.0 {
        format!("${:.1}", x)
    } else {
        format!("${:.2}", x)
    }
}

fn pct(x: Option<f64>) -> String {
    x.map_or("n/a".into(), |x| format!("{:.0}%", x * 100.0))
}

fn mtok(t: u64) -> String {
    format!("{:.1}M", t as f64 / 1e6)
}

fn bar(label: &str, value: f64, max: f64, note: &str) -> String {
    let w = if max > 0.0 { value * 100.0 / max } else { 0.0 };
    format!(
        r#"<div class="hbar"><span class="name">{}</span><span class="bar"><i style="width:{:.0}%"></i></span><span class="n">{}</span><span class="dim">{}</span></div>"#,
        esc(label),
        w.clamp(0.0, 100.0),
        usd(value),
        esc(note)
    )
}

/// The last `days` days ending at `today`, oldest first, zero where the report has no entry.
fn last_days(r: &CostReport, today: chrono::NaiveDate, days: i64) -> Vec<(String, f64, f64)> {
    let by: std::collections::BTreeMap<&str, (f64, f64)> = r
        .daily
        .iter()
        .map(|d| (d.day.as_str(), (d.usd, d.usd_subagents_down)))
        .collect();
    (0..days)
        .rev()
        .map(|i| {
            let d = (today - chrono::Duration::days(i))
                .format("%Y-%m-%d")
                .to_string();
            let (u, down) = by.get(d.as_str()).copied().unwrap_or((0.0, 0.0));
            (d, u, down)
        })
        .collect()
}

fn table(head: &str, rows: String) -> String {
    format!(
        r#"<div class="scroll"><table class="data-table"><thead><tr>{head}</tr></thead><tbody>{rows}</tbody></table></div>"#
    )
}

fn breakdowns(w: &Window) -> String {
    let repos: String = w
        .by_repo
        .iter()
        .take(12)
        .map(|r| bar(&r.repo, r.usd, w.by_repo.first().map_or(0.0, |x| x.usd), ""))
        .collect();
    let models: String = w
        .by_model
        .iter()
        .map(|m| {
            format!(
                r#"<tr><td>{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td></tr>"#,
                esc(&m.model),
                usd(m.usd),
                mtok(m.tokens.input + m.tokens.cache_write_5m + m.tokens.cache_write_1h),
                mtok(m.tokens.cache_read),
                mtok(m.tokens.output),
                m.tokens.requests
            )
        })
        .collect();
    let cats: String = w
        .by_category
        .iter()
        .map(|c| {
            bar(
                &c.category,
                c.usd,
                w.by_category.first().map_or(0.0, |x| x.usd),
                &format!("{} subagents", c.subagents),
            )
        })
        .collect();
    let empty = |s: String| {
        if s.is_empty() {
            r#"<p class="empty">Nothing in this window.</p>"#.to_string()
        } else {
            s
        }
    };
    format!(
        r#"<div class="split"><section class="panel"><h3>Top repositories</h3><div class="hbars">{}</div></section><section class="panel"><h3>By task category · subagents</h3><div class="hbars">{}</div><div class="note">A mechanical classifier over each Agent call's short description and subagent type: approximate. Main-session spend has no category.</div></section></div><section class="panel"><h3>By model</h3>{}</section>"#,
        empty(repos),
        empty(cats),
        table(
            r#"<th>Model</th><th class="n">Notional $</th><th class="n">Input + cache writes</th><th class="n">Cache reads</th><th class="n">Output</th><th class="n">Requests</th>"#,
            models
        ),
    )
}

/// Notional $ of an ISO week, from the daily rollup.
fn week_usd(r: &CostReport, week: &str) -> f64 {
    r.daily
        .iter()
        .filter(|d| {
            chrono::NaiveDate::parse_from_str(&d.day, "%Y-%m-%d").is_ok_and(|n| {
                let w = chrono::Datelike::iso_week(&n);
                format!("{}-W{:02}", w.year(), w.week()) == week
            })
        })
        .map(|d| d.usd)
        .sum()
}

/// Summed effort rows of a week.
fn effort_total(w: &Week) -> Effort {
    let mut t = Effort::default();
    for e in &w.effort {
        for (k, n) in &e.tool_uses {
            *t.tool_uses.entry(k.clone()).or_default() += n;
        }
        for (k, n) in &e.tool_errors {
            *t.tool_errors.entry(k.clone()).or_default() += n;
        }
        t.permission_denials += e.permission_denials;
        t.repeated_failures += e.repeated_failures;
        // measured when any row measured it
        if let Some(n) = e.loop_guard_fired {
            *t.loop_guard_fired.get_or_insert(0) += n;
        }
    }
    t
}

/// A count that may not have been measured: "not measured" is not 0.
fn measured(n: Option<u64>) -> String {
    n.map_or_else(|| "not measured".into(), |n| n.to_string())
}

fn rate(errors: u64, uses: u64) -> String {
    if uses == 0 {
        "n/a".into()
    } else {
        format!("{:.1}%", errors as f64 * 100.0 / uses as f64)
    }
}

/// Schema 2 (REQ-COSTS-002): the newest week against the one before, and
/// where the failed tool calls of the newest week were.
fn weekly(r: &CostReport) -> String {
    let Some(weeks) = r.weeks.as_ref() else {
        return String::new();
    };
    let mut ws: Vec<&Week> = weeks.iter().collect();
    ws.sort_by(|a, b| b.week.cmp(&a.week));
    let Some(cur) = ws.first() else {
        return r#"<section class="panel"><h3>Token check</h3><p class="empty">The report has no week yet.</p></section>"#.into();
    };
    let prev = ws.get(1);
    let (ec, ep) = (effort_total(cur), prev.map(|p| effort_total(p)));
    let sum = |m: &BTreeMap<String, u64>| m.values().sum::<u64>();
    let row = |label: &str, c: String, p: Option<String>| {
        format!(
            r#"<tr><td>{}</td><td class="n">{}</td><td class="n"><b>{}</b></td></tr>"#,
            esc(label),
            esc(&p.unwrap_or_else(|| "n/a".into())),
            esc(&c)
        )
    };
    let side = |w: &Week, f: fn(&Signals) -> u64| format!("{} / {}", f(&w.main), f(&w.sidechain));
    let mut rows = String::new();
    rows += &row(
        "Sessions",
        cur.sessions.to_string(),
        prev.map(|p| p.sessions.to_string()),
    );
    rows += &row(
        "Notional $",
        usd(week_usd(r, &cur.week)),
        prev.map(|p| usd(week_usd(r, &p.week))),
    );
    for (label, f) in [
        (
            "Advisor calls · main / subagents",
            (|s: &Signals| s.advisor_calls) as fn(&Signals) -> u64,
        ),
        ("Compactions · main / subagents", |s| s.compactions),
        ("Cross-session messages in · main / subagents", |s| {
            s.cross_session_in
        }),
        ("SendMessage calls · main / subagents", |s| {
            s.sendmessage_calls
        }),
    ] {
        rows += &row(label, side(cur, f), prev.map(|p| side(p, f)));
    }
    let errs = |e: &Effort| {
        format!(
            "{} of {} ({})",
            sum(&e.tool_errors),
            sum(&e.tool_uses),
            rate(sum(&e.tool_errors), sum(&e.tool_uses))
        )
    };
    rows += &row("Tool errors", errs(&ec), ep.as_ref().map(errs));
    rows += &row(
        "Permission denials",
        ec.permission_denials.to_string(),
        ep.as_ref().map(|e| e.permission_denials.to_string()),
    );
    rows += &row(
        "Repeated failures",
        ec.repeated_failures.to_string(),
        ep.as_ref().map(|e| e.repeated_failures.to_string()),
    );
    rows += &row(
        "Loop guard fired",
        measured(ec.loop_guard_fired),
        ep.as_ref().map(|e| measured(e.loop_guard_fired)),
    );
    let check = table(
        &format!(
            r#"<th></th><th class="n">{}</th><th class="n">{}</th>"#,
            esc(prev.map_or("previous", |p| p.week.as_str())),
            esc(&cur.week)
        ),
        rows,
    );
    // by tool, worst error count first
    let mut tools: Vec<(&String, u64, u64)> = ec
        .tool_uses
        .iter()
        .map(|(t, u)| (t, ec.tool_errors.get(t).copied().unwrap_or(0), *u))
        .filter(|x| x.1 > 0)
        .collect();
    tools.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let tool_rows: String = tools
        .iter()
        .take(10)
        .map(|(t, e, u)| {
            format!(
                r#"<tr><td class="mono">{}</td><td class="n">{e}</td><td class="n">{u}</td><td class="n">{}</td></tr>"#,
                esc(t),
                rate(*e, *u)
            )
        })
        .collect();
    // by repository and skill, most wasted first
    let mut by: Vec<&Effort> = cur.effort.iter().collect();
    let waste = |e: &Effort| {
        sum(&e.tool_errors)
            + e.permission_denials
            + e.repeated_failures
            + e.loop_guard_fired.unwrap_or(0)
    };
    by.sort_by(|a, b| waste(b).cmp(&waste(a)).then(a.repo.cmp(&b.repo)));
    let by_rows: String = by
        .iter()
        .filter(|e| waste(e) > 0)
        .take(10)
        .map(|e| {
            format!(
                r#"<tr><td>{}</td><td class="mono">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td></tr>"#,
                esc(&e.repo),
                esc(&e.skill),
                e.sessions,
                errs(e),
                e.permission_denials,
                e.repeated_failures,
                measured(e.loop_guard_fired),
                sum(&e.tool_uses)
            )
        })
        .collect();
    let detail = if tool_rows.is_empty() && by_rows.is_empty() {
        format!(
            r#"<p class="empty">No failed tool call in {}.</p>"#,
            esc(&cur.week)
        )
    } else {
        format!(
            r#"<div class="split"><section class="panel"><h3>Failed calls by tool · {w}</h3>{}</section><section class="panel"><h3>Wasted effort by repository and skill · {w}</h3>{}</section></div>"#,
            table(
                r#"<th>Tool</th><th class="n">Errors</th><th class="n">Uses</th><th class="n">Rate</th>"#,
                tool_rows
            ),
            table(
                r#"<th>Repository</th><th>Skill</th><th class="n">Sessions</th><th class="n">Tool errors</th><th class="n">Denials</th><th class="n">Streaks</th><th class="n">Loop guard</th><th class="n">Uses</th>"#,
                by_rows
            ),
            w = esc(&cur.week)
        )
    };
    format!(
        r#"<section class="panel"><h3>Token check · week against week</h3>{check}<div class="note">Counts from the transcripts, by ISO week (UTC); subagent records apart from the main session. A permission denial is a refused tool call, not counted as a tool error. A streak is {} or more failed calls of the same tool in a row. No message or error text leaves the workstation.</div></section>{detail}"#,
        crate::costs::STREAK
    )
}

pub fn view(costs: Option<&Costs>, now: i64) -> String {
    let banner = format!(
        r#"<div class="err"><b>{}.</b> List prices applied to the token counts in agent transcripts. The owner is on a subscription: none of this is billed.</div>"#,
        esc(LABEL)
    );
    let Some(stored) = costs.and_then(Costs::latest) else {
        let why = match costs.and_then(Costs::rejected) {
            Some(why) => format!(
                r#"<div class="finding red">The last report was rejected and is not shown: {}</div>"#,
                esc(&why)
            ),
            None => String::new(),
        };
        return format!(
            r#"<div class="overview"><h2>Costs</h2>{banner}{why}<p class="empty">No cost report received yet. A workstation job sends one nightly.</p></div>"#
        );
    };
    let r = &stored.report;
    let age = now - stored.received_at;
    let (pill, word) = if age <= STALE_S {
        ("pill", "fresh")
    } else {
        ("pill stale", "stale")
    };
    let today = chrono::DateTime::from_timestamp(now, 0)
        .unwrap_or_default()
        .date_naive();
    let w30 = r.windows.get("last_30d").cloned().unwrap_or_default();
    let all = r.windows.get("all").cloned().unwrap_or_default();
    let month = today.format("%Y-%m").to_string();
    let wm = r.windows.get(&month).cloned().unwrap_or_default();
    let ch = &w30.cheaper_tier;
    let kpis = format!(
        r#"<div class="kpis"><div class="kpi"><div class="l">Last 30 days</div><div class="v">{}</div><div class="s">{} sessions</div></div><div class="kpi"><div class="l">{}</div><div class="v">{}</div><div class="s">month to date</div></div><div class="kpi"><div class="l">All history</div><div class="v">{}</div><div class="s">{} sessions</div></div><div class="kpi"><div class="l">Cheaper tier · 30 d</div><div class="v">{}</div><div class="s">of subagent calls ({} of {}); {} of subagent $</div></div><div class="kpi"><div class="l">Split · 30 d</div><div class="v mono">{} / {} / {}</div><div class="s">main / subagents / advisor</div></div></div>"#,
        usd(w30.usd),
        w30.sessions,
        esc(&month),
        usd(wm.usd),
        usd(all.usd),
        all.sessions,
        pct(ch.call_share),
        ch.calls_down,
        ch.calls_down + ch.calls_same + ch.calls_up,
        pct(ch.usd_share),
        usd(w30.usd_main),
        usd(w30.usd_subagents),
        usd(w30.usd_advisor),
    );
    let days = last_days(r, today, 30);
    let max = days.iter().map(|d| d.1).fold(0.0, f64::max);
    let daily: String = days
        .iter()
        .map(|(d, u, down)| {
            let note = if *down > 0.0 {
                format!("{} cheaper tier", usd(*down))
            } else {
                String::new()
            };
            bar(&d[5..], *u, max, &note)
        })
        .collect();
    let since = (today - chrono::Duration::days(29))
        .format("%Y-%m-%d")
        .to_string();
    let top: String = r
        .sessions
        .iter()
        .filter(|s| s.date >= since)
        .take(15)
        .map(|s| {
            format!(
                r#"<tr><td>{}</td><td>{}</td><td>{}</td><td class="dim">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n"><b>{}</b></td></tr>"#,
                esc(&s.date),
                esc(&s.machine),
                esc(&s.repo),
                esc(&s.model),
                s.n_subagents,
                usd(s.usd_main),
                usd(s.usd_subagents),
                usd(s.usd_advisor),
                usd(s.usd)
            )
        })
        .collect();
    let unpriced = if all.unpriced_requests > 0 {
        format!(
            r#"<div class="finding amber">{} requests on models without a list price are not included: {}</div>"#,
            all.unpriced_requests,
            esc(&r
                .unpriced_models
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "))
        )
    } else {
        String::new()
    };
    format!(
        r#"<div class="overview"><h2>Costs <span class="{pill}">{word} · {} ago</span></h2>{banner}{unpriced}<section class="panel">{kpis}</section><section class="panel"><h3>Notional $ per day · last 30 days</h3><div class="hbars">{daily}</div></section><section class="panel"><h3>Top sessions · last 30 days</h3>{}</section>{}{}<div class="note">Prices: list prices of {} (<a href="{}">source</a>), per request: 5m and 1h cache writes apart, the long-prompt tier where a model has one, advisor calls on their own model. Report generated {}, received {} ago. A session's date is its first message; the daily bars use each message's own date.</div></div>"#,
        dur(age),
        table(
            r#"<th>Date</th><th>Machine</th><th>Repository</th><th>Model</th><th class="n">Subagents</th><th class="n">Main</th><th class="n">Subagents</th><th class="n">Advisor</th><th class="n">Total</th>"#,
            top
        ),
        breakdowns(&w30),
        weekly(r),
        esc(&r.prices.version),
        esc(&r.prices.source),
        esc(&r.generated_at),
        dur(age),
    )
}

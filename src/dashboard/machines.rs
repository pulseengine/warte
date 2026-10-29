//! Machines view: every machine the ground station hears from (REQ-PROBE-001,
//! REQ-PROBE-003), plus the CI host as the smithy agent reports it.
//! Host names, paths and tool output come from the machines: escaped.

use super::fleet::{dur_pub as dur, esc};
use crate::probe::{findings, Finding, Machines, Seen, MACHINE_STALE_S};

fn meter(label: &str, value: String, pct: f64, hot: bool) -> String {
    format!(
        r#"<div class="hbar"><span class="name">{label}</span><span class="bar"><i class="{}" style="width:{:.0}%"></i></span><span class="n">{value}</span></div>"#,
        if hot { "hot" } else { "" },
        pct.clamp(0.0, 100.0)
    )
}

fn card(s: &Seen, mine: &[&Finding], now: i64) -> String {
    let r = &s.report;
    let age = now - s.received_at;
    let (pill, word) = if age <= MACHINE_STALE_S {
        ("pill", "live")
    } else {
        ("pill stale", "stale")
    };
    let mut body = String::new();
    if r.cpu.ncpu > 0 {
        body += &meter(
            "load",
            format!("{:.1} on {}", r.cpu.load[0], r.cpu.ncpu),
            r.cpu.load[0] * 100.0 / r.cpu.ncpu as f64,
            r.cpu.load[1] > r.cpu.ncpu as f64,
        );
    }
    if r.memory.total_mb > 0 {
        let used = r.memory.total_mb.saturating_sub(r.memory.avail_mb);
        let pct = used as f64 * 100.0 / r.memory.total_mb as f64;
        body += &meter(
            "memory",
            format!(
                "{:.1} of {:.1} GB",
                used as f64 / 1024.0,
                r.memory.total_mb as f64 / 1024.0
            ),
            pct,
            pct >= 90.0,
        );
    }
    for d in &r.disks {
        body += &meter(
            &esc(&d.mount),
            format!("{:.0}% of {:.0} GB", d.used_pct, d.size_gb),
            d.used_pct,
            d.used_pct >= 85.0,
        );
    }
    let tools: String = r
        .tools
        .iter()
        .map(|t| match (&t.path, &t.version) {
            (None, _) => format!(r#"<span class="chip off">{}</span>"#, esc(&t.name)),
            (_, Some(v)) => format!(
                r#"<span class="chip on">{} {}</span>"#,
                esc(&t.name),
                esc(v)
            ),
            (_, None) => format!(
                r#"<span class="chip warn" title="{}">{} ?</span>"#,
                esc(t.error.as_deref().unwrap_or_default()),
                esc(&t.name)
            ),
        })
        .collect();
    let mut agents: std::collections::BTreeMap<&str, usize> = Default::default();
    for x in &r.sessions {
        *agents.entry(x.agent.as_str()).or_default() += 1;
    }
    let sessions = if agents.is_empty() {
        r#"<span class="dim">no agent sessions</span>"#.to_string()
    } else {
        agents
            .iter()
            .map(|(a, n)| format!(r#"<span class="badge green">{n} {}</span>"#, esc(a)))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let notes: String = mine
        .iter()
        .map(|f| {
            format!(
                r#"<div class="finding {}">{}</div>"#,
                f.severity,
                esc(&f.text)
            )
        })
        .collect();
    let uptime = if r.uptime_s >= 86400 {
        format!("up {}d", r.uptime_s / 86400)
    } else {
        format!("up {}", dur(r.uptime_s as i64))
    };
    format!(
        r#"<div class="machine{}"><div class="mhead"><span class="mname">{}</span><span class="{pill}">{word} · {} ago</span></div><div class="mos">{} · {} · {} · {uptime}</div>{notes}<div class="hbars">{body}</div><div class="mtools">{tools}</div><div class="msess">{sessions}</div><div class="mvia">via {}</div></div>"#,
        if mine.iter().any(|f| f.severity == "red") {
            " red"
        } else if mine.is_empty() {
            ""
        } else {
            " amber"
        },
        esc(&r.host),
        dur(age),
        esc(&r.os.name),
        esc(&r.os.kernel),
        esc(&r.os.arch),
        esc(&s.via),
    )
}

/// The CI host as the smithy agent reports it: a smaller card, no tools.
fn ci_card(hub: &crate::hub::Hub, now: i64) -> Option<String> {
    let (s, at) = hub.snapshot()?;
    let m = &s.machine;
    let mut body = String::new();
    if m.ncpu > 0 {
        body += &meter(
            "load",
            format!("{:.1} on {}", m.load[0], m.ncpu),
            m.load[0] * 100.0 / m.ncpu as f64,
            m.load[1] > m.ncpu as f64,
        );
        let used = m.mem_total_mb.saturating_sub(m.mem_avail_mb);
        body += &meter(
            "memory",
            format!(
                "{:.0} of {:.0} GB",
                used as f64 / 1024.0,
                m.mem_total_mb as f64 / 1024.0
            ),
            used as f64 * 100.0 / m.mem_total_mb.max(1) as f64,
            false,
        );
    }
    for d in &m.disks {
        body += &meter(
            &esc(&d.mount),
            format!("{:.0}% of {:.0} GB", d.used_pct, d.size_gb),
            d.used_pct,
            d.used_pct >= 85.0,
        );
    }
    let busy = s.runners.iter().filter(|r| r.local_busy).count();
    Some(format!(
        r#"<div class="machine"><div class="mhead"><span class="mname">{}</span><span class="pill">live · {} ago</span></div><div class="mos">CI host · {busy}/{} runners busy</div><div class="hbars">{body}</div><div class="mvia">via smithy agent {} (CCSDS); details under Health</div></div>"#,
        esc(&s.host),
        dur(now - at),
        s.runners.len(),
        esc(&s.agent_version),
    ))
}

pub fn view(machines: Option<&Machines>, hub: Option<&crate::hub::Hub>) -> String {
    let now = chrono::Utc::now().timestamp();
    let seen = machines.map(Machines::list).unwrap_or_default();
    let all = findings(&seen, now);
    let mut cards: String = seen
        .iter()
        .map(|s| {
            let mine: Vec<&Finding> = all.iter().filter(|f| f.host == s.report.host).collect();
            card(s, &mine, now)
        })
        .collect();
    if let Some(c) = hub.and_then(|h| ci_card(h, now)) {
        cards += &c;
    }
    let summary = if all.is_empty() {
        r#"<div class="allclear"><span class="pill">no drift</span> Every machine reporting is current and consistent.</div>"#.to_string()
    } else {
        all.iter()
            .map(|f| {
                format!(
                    r#"<div class="incident sev{}"><span class="badge {}">{}</span><b>{}</b>&nbsp;{}</div>"#,
                    if f.severity == "red" { 2 } else { 1 },
                    if f.severity == "red" { "red" } else { "yellow" },
                    if f.severity == "red" { "critical" } else { "drift" },
                    esc(&f.host),
                    esc(&f.text)
                )
            })
            .collect()
    };
    let empty = if cards.is_empty() {
        r#"<p class="empty">No machine has reported yet. The ground station reports itself within a minute of starting; other machines appear once enrolled (<code>warte probe</code>).</p>"#
    } else {
        ""
    };
    format!(
        r#"<div class="overview"><h2>Machines</h2><section class="panel"><h3>Drift and problems</h3>{summary}</section>{empty}<div class="machines">{cards}</div><div class="note">Each machine reports every minute with <code>warte probe</code>. Agent sessions show the agent and working directory only, never command lines. More machines appear as they are enrolled.</div></div>"#
    )
}

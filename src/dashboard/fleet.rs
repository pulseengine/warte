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

/// `dur` for other modules (the probe's findings).
pub fn dur_pub(s: i64) -> String {
    dur(s)
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

/// Queue view: the fleet's queue at a glance, each pool with its measured
/// waits, and every queued run on the departures board.
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
    let nbusy = s.runners.iter().filter(|r| busy(r)).count();
    let self_hosted: u32 = q
        .pools
        .iter()
        .filter(|(p, _)| p.as_str() != "hosted")
        .map(|(_, p)| p.jobs_queued)
        .sum();
    let mut kpis = kpi(
        "Runners busy",
        &format!("{nbusy}/{}", s.runners.len()),
        "",
        "host view",
        "",
    );
    kpis += &kpi(
        "Holding · self-hosted",
        &self_hosted.to_string(),
        if self_hosted >= 50 { "warn" } else { "" },
        &format!(
            "jobs in {} runs{}",
            q.runs_queued,
            if q.truncated { " (lower bound)" } else { "" }
        ),
        "",
    );
    kpis += &kpi(
        "GitHub-hosted",
        &q.hosted_queued.to_string(),
        "",
        &format!("queued · {} running", q.hosted_running),
        "",
    );
    kpis += &kpi(
        "Repos scanned",
        &q.repos_scanned.to_string(),
        if q.queued_complete { "" } else { "warn" },
        if q.queued_complete {
            "queue list complete"
        } else {
            "list incomplete"
        },
        "",
    );
    let peak = q
        .pools
        .values()
        .map(|p| p.jobs_queued)
        .max()
        .unwrap_or(0)
        .max(1);
    let pools: String = q
        .pools
        .iter()
        .map(|(pool, p)| {
            let w = q
                .waits
                .get(pool)
                .map(|w| format!("p50 {} · p90 {}", dur(w.p50), dur(w.p90)))
                .unwrap_or_else(|| "no recent starts".into());
            format!(
                r#"<div class="pool"><span class="name">{}</span>{}<span class="n">{}</span><span class="dim">oldest {} · median {}</span><span class="dim">waits {w}</span></div>"#,
                esc(pool),
                meter(p.jobs_queued as f64 * 100.0 / peak as f64, p.jobs_queued >= 50),
                p.jobs_queued,
                dur(p.oldest_s),
                dur(p.median_s),
            )
        })
        .collect();
    format!(
        r#"<section class="kpis">{kpis}</section><section class="panel"><h3>Pools · jobs waiting</h3><div class="pools">{pools}</div></section><section class="panel"><h3>Departures · every queued run</h3>{}</section>{}"#,
        departures(q, now(), usize::MAX),
        freshness(at)
    )
}

/// Critical-path inputs: which repos hold the queue, and how long each pool
/// really makes jobs wait (p50, p90, max on one scale).
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
    let mut per_repo: std::collections::BTreeMap<&str, (u32, usize)> = Default::default();
    for e in &q.queued {
        let r = per_repo.entry(e.repo.as_str()).or_default();
        r.0 += e.jobs;
        r.1 += 1;
    }
    let mut repos: Vec<(&str, (u32, usize))> = per_repo.into_iter().collect();
    repos.sort_by_key(|r| std::cmp::Reverse(r.1 .0));
    let top = repos.first().map(|r| r.1 .0).unwrap_or(0).max(1);
    let total: u32 = repos.iter().map(|r| r.1 .0).sum();
    let bars = if repos.is_empty() {
        r#"<p class="empty">Nothing queued.</p>"#.to_string()
    } else {
        repos
            .iter()
            .take(12)
            .map(|(r, (jobs, runs))| {
                format!(
                    r#"<div class="hbar"><span class="name">{}</span>{}<span class="n">{jobs} jobs queued</span><span class="dim">{runs} runs · {:.0}%</span></div>"#,
                    esc(r),
                    meter(*jobs as f64 * 100.0 / top as f64, *jobs as f64 >= total as f64 * 0.4),
                    *jobs as f64 * 100.0 / total.max(1) as f64,
                )
            })
            .collect()
    };
    let waits = if q.waits.is_empty() {
        r#"<p class="empty">No measured waits in the latest snapshot.</p>"#.to_string()
    } else {
        let scale = q.waits.values().map(|w| w.max).max().unwrap_or(1).max(1) as f64;
        let rows: String = q
            .waits
            .iter()
            .map(|(pool, w)| {
                let pct = |v: i64| v as f64 * 100.0 / scale;
                format!(
                    r#"<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class="viz"><span class="range"><i class="p90" style="width:{:.0}%"></i><i class="p50" style="width:{:.0}%"></i><i class="max" style="left:{:.0}%"></i></span></td></tr>"#,
                    esc(pool),
                    w.n,
                    dur(w.p50),
                    dur(w.p90),
                    dur(w.max),
                    pct(w.p90),
                    pct(w.p50),
                    pct(w.max).min(99.0),
                )
            })
            .collect();
        format!(
            r#"<table class="data-table"><thead><tr><th>Runner pool</th><th>Recent starts</th><th>Wait p50</th><th>p90</th><th>max</th><th>p50 ▮ p90 ▯ max |</th></tr></thead><tbody>{rows}</tbody></table>"#
        )
    };
    format!(
        r#"<section class="panel"><h3>Who holds the queue</h3><div class="hbars">{bars}</div><div class="note">Share of all queued jobs; amber when one repo holds 40% or more.</div></section><section class="panel"><h3>Measured pool waits · last 3 h</h3>{waits}<div class="note">How long jobs that started recently actually waited, per pool, all on one scale. Ask uses these to estimate a PR's start.</div></section>{}"#,
        freshness(at)
    )
}

/// Health: the CI host's machine, the telemetry link per stream, and what
/// the agent could not collect.
pub fn health_view(hub: Option<&Hub>) -> String {
    let Some((hub, (s, at))) = hub.and_then(|h| h.snapshot().map(|v| (h, v))) else {
        return format!(r#"<section class="panel">{}</section>"#, unavailable(hub));
    };
    let m = &s.machine;
    let pct = |a: f64, b: f64| if b > 0.0 { a * 100.0 / b } else { 0.0 };
    let line = |label: &str, value: String, p: f64, hot: bool| {
        format!(
            r#"<div class="hbar"><span class="name">{label}</span>{}<span class="n">{value}</span></div>"#,
            meter(p, hot)
        )
    };
    let mut host = String::new();
    if m.ncpu > 0 {
        host += &line(
            "load",
            format!(
                "{:.1} / {:.1} / {:.1} on {}",
                m.load[0], m.load[1], m.load[2], m.ncpu
            ),
            pct(m.load[0], m.ncpu as f64),
            m.load[0] > m.ncpu as f64,
        );
        let used = m.mem_total_mb.saturating_sub(m.mem_avail_mb);
        host += &line(
            "memory",
            format!(
                "{:.0} of {:.0} GB used",
                used as f64 / 1024.0,
                m.mem_total_mb as f64 / 1024.0
            ),
            pct(used as f64, m.mem_total_mb as f64),
            pct(used as f64, m.mem_total_mb as f64) >= 90.0,
        );
        host += &line(
            "swap",
            format!(
                "{:.1} of {:.0} GB",
                m.swap_used_mb as f64 / 1024.0,
                m.swap_total_mb as f64 / 1024.0
            ),
            pct(m.swap_used_mb as f64, m.swap_total_mb as f64),
            m.swap_used_mb > 0 && pct(m.swap_used_mb as f64, m.swap_total_mb as f64) >= 50.0,
        );
        for (name, p) in [
            ("cpu pressure", m.psi.cpu),
            ("mem pressure", m.psi.memory),
            ("io pressure", m.psi.io),
        ] {
            host += &line(
                name,
                format!(
                    "{:.1}% (10 s {:.1}%, 5 min {:.1}%)",
                    p.avg60, p.avg10, p.avg300
                ),
                p.avg60,
                p.avg60 >= 10.0,
            );
        }
    }
    for d in &m.disks {
        host += &line(
            &esc(&d.mount),
            format!("{:.0}% of {:.0} GB", d.used_pct, d.size_gb),
            d.used_pct,
            d.used_pct >= 85.0,
        );
    }
    let t = now();
    let link: String = hub
        .link()
        .iter()
        .map(|l| {
            let rate = l.lost as f64 * 100.0 / (l.received + l.lost).max(1) as f64;
            format!(
                r#"<tr class="{}"><td><code>{}</code></td><td>{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{rate:.2}%</td><td class="n">{}</td><td class="n">{}</td></tr>"#,
                if rate >= 1.0 { "warnrow" } else { "" },
                esc(&l.apid),
                esc(&l.stream),
                l.received,
                l.lost,
                l.restarts,
                if l.last_seen > 0 { format!("{} ago", dur(t - l.last_seen)) } else { "—".into() },
            )
        })
        .collect();
    let link = if link.is_empty() {
        r#"<tr><td colspan="7" class="dim">No packets since warte started.</td></tr>"#.to_string()
    } else {
        link
    };
    let errors = if s.errors.is_empty() {
        r#"<p class="empty">The agent collected everything in its last snapshot.</p>"#.to_string()
    } else {
        s.errors
            .iter()
            .map(|e| format!(r#"<div class="err">{}</div>"#, esc(e)))
            .collect()
    };
    format!(
        r#"<div class="split"><section class="panel"><h3>CI host · {}</h3><div class="hbars">{host}</div><div class="note">smithy agent {} · pressure is the share of time tasks stalled (PSI avg60).</div></section><section class="panel"><h3>Telemetry link · CCSDS</h3><div class="scroll"><table class="data-table"><thead><tr><th>APID</th><th>Stream</th><th class="n">Received</th><th class="n">Lost</th><th class="n">Loss</th><th class="n">Restarts</th><th class="n">Last</th></tr></thead><tbody>{link}</tbody></table></div><div class="note">Counted since warte last started. A lost packet is a gap in the stream's sequence count.</div><h3 style="margin-top:1rem">Collection errors</h3>{errors}</section></div>{}"#,
        esc(&s.host),
        esc(&s.agent_version),
        freshness(at)
    )
}

/// Problems the ground station can see right now, worst first.
pub fn incidents(hub: Option<&Hub>) -> String {
    let Some((hub, (s, at))) = hub.and_then(|h| h.snapshot().map(|v| (h, v))) else {
        return format!(r#"<section class="panel">{}</section>"#, unavailable(hub));
    };
    let t = now();
    let mut list: Vec<(u8, String)> = Vec::new(); // 2 = red, 1 = amber
    if t - at > STALE_AFTER_S {
        list.push((
            2,
            format!(
                "No telemetry for {}: the agent or the link is down.",
                dur(t - at)
            ),
        ));
    }
    let offline: Vec<String> = s
        .runners
        .iter()
        .filter(|r| r.gh_status.as_deref() == Some("offline"))
        .map(|r| format!("runner{}", r.id))
        .collect();
    if !offline.is_empty() {
        list.push((2, format!("Offline runners: {}.", offline.join(", "))));
    }
    for r in &s.runners {
        if let Some(st) = r.stats.as_ref().filter(|st| st.oom_kills > 0) {
            list.push((
                1,
                format!(
                    "runner{}: {} job(s) OOM-killed since the service started.",
                    r.id, st.oom_kills
                ),
            ));
        }
    }
    for d in s.machine.disks.iter().filter(|d| d.used_pct >= 85.0) {
        list.push((
            if d.used_pct >= 95.0 { 2 } else { 1 },
            format!("Disk {} is {:.0}% full.", esc(&d.mount), d.used_pct),
        ));
    }
    let psi = &s.machine.psi;
    for (name, v) in [
        ("CPU", psi.cpu.avg60),
        ("memory", psi.memory.avg60),
        ("IO", psi.io.avg60),
    ] {
        if v >= 10.0 {
            list.push((1, format!("{name} pressure {v:.1}%: tasks are stalling.")));
        }
    }
    if let Some(q) = &s.queue {
        if !q.queued_complete {
            list.push((
                1,
                "The last queue scan was incomplete: Ask and the boards may miss runs.".into(),
            ));
        }
        let overdue = q
            .queued
            .iter()
            .filter(|e| q.waits.get(&e.pool).is_some_and(|w| t - e.since > w.p90))
            .count();
        if overdue > 0 {
            list.push((
                1,
                format!(
                    "{overdue} queued run(s) waited longer than 90% of recent jobs in their pool."
                ),
            ));
        }
    }
    for l in hub.link() {
        let rate = l.lost as f64 * 100.0 / (l.received + l.lost).max(1) as f64;
        if rate >= 1.0 {
            list.push((
                1,
                format!(
                    "Telemetry stream {} ({}) loses {rate:.1}% of packets.",
                    esc(&l.apid),
                    esc(&l.stream)
                ),
            ));
        }
    }
    for e in &s.errors {
        list.push((1, format!("Agent could not collect: {}", esc(e))));
    }
    list.sort_by_key(|i| std::cmp::Reverse(i.0));
    let body = if list.is_empty() {
        r#"<div class="allclear"><span class="pill">all clear</span> Nothing the ground station can see is wrong.</div>"#.to_string()
    } else {
        list.iter()
            .map(|(sev, msg)| {
                format!(
                    r#"<div class="incident sev{sev}"><span class="badge {}">{}</span>{msg}</div>"#,
                    if *sev == 2 { "red" } else { "yellow" },
                    if *sev == 2 { "down" } else { "degraded" }
                )
            })
            .collect()
    };
    format!(
        r#"<section class="panel"><h3>Detected now · CI fleet and ground station</h3>{body}<div class="note">From the latest snapshot and the telemetry link. GitHub's own status is not monitored yet.</div></section>{}"#,
        freshness(at)
    )
}

/// Percent-encode a query value (RFC 3986 unreserved characters pass).
fn qv(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// What is in flight right now, one row per PR or branch, for Ask: click a
/// row instead of typing. Also the repos for the input's suggestions.
pub fn ask_picker(hub: Option<&Hub>) -> (String, String) {
    let Some((s, at)) = hub.and_then(Hub::snapshot) else {
        return (String::new(), String::new());
    };
    #[derive(Default)]
    struct Row {
        queued_jobs: u32,
        oldest: i64,
        running: usize,
        workflows: std::collections::BTreeSet<String>,
    }
    let t = now();
    let bare = |r: &str| r.rsplit('/').next().unwrap_or(r).to_string();
    let target = |pr: Option<u32>, branch: &str| match pr {
        Some(n) => (Some(n), String::new()),
        None => (None, branch.to_string()),
    };
    let mut rows: std::collections::BTreeMap<(String, Option<u32>, String), Row> =
        Default::default();
    if let Some(q) = &s.queue {
        for e in &q.queued {
            let (pr, br) = target(e.pr, &e.branch);
            let r = rows.entry((bare(&e.repo), pr, br)).or_default();
            r.queued_jobs += e.jobs;
            r.oldest = r.oldest.max(t - e.since);
            r.workflows.insert(e.workflow.clone());
        }
        for j in q.hosted.iter().filter(|j| j.status == "in_progress") {
            let (pr, br) = target(j.pr, &j.branch);
            let r = rows.entry((bare(&j.repo), pr, br)).or_default();
            r.running += 1;
            r.workflows.insert(j.workflow.clone());
        }
    }
    for c in s
        .runners
        .iter()
        .filter(|r| r.local_busy)
        .filter_map(|r| r.ctx.as_ref())
    {
        let (pr, br) = target(c.pr, &c.branch);
        let r = rows.entry((bare(&c.repo), pr, br)).or_default();
        r.running += 1;
        r.workflows.insert(c.workflow.clone());
    }
    let repos: std::collections::BTreeSet<&String> = rows.keys().map(|k| &k.0).collect();
    let datalist = repos
        .iter()
        .map(|r| format!(r#"<option value="{}">"#, esc(r)))
        .collect::<String>();
    let mut list: Vec<_> = rows.into_iter().collect();
    list.sort_by_key(|(_, r)| std::cmp::Reverse((r.oldest, r.running)));
    if list.is_empty() {
        return (
            format!(
                r#"<section class="panel"><h3>In flight</h3><p class="empty">Nothing queued or running.</p>{}</section>"#,
                freshness(at)
            ),
            datalist,
        );
    }
    let body: String = list
        .iter()
        .map(|((repo, pr, branch), r)| {
            let (what, q) = match pr {
                Some(n) => (format!("#{n}"), format!("pr={n}")),
                None => (esc(branch), format!("branch={}", qv(branch))),
            };
            let state = match (r.running, r.queued_jobs) {
                (0, _) => r#"<span class="badge yellow">queued</span>"#.to_string(),
                (n, 0) => format!(r#"<span class="badge green">{n} running</span>"#),
                (n, _) => format!(r#"<span class="badge green">{n} running</span> <span class="badge yellow">queued</span>"#),
            };
            let wfs: Vec<&str> = r.workflows.iter().map(String::as_str).take(3).collect();
            format!(
                r##"<tr class="pick" tabindex="0" title="ask when it runs" hx-get="api/dashboard/ask-partial?repo={}&amp;{q}" hx-target="#content" hx-swap="innerHTML" hx-trigger="click, keyup[key=='Enter']"><td class="flap">{}</td><td>{what}</td><td class="dim">{}</td><td>{state}</td><td class="n">{}</td><td class="n">{}</td></tr>"##,
                qv(repo),
                esc(repo),
                esc(&wfs.join(", ")),
                r.queued_jobs,
                if r.oldest > 0 { dur(r.oldest) } else { "—".into() },
            )
        })
        .collect();
    (
        format!(
            r#"<section class="panel"><h3>In flight · click a row to ask</h3><div class="scroll"><table class="board pickable"><thead><tr><th>Repo</th><th>PR / branch</th><th>Workflows</th><th>State</th><th class="n">Queued jobs</th><th class="n">Waiting</th></tr></thead><tbody>{body}</tbody></table></div>{}</section>"#,
            freshness(at)
        ),
        datalist,
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

/// Labels every self-hosted runner carries: not worth a chip.
const GENERIC_LABELS: [&str; 4] = ["self-hosted", "linux", "x64", "hetzner"];
/// The traffic chart's window, and the KPI sparklines'.
const TRAFFIC_S: i64 = 6 * 3600;
const SPARK_S: i64 = 2 * 3600;
/// Departures shown before the board says "and N more".
const BOARD_ROWS: usize = 12;

/// The host's own view (a `Runner.Worker` process) decides; GitHub's flag
/// lags in both directions and is shown beside it where they disagree.
fn busy(r: &crate::hub::model::Runner) -> bool {
    r.local_busy
}

/// A polyline over `pts` (t, value), scaled into a `w`×`h` box: time on x
/// spanning exactly [t0, t1], value on y from 0 to `max`.
fn line(pts: &[(i64, f64)], t0: i64, t1: i64, max: f64, w: f64, h: f64) -> String {
    let span = (t1 - t0).max(1) as f64;
    let max = if max > 0.0 { max } else { 1.0 };
    pts.iter()
        .map(|(t, v)| {
            format!(
                "{:.1},{:.1}",
                (t - t0) as f64 / span * w,
                h - (v / max).clamp(0.0, 1.0) * (h - 2.0) - 1.0
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn spark(pts: &[(i64, f64)], t0: i64, t1: i64) -> String {
    if pts.len() < 2 {
        return String::new();
    }
    let max = pts.iter().map(|p| p.1).fold(0.0, f64::max);
    format!(
        r#"<svg class="spark" viewBox="0 0 120 24" preserveAspectRatio="none" aria-hidden="true"><polyline points="{}"/></svg>"#,
        line(pts, t0, t1, max, 120.0, 24.0)
    )
}

fn kpi(label: &str, value: &str, class: &str, sub: &str, trend: &str) -> String {
    format!(
        r#"<div class="kpi"><div class="l">{}</div><div class="v {class}">{}</div><div class="s">{}</div>{trend}</div>"#,
        esc(label),
        esc(value),
        esc(sub)
    )
}

fn meter(pct: f64, hot: bool) -> String {
    format!(
        r#"<span class="bar"><i class="{}" style="width:{:.0}%"></i></span>"#,
        if hot { "hot" } else { "" },
        pct.clamp(0.0, 100.0)
    )
}

/// Six hours of traffic: busy runners as a line against the fleet size, and
/// self-hosted jobs waiting as an area on its own scale.
fn traffic(hist: &[crate::hub::model::Point], total: usize, t1: i64) -> String {
    if hist.len() < 2 {
        return r#"<p class="empty">Traffic appears after a few minutes of telemetry.</p>"#.into();
    }
    let t0 = t1 - TRAFFIC_S;
    let (w, h) = (600.0, 120.0);
    let busy: Vec<(i64, f64)> = hist.iter().map(|p| (p.t, p.local_busy as f64)).collect();
    let queued: Vec<(i64, f64)> = hist
        .iter()
        .filter_map(|p| p.sh_queued.or(p.jobs_queued).map(|q| (p.t, q as f64)))
        .collect();
    let qmax = queued.iter().map(|p| p.1).fold(0.0, f64::max);
    let area = if queued.len() >= 2 {
        let first = (queued[0].0 - t0) as f64 / TRAFFIC_S as f64 * w;
        let last = (queued[queued.len() - 1].0 - t0) as f64 / TRAFFIC_S as f64 * w;
        format!(
            r#"<polygon class="q" points="{first:.1},{h} {} {last:.1},{h}"/>"#,
            line(&queued, t0, t1, qmax, w, h)
        )
    } else {
        String::new()
    };
    let ticks: String = (1..6)
        .map(|i| {
            let x = i as f64 * w / 6.0;
            format!(r#"<line class="tick" x1="{x:.0}" y1="0" x2="{x:.0}" y2="{h}"/>"#)
        })
        .collect();
    format!(
        r#"<svg class="traffic" viewBox="0 0 {w} {h}" preserveAspectRatio="none" role="img" aria-label="busy runners and queued jobs over the last 6 hours">{ticks}{area}<polyline class="b" points="{}"/></svg><div class="axis"><span>−6 h</span><span>−4 h</span><span>−2 h</span><span>now</span></div><div class="legend"><span class="lb">busy runners (of {total})</span><span class="lq">self-hosted jobs waiting (peak {qmax:.0})</span></div>"#,
        line(&busy, t0, t1, total as f64, w, h)
    )
}

/// The departures board: queued runs, longest waiting first, with the pool's
/// measured median wait as the boarding estimate.
fn departures(q: &crate::hub::model::Queue, t: i64, limit: usize) -> String {
    if q.queued.is_empty() {
        return r#"<p class="empty">Nothing waiting. Every runway is clear.</p>"#.into();
    }
    let mut runs: Vec<_> = q.queued.iter().collect();
    runs.sort_by_key(|e| e.since);
    let rows: String = runs
        .iter()
        .take(limit)
        .map(|e| {
            let repo = e.repo.rsplit('/').next().unwrap_or(&e.repo);
            let what = match e.pr {
                Some(n) => format!("#{n}"),
                None => esc(&e.branch),
            };
            let waited = t - e.since;
            let (eta, cls) = match q.waits.get(&e.pool) {
                Some(w) if waited > w.p90 => ("overdue".to_string(), "late"),
                Some(w) => (format!("~{}", dur((w.p50 - waited).max(0))), ""),
                None => ("—".to_string(), ""),
            };
            format!(
                r#"<tr class="{cls}"><td class="flap">{}</td><td class="pr" title="{what}">{what}<span class="wf">{}</span></td><td class="dim">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n eta">{eta}</td></tr>"#,
                esc(repo),
                esc(&e.workflow),
                esc(&e.pool),
                e.jobs,
                dur(waited),
            )
        })
        .collect();
    let more = match runs.len().saturating_sub(limit) {
        0 => String::new(),
        n => format!(r#"<div class="note">and {n} more runs waiting; see Queue.</div>"#),
    };
    format!(
        r#"<div class="scroll"><table class="board"><thead><tr><th>Repo</th><th>PR · workflow</th><th>Gate</th><th class="n">Jobs</th><th class="n">Waiting</th><th class="n">Boards</th></tr></thead><tbody>{rows}</tbody></table></div>{more}"#
    )
}

/// One runner as an air-traffic flight strip: callsign, state, what it is
/// flying (repo, PR, job) and how long, and its own cgroup's CPU and memory.
fn strip(r: &crate::hub::model::Runner, t: i64) -> String {
    let b = busy(r);
    let (cls, st) = match (r.gh_status.as_deref(), b) {
        (Some("offline"), _) => ("strip off", "OFF"),
        (_, true) => ("strip busy", "AIRBORNE"),
        _ => ("strip", "HOLDING"),
    };
    let disagree = match r.gh_busy {
        Some(g) if g != r.local_busy => format!(
            r#"<span class="dis" title="GitHub's busy flag lags in both directions">GH {}</span>"#,
            if g { "busy" } else { "idle" }
        ),
        _ => String::new(),
    };
    let (what, route, clock) = match (&r.ctx, &r.job, b) {
        (ctx, Some(job), true) => {
            let route = ctx
                .as_ref()
                .map(|c| {
                    let repo = c.repo.rsplit('/').next().unwrap_or(&c.repo);
                    let tail = match c.pr {
                        Some(n) => format!(" #{n}"),
                        None => format!(" @ {}", esc(&c.branch)),
                    };
                    format!("{}{tail} · {}", esc(repo), esc(&c.workflow))
                })
                .unwrap_or_default();
            let clock = r.job_started.map(|s| dur(t - s)).unwrap_or_default();
            (esc(job), route, clock)
        }
        _ => (
            format!("last: {}", esc(r.last_result.as_deref().unwrap_or("—"))),
            String::new(),
            String::new(),
        ),
    };
    let res = r
        .stats
        .as_ref()
        .map(|s| {
            let cpu = s
                .cpu_cores
                .map(|c| format!("{c:.1}c"))
                .unwrap_or_else(|| "—".into());
            let (pct, max) = match s.mem_max_mb {
                Some(m) if m > 0 => (s.mem_mb as f64 * 100.0 / m as f64, format!("/{:.0}G", m as f64 / 1024.0)),
                _ => (0.0, String::new()),
            };
            let hot = pct >= 85.0 || s.oom_kills > 0;
            let oom = if s.oom_kills > 0 {
                format!(r#" <b class="hot">{} OOM</b>"#, s.oom_kills)
            } else {
                String::new()
            };
            format!(
                r#"<div class="res"><span>cpu {cpu}</span><span>mem {:.1}G{max}{oom}</span>{}</div>"#,
                s.mem_mb as f64 / 1024.0,
                meter(pct, hot)
            )
        })
        .unwrap_or_default();
    let chips: String = r
        .labels
        .iter()
        .filter(|l| !GENERIC_LABELS.contains(&l.as_str()))
        .map(|l| format!(r#"<span class="chip">{}</span>"#, esc(l)))
        .collect();
    format!(
        r#"<div class="{cls}"><div class="cs"><span class="n">R{:02}</span><span class="st">{st}</span></div><div class="body"><div class="job">{what}</div><div class="route">{route}</div>{res}<div class="meta">{chips}{disagree}</div></div><div class="clock">{clock}</div></div>"#,
        r.id
    )
}

/// The tower's main view: a runway gauge (one cell per runner), KPIs with
/// two-hour trends, six hours of traffic, the departures board and the strips.
pub fn overview(hub: Option<&Hub>) -> String {
    let Some((hub, (s, at))) = hub.and_then(|h| h.snapshot().map(|v| (h, v))) else {
        return format!(r#"<section class="panel">{}</section>"#, unavailable(hub));
    };
    let t = now();
    let hist = hub.history_since(t - TRAFFIC_S);
    let recent: Vec<_> = hist.iter().filter(|p| p.t >= t - SPARK_S).collect();
    let series = |f: &dyn Fn(&crate::hub::model::Point) -> Option<f64>| -> String {
        let pts: Vec<(i64, f64)> = recent
            .iter()
            .filter_map(|p| f(p).map(|v| (p.t, v)))
            .collect();
        spark(&pts, t - SPARK_S, t)
    };

    let total = s.runners.len();
    let nbusy = s.runners.iter().filter(|r| busy(r)).count();
    let online = s
        .runners
        .iter()
        .filter(|r| r.gh_status.as_deref() == Some("online"))
        .count();
    let gh_busy = s.runners.iter().filter(|r| r.gh_busy == Some(true)).count();
    let age = t - at;
    let (pill, word) = if age <= STALE_AFTER_S {
        ("pill", "live")
    } else {
        ("pill stale", "stale")
    };
    let cells: String = s
        .runners
        .iter()
        .map(|r| {
            let c = match (r.gh_status.as_deref(), busy(r)) {
                (Some("offline"), _) => "off",
                (_, true) => "on",
                _ => "",
            };
            format!(r#"<i class="{c}" title="runner{}"></i>"#, r.id)
        })
        .collect();
    let m = &s.machine;
    let hero = format!(
        r#"<section class="hero"><div class="radar" aria-hidden="true"><span></span></div><div><div class="l">Runways in use</div><div class="big">{nbusy}<small>/{total}</small></div></div><div class="runway" aria-label="{nbusy} of {total} runners busy">{cells}</div><div class="r"><span class="{pill}">{word} · {} ago</span><span>{online} online · GitHub says {gh_busy} busy</span><span class="host">{}{}</span></div></section>"#,
        dur(age),
        esc(&s.host),
        if m.ncpu > 0 {
            format!(" · load {:.0}/{}", m.load[0], m.ncpu)
        } else {
            String::new()
        },
    );

    let mut kpis = kpi(
        "Runners online",
        &format!("{online}/{total}"),
        if online < total { "warn" } else { "" },
        if online == total {
            "all online"
        } else {
            "some offline"
        },
        &series(&|p| Some(p.online as f64)),
    );
    if let Some(q) = &s.queue {
        let sh: u32 = q
            .pools
            .iter()
            .filter(|(p, _)| p.as_str() != "hosted")
            .map(|(_, p)| p.jobs_queued)
            .sum();
        let oldest = q
            .pools
            .iter()
            .filter(|(p, _)| p.as_str() != "hosted")
            .map(|(_, p)| p.oldest_s)
            .max()
            .unwrap_or(0);
        kpis += &kpi(
            "Holding · self-hosted",
            &sh.to_string(),
            if sh >= 50 { "warn" } else { "" },
            &format!("jobs · oldest {} · {} runs", dur(oldest), q.runs_queued),
            &series(&|p| p.sh_queued.or(p.jobs_queued).map(f64::from)),
        );
        kpis += &kpi(
            "GitHub-hosted",
            &q.hosted_running.to_string(),
            "",
            &format!("running · {} queued", q.hosted_queued),
            &series(&|p| p.hosted_running.map(f64::from)),
        );
    }
    if m.ncpu > 0 {
        let hot = |v: f64| if v >= 10.0 { "warn" } else { "" };
        kpis += &kpi(
            "CPU pressure",
            &format!("{:.1}%", m.psi.cpu.avg60),
            hot(m.psi.cpu.avg60),
            &format!("load {:.0} on {} cores", m.load[0], m.ncpu),
            &series(&|p| Some(p.psi_cpu)),
        );
        kpis += &kpi(
            "Memory pressure",
            &format!("{:.1}%", m.psi.memory.avg60),
            hot(m.psi.memory.avg60),
            &format!("{:.0} GB free", m.mem_avail_mb as f64 / 1024.0),
            &series(&|p| Some(p.psi_mem)),
        );
    }
    if let Some(d) = m
        .disks
        .iter()
        .max_by(|a, b| a.used_pct.total_cmp(&b.used_pct))
    {
        kpis += &kpi(
            &format!("Disk {}", d.mount),
            &format!("{:.0}%", d.used_pct),
            if d.used_pct >= 85.0 { "err" } else { "" },
            &format!("{:.0} GB free", d.size_gb - d.used_gb),
            &series(&|p| p.disk_runners_pct),
        );
    }

    let errors: String = s
        .errors
        .iter()
        .map(|e| format!(r#"<div class="err">{}</div>"#, esc(e)))
        .collect();
    let board = match &s.queue {
        Some(q) => departures(q, t, BOARD_ROWS),
        None => r#"<p class="empty">No queue scan in the latest snapshot.</p>"#.into(),
    };
    let strips: String = s.runners.iter().map(|r| strip(r, t)).collect();

    format!(
        r#"{hero}<section class="kpis">{kpis}</section>{errors}<section class="panel"><h3>Traffic · last 6 h</h3>{}</section><div class="split"><section class="panel"><h3>Runways</h3><div class="strips">{strips}</div></section><section class="panel"><h3>Departures</h3>{board}<div class="note">"Boards" is the pool's measured median wait minus time already waited; past the pool's p90 a run is marked overdue. Ask for any PR in <b>Ask</b>; full detail and the telemetry link on the <a href="ci/">ground station ↗</a>.</div></section></div>"#,
        traffic(&hist, total, t)
    )
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

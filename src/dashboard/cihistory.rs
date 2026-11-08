//! CI history (REQ-SMITHY-006): what the ground station's `/ci` page showed
//! beyond warte's live views, so that page can be retired. Fleet trends over
//! 6 h to 7 d from the ground station's per-minute points, and the monthly
//! conveyor-belt report from the finished-job ledger.

use super::charts;
use super::fleet::{dur_pub as dur, esc};
use crate::history::Series;
use crate::hub::belt::BeltReport;
use crate::hub::model::Point;

/// The ranges offered: (key, seconds, label). The ground station keeps 7 days.
pub const RANGES: [(&str, i64, &str); 3] = [
    ("6h", 6 * 3600, "6 h"),
    ("24h", 86_400, "24 h"),
    ("7d", 7 * 86_400, "7 d"),
];

/// The range for a key, defaulting to 24 h.
pub fn range(key: Option<&str>) -> (&'static str, i64) {
    RANGES
        .iter()
        .find(|(k, _, _)| Some(*k) == key)
        .map(|(k, s, _)| (*k, *s))
        .unwrap_or(("24h", 86_400))
}

/// One value per minute from `from` to `to`; a minute without a point, or
/// whose point lacks the value, is a gap.
pub fn series(
    points: &[Point],
    from: i64,
    to: i64,
    metric: &str,
    unit: &'static str,
    f: impl Fn(&Point) -> Option<f64>,
) -> Series {
    let n = ((to - from) / 60).max(1) as usize;
    let mut values = vec![None; n];
    for p in points {
        if p.t < from || p.t >= to {
            continue;
        }
        let i = ((p.t - from) / 60) as usize;
        if let Some(slot) = values.get_mut(i) {
            *slot = f(p);
        }
    }
    Series {
        metric: metric.into(),
        unit,
        t0: from,
        step: 60,
        values,
        min: None,
        max: None,
    }
}

type Pick = fn(&Point) -> Option<f64>;

/// The trends drawn, in order: (title, unit, value of a point).
pub const TRENDS: [(&str, &str, Pick); 8] = [
    ("self-hosted runners busy", "", |p| {
        Some(p.local_busy as f64)
    }),
    ("self-hosted runners online", "", |p| Some(p.online as f64)),
    ("self-hosted jobs queued", "", |p| {
        p.sh_queued.or(p.jobs_queued).map(f64::from)
    }),
    ("longest self-hosted wait", "min", |p| {
        p.sh_oldest_s.map(|s| s as f64 / 60.0)
    }),
    ("GitHub-hosted jobs queued", "", |p| {
        p.hosted_queued.map(f64::from)
    }),
    ("GitHub-hosted jobs running", "", |p| {
        p.hosted_running.map(f64::from)
    }),
    ("PRs waiting", "", |p| p.prs_waiting.map(f64::from)),
    ("CI host load (1 min)", "", |p| Some(p.load1)),
];

/// How many pools get their own charts.
pub const POOL_CHARTS: usize = 8;

/// Queued and busy per pool, hosted pools split by label (REQ-RUNNERS-003):
/// the pools seen in the range, busiest first. A point recorded before
/// warte kept pools is a gap; a pool missing from a point that has pools was
/// idle (0).
pub fn pool_trends(
    points: &[Point],
    from: i64,
    now: i64,
    cfg: &crate::hub::pools::RunnersConfig,
    self_hosted: &[String],
) -> String {
    let mut total: std::collections::BTreeMap<&str, u64> = Default::default();
    for p in points.iter().filter(|p| p.t >= from) {
        for (k, c) in &p.pools {
            *total.entry(k.as_str()).or_default() += u64::from(c[0] + c[1]);
        }
    }
    if total.is_empty() {
        return String::new();
    }
    let mut pools: Vec<(&str, u64)> = total.into_iter().collect();
    pools.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let kind = |k: crate::hub::pools::Kind| match k {
        crate::hub::pools::Kind::StandardHosted => "GitHub-hosted",
        crate::hub::pools::Kind::LargerHosted => "GitHub-hosted, larger",
        crate::hub::pools::Kind::SelfHosted => "self-hosted",
    };
    let at = |pool: &str, i: usize| {
        let pool = pool.to_string();
        move |p: &Point| {
            if p.pools.is_empty() {
                None
            } else {
                Some(p.pools.get(&pool).map(|c| c[i]).unwrap_or(0) as f64)
            }
        }
    };
    let charts: String = pools
        .iter()
        .take(POOL_CHARTS)
        .map(|(pool, _)| {
            let info = crate::hub::pools::info(pool, cfg, self_hosted);
            let label =
                format!("{} ({}, {:?})", info.name, kind(info.kind), info.os).to_lowercase();
            let q = format!("{label} · queued");
            let b = format!(
                "{label} · {}",
                if crate::hub::pools::is_hosted(pool) {
                    "running"
                } else {
                    "busy"
                }
            );
            charts::chart(&q, &series(points, from, now, &q, "", at(pool, 0)))
                + &charts::chart(&b, &series(points, from, now, &b, "", at(pool, 1)))
        })
        .collect();
    let more = if pools.len() > POOL_CHARTS {
        format!(
            r#"<div class="note">{} more pools in this range are not drawn.</div>"#,
            pools.len() - POOL_CHARTS
        )
    } else {
        String::new()
    };
    format!(
        r#"<section class="panel"><h3>Per pool</h3><div class="hcharts">{charts}</div>{more}<div class="note">GitHub-hosted pools are split by runs-on label: standard images, larger runners and self-hosted pools each wait on their own runners.</div></section>"#
    )
}

/// The trend charts for one range, with the range switch.
pub fn trends(points: &[Point], key: &str, span: i64, now: i64, month: &str) -> String {
    let switch: String = RANGES
        .iter()
        .map(|(k, _, l)| {
            format!(
                r##"<button class="rbtn{}" hx-get="api/dashboard/ci-history-partial?range={k}&amp;month={}" hx-target="#content" hx-swap="innerHTML">{l}</button>"##,
                if *k == key { " on" } else { "" },
                esc(month),
            )
        })
        .collect();
    let from = now - span;
    let charts: String = if points.iter().any(|p| p.t >= from) {
        TRENDS
            .iter()
            .map(|(title, unit, f)| {
                charts::chart(title, &series(points, from, now, title, unit, f))
            })
            .collect()
    } else {
        r#"<p class="empty">No fleet points in this range yet.</p>"#.into()
    };
    format!(
        r#"<section class="panel"><h3>Fleet trends</h3><div class="hrange" role="group" aria-label="trend range">range {switch}</div><div class="hcharts">{charts}</div><div class="note">One point per ground-station snapshot (about a minute); a break in a line is a gap in telemetry. The ground station keeps 7 days; machine history (Machines) keeps 90.</div></section>"#
    )
}

/// This month and the two before it, newest first, as `YYYY-MM`.
pub fn months(now: i64) -> Vec<String> {
    use chrono::Datelike;
    let d = chrono::DateTime::from_timestamp(now, 0).unwrap_or_default();
    let (mut y, mut m) = (d.year(), d.month());
    let mut out = Vec::new();
    for _ in 0..3 {
        out.push(format!("{y:04}-{m:02}"));
        if m == 1 {
            y -= 1;
            m = 12;
        } else {
            m -= 1;
        }
    }
    out
}

/// The monthly conveyor-belt report as HTML: work against waiting per pool,
/// whether the runner pool is the bottleneck by hour, the busiest workflows
/// and the runners.
pub fn belt(b: &BeltReport, months: &[String], key: &str) -> String {
    let h = |x: f64| format!("{x:.0} h");
    let p = |x: Option<f64>| {
        x.map(|v| format!("{:.0}%", v * 100.0))
            .unwrap_or_else(|| "—".into())
    };
    let pick: String = months
        .iter()
        .map(|m| {
            format!(
                r##"<button class="rbtn{}" hx-get="api/dashboard/ci-history-partial?range={key}&amp;month={m}" hx-target="#content" hx-swap="innerHTML">{m}</button>"##,
                if *m == b.month { " on" } else { "" },
            )
        })
        .collect();
    let (wk, wt) = b
        .pools
        .iter()
        .fold((0.0, 0.0), |(a, c), x| (a + x.work_h, c + x.wait_h));
    let lead = format!(
        "<p><b>{} of work, {} of waiting: flow efficiency {}.</b> {} days with data, {} complete.</p>",
        h(wk),
        h(wt),
        p((wk + wt > 0.0).then(|| wk / (wk + wt))),
        b.days.len(),
        b.complete_days,
    );
    let prelim = if b.preliminary {
        r#"<p class="warn">Preliminary: no day is complete yet (a day closes three days after it ends, once every run of it was read). These figures miss long jobs that have not finished, so they overstate waiting; the bottleneck verdict is withheld.</p>"#
    } else {
        ""
    };
    let kind = |k: Option<crate::hub::pools::Kind>| match k {
        Some(crate::hub::pools::Kind::StandardHosted) => "GitHub-hosted",
        Some(crate::hub::pools::Kind::LargerHosted) => "GitHub-hosted, larger",
        Some(crate::hub::pools::Kind::SelfHosted) => "self-hosted",
        None => "",
    };
    let priced = b.pools.iter().any(|x| x.cost.is_some());
    let cur = b.currency.as_deref().unwrap_or("");
    let pools: String = b
        .pools
        .iter()
        .map(|x| {
            format!(
                r#"<tr><td>{}<div class="sub">{}</div></td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td>{}</tr>"#,
                esc(x.name.as_deref().unwrap_or(&x.pool)),
                esc(kind(x.kind)),
                x.jobs_ran,
                x.jobs_not_run,
                h(x.work_h),
                h(x.wait_h),
                p(x.flow_efficiency),
                dur(x.wait_p50_s),
                dur(x.wait_p90_s),
                x.minutes,
                if priced {
                    format!(
                        r#"<td class="n">{}</td>"#,
                        x.cost
                            .map(|c| format!("{c:.2} {}", esc(cur)))
                            .unwrap_or_default()
                    )
                } else {
                    String::new()
                },
            )
        })
        .collect();
    let cost_head = if priced {
        r#"<th class="n">Cost*</th>"#
    } else {
        ""
    };
    let cost_note = if priced {
        "<br>* Notional: runner minutes (each job rounded up to the minute) at the per-minute rate in warte's config; list price, not a bill."
    } else {
        ""
    };
    let hours = if b.hours.is_empty() {
        r#"<p class="empty">Withheld: needs at least one complete day.</p>"#.to_string()
    } else {
        let rows: String = b
            .hours
            .iter()
            .map(|x| {
                format!(
                    r#"<tr class="{}"><td>{:02}:00</td><td class="n">{}</td><td class="n">{:.1}</td><td class="n">{}</td><td>{}</td></tr>"#,
                    if x.verdict == "capacity" { "warnrow" } else { "" },
                    x.hour_utc,
                    p(x.utilisation),
                    x.waiting_jobs,
                    p(x.eligible_utilisation),
                    esc(&x.verdict),
                )
            })
            .collect();
        format!(
            r#"<div class="scroll"><table class="data-table"><thead><tr><th>Hour (UTC)</th><th class="n">All runners busy</th><th class="n">Jobs waiting</th><th class="n">Eligible busy</th><th>Verdict</th></tr></thead><tbody>{rows}</tbody></table></div><div class="note">An hour where jobs wait is <b>scheduling</b> when the runners that could take them were mostly free, <b>capacity</b> when they were busy.</div>"#
        )
    };
    let workflows: String = b
        .workflows
        .iter()
        .take(15)
        .map(|x| {
            format!(
                r#"<tr><td>{}</td><td>{}</td><td>{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{}</td></tr>"#,
                esc(&x.repo),
                esc(&x.workflow),
                esc(&x.pool),
                x.jobs,
                h(x.work_h),
                h(x.wait_h),
                dur(x.wait_p50_s),
            )
        })
        .collect();
    let runners: String = b
        .runners
        .iter()
        .map(|x| {
            format!(
                r#"<tr><td>{}</td><td class="n">{}</td><td class="n">{}</td><td class="n">{:.0}%</td></tr>"#,
                esc(&x.runner),
                x.jobs,
                h(x.work_h),
                x.utilisation * 100.0,
            )
        })
        .collect();
    if b.pools.is_empty() {
        return format!(
            r#"<section class="panel"><h3>Conveyor belt · {}</h3><div class="hrange" role="group" aria-label="month">month {pick}</div><p class="empty">No finished jobs recorded for this month yet.</p></section>"#,
            esc(&b.month)
        );
    }
    format!(
        r#"<section class="panel"><h3>Conveyor belt · {}</h3><div class="hrange" role="group" aria-label="month">month {pick}</div>{lead}{prelim}
<h4>Pools</h4><div class="scroll"><table class="data-table"><thead><tr><th>Pool</th><th class="n">Ran</th><th class="n">Never ran</th><th class="n">Work</th><th class="n">Wait</th><th class="n">Flow eff.</th><th class="n">Wait p50</th><th class="n">Wait p90</th><th class="n">Minutes</th>{cost_head}</tr></thead><tbody>{pools}</tbody></table></div>
<h4>Is the runner pool the bottleneck?</h4>{hours}
<h4>Busiest workflows</h4><div class="scroll"><table class="data-table"><thead><tr><th>Repo</th><th>Workflow</th><th>Pool</th><th class="n">Jobs</th><th class="n">Work</th><th class="n">Wait</th><th class="n">Wait p50</th></tr></thead><tbody>{workflows}</tbody></table></div>
<h4>Runners</h4><div class="scroll"><table class="data-table"><thead><tr><th>Runner</th><th class="n">Jobs</th><th class="n">Work</th><th class="n">Busy</th></tr></thead><tbody>{runners}</tbody></table></div>
<div class="note">{}.{cost_note}<br>{}<br>As Markdown: <a href="api/report.md?month={}" target="_blank" rel="noopener">report.md</a> · JSON: <a href="api/report?month={}" target="_blank" rel="noopener">api/report</a>.</div></section>"#,
        esc(&b.month),
        esc(b.definitions),
        esc(b.note),
        esc(&b.month),
        esc(&b.month),
    )
}

/// The CI host's systemd timers: name, active, next run. The automation's
/// log lines stay off this unauthenticated view (raw output, warte#46).
pub fn timers(a: &crate::hub::model::Automation) -> String {
    let rows: String = a
        .timers
        .iter()
        .map(|t| {
            format!(
                r#"<tr class="{}"><td><code>{}</code></td><td>{}</td><td>{}</td></tr>"#,
                if t.active { "" } else { "warnrow" },
                esc(&t.unit),
                if t.active { "active" } else { "inactive" },
                esc(t.next.as_deref().unwrap_or("—")),
            )
        })
        .collect();
    let rows = if rows.is_empty() {
        r#"<tr><td colspan="3" class="dim">The agent reported no timers.</td></tr>"#.to_string()
    } else {
        rows
    };
    format!(
        r#"<section class="panel"><h3>Timers on the CI host</h3><div class="scroll"><table class="data-table"><thead><tr><th>Timer</th><th>State</th><th>Next run</th></tr></thead><tbody>{rows}</tbody></table></div></section>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(t: i64, busy: u32) -> Point {
        Point {
            t,
            local_busy: busy,
            ..Default::default()
        }
    }

    #[test]
    fn a_missing_minute_is_a_gap_not_a_zero() {
        let s = series(&[pt(0, 3), pt(120, 5)], 0, 180, "busy", "", |p| {
            Some(p.local_busy as f64)
        });
        assert_eq!(s.values, vec![Some(3.0), None, Some(5.0)]);
        assert_eq!((s.t0, s.step), (0, 60));
    }

    #[test]
    fn months_go_back_across_the_year() {
        // 2026-01-15
        assert_eq!(months(1_768_435_200), ["2026-01", "2025-12", "2025-11"]);
    }

    #[test]
    fn unknown_range_is_a_day() {
        assert_eq!(range(Some("90d")), ("24h", 86_400));
        assert_eq!(range(Some("7d")), ("7d", 7 * 86_400));
    }
}

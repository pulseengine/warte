//! Coming up: forecasts before a limit is reached (REQ-PREDICT-001/002,
//! DD-PREDICT-001).
//!
//! For every reporting machine, warte fits a straight line (ordinary least
//! squares) through the hourly means of each disk's used share and of memory
//! use over the last seven days. A forecast is listed only when the line
//! rises steadily, rests on enough data, and reaches the limit within the
//! horizon at its current rate. Everything else is either steady or "not
//! enough data": warte never guesses past its evidence.

use crate::dashboard::fleet::{esc, Detected};
use crate::history::{History, Series};
use serde::Serialize;

/// How far back a fit looks: just over seven days, so the store answers with
/// hourly means.
pub const WINDOW_S: i64 = 7 * 86_400 + 3_600;
/// The least span of data a forecast rests on.
pub const MIN_SPAN_S: i64 = 24 * 3_600;
/// The fewest hourly points a forecast rests on.
pub const MIN_POINTS: usize = 12;
/// The least goodness of fit: a weekly cycle or a jump fits worse and is
/// held back rather than guessed at.
pub const MIN_R2: f64 = 0.7;
/// Forecasts further out than this are not listed.
pub const HORIZON_DAYS: f64 = 14.0;
/// Within this many days a forecast is an amber finding (alerts notify).
pub const AMBER_DAYS: f64 = 3.0;
/// Within this many days, a red one.
pub const RED_DAYS: f64 = 1.0;
/// The limit for every watched share.
pub const LIMIT_PCT: f64 = 95.0;

/// A least-squares line through `(t, value)` points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// Change per second.
    pub slope: f64,
    /// Goodness of fit, 0..1; 0 for a flat series.
    pub r2: f64,
    pub n: usize,
    /// Seconds from the first point to the last.
    pub span_s: i64,
}

/// The line through `points` (sorted by time), or none for fewer than two.
pub fn fit(points: &[(i64, f64)]) -> Option<Fit> {
    let n = points.len();
    if n < 2 {
        return None;
    }
    let t0 = points[0].0;
    let nf = n as f64;
    let mt = points.iter().map(|(t, _)| (t - t0) as f64).sum::<f64>() / nf;
    let my = points.iter().map(|(_, y)| y).sum::<f64>() / nf;
    let (mut stt, mut sty, mut syy) = (0.0, 0.0, 0.0);
    for (t, y) in points {
        let (dt, dy) = ((t - t0) as f64 - mt, y - my);
        stt += dt * dt;
        sty += dt * dy;
        syy += dy * dy;
    }
    if stt == 0.0 {
        return None;
    }
    let slope = sty / stt;
    let r2 = if syy == 0.0 {
        0.0
    } else {
        ((sty * sty) / (stt * syy)).clamp(0.0, 1.0)
    };
    Some(Fit {
        slope,
        r2,
        n,
        span_s: points[n - 1].0 - t0,
    })
}

/// One forecast with its evidence.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Item {
    pub host: String,
    pub metric: String,
    /// What it is, in words: `disk /var`, `memory`.
    pub what: String,
    /// The latest hourly mean.
    pub now: f64,
    pub limit: f64,
    /// Change per day, from the fitted line.
    pub per_day: f64,
    /// When the limit is reached at that rate (unix seconds).
    pub due_at: i64,
    pub days: f64,
    /// Hours and points of data the line rests on.
    pub span_h: i64,
    pub points: usize,
    pub r2: f64,
}

/// What one series says.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Due(Item),
    /// Enough data, nothing reaches the limit within the horizon.
    Steady,
    NotEnough,
}

/// The answer of `/api/forecast`.
#[derive(Serialize, Debug, Clone, Default, PartialEq)]
pub struct Forecast {
    pub generated_at: i64,
    pub horizon_days: f64,
    pub limit_pct: f64,
    /// Series looked at.
    pub watched: usize,
    /// Of those, series with too little data to forecast.
    pub not_enough_data: usize,
    /// Soonest first.
    pub items: Vec<Item>,
}

/// The watched series, in words; none for every other series.
pub fn what(metric: &str) -> Option<String> {
    if metric == "mem.used_pct" {
        return Some("memory".into());
    }
    let mount = metric
        .strip_prefix("disk.used_pct{mount=")?
        .strip_suffix('}')?;
    Some(format!("disk {mount}"))
}

/// The forecast for one series of points.
pub fn judge(host: &str, metric: &str, points: &[(i64, f64)], now: i64) -> Outcome {
    let Some(what) = what(metric) else {
        return Outcome::Steady;
    };
    let Some(f) = fit(points) else {
        return Outcome::NotEnough;
    };
    if f.n < MIN_POINTS || f.span_s < MIN_SPAN_S {
        return Outcome::NotEnough;
    }
    let latest = points[points.len() - 1].1;
    // already over: that is today's finding, not a forecast
    if f.slope <= 0.0 || f.r2 < MIN_R2 || latest >= LIMIT_PCT {
        return Outcome::Steady;
    }
    let per_day = f.slope * 86_400.0;
    let days = (LIMIT_PCT - latest) / per_day;
    if !days.is_finite() || days > HORIZON_DAYS {
        return Outcome::Steady;
    }
    Outcome::Due(Item {
        host: host.into(),
        metric: metric.into(),
        what,
        now: latest,
        limit: LIMIT_PCT,
        per_day,
        due_at: now + (days * 86_400.0) as i64,
        days,
        span_h: f.span_s / 3_600,
        points: f.n,
        r2: f.r2,
    })
}

/// The points of a series the store returned, gaps left out.
pub fn points(s: &Series) -> Vec<(i64, f64)> {
    s.values
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.map(|v| (s.t0 + i as i64 * s.step, v)))
        .collect()
}

/// Forecasts for `hosts` from the history store.
pub fn forecast(h: &History, hosts: &[String], now: i64) -> Forecast {
    let mut out = Forecast {
        generated_at: now,
        horizon_days: HORIZON_DAYS,
        limit_pct: LIMIT_PCT,
        ..Default::default()
    };
    for host in hosts {
        let metrics: Vec<String> = h
            .metrics(host)
            .into_iter()
            .filter(|m| what(m).is_some())
            .collect();
        if metrics.is_empty() {
            continue;
        }
        for s in h.series(host, &metrics, now - WINDOW_S, now) {
            out.watched += 1;
            match judge(host, &s.metric, &points(&s), now) {
                Outcome::Due(i) => out.items.push(i),
                Outcome::NotEnough => out.not_enough_data += 1,
                Outcome::Steady => {}
            }
        }
    }
    out.items.sort_by(|a, b| a.days.total_cmp(&b.days));
    out
}

/// The forecasts of every machine that reports, if the ground station keeps
/// history.
pub fn of_machines(m: &crate::probe::Machines, now: i64) -> Option<Forecast> {
    let h = m.history()?;
    let mut hosts: Vec<String> = m.list().into_iter().map(|s| s.report.host).collect();
    hosts.sort();
    hosts.dedup();
    Some(forecast(h, &hosts, now))
}

/// "about 2 days", "about 5 hours".
pub fn in_words(days: f64) -> String {
    if days < 1.0 {
        let h = (days * 24.0).round().max(1.0) as i64;
        format!("about {h} hour{}", if h == 1 { "" } else { "s" })
    } else {
        let d = days.round() as i64;
        format!("about {d} day{}", if d == 1 { "" } else { "s" })
    }
}

/// Forecasts due soon, as outage findings, so alerts notify (REQ-PREDICT-002).
pub fn detected(f: &Forecast) -> Vec<Detected> {
    f.items
        .iter()
        .filter(|i| i.days <= AMBER_DAYS)
        .map(|i| Detected {
            // stable while the forecast lasts: no numbers in the key
            key: format!("forecast:{}:{}", i.host, i.metric),
            sev: if i.days <= RED_DAYS { 2 } else { 1 },
            text: format!(
                "{}: {} reaches {:.0}% in {} ({:.0}% now, +{:.1}%/day)",
                i.host,
                i.what,
                i.limit,
                in_words(i.days),
                i.now,
                i.per_day
            ),
        })
        .collect()
}

/// The overview's Coming up panel.
pub fn panel(f: Option<&Forecast>) -> String {
    let Some(f) = f else {
        return String::new();
    };
    let note = if f.not_enough_data > 0 {
        format!(
            "; {} with not enough data yet (a forecast needs {} hours)",
            f.not_enough_data,
            MIN_SPAN_S / 3_600
        )
    } else {
        String::new()
    };
    let head = format!(
        r#"<section class="panel coming-up"><h3>Coming up</h3><p class="note">Disks and memory forecast to reach {:.0}% within {:.0} days, from a straight line through the last week's hourly means. {} series watched{}.</p>"#,
        f.limit_pct, f.horizon_days, f.watched, note
    );
    if f.items.is_empty() {
        return head + r#"<p>Nothing is forecast to reach its limit.</p></section>"#;
    }
    let mut rows = String::new();
    for i in &f.items {
        let when = chrono::DateTime::from_timestamp(i.due_at, 0)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let cls = if i.days <= RED_DAYS {
            "badrow"
        } else if i.days <= AMBER_DAYS {
            "warnrow"
        } else {
            ""
        };
        rows += &format!(
            r#"<tr class="{cls}"><td>{when}<br><span class="dim">in {}</span></td><td class="mono">{}</td><td>{}</td><td>{:.0}% now, +{:.1}%/day</td><td class="dim">{} h, {} points, R² {:.2}</td></tr>"#,
            in_words(i.days),
            esc(&i.host),
            esc(&i.what),
            i.now,
            i.per_day,
            i.span_h,
            i.points,
            i.r2
        );
    }
    head + &format!(
        r#"<table><thead><tr><th>Reaches {:.0}%</th><th>Machine</th><th>What</th><th>Trend</th><th>Evidence</th></tr></thead><tbody>{rows}</tbody></table></section>"#,
        f.limit_pct
    )
}

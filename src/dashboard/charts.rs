//! Server-side SVG trend lines for machine history (REQ-MHIST-003).
//!
//! No charting library: a path whose `M` restarts after every gap, so an
//! outage shows as a break and never as a line drawn across it. Long series
//! are bucketed to at most `BUCKETS` columns (average of the bucket, a gap if
//! the whole bucket is empty); each column carries a `<title>` for hover.

use crate::history::Series;

/// Most columns drawn per chart.
pub const BUCKETS: usize = 240;

/// The time ranges the Machines view offers: (key, seconds, label).
pub const RANGES: [(&str, i64, &str); 4] = [
    ("1h", 3600, "1 h"),
    ("24h", 86_400, "24 h"),
    ("7d", 7 * 86_400, "7 d"),
    ("90d", 90 * 86_400, "90 d"),
];

/// The range for a key, defaulting to 24 h.
pub fn range(key: Option<&str>) -> (&'static str, i64) {
    RANGES
        .iter()
        .find(|(k, _, _)| Some(*k) == key)
        .map(|(k, s, _)| (*k, *s))
        .unwrap_or(("24h", 86_400))
}

/// A readable title for a series name: `disk.used_pct{mount=/var}` →
/// `disk /var used`.
pub fn title(metric: &str) -> String {
    let (base, label) = match metric.split_once('{') {
        Some((b, l)) => (
            b,
            l.trim_end_matches('}')
                .split_once('=')
                .map(|(_, v)| v)
                .unwrap_or(""),
        ),
        None => (metric, ""),
    };
    let t = match base {
        "load.1" => "load, 1 min",
        "load.5" => "load, 5 min",
        "load.15" => "load, 15 min",
        "load.per_core" => "load per core",
        "psi.cpu" => "CPU pressure",
        "psi.mem" => "memory pressure",
        "psi.io.some" => "IO pressure (some)",
        "psi.io.full" => "IO pressure (full)",
        "mem.used_pct" => "memory used",
        "swap.used_mb" => "swap used",
        "disk.used_pct" => "disk used",
        "io.busy_pct" => "busy",
        "io.await_ms" => "wait",
        "io.read_kbs" => "read",
        "io.write_kbs" => "write",
        "runners.online" => "runners online",
        "runners.busy" => "runners busy",
        "runners.offline" => "runners offline",
        "runner.cpu" => "runner CPU",
        "runner.mem_mb" => "runner memory",
        "queue.self_hosted" => "queued, self-hosted",
        "queue.hosted" => "queued, GitHub-hosted",
        "queue.oldest_s" => "longest wait",
        "sessions" => "agent sessions",
        "uptime" => "uptime",
        other => other,
    };
    if label.is_empty() {
        t.to_string()
    } else {
        format!("{t} · {label}")
    }
}

/// Average values into at most `n` buckets; `(start index, value)` per bucket.
pub fn bucket(values: &[Option<f64>], n: usize) -> Vec<(usize, Option<f64>)> {
    if values.is_empty() || n == 0 {
        return Vec::new();
    }
    let per = values.len().div_ceil(n);
    values
        .chunks(per)
        .enumerate()
        .map(|(i, c)| {
            let got: Vec<f64> = c.iter().flatten().copied().collect();
            let v = (!got.is_empty()).then(|| got.iter().sum::<f64>() / got.len() as f64);
            (i * per, v)
        })
        .collect()
}

fn top(values: &[(usize, Option<f64>)], unit: &str) -> f64 {
    let m = values
        .iter()
        .filter_map(|(_, v)| *v)
        .fold(0.0_f64, f64::max);
    if unit == "%" {
        100.0
    } else if m <= 0.0 {
        1.0
    } else {
        m * 1.1
    }
}

/// The SVG path of bucketed values on a `w`×`h` canvas; a new `M` after each gap.
pub fn path(values: &[(usize, Option<f64>)], max: f64, w: f64, h: f64) -> String {
    let n = values.len().max(2) - 1;
    let mut d = String::new();
    let mut pen_down = false;
    for (i, (_, v)) in values.iter().enumerate() {
        match v {
            Some(v) => {
                let x = w * i as f64 / n as f64;
                let y = h - h * (v / max).clamp(0.0, 1.0);
                d += &format!("{}{x:.1},{y:.1} ", if pen_down { "L" } else { "M" });
                pen_down = true;
            }
            None => pen_down = false,
        }
    }
    d.trim_end().to_string()
}

/// The last value of a series, if any.
pub fn last(s: &Series) -> Option<f64> {
    s.values.iter().rev().flatten().next().copied()
}

pub fn fmt(v: f64, unit: &str) -> String {
    let n = if v.abs() >= 100.0 {
        format!("{v:.0}")
    } else if v.abs() >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    };
    if unit.is_empty() {
        n
    } else if unit == "%" {
        format!("{n}%")
    } else {
        format!("{n} {unit}")
    }
}

/// A small trend line for a card: label, line, last value.
pub fn spark(label: &str, s: &Series) -> String {
    let b = bucket(&s.values, 60);
    let max = top(&b, s.unit);
    let now = last(s)
        .map(|v| fmt(v, s.unit))
        .unwrap_or_else(|| "–".into());
    format!(
        r#"<div class="spark"><span class="name">{}</span><svg viewBox="0 0 120 24" preserveAspectRatio="none" role="img" aria-label="{} trend"><path d="{}"/></svg><span class="n">{now}</span></div>"#,
        super::fleet::esc(label),
        super::fleet::esc(label),
        path(&b, max, 120.0, 24.0),
    )
}

fn when(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .map(|d| d.format("%d %b %H:%M").to_string())
        .unwrap_or_default()
}

/// A larger chart: line, min/max band for hourly data, hover per column,
/// start and end times and the scale.
pub fn chart(title: &str, s: &Series) -> String {
    let (w, h) = (480.0, 60.0);
    let b = bucket(&s.values, BUCKETS);
    let max = top(&b, s.unit);
    let n = b.len().max(2) - 1;
    let col = w / b.len().max(1) as f64;
    let band = match (&s.min, &s.max) {
        (Some(lo), Some(hi)) => {
            let lo = bucket(lo, BUCKETS);
            let hi = bucket(hi, BUCKETS);
            let mut d = String::new();
            for (i, ((_, l), (_, u))) in lo.iter().zip(&hi).enumerate() {
                if let (Some(l), Some(u)) = (l, u) {
                    let x = w * i as f64 / n as f64;
                    let y = |v: f64| h - h * (v / max).clamp(0.0, 1.0);
                    d += &format!("M{x:.1},{:.1} L{x:.1},{:.1} ", y(*l), y(*u));
                }
            }
            format!(r#"<path class="band" d="{}"/>"#, d.trim_end())
        }
        _ => String::new(),
    };
    let hover: String = b
        .iter()
        .enumerate()
        .map(|(i, (k, v))| {
            let t = s.t0 + s.step * *k as i64;
            let val = v.map(|v| fmt(v, s.unit)).unwrap_or_else(|| "no data".into());
            format!(
                r#"<rect x="{:.1}" y="0" width="{col:.2}" height="{h}"><title>{} UTC · {val}</title></rect>"#,
                col * i as f64,
                when(t)
            )
        })
        .collect();
    let end = s.t0 + s.step * (s.values.len() as i64 - 1).max(0);
    let now = last(s)
        .map(|v| fmt(v, s.unit))
        .unwrap_or_else(|| "no data".into());
    format!(
        r#"<figure class="hchart"><figcaption><b>{}</b> <span class="n">{now}</span></figcaption><svg viewBox="0 0 {w} {h}" preserveAspectRatio="none" role="img" aria-label="{} over time">{band}<path class="line" d="{}"/><g class="hover">{hover}</g></svg><div class="axis"><span>{}</span><span>max {}</span><span>{}</span></div></figure>"#,
        super::fleet::esc(title),
        super::fleet::esc(title),
        path(&b, max, w, h),
        when(s.t0),
        fmt(max, s.unit),
        when(end),
    )
}

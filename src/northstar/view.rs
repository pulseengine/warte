//! The North Star view (REQ-NS-005): the published North Star, one card per
//! objective, as pulseengine.eu's /north-star page shows it, plus the trend
//! warte observed. Every number on the page is a published field; warte
//! compares, orders and draws, and never derives an indicator.

use super::{measured_unix, num, Board, Commitment, Observation, Row};
use crate::dashboard::fleet::esc;

/// The varve layer (`2026.11`) a unix time falls in.
fn layer_of(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .map(|d| d.format("%Y.%m").to_string())
        .unwrap_or_default()
}

/// "N of M", "N", or nothing, from a published value and denominator.
fn n_of_m(o: &Observation) -> Option<String> {
    match (o.value, o.of) {
        (Some(v), Some(of)) => Some(format!("{} of {}", num(v), num(of))),
        (Some(v), None) => Some(num(v)),
        _ => None,
    }
}

/// The layers of the period `now` falls in, from the published commitments;
/// only `now`'s own layer when no period names it.
fn current_layers(board: &Board, now: i64) -> (Option<String>, Vec<String>) {
    let here = layer_of(now);
    board
        .commitments
        .iter()
        .find(|c| c.layers.contains(&here))
        .map(|c| (Some(c.period.clone()), c.layers.clone()))
        .unwrap_or((None, vec![here]))
}

/// The current period's commitment for `id` with a published target, and
/// whether the latest value is below it. Without a target there is no
/// off-target state.
fn off_target(board: &Board, id: &str, latest: &Observation, period: Option<&str>) -> bool {
    let Some(period) = period else { return false };
    board
        .commitments
        .iter()
        .filter(|c| c.objective == id && c.period == period)
        .filter_map(|c| c.target_value)
        .any(|t| latest.value.is_some_and(|v| v < t))
}

/// The state shown for a row: the poll's state, or `off-target`.
pub fn shown_state(board: &Board, r: &Row, now: i64) -> String {
    let (period, _) = current_layers(board, now);
    match &r.latest {
        Some(l) if r.state == "measured" && off_target(board, &r.id, l, period.as_deref()) => {
            "off-target".into()
        }
        _ => r.state.clone(),
    }
}

fn commitment_line(c: &Commitment, current: bool) -> String {
    let target = match (c.target_value, c.target_of) {
        (Some(v), Some(of)) => format!(
            " <span class=\"ns-target\">target {} of {}</span>",
            num(v),
            num(of)
        ),
        (Some(v), None) => format!(" <span class=\"ns-target\">target {}</span>", num(v)),
        _ => String::new(),
    };
    format!(
        r#"<li><span class="ns-period-id">{}{}</span> <span class="ns-kind ns-kind--{}">{}</span> {}{}</li>"#,
        esc(&c.period),
        if current { " (now)" } else { "" },
        esc(&c.kind),
        esc(&c.kind),
        esc(&c.target),
        target
    )
}

fn card(board: &Board, r: &Row, history: &[Observation], now: i64, stale_days: i64) -> String {
    let o = board.objectives.iter().find(|o| o.id == r.id);
    let (period, layers) = current_layers(board, now);
    let state = shown_state(board, r, now);
    let mut h = format!(
        r#"<article class="ns-card ns-card--{state}" id="ns-{id}"><div class="ns-top"><span class="ns-id">{id}</span><h3 class="ns-title">{title}</h3><span class="ns-state ns-state--{state}">{state}</span></div>"#,
        state = esc(&state),
        id = esc(&r.id),
        title = esc(&r.title)
    );
    if let Some(o) = o.filter(|o| !o.summary.is_empty()) {
        h += &format!(r#"<p class="ns-summary">{}</p>"#, esc(&o.summary));
    }
    match &r.latest {
        None => h += r#"<p class="ns-value ns-unmeasured">unmeasured</p>"#,
        Some(l) => {
            if l.of == Some(0.0) {
                h += r#"<p class="ns-value">no repo in the denominator</p>"#;
            } else if let Some(v) = n_of_m(l) {
                let bar = match (l.value, l.of) {
                    (Some(v), Some(of)) if of > 0.0 => format!(
                        r#"<div class="ns-bar" role="img" aria-label="{} of {}"><span style="width:{:.1}%"></span></div>"#,
                        num(v),
                        num(of),
                        (v / of * 100.0).clamp(0.0, 100.0)
                    ),
                    _ => String::new(),
                };
                h += &format!(
                    r#"<p class="ns-value"><strong>{}</strong></p>{bar}"#,
                    esc(&v)
                );
            }
            let age = r.age_days.unwrap_or(0);
            let stale = age > stale_days;
            h += &format!(
                r#"<p class="ns-meta">measured {}{} · <span class="{}">{} d ago</span></p>"#,
                esc(&l.measured_at),
                l.verifier
                    .as_deref()
                    .map(|v| format!(" with {}", esc(v)))
                    .unwrap_or_default(),
                if stale {
                    "ns-age ns-age--stale"
                } else {
                    "ns-age"
                },
                age
            );
            h += &match r.previous.as_ref().and_then(|p| n_of_m(p).map(|v| (p, v))) {
                Some((p, v)) => format!(
                    r#"<p class="ns-meta">was {} on {}</p>"#,
                    esc(&v),
                    esc(&p.measured_at)
                ),
                None => r#"<p class="ns-meta">first observation</p>"#.into(),
            };
            if let Some(p) = &l.period {
                h += &format!(r#"<p class="ns-meta">period {}</p>"#, esc(p));
            }
            if !l.evidence.is_empty() {
                h += r#"<ul class="ns-rows">"#;
                for e in &l.evidence {
                    let label = if e.url.is_empty() {
                        esc(&e.label)
                    } else {
                        format!(
                            r#"<a href="{}" rel="noopener">{}</a>"#,
                            esc(&e.url),
                            esc(&e.label)
                        )
                    };
                    let release = if e.release.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", esc(&e.release))
                    };
                    let counts = match e.counts {
                        Some(true) => r#" <span class="ns-counts ns-counts--yes">counts</span>"#,
                        Some(false) => {
                            r#" <span class="ns-counts ns-counts--no">does not count</span>"#
                        }
                        None => "",
                    };
                    let verdict = if e.verdict.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", esc(&e.verdict))
                    };
                    let controls = if e.controls.is_empty() {
                        String::new()
                    } else {
                        let c: Vec<String> = e
                            .controls
                            .iter()
                            .map(|(k, v)| format!("{} {}", esc(k), esc(v)))
                            .collect();
                        format!(
                            r#" <span class="ns-controls">negative controls: {}</span>"#,
                            c.join(", ")
                        )
                    };
                    h += &format!("<li>{label}{release}{verdict}{counts}{controls}</li>");
                }
                h += "</ul>";
            }
            if !l.not_counted.is_empty() {
                let n: Vec<String> = l
                    .not_counted
                    .iter()
                    .map(|n| format!("{} ({})", esc(&n.what), esc(&n.reason)))
                    .collect();
                h += &format!(r#"<p class="ns-blind">Not counted: {}.</p>"#, n.join("; "));
            }
        }
    }
    // the trend, one value per varve layer of the current period, never
    // interpolated: a layer without an observation shows a dash
    if r.latest.is_some() {
        let mut mine: Vec<&Observation> = history.iter().filter(|x| x.objective == r.id).collect();
        mine.sort_by_key(|x| measured_unix(&x.measured_at).unwrap_or(0));
        let cells: Vec<String> = layers
            .iter()
            .map(|layer| {
                let v = mine
                    .iter()
                    .rev()
                    .find(|x| measured_unix(&x.measured_at).map(layer_of).as_deref() == Some(layer))
                    .and_then(|x| n_of_m(x));
                format!(
                    r#"<li><span class="ns-layer">{}</span> {}</li>"#,
                    esc(layer),
                    v.map(|v| esc(&v)).unwrap_or_else(|| "—".into())
                )
            })
            .collect();
        h += &format!(
            r#"<div class="ns-trend"><span class="ns-trend-label">{} · observed by warte, not the official per-layer series</span><ul>{}</ul></div>"#,
            period
                .as_deref()
                .map(esc)
                .unwrap_or_else(|| "this month".into()),
            cells.join("")
        );
    }
    let commitments: Vec<String> = board
        .commitments
        .iter()
        .filter(|c| c.objective == r.id)
        .map(|c| commitment_line(c, period.as_deref() == Some(c.period.as_str())))
        .collect();
    if !commitments.is_empty() {
        h += &format!(
            r#"<ul class="ns-commitments">{}</ul>"#,
            commitments.join("")
        );
    }
    let pullers: Vec<String> = board
        .pullers
        .iter()
        .filter(|p| p.pulls.contains(&r.id))
        .map(|p| {
            format!(
                r#"<a class="ns-repo" href="https://github.com/pulseengine/{n}" rel="noopener">{n}</a>{}"#,
                if p.product { "" } else { " <span class=\"dim\">(tool)</span>" },
                n = esc(&p.name)
            )
        })
        .collect();
    let blind = r
        .latest
        .as_ref()
        .and_then(|l| l.blind_spot.clone())
        .map(|b| esc(&b))
        .unwrap_or_else(|| {
            if r.latest.is_some() {
                "not published".into()
            } else {
                "—".into()
            }
        });
    h += &format!(
        r#"<dl class="ns-dl"><dt>Indicator</dt><dd>{}</dd><dt>Guard</dt><dd>{}</dd><dt>Not checked</dt><dd>{}</dd><dt>Pulled by</dt><dd>{}</dd></dl></article>"#,
        o.map(|o| esc(&o.indicator))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "—".into()),
        if r.guard.is_empty() {
            "—".into()
        } else {
            esc(&r.guard)
        },
        blind,
        if pullers.is_empty() {
            "no product or tool repository names it".into()
        } else {
            pullers.join(", ")
        }
    );
    h
}

/// The whole view. `board` is the last North Star file read; `rows` come
/// from [`super::rows`]; `history` is every kept observation.
pub fn page(
    board: Option<&Board>,
    rows: &[Row],
    history: &[Observation],
    last_poll: Option<&(i64, String)>,
    now: i64,
    stale_days: i64,
) -> String {
    let poll = match last_poll {
        Some((t, r)) => format!(
            "read {} ago: {}",
            crate::dashboard::fleet::dur_pub((now - t).max(0)),
            esc(r)
        ),
        None => "not read yet".into(),
    };
    let Some(board) = board else {
        return format!(
            r#"<div class="overview north-star"><h2>North Star</h2><p class="empty">The North Star has not been read yet ({poll}).</p></div>"#
        );
    };
    let cards: Vec<String> = rows
        .iter()
        .map(|r| card(board, r, history, now, stale_days))
        .collect();
    format!(
        r#"<div class="overview north-star"><h2>North Star</h2><p class="ns-star">{}</p><p class="ns-meta">Updated {} · <a href="https://pulseengine.eu/north-star/" rel="noopener">published North Star ↗</a> · {poll}</p><div class="ns-grid">{}</div></div>"#,
        esc(&board.north_star),
        esc(&board.updated),
        cards.join("")
    )
}

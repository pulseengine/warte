//! "Needs a look" (REQ-DASH-021, warte#235): the items behind each top-bar
//! badge, listed at the top of the view that holds them. The badges and the
//! blocks are computed from this one list, so a badge's number is the number
//! of lines in its block by construction.

use crate::server::AppState;
use crate::temper::report::Severity;

/// One line in a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub red: bool,
    pub text: String,
}

/// The badged views, by view slug, with their items, red first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Look {
    pub outages: Vec<Item>,
    pub machines: Vec<Item>,
    pub compliance: Vec<Item>,
}

impl Look {
    /// Each badged view with the group its row sits in.
    pub fn views(&self) -> [(&'static str, &'static str, &[Item]); 3] {
        [
            ("outages", "now", &self.outages),
            ("machines", "machines", &self.machines),
            ("compliance", "compliance", &self.compliance),
        ]
    }
}

fn sorted(mut v: Vec<Item>) -> Vec<Item> {
    // stable: within a colour the source's own order (worst or soonest first)
    v.sort_by_key(|i| !i.red);
    v
}

/// Outages: what the ground station detects, with the configured disk
/// levels, and the machines' red findings. The outages view renders the
/// same two lists.
pub fn outages(state: &AppState, now: i64) -> Vec<Item> {
    let fleet = state
        .hub
        .as_ref()
        .and_then(|h| super::fleet::detected_with(h, &state.config.disk_levels()))
        .map(|(l, _)| l)
        .unwrap_or_default();
    let hosts = super::fleet::host_detected(state, now);
    sorted(
        fleet
            .iter()
            .chain(&hosts)
            .map(|d| Item {
                red: d.sev >= 2,
                text: d.text.clone(),
            })
            .collect(),
    )
}

/// Machines: series forecast to reach their limit within the horizon.
pub fn machines(f: Option<&crate::forecast::Forecast>) -> Vec<Item> {
    f.map(|f| {
        f.items
            .iter()
            .map(|i| Item {
                red: false,
                text: format!(
                    "{}: {} reaches {:.0}% in {}",
                    i.host,
                    i.what,
                    i.limit,
                    crate::forecast::in_words(i.days)
                ),
            })
            .collect()
    })
    .unwrap_or_default()
}

/// Compliance: every failing decided check, and the report itself when it
/// is late or missing.
pub fn compliance(
    configured: bool,
    latest: Option<&crate::temper::Stored>,
    stale_after_s: i64,
    now: i64,
) -> Vec<Item> {
    if !configured {
        return vec![];
    }
    let Some(s) = latest else {
        return vec![Item {
            red: false,
            text: "no temper report received yet".into(),
        }];
    };
    let mut v: Vec<(u8, Item)> = s
        .report
        .failures()
        .iter()
        .map(|(repo, c)| {
            let rank = match c.severity {
                Severity::Critical => 0,
                Severity::High => 1,
                Severity::Medium => 2,
                Severity::Low => 3,
                Severity::Info => 4,
            };
            (
                rank,
                Item {
                    red: true,
                    text: format!("{repo}: {} ({})", c.id.as_str(), c.code.as_str()),
                },
            )
        })
        .collect();
    v.sort_by_key(|(r, _)| *r);
    let mut out: Vec<Item> = v.into_iter().map(|(_, i)| i).collect();
    if now - s.received_at > stale_after_s {
        out.push(Item {
            red: false,
            text: format!(
                "the temper report is {} h old",
                (now - s.received_at) / 3600
            ),
        });
    }
    out
}

/// All three, from the state.
pub fn of(state: &AppState, now: i64) -> Look {
    let stale = state
        .config
        .temper
        .as_ref()
        .map(|t| t.stale_after_s)
        .unwrap_or(26 * 3600);
    let forecast = state
        .machines
        .as_ref()
        .and_then(|m| crate::forecast::of_machines(m, now));
    Look {
        outages: outages(state, now),
        machines: machines(forecast.as_ref()),
        compliance: compliance(
            state.temper.is_some(),
            state.temper.as_ref().and_then(|t| t.latest()).as_ref(),
            stale,
            now,
        ),
    }
}

/// The block at the top of a view; nothing when there is nothing to look at.
pub fn block(items: &[Item]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let worst = if items.iter().any(|i| i.red) {
        "red"
    } else {
        "amber"
    };
    let lines: String = items
        .iter()
        .map(|i| {
            format!(
                r#"<li class="{}">{}</li>"#,
                if i.red { "red" } else { "amber" },
                super::fleet::esc(&i.text)
            )
        })
        .collect();
    format!(
        r#"<section class="panel needs-a-look {worst}" id="needs-a-look"><h3>Needs a look <span class="nav-badge {worst}">{}</span></h3><ul>{lines}</ul></section>"#,
        items.len()
    )
}

/// Insert the block right after a view's `<h2>…</h2>` heading, or at the
/// top when the view has none.
pub fn prepend(page: String, items: &[Item]) -> String {
    let b = block(items);
    if b.is_empty() {
        return page;
    }
    match page.find("</h2>") {
        Some(i) => {
            let at = i + "</h2>".len();
            format!("{}{b}{}", &page[..at], &page[at..])
        }
        None => format!("{b}{page}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(red: bool, t: &str) -> Item {
        Item {
            red,
            text: t.into(),
        }
    }

    // rivet: verifies REQ-DASH-021
    #[test]
    fn the_block_lists_every_item_red_first_and_is_absent_when_empty() {
        assert_eq!(block(&[]), "");
        let items = sorted(vec![it(false, "a <b>"), it(true, "c"), it(false, "d")]);
        assert_eq!(items[0], it(true, "c"));
        let b = block(&items);
        assert_eq!(b.matches("<li ").count(), 3);
        assert!(b.contains(r#"<span class="nav-badge red">3</span>"#), "{b}");
        assert!(b.contains("a &lt;b&gt;") && !b.contains("a <b>"));
        let p = prepend("<div><h2>X</h2><p>rest</p></div>".into(), &items);
        assert!(p.starts_with("<div><h2>X</h2><section"), "{p}");
        assert!(p.ends_with("<p>rest</p></div>"));
        assert_eq!(prepend("<p>x</p>".into(), &[]), "<p>x</p>");
    }

    // rivet: verifies REQ-DASH-021
    #[test]
    fn forecasts_are_amber_lines_with_host_what_and_when() {
        let f = crate::forecast::Forecast {
            items: vec![crate::forecast::Item {
                host: "example-host".into(),
                metric: "disk./var.used_pct".into(),
                what: "disk /var".into(),
                now: 80.0,
                limit: 90.0,
                per_day: 2.0,
                due_at: 0,
                days: 5.0,
                span_h: 48,
                points: 48,
                r2: 0.9,
            }],
            ..Default::default()
        };
        let v = machines(Some(&f));
        assert_eq!(v.len(), 1);
        assert!(!v[0].red);
        assert!(
            v[0].text
                .starts_with("example-host: disk /var reaches 90% in about 5"),
            "{}",
            v[0].text
        );
        assert!(machines(None).is_empty());
    }
}

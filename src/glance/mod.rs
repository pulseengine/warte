//! The night glance (REQ-GLANCE-001, warte#179): one line that says whether
//! anything needs the owner now. `needs-you` lists one item per urgent thing:
//! an open decision tagged with an always-open kind, a red main, a failed
//! release not rerun green, a critical disk. Otherwise `quiet`, with counts
//! only: routine decisions waiting, and agents silent or never heard from.
//! A routine decision is never an item; silence is never an item in v1.
//! Read-only: it answers nothing and triggers nothing.

use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Silence starts after this long without a heartbeat.
pub const SILENT_S: i64 = 30 * 60;
/// A decision title is cut to this many characters.
pub const TITLE_MAX: usize = 80;
/// The always-open kinds when `[glance] always_open` is not set.
pub const ALWAYS_OPEN: [&str; 2] = ["security", "red-main"];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Item {
    /// `decision`, `red-main`, `release` or `disk`.
    pub kind: String,
    /// The repository or host.
    pub subject: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Glance {
    /// `needs-you` or `quiet`.
    pub verdict: String,
    pub items: Vec<Item>,
    /// Open decisions that are not urgent: counted, never listed.
    pub routine_decisions: usize,
    /// Agents whose last heartbeat is older than 30 minutes.
    pub silent: usize,
    /// Agents that never sent a heartbeat.
    pub unknown: usize,
}

/// What the glance reads, gathered by the server.
#[derive(Debug, Default, Clone)]
pub struct Inputs {
    /// The `notice.v1` contents, every revision.
    pub notices: Vec<Value>,
    /// (repository, workflow) red on the default branch now.
    pub red_mains: Vec<(String, String)>,
    /// (repository, tag) whose latest release run failed.
    pub failed_releases: Vec<(String, String)>,
    /// Hosts with a disk whose pressure verdict is critical.
    pub critical_disks: Vec<String>,
    /// (host, rule) of each open high or critical security finding
    /// (REQ-FINDINGS-001).
    pub security_findings: Vec<(String, String)>,
    /// Hosts whose security detector stopped reporting.
    pub silent_detectors: Vec<String>,
    /// Each agent's last heartbeat, unix seconds; None if it never beat.
    pub heartbeats: Vec<Option<i64>>,
    pub always_open: Vec<String>,
    pub now: i64,
}

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// The newest revision of each open `needs-decision` notice.
fn open_decisions(notices: &[Value]) -> Vec<&Value> {
    let mut latest: BTreeMap<&str, &Value> = BTreeMap::new();
    for n in notices {
        let Some(id) = n["id"].as_str() else { continue };
        let rev = n["revision"].as_u64().unwrap_or(0);
        let newer = latest
            .get(id)
            .is_none_or(|old| rev > old["revision"].as_u64().unwrap_or(0));
        if newer {
            latest.insert(id, n);
        }
    }
    latest
        .into_values()
        .filter(|n| n["kind"] == "needs-decision")
        .filter(|n| {
            n["status"]
                .as_str()
                .is_none_or(|s| s == "open" || s == "queued")
        })
        .collect()
}

pub fn glance(i: &Inputs) -> Glance {
    let mut items = Vec::new();
    let mut routine = 0;
    let open = |s: &str| i.always_open.iter().any(|k| k == s);
    for n in open_decisions(&i.notices) {
        let tags: Vec<&str> = n["tags"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let urgent = n["kind"].as_str().is_some_and(open) || tags.iter().any(|t| open(t));
        if urgent {
            let kind = tags.iter().find(|t| open(t)).copied().unwrap_or("decision");
            items.push(Item {
                kind: "decision".into(),
                subject: n["repo"].as_str().unwrap_or("").to_string(),
                text: format!(
                    "{kind}: {}",
                    cut(n["title"].as_str().unwrap_or(""), TITLE_MAX)
                ),
            });
        } else {
            routine += 1;
        }
    }
    for (repo, wf) in &i.red_mains {
        items.push(Item {
            kind: "red-main".into(),
            subject: repo.clone(),
            text: format!(
                "{repo} is red on its default branch: {}",
                cut(wf, TITLE_MAX)
            ),
        });
    }
    for (repo, tag) in &i.failed_releases {
        items.push(Item {
            kind: "release".into(),
            subject: repo.clone(),
            text: format!("the release run for {repo} {} failed", cut(tag, 40)),
        });
    }
    for host in &i.critical_disks {
        items.push(Item {
            kind: "disk".into(),
            subject: host.clone(),
            text: format!("a disk on {host} is critically full"),
        });
    }
    for (host, rule) in &i.security_findings {
        items.push(Item {
            kind: "security".into(),
            subject: host.clone(),
            text: format!("a security finding on {host}: {}", cut(rule, TITLE_MAX)),
        });
    }
    for host in &i.silent_detectors {
        items.push(Item {
            kind: "security".into(),
            subject: host.clone(),
            text: format!("the security detector on {host} stopped reporting"),
        });
    }
    let silent = i
        .heartbeats
        .iter()
        .filter(|h| h.is_some_and(|t| i.now - t > SILENT_S))
        .count();
    let unknown = i.heartbeats.iter().filter(|h| h.is_none()).count();
    Glance {
        verdict: if items.is_empty() {
            "quiet"
        } else {
            "needs-you"
        }
        .into(),
        items,
        routine_decisions: routine,
        silent,
        unknown,
    }
}

/// One tag's release runs: (run, attempt) to (last created, any job failed).
type RunsOf = BTreeMap<(u64, u32), (i64, bool)>;

/// (repository, tag) whose latest release run failed: workflows named
/// release, run on a pushed tag, judged by the newest run per tag (a rerun
/// that went green clears it).
pub fn failed_releases(recs: &[crate::hub::model::JobRecord]) -> Vec<(String, String)> {
    // (repo, tag) -> run -> (created, failed?)
    let mut runs: BTreeMap<(&str, &str), RunsOf> = BTreeMap::new();
    for r in recs {
        let Some(tag) = r.br.as_deref() else { continue };
        if !r.wf.to_ascii_lowercase().contains("release") || r.ev != "push" {
            continue;
        }
        let e = runs
            .entry((r.repo.as_str(), tag))
            .or_default()
            .entry((r.run, r.att))
            .or_insert((r.c, false));
        e.0 = e.0.max(r.c);
        e.1 |= matches!(r.r.as_str(), "failure" | "timed_out" | "startup_failure");
    }
    runs.into_iter()
        .filter_map(|((repo, tag), by_run)| {
            let latest = by_run.values().max_by_key(|(c, _)| *c)?;
            latest.1.then(|| (repo.to_string(), tag.to_string()))
        })
        .collect()
}

/// The glance as one phone-sized page.
pub fn page(g: &Glance) -> String {
    let esc = crate::dashboard::fleet::esc;
    let body = if g.items.is_empty() {
        let mut line = format!(
            "All quiet. {} decision(s) wait for the next window.",
            g.routine_decisions
        );
        if g.silent + g.unknown > 0 {
            line.push_str(&format!(
                " {} agent(s) silent for more than 30 minutes, {} never heard from.",
                g.silent, g.unknown
            ));
        }
        format!(r#"<p class="quiet">{}</p>"#, esc(&line))
    } else {
        format!(
            r#"<p class="needs">Needs you now:</p><ul>{}</ul><p class="dim">{} routine decision(s) wait for the next window.</p>"#,
            g.items
                .iter()
                .map(|i| format!("<li><b>{}</b> {}</li>", esc(&i.kind), esc(&i.text)))
                .collect::<String>(),
            g.routine_decisions
        )
    };
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>warte glance</title><style>body{{font:18px/1.5 system-ui,sans-serif;margin:1rem;background:#fff;color:#111}}@media (prefers-color-scheme:dark){{body{{background:#111;color:#eee}}}}.needs{{color:#c0392b;font-weight:600}}.dim{{opacity:.7}}li{{margin:.4rem 0}}</style></head><body>{body}</body></html>"#
    )
}

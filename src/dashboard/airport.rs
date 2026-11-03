//! The airport map's data (REQ-DASH-015): one pier per machine, planes for
//! agent sessions, stands for bench boards, runways for busy runners, a
//! holding pattern for the queue. The page (templates/airport.js) draws it
//! and puts every string in as text, never as markup.
//!
//! Machines come from the fleet model when there is one (REQ-FLEET-001), so
//! a modelled machine that does not report still gets its pier, dashed;
//! otherwise from what reports.

use serde::Serialize;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Pier {
    pub name: String,
    /// terminal, workshop, cargo_hall, server or github_hosted.
    pub kind: String,
    /// lan or remote.
    pub location: String,
    pub cores: u32,
    pub runners: u32,
    /// Catalogue names of the boards on this machine.
    pub boards: Vec<String>,
    /// Agent sessions the machine's probe reports.
    pub agents: u32,
    /// The sessions, for the walkers (REQ-PROBE-009): waiting first, then
    /// those with a verb. Repo is the folder name only; no pid, no path.
    pub sessions: Vec<Walker>,
    pub ground_station: bool,
    /// Scheduled jobs on this machine that are fine / overdue or missing
    /// (REQ-JOBS-001).
    pub jobs_ok: u32,
    pub jobs_bad: u32,
    /// Reporting now (a probe, or the CI host's agent).
    pub reporting: bool,
    /// What is wrong with it, for its label (findings, "not reporting").
    pub note: String,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Walker {
    pub agent: String,
    pub repo: String,
    /// A verb from the agent's hook, or empty.
    pub activity: String,
    pub minutes: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Runway {
    pub id: String,
    /// `repo #pr` or `repo @ branch`.
    pub flight: String,
    pub job: String,
    pub elapsed_s: i64,
}

#[derive(Serialize, Debug, Clone, PartialEq, Default)]
pub struct Airport {
    pub tower: String,
    pub piers: Vec<Pier>,
    pub runways: Vec<Runway>,
    /// Runners in the fleet, busy or not.
    pub runner_total: u32,
    /// Jobs waiting for a runner.
    pub holding: u32,
}

fn kind_of(report: &crate::probe::Report) -> &'static str {
    let os = report.os.name.to_ascii_lowercase();
    if os.contains("macos") || os.contains("windows") {
        "terminal"
    } else if report.cpu.ncpu <= 4 {
        "workshop"
    } else {
        "server"
    }
}

fn walkers_of(r: &crate::probe::Report, now: i64) -> Vec<Walker> {
    let mut w: Vec<Walker> = r
        .sessions
        .iter()
        .map(|s| Walker {
            agent: s.agent.clone(),
            repo: s.dir.clone().unwrap_or_default(),
            activity: s.activity.clone().unwrap_or_default(),
            minutes: (now as u64).saturating_sub(s.started_at) / 60,
        })
        .collect();
    let rank = |w: &Walker| match w.activity.as_str() {
        crate::probe::activity::WAITING => 0,
        "" => 2,
        _ => 1,
    };
    w.sort_by_key(|w| (rank(w), w.minutes));
    w
}

pub fn build(
    fleet: Option<&crate::fleet::Fleet>,
    machines: &[crate::probe::Seen],
    hub: Option<(crate::hub::model::Snapshot, i64)>,
    jobs: &[crate::jobs::Job],
    now: i64,
) -> Airport {
    let ci_host = hub
        .as_ref()
        .map(|(s, _)| s.host.clone())
        .unwrap_or_default();
    let seen = |h: &str| {
        machines
            .iter()
            .find(|m| m.report.host.eq_ignore_ascii_case(h))
    };
    let reporting =
        |h: &str| seen(h).is_some() || (!ci_host.is_empty() && ci_host.eq_ignore_ascii_case(h));
    let agents = |h: &str| seen(h).map(|m| m.report.sessions.len() as u32).unwrap_or(0);
    let walkers = |h: &str| {
        seen(h)
            .map(|m| walkers_of(&m.report, now))
            .unwrap_or_default()
    };
    let findings = crate::probe::findings(machines, now);
    let note = |h: &str, up: bool| {
        if !up {
            return "not reporting".to_string();
        }
        findings
            .iter()
            .filter(|f| f.host.eq_ignore_ascii_case(h))
            .map(|f| f.text.clone())
            .next()
            .unwrap_or_default()
    };
    let mut piers: Vec<Pier> = match fleet {
        Some(f) => f
            .machines
            .iter()
            .map(|m| {
                let up = reporting(&m.host);
                Pier {
                    name: m.host.clone(),
                    kind: m.kind.clone(),
                    location: m.location.clone(),
                    cores: m.cores.unwrap_or(0),
                    runners: m.runners.unwrap_or(0),
                    boards: m.boards.iter().map(|b| b.catalog.clone()).collect(),
                    agents: agents(&m.host),
                    sessions: walkers(&m.host),
                    ground_station: m.ground_station,
                    jobs_ok: 0,
                    jobs_bad: 0,
                    reporting: up,
                    note: note(&m.host, up),
                }
            })
            .collect(),
        None => machines
            .iter()
            .map(|m| Pier {
                name: m.report.host.clone(),
                kind: kind_of(&m.report).into(),
                location: "lan".into(),
                cores: m.report.cpu.ncpu as u32,
                runners: 0,
                boards: vec![],
                agents: m.report.sessions.len() as u32,
                sessions: walkers_of(&m.report, now),
                ground_station: m.via == "local",
                jobs_ok: 0,
                jobs_bad: 0,
                reporting: true,
                note: note(&m.report.host, true),
            })
            .collect(),
    };
    let (mut runways, mut runner_total, mut holding) = (vec![], 0, 0);
    if let Some((s, _)) = &hub {
        runner_total = s.runners.len() as u32;
        holding = s.queue.as_ref().map(|q| q.jobs_queued).unwrap_or(0);
        for r in s.runners.iter().filter(|r| r.local_busy) {
            let flight = match &r.ctx {
                Some(c) => {
                    let repo = c.repo.rsplit('/').next().unwrap_or(&c.repo);
                    match c.pr {
                        Some(n) => format!("{repo} #{n}"),
                        None => format!("{repo} @ {}", c.branch),
                    }
                }
                None => String::new(),
            };
            runways.push(Runway {
                id: format!("R{:02}", r.id),
                flight,
                job: r.job.clone().unwrap_or_default(),
                elapsed_s: r.job_started.map(|t| (now - t).max(0)).unwrap_or(0),
            });
        }
        // the CI host gets its pier even without a model or a probe
        if fleet.is_none()
            && !s.host.is_empty()
            && !piers.iter().any(|p| p.name.eq_ignore_ascii_case(&s.host))
        {
            piers.push(Pier {
                name: s.host.clone(),
                kind: "cargo_hall".into(),
                location: "remote".into(),
                cores: s.machine.ncpu,
                runners: runner_total,
                boards: vec![],
                agents: 0,
                sessions: vec![],
                ground_station: false,
                jobs_ok: 0,
                jobs_bad: 0,
                reporting: true,
                note: String::new(),
            });
        }
    }
    // jobs onto their machine's pier; a bad one becomes the pier's note
    for p in &mut piers {
        let mine: Vec<&crate::jobs::Job> = jobs
            .iter()
            .filter(|j| j.host.eq_ignore_ascii_case(&p.name))
            .collect();
        p.jobs_bad = mine.iter().filter(|j| j.bad()).count() as u32;
        p.jobs_ok = mine.len() as u32 - p.jobs_bad;
        if let Some(j) = mine.iter().find(|j| j.bad()) {
            if p.note.is_empty() {
                p.note = format!("job {} {}", j.name, j.state);
            }
        }
    }
    let tower = piers
        .iter()
        .find(|p| p.ground_station)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "warte".into());
    Airport {
        tower,
        piers,
        runways,
        runner_total,
        holding,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(host: &str, sessions: usize) -> crate::probe::Seen {
        let mut r: crate::probe::Report = serde_json::from_value(serde_json::json!({
            "probe_version": "t", "host": host, "taken_at": 1, "uptime_s": 1,
            "os": {"name": "macOS 27", "kernel": "27", "arch": "arm64"},
            "cpu": {"ncpu": 10, "load": [0.0, 0.0, 0.0]},
            "memory": {"total_mb": 1, "avail_mb": 1, "swap_total_mb": 0, "swap_used_mb": 0},
            "disks": [], "tools": [], "sessions": []
        }))
        .unwrap();
        for i in 0..sessions {
            r.sessions.push(crate::probe::Session {
                agent: "claude".into(),
                pid: i as u32,
                started_at: 0,
                dir: None,
                activity: None,
            });
        }
        crate::probe::Seen {
            report: r,
            received_at: 1_000,
            via: "probe".into(),
        }
    }

    // rivet: partially-verifies REQ-DASH-015
    #[test]
    fn piers_come_from_the_model_and_silent_machines_stay_dashed() {
        let f = crate::fleet::from_spar(include_str!(
            "../../tests/fixtures/example_fleet.instance.json"
        ))
        .unwrap();
        let a = build(Some(&f), &[seen("laptop", 3)], None, &[], 1_000);
        let p = |n: &str| a.piers.iter().find(|p| p.name == n).unwrap().clone();
        assert_eq!(a.piers.len(), f.machines.len());
        assert!(p("laptop").reporting && p("laptop").agents == 3);
        assert!(!p("tower").reporting && p("tower").note == "not reporting");
        assert_eq!(p("tower").boards, vec!["pixhawk-6xrt"]);
        assert_eq!(p("ci").location, "remote");
        assert_eq!(a.tower, "tower");
    }

    // rivet: partially-verifies REQ-DASH-015
    #[test]
    fn without_a_model_the_reporting_machines_and_the_ci_host_are_drawn() {
        let snap = crate::hub::model::Snapshot {
            host: "ci-test".into(),
            runners: vec![crate::hub::model::Runner {
                id: 3,
                local_busy: true,
                job: Some("Build".into()),
                job_started: Some(400),
                ctx: Some(crate::hub::model::JobContext {
                    repo: "org/spar".into(),
                    pr: Some(7),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let a = build(None, &[seen("Mac", 1)], Some((snap, 1_000)), &[], 1_000);
        assert_eq!(
            a.piers.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["Mac", "ci-test"]
        );
        assert_eq!(a.piers[0].kind, "terminal");
        assert_eq!(
            a.runways,
            vec![Runway {
                id: "R03".into(),
                flight: "spar #7".into(),
                job: "Build".into(),
                elapsed_s: 600
            }]
        );
        assert_eq!(a.runner_total, 1);
    }
}

//! Oracles for `warte probe report` (REQ-PROBE-001, first increment).

// rivet: partially-verifies REQ-PROBE-001
#[test]
fn report_describes_this_machine_as_json() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_warte"))
        .args(["--format", "json", "probe", "report"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let r: warte::probe::Report = serde_json::from_slice(&out.stdout).expect("a Report");
    assert!(!r.host.is_empty());
    assert!(r.cpu.ncpu > 0 && r.memory.total_mb > 0);
    assert!(r.taken_at > 1_700_000_000);
    // every known tool is listed, installed or not
    let names: Vec<&str> = r.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, warte::probe::TOOLS);
    for t in &r.tools {
        assert!(
            t.path.is_none() || t.version.is_some() || t.error.is_some(),
            "{} found but neither version nor error: {t:?}",
            t.name
        );
    }
    for d in &r.disks {
        assert!((0.0..=100.0).contains(&d.used_pct), "{d:?}");
    }
}

// rivet: partially-verifies REQ-PROBE-001
#[test]
fn sessions_carry_no_command_lines() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_warte"))
        .args(["--format", "json", "probe", "report"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    for s in v["sessions"].as_array().unwrap() {
        let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        // an agent with the hook installed also carries its verb (REQ-PROBE-009)
        if let Some(a) = s.get("activity") {
            let verb = a.as_str().unwrap();
            assert!(
                warte::probe::activity::VERBS.contains(&verb),
                "activity must be one of the probe's verbs: {s}"
            );
            keys.retain(|k| *k != "activity");
        }
        assert_eq!(keys, ["agent", "dir", "pid", "started_at"], "{s}");
    }
}

fn report(host: &str, kernel: &str, claude: Option<&str>) -> warte::probe::Report {
    use warte::probe::*;
    Report {
        probe_version: "t".into(),
        host: host.into(),
        taken_at: 0,
        os: Os {
            name: "Linux (Debian GNU/Linux 13)".into(),
            kernel: kernel.into(),
            arch: "aarch64".into(),
        },
        uptime_s: 60,
        cpu: Cpu {
            ncpu: 4,
            load: [0.1, 0.1, 0.1],
        },
        memory: Memory {
            total_mb: 1800,
            avail_mb: 1200,
            swap_total_mb: 0,
            swap_used_mb: 0,
        },
        disks: vec![],
        tools: TOOLS
            .iter()
            .map(|t| Tool {
                name: t.to_string(),
                path: (*t == "claude" && claude.is_some()).then(|| "/usr/bin/claude".into()),
                version: if *t == "claude" {
                    claude.map(String::from)
                } else {
                    None
                },
                error: None,
            })
            .collect(),
        sessions: vec![],
        io: None,
        inventory: None,
        health: None,
        hardware: None,
    }
}

// rivet: partially-verifies REQ-PROBE-003
#[test]
fn findings_name_drift_silence_and_full_disks_worst_first() {
    use warte::probe::{findings, Seen};
    let now = 1_800_000_000;
    let seen = |r, age| Seen {
        report: r,
        received_at: now - age,
        via: "probe".into(),
        cert_until: None,
    };
    let mut full = report("gale", "6.18.50+rpt", Some("2.1.312"));
    full.disks.push(warte::probe::Disk {
        mount: "/".into(),
        fs: "ext4".into(),
        size_gb: 7.1,
        used_gb: 6.9,
        used_pct: 97.0,
        removable: false,
    });
    let list = vec![
        seen(report("fourpi", "6.12.75+rpt", Some("2.1.9")), 30),
        seen(report("wohl", "6.18.50+rpt", None), 3600),
        seen(full, 30),
    ];
    let f = findings(&list, now);
    let text = |h: &str| {
        f.iter()
            .filter(|x| x.host == h)
            .map(|x| x.text.clone())
            .collect::<Vec<_>>()
    };
    assert!(
        text("fourpi")
            .iter()
            .any(|t| t.contains("kernel 6.12.75+rpt behind 6.18.50+rpt")),
        "{f:?}"
    );
    // 2.1.9 < 2.1.312 numerically, not as text
    assert!(
        text("fourpi")
            .iter()
            .any(|t| t == "claude 2.1.9 behind 2.1.312"),
        "{f:?}"
    );
    assert!(
        text("wohl").iter().any(|t| t.starts_with("no report for")),
        "{f:?}"
    );
    assert!(
        text("gale").iter().any(|t| t == "disk / is 97% full"),
        "{f:?}"
    );
    assert!(
        !text("gale").iter().any(|t| t.contains("kernel")),
        "gale is current"
    );
    // red first
    assert_eq!(f[0].severity, "red");
    assert!(f
        .iter()
        .skip_while(|x| x.severity == "red")
        .all(|x| x.severity == "amber"));
}

// rivet: partially-verifies REQ-PROBE-003
#[test]
fn the_view_escapes_what_machines_report() {
    let dir = tempfile::tempdir().unwrap();
    let m = warte::probe::Machines::open(dir.path().to_str().unwrap());
    let mut r = report("<script>h</script>", "6.1", Some("<img src=x>"));
    r.disks.push(warte::probe::Disk {
        mount: "/<b>".into(),
        fs: "x".into(),
        size_gb: 1.0,
        used_gb: 0.5,
        used_pct: 50.0,
        removable: false,
    });
    m.record(r, "probe", chrono::Utc::now().timestamp());
    let html = warte::dashboard::machines::view(Some(&m), None);
    assert!(
        !html.contains("<script>h") && !html.contains("<img src=x>") && !html.contains("/<b>"),
        "{html}"
    );
    assert!(html.contains("&lt;script&gt;h&lt;/script&gt;"));
    // it persists: a restarted ground station still knows the machine
    let again = warte::probe::Machines::open(dir.path().to_str().unwrap());
    assert_eq!(again.list().len(), 1);
    assert!(warte::dashboard::machines::view(None, None).contains("No machine has reported yet"));
}

// rivet: verifies REQ-PROBE-008
#[test]
fn io_is_optional_and_becomes_findings() {
    use warte::probe::io::{Device, Io};
    use warte::probe::{findings, Seen};
    // an old probe's report (no io) still parses, and a new one round-trips
    let mut r = report("pi", "6.12", None);
    let old = serde_json::to_string(&r).unwrap();
    assert!(!old.contains("\"io\""), "no io key when there is none");
    let back: warte::probe::Report = serde_json::from_str(&old).unwrap();
    assert_eq!(back.io, None);
    r.io = Some(Io {
        psi_some_avg60: Some(55.0),
        psi_full_avg60: Some(10.0),
        devices: vec![Device {
            name: "sda".into(),
            mounts: vec!["/srv/backup".into()],
            rotational: Some(true),
            busy_pct: Some(70.0),
            await_ms: Some(30.0),
            read_kbs: Some(1.0),
            write_kbs: Some(2.0),
        }],
    });
    let back: warte::probe::Report =
        serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
    assert_eq!(back, r);
    let f = findings(
        &[Seen {
            report: r,
            received_at: 100,
            via: "probe".into(),
            cert_until: None,
        }],
        100,
    );
    let io: Vec<(&str, &str)> = f
        .iter()
        .filter(|f| f.text.contains("IO") || f.text.contains("spinning"))
        .map(|f| (f.severity, f.text.as_str()))
        .collect();
    assert_eq!(io.len(), 2, "{f:?}");
    assert!(io
        .iter()
        .any(|(s, t)| *s == "red" && t.contains("stalled on IO 55%")));
    assert!(io
        .iter()
        .any(|(s, t)| *s == "amber" && t.contains("sda") && t.contains("70% busy")));
}

// rivet: verifies REQ-PROBE-008
#[test]
fn every_report_field_is_published_or_hidden_on_purpose() {
    use warte::server::api::{public_report, HIDDEN_REPORT_FIELDS};
    // a report with every optional part present, so every key serializes
    let mut r = report("pi", "6.12", Some("2.1.4"));
    r.io = Some(warte::probe::io::Io::default());
    let all = serde_json::to_value(&r).unwrap();
    let public = public_report(&r);
    for k in all.as_object().unwrap().keys() {
        assert!(
            public.get(k).is_some() || HIDDEN_REPORT_FIELDS.contains(&k.as_str()),
            "Report field {k:?} is neither published by public_report() nor listed in HIDDEN_REPORT_FIELDS"
        );
    }
    // and what is hidden really is not published as it is
    // REQ-PROBE-013: the version is published, but only as a plain version string
    assert!(public.get("probe_version").is_some());
    assert!(public["tools"][0].get("path").is_none());
    assert!(public["sessions"].is_object());
    assert!(public.get("io").is_some());
}

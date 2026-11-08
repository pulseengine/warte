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
        // and since when, a time only (REQ-PROBE-014)
        if let Some(t) = s.get("activity_at") {
            assert!(t.is_i64(), "activity_at must be a time: {s}");
            keys.retain(|k| *k != "activity_at");
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
        pressure: None,
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

// rivet: verifies REQ-PROBE-015
#[test]
fn kernels_are_compared_only_within_one_architecture_and_flavour() {
    use warte::probe::{findings, kernel_flavour, Seen};
    let now = 1_800_000_000;
    let seen = |r| Seen {
        report: r,
        received_at: now,
        via: "probe".into(),
        cert_until: None,
    };
    let pi = report("pi", "6.18.60+rpt-rpi-v8", None);
    let mut vps = report("vps", "6.12.115+deb13.1-amd64", None);
    vps.os.arch = "x86_64".into();
    let mut old_vps = report("vps2", "6.12.69+deb13-amd64", None);
    old_vps.os.arch = "x86_64".into();
    let f = findings(&[seen(pi), seen(vps), seen(old_vps)], now);
    let kernel: Vec<_> = f.iter().filter(|x| x.text.starts_with("kernel")).collect();
    // the amd64 server is not "behind" the Raspberry Pi kernel ...
    assert!(!kernel.iter().any(|x| x.host == "vps"), "{kernel:?}");
    // ... but the older amd64 one is behind the newer amd64 one
    assert!(
        kernel
            .iter()
            .any(|x| x.host == "vps2" && x.text.contains("behind 6.12.115+deb13.1-amd64")),
        "{kernel:?}"
    );
    assert_eq!(kernel_flavour("6.12.115+deb13.1-amd64"), "amd64");
    assert_eq!(kernel_flavour("6.18.60+rpt-rpi-v8"), "v8");
}

// rivet: verifies REQ-PROBE-015
#[test]
fn a_pending_reboot_is_found_from_the_kernel_images_and_debians_file() {
    use warte::probe::health::{findings, reboot_status, Health};
    let day = 86_400;
    let now = 1_800_000_000;
    let images = vec![
        ("6.12.69+deb13-amd64".to_string(), now - 30 * day),
        ("6.12.115+deb13.1-amd64".to_string(), now - 22 * day),
        // another flavour never counts
        ("6.18.60+rpt-rpi-v8".to_string(), now),
    ];
    // a newer kernel installed 22 days ago, no Debian file (none survives
    // a cleared /run, and not every distribution writes one)
    let (r, since) = reboot_status("6.12.69+deb13-amd64", &images, None).unwrap();
    assert_eq!(r.reason, "kernel");
    assert_eq!(r.newest_kernel.as_deref(), Some("6.12.115+deb13.1-amd64"));
    assert_eq!(since, now - 22 * day);
    // running the newest: nothing (negative control)
    assert!(reboot_status("6.12.115+deb13.1-amd64", &images, None).is_none());
    // Debian's file alone: packages
    let (r, since) = reboot_status(
        "6.12.115+deb13.1-amd64",
        &images,
        Some((now - day, Some(3))),
    )
    .unwrap();
    assert_eq!((r.reason.as_str(), r.pkgs), ("packages", Some(3)));
    assert_eq!(since, now - day);

    // findings: amber at once, red after 7 days, with the reason
    let h = |since: i64, newest: &str| Health {
        reboot_required_since: Some(since),
        reboot: Some(warte::probe::health::Reboot {
            reason: "kernel".into(),
            pkgs: None,
            newest_kernel: Some(newest.into()),
        }),
        ..Default::default()
    };
    let f = findings(&h(now - 3600, "6.12.115"), now);
    assert_eq!(f[0].0, "amber");
    assert!(
        f[0].1
            .contains("reboot required: kernel 6.12.115 is installed"),
        "{f:?}"
    );
    let f = findings(&h(now - 22 * day, "6.12.115"), now);
    assert_eq!(f[0].0, "red");
    assert!(f[0].1.contains("for 22 day(s)"), "{f:?}");
}

// rivet: verifies REQ-PROBE-016
#[test]
fn a_crash_loop_is_red_although_the_unit_never_fails() {
    use std::collections::BTreeMap;
    use warte::probe::health::{findings, parse_restarts, restarting, Health};
    let shown = "Id=temper.service\nNRestarts=1101\nSubState=auto-restart\n\nId=nginx.service\nNRestarts=0\nSubState=running\n\nId=ollama.service\nNRestarts=4\nSubState=running\n\nId=getty@tty1.service\nNRestarts=1\nSubState=running\n";
    let now = parse_restarts(shown);
    assert_eq!(now.len(), 4);
    let prev = BTreeMap::from([
        ("temper.service".to_string(), 1090),
        ("nginx.service".to_string(), 0),
        ("ollama.service".to_string(), 3),
        ("getty@.service".to_string(), 1),
    ]);
    let r = restarting(&now, &prev);
    // nginx and getty did not restart (negative control)
    assert_eq!(
        r.iter()
            .map(|x| (x.unit.as_str(), x.delta))
            .collect::<Vec<_>>(),
        vec![("temper.service", 11), ("ollama.service", 1)]
    );
    let f = findings(
        &Health {
            restarting: r,
            ..Default::default()
        },
        0,
    );
    assert_eq!(f[0].0, "red");
    assert!(
        f[0].1
            .starts_with("temper.service is crash-looping: 11 restart(s)"),
        "{f:?}"
    );
    assert_eq!(f[1].0, "amber");
    // a counter that went down (restarted by hand, rebooted) counts from zero
    let r = restarting(
        &[("x.service".into(), 2, "running".into())],
        &BTreeMap::from([("x.service".to_string(), 50)]),
    );
    assert_eq!(r[0].delta, 2);
}

// rivet: verifies REQ-PROBE-017
#[test]
fn ext4_errors_are_amber_and_a_rising_count_is_red() {
    use std::collections::BTreeMap;
    use warte::probe::health::{findings, fs_errors, Health};
    let prev = BTreeMap::from([("mmcblk0p2".to_string(), 0), ("sda1".to_string(), 2)]);
    let now = vec![
        ("mmcblk0p2".to_string(), 68),
        ("sda1".to_string(), 2),
        ("nvme0n1p1".to_string(), 0),
    ];
    let e = fs_errors(&now, &prev);
    // a clean filesystem is not listed (negative control)
    assert_eq!(e.len(), 2);
    let f = findings(
        &Health {
            fs_errors: e,
            ..Default::default()
        },
        0,
    );
    assert_eq!(f[0].0, "red");
    assert!(
        f[0].1
            .starts_with("filesystem mmcblk0p2 recorded 68 new error(s)"),
        "{f:?}"
    );
    assert_eq!(f[1].0, "amber");
    assert!(
        f[1].1
            .starts_with("filesystem sda1 has 2 error(s) recorded"),
        "{f:?}"
    );
}

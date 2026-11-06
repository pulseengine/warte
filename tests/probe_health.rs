//! Oracles for the probe's health, network, temperature, battery and process
//! readings (REQ-PROBE-011).

use std::collections::BTreeMap;
use warte::probe::health::*;

// rivet: verifies REQ-PROBE-011
#[test]
fn names_pass_a_whitelist_or_are_dropped() {
    assert_eq!(clean("rustc"), Some("rustc".into()));
    assert_eq!(
        clean("com.apple.WebKit.WebContent"),
        Some("com.apple.WebKit.WebContent".into())
    );
    assert_eq!(
        clean("nginx@main.service"),
        Some("nginx@main.service".into())
    );
    for bad in [
        "",
        "/usr/bin/x",
        "a=b",
        "--token=s3cret",
        ".hidden",
        "a\nb",
        "x;rm",
        &"y".repeat(41),
    ] {
        assert_eq!(clean(bad), None, "{bad:?}");
    }
}

// rivet: verifies REQ-PROBE-011
#[test]
fn processes_are_summed_by_name_and_never_carry_arguments() {
    let p = vec![
        ("node".to_string(), 300.0, 100 << 20),
        ("node".to_string(), 385.0, 26 << 20),
        ("/opt/homebrew/bin/python3".to_string(), 1.0, 300 << 20),
        ("curl --token=s3cret".to_string(), 50.0, 1 << 20),
        ("idle".to_string(), 0.1, 1 << 20),
    ];
    let (cpu, mem) = top_procs(&p);
    assert_eq!(cpu[0].name, "node");
    assert_eq!(cpu[0].count, 2);
    assert!((cpu[0].cpu_pct - 685.0).abs() < 1e-9);
    assert!(
        !cpu.iter().any(|x| x.name == "idle"),
        "under 0.5% is not a top process"
    );
    assert_eq!(mem[0].name, "python3", "basename, not the path");
    let all = format!("{cpu:?}{mem:?}");
    assert!(!all.contains("s3cret") && !all.contains("/opt"), "{all}");
}

// rivet: verifies REQ-PROBE-011
#[test]
fn sensors_are_grouped_to_their_hottest_member() {
    let r = vec![
        ("PMU tdie4".to_string(), 63.0, None),
        ("PMU tdie8".to_string(), 69.9, None),
        ("NAND CH0 temp".to_string(), 44.0, None),
        ("Package id 0".to_string(), 80.0, Some(100.0)),
        ("bogus".to_string(), f64::NAN, None),
        ("hot".to_string(), 250.0, None),
    ];
    let t = temps_from(&r);
    assert_eq!(
        t.iter()
            .map(|x| (x.sensor.as_str(), x.c))
            .collect::<Vec<_>>(),
        vec![
            ("Package id", 80.0),
            ("PMU tdie", 69.9),
            ("NAND CH temp", 44.0)
        ]
    );
    assert_eq!(t[0].crit, Some(100.0));
}

// rivet: verifies REQ-PROBE-011
#[test]
fn network_rates_skip_loopback_virtual_and_reset_counters() {
    let mut prev = BTreeMap::new();
    let mut cur = BTreeMap::new();
    prev.insert("en0".to_string(), (1_000_000, 500_000, 0));
    cur.insert(
        "en0".to_string(),
        (1_000_000 + 60 * 1024 * 10, 500_000 + 60 * 1024, 2),
    );
    prev.insert("lo0".to_string(), (0, 0, 0));
    cur.insert("lo0".to_string(), (1 << 30, 1 << 30, 0));
    prev.insert("utun3".to_string(), (0, 0, 0));
    cur.insert("utun3".to_string(), (1 << 20, 0, 0));
    prev.insert("eth1".to_string(), (9_000, 9_000, 0));
    cur.insert("eth1".to_string(), (10, 10, 0)); // counters reset
    let r = net_rates(&prev, &cur, 60_000);
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(r[0].name, "en0");
    assert!((r[0].rx_kbs - 10.0).abs() < 1e-9 && (r[0].tx_kbs - 1.0).abs() < 1e-9);
    assert!((r[0].errors - 2.0).abs() < 1e-9);
    assert!(net_rates(&prev, &cur, 0).is_empty());
}

// rivet: verifies REQ-PROBE-011
#[test]
fn command_outputs_parse() {
    assert_eq!(parse_vmstat_oom("pgfault 1\noom_kill 3\n"), Some(3));
    assert_eq!(parse_vmstat_oom("pgfault 1\n"), None);
    assert_eq!(
        parse_failed_units(r#"[{"unit":"backup.service","load":"loaded"},{"unit":"bad;unit"}]"#),
        Some(vec!["backup.service".into()])
    );
    assert_eq!(parse_failed_units("[]"), Some(vec![]));
    assert_eq!(parse_failed_units("not json"), None);
    let apt = "Inst libc6 [2.36-9] (2.36-9+deb12u1 Debian-Security:12/stable-security [arm64])\nInst vim [2:9.0] (2:9.0.1 Debian:12.5/stable [arm64])\nConf libc6\n";
    assert_eq!(parse_apt_sim(apt), (2, 1));
    let su = "Software Update Tool\n\nFinding available software\nSoftware Update found the following new or updated software:\n* Label: Safari19.1-19.1\n\tTitle: Safari, Version: 19.1\n* Label: macOS Tahoe 27.1-27.1\n";
    assert_eq!(parse_softwareupdate(su), 2);
    assert_eq!(parse_softwareupdate("No new software available.\n"), 0);
    assert_eq!(
        parse_pmset("Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t87%; charging; 0:40 remaining present: true\n"),
        Some((87.0, true))
    );
    assert_eq!(
        parse_pmset(" -InternalBattery-0 (id=1)\t54%; discharging; 3:10 remaining\n"),
        Some((54.0, false))
    );
    assert_eq!(parse_pmset("Now drawing from 'AC Power'\n"), None);
    assert_eq!(parse_ioreg_cycles("  \"CycleCount\" = 312\n"), Some(312));
    assert_eq!(
        parse_timedatectl("Timezone=UTC\nNTPSynchronized=yes\n"),
        Some(true)
    );
    assert_eq!(parse_timedatectl("NTPSynchronized=no\n"), Some(false));
    let off = parse_chrony_offset_ms(
        "Reference ID    : 1\nSystem time     : 0.001500000 seconds fast of NTP time\n",
    )
    .unwrap();
    assert!((off - 1.5).abs() < 1e-9);
    let off =
        parse_chrony_offset_ms("System time     : 0.250000000 seconds slow of NTP time\n").unwrap();
    assert!((off + 250.0).abs() < 1e-9);
}

fn health() -> Health {
    Health::default()
}

// rivet: verifies REQ-PROBE-011
#[test]
fn findings_say_what_needs_a_look() {
    let now = 1_800_000_000;
    let mut h = health();
    assert!(findings(&h, now).is_empty());
    h.failed_units = Some(vec![
        "a.service".into(),
        "b.timer".into(),
        "c.service".into(),
        "d.service".into(),
    ]);
    h.reboot_required_since = Some(now - 4 * 86_400);
    h.updates = Some(Updates {
        count: 5,
        security: Some(2),
        checked_at: now,
        security_since: Some(now - 8 * 86_400),
    });
    h.clock_offset_ms = Some(600.0);
    h.ntp_synced = Some(false);
    h.last_oom_at = Some(now - 120);
    h.temps = vec![Temp {
        sensor: "Package id".into(),
        c: 97.0,
        crit: Some(100.0),
    }];
    let f: Vec<String> = findings(&h, now)
        .into_iter()
        .map(|(s, t)| format!("{s}: {t}"))
        .collect();
    let all = f.join("\n");
    for want in [
        "amber: 4 failed systemd unit(s): a.service, b.timer, c.service and 1 more",
        "amber: reboot required for 4 days",
        "amber: 2 security update(s) pending for 8 days",
        "amber: clock off by 600 ms",
        "amber: clock not synchronised (NTP)",
        "amber: memory exhausted: a process was killed (OOM) in the last hour",
        "amber: Package id at 97 °C, near critical 100 °C",
    ] {
        assert!(all.contains(want), "missing {want:?} in\n{all}");
    }
    h.clock_offset_ms = Some(-6000.0);
    h.temps[0].c = 101.0;
    let all = format!("{:?}", findings(&h, now));
    assert!(all.contains("\"red\", \"clock off by -6.0 s\""), "{all}");
    assert!(
        all.contains("\"red\", \"Package id at 101 °C, at or over critical 100 °C\""),
        "{all}"
    );
    // young problems wait: a fresh reboot need, fresh security updates, an old OOM
    h.reboot_required_since = Some(now - 3600);
    h.updates.as_mut().unwrap().security_since = Some(now - 3600);
    h.last_oom_at = Some(now - 7200);
    let all = format!("{:?}", findings(&h, now));
    assert!(
        !all.contains("reboot") && !all.contains("security") && !all.contains("OOM"),
        "{all}"
    );
}

// rivet: verifies REQ-PROBE-011
#[test]
fn health_reaches_history_and_the_api_without_error_text() {
    let mut h = health();
    h.psi_mem = Some(12.5);
    h.oom_kills = Some(1);
    h.net = vec![Iface {
        name: "eth0".into(),
        rx_kbs: 3.0,
        tx_kbs: 1.0,
        errors: 0.0,
    }];
    h.temps = vec![Temp {
        sensor: "PMU tdie".into(),
        c: 60.0,
        crit: None,
    }];
    h.failed_units = Some(vec![]);
    h.updates = Some(Updates {
        count: 2,
        security: Some(0),
        checked_at: 1,
        security_since: None,
    });
    h.battery = Some(Battery {
        pct: 80.0,
        cycles: Some(300),
        charging: Some(false),
    });
    h.top_cpu = vec![Proc {
        name: "node".into(),
        cpu_pct: 50.0,
        mem_mb: 10.0,
        count: 1,
    }];
    h.errors = vec!["chronyc: /usr/bin/chronyc SECRET-ERROR".into()];
    let s = warte::history::project::from_health(&h);
    let get = |n: &str| s.iter().find(|(k, _)| k == n).map(|(_, v)| *v);
    assert_eq!(get("psi.mem"), Some(12.5));
    assert_eq!(get("oom.kills"), Some(1.0));
    assert_eq!(get("net.rx_kbs{if=eth0}"), Some(3.0));
    assert_eq!(get("temp.c{sensor=PMU tdie}"), Some(60.0));
    assert_eq!(get("health.failed_units"), Some(0.0));
    assert_eq!(get("health.reboot_required"), Some(0.0));
    assert_eq!(get("health.updates"), Some(2.0));
    assert_eq!(get("battery.cycles"), Some(300.0));
    assert!(
        !s.iter().any(|(k, _)| k.contains("node")),
        "processes are not series"
    );
    let p = public(&h).to_string();
    assert!(!p.contains("SECRET") && !p.contains("chronyc"), "{p}");
    assert!(p.contains(r#""not_read":1"#), "{p}");
}

// rivet: verifies REQ-PROBE-011
#[test]
fn the_history_panel_lists_processes_now_by_name() {
    let t = tempfile::tempdir().unwrap();
    let h = warte::history::History::open(t.path().to_str().unwrap()).unwrap();
    let mut x = health();
    x.top_cpu = vec![Proc {
        name: "loriot-server".into(),
        cpu_pct: 99.6,
        mem_mb: 0.1,
        count: 1,
    }];
    x.top_mem = vec![Proc {
        name: "claude".into(),
        cpu_pct: 6.7,
        mem_mb: 2735.0,
        count: 12,
    }];
    let html = warte::dashboard::machines::history_panel_with(&h, "box", None, Some(&x));
    assert!(html.contains("Processes now"), "{html}");
    assert!(
        html.contains("<b>loriot-server</b> <span class=\"dim\">100% CPU</span>"),
        "{html}"
    );
    assert!(
        html.contains("<b>claude</b> ×12 <span class=\"dim\">2735 MB</span>"),
        "{html}"
    );
}

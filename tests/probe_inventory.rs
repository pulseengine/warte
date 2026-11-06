//! Oracles for the probe's agent and toolchain inventory (REQ-PROBE-010).

use std::os::unix::fs::PermissionsExt;
use warte::probe::inventory::{
    cron_from, launchctl_status, launchd_schedule, origin, plugins_from, public, shadowed,
    timers_from, Inventory, Job, Plugin, Shadowed, Shadowing,
};

fn exe(dir: &std::path::Path, name: &str) {
    let p = dir.join(name);
    std::fs::write(&p, "#!/bin/sh\necho 1.0.0\n").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

// rivet: verifies REQ-PROBE-010
#[test]
fn a_tool_earlier_on_path_than_its_shim_is_shadowed() {
    let t = tempfile::tempdir().unwrap();
    let (shims, cargo) = (t.path().join("shims"), t.path().join("cargo"));
    std::fs::create_dir_all(&shims).unwrap();
    std::fs::create_dir_all(&cargo).unwrap();
    for n in ["rivet", "spar"] {
        exe(&shims, n);
    }
    exe(&cargo, "rivet");

    // cargo first: rivet is shadowed, spar resolves to its shim
    let path = std::env::join_paths([&cargo, &shims]).unwrap();
    let (on, found) = shadowed(&path, &shims);
    assert!(on);
    assert_eq!(found, vec![("rivet".to_string(), cargo.join("rivet"))]);

    // shims first: nothing is shadowed
    let path = std::env::join_paths([&shims, &cargo]).unwrap();
    assert_eq!(shadowed(&path, &shims), (true, vec![]));

    // shims missing from PATH: said, and what runs instead is named
    let path = std::env::join_paths([&cargo]).unwrap();
    let (on, found) = shadowed(&path, &shims);
    assert!(!on);
    assert_eq!(found.len(), 1);
}

// rivet: verifies REQ-PROBE-010
#[test]
fn plugins_scheduled_jobs_and_cron_are_parsed_without_arguments() {
    let p = plugins_from(
        r#"{"version":2,"plugins":{
            "pulseengine-claude@pulseengine-eu":[{"scope":"user","version":"0.56.0"},{"scope":"project","version":"0.40.0"}],
            "superpowers@claude-plugins-official":[{"scope":"user","version":"7.0.0"}]}}"#,
    )
    .unwrap();
    assert_eq!(p.len(), 3);
    assert_eq!(
        (p[0].name.as_str(), p[0].marketplace.as_str()),
        ("pulseengine-claude", "pulseengine-eu")
    );
    assert!(plugins_from("[]").is_err());

    let st = launchctl_status("PID\tStatus\tLabel\n-\t0\teu.a\n123\t0\teu.b\n-\t-9\teu.monthly\n");
    assert_eq!(st["eu.monthly"], (false, Some(-9)));
    assert_eq!(st["eu.b"], (true, Some(0)));
    let plist: serde_json::Value = serde_json::json!({
        "Label": "eu.monthly",
        "StartCalendarInterval": {"Day": 1, "Hour": 5, "Minute": 0}
    });
    assert_eq!(launchd_schedule(&plist), "Day=1 Hour=5 Minute=0");
    assert_eq!(
        launchd_schedule(&serde_json::json!({"StartInterval": 3600})),
        "every 3600 s"
    );
    assert_eq!(
        launchd_schedule(&serde_json::json!({"RunAtLoad": true})),
        "at login"
    );

    let timers = timers_from(
        r#"[{"next":1,"left":2,"last":3,"passed":4,"unit":"backup.timer","activates":"backup.service"}]"#,
    );
    assert_eq!(
        timers,
        vec![("backup.timer".into(), "backup.service".into())]
    );

    // arguments and environment lines are dropped: they can carry tokens
    let c = cron_from(
        "# comment\nMAILTO=me\nAPI_TOKEN=s3cret\n0 3 * * * /usr/local/bin/backup --token s3cret\n@daily curl -H 'Authorization: x' https://x\n",
    );
    assert_eq!(
        c,
        vec![
            ("0 3 * * *".to_string(), "backup".to_string()),
            ("@daily".to_string(), "curl".to_string())
        ]
    );
    assert!(!format!("{c:?}").contains("s3cret"));
}

fn inv() -> Inventory {
    Inventory {
        shadowing: Some(Shadowing {
            at: 1,
            shims_on_path: false,
            tools: vec![Shadowed {
                tool: "rivet".into(),
                path: "/Users/secret/.cargo/bin/rivet".into(),
                version: Some("0.38.0".into()),
            }],
        }),
        plugins: vec![
            Plugin {
                name: "pulseengine-claude".into(),
                marketplace: "pulseengine-eu".into(),
                version: "0.40.0".into(),
                scope: "user".into(),
            },
            Plugin {
                name: "github".into(),
                marketplace: "claude-plugins-official".into(),
                version: "fa4eb54397ee".into(),
                scope: "user".into(),
            },
        ],
        scheduled: vec![Job {
            name: "eu.monthly<x>".into(),
            via: "launchd".into(),
            schedule: "Day=1 Hour=5".into(),
            last_exit: Some(-9),
            failed: true,
            running: false,
        }],
        errors: vec!["plutil exited 1: /Users/secret/Library/x".into()],
    }
}

fn report(host: &str, i: Inventory) -> warte::probe::Report {
    let mut r: warte::probe::Report = serde_json::from_value(serde_json::json!({
        "probe_version": "t", "host": host, "taken_at": 0,
        "os": {"name": "macOS 26", "kernel": "25.0", "arch": "arm64"},
        "uptime_s": 1, "cpu": {"ncpu": 8, "load": [0.1, 0.1, 0.1]},
        "memory": {"total_mb": 1000, "avail_mb": 900, "swap_total_mb": 0, "swap_used_mb": 0},
        "disks": [], "tools": [], "sessions": []
    }))
    .expect("an older report without an inventory still parses");
    assert!(r.inventory.is_none());
    r.inventory = Some(i);
    r
}

// rivet: verifies REQ-PROBE-010
#[test]
fn drift_findings_name_unpinned_tools_failed_jobs_and_old_plugins() {
    use warte::probe::{findings, Seen};
    let now = 1_800_000_000;
    let mut newer = inv();
    newer.plugins[0].version = "0.56.0".into();
    newer.plugins[1].version = "0123abc".into();
    newer.shadowing = None;
    newer.scheduled.clear();
    let seen: Vec<Seen> = [report("mac", inv()), report("box", newer)]
        .into_iter()
        .map(|r| Seen {
            report: r,
            received_at: now,
            via: "probe".into(),
            cert_until: None,
        })
        .collect();
    let f: Vec<String> = findings(&seen, now)
        .into_iter()
        .map(|f| format!("{}: {}", f.host, f.text))
        .collect();
    let all = f.join("\n");
    assert!(
        all.contains("mac: varve's shims are not on agents' PATH: 1 tool(s) run unpinned (rivet 0.38.0 from cargo install)"),
        "{all}"
    );
    assert!(
        all.contains("mac: scheduled job eu.monthly<x> last run killed by signal 9"),
        "{all}"
    );
    assert!(
        all.contains("mac: plugin pulseengine-claude 0.40.0 behind 0.56.0"),
        "{all}"
    );
    // a git hash is not a version: no finding either way
    assert!(!all.contains("plugin github"), "{all}");
    assert!(!all.contains("/Users/secret"), "{all}");
}

// rivet: verifies REQ-PROBE-010
#[test]
fn the_published_inventory_carries_no_paths_or_error_output() {
    assert_eq!(origin("/Users/x/.cargo/bin/rivet"), "cargo install");
    assert_eq!(origin("/opt/homebrew/bin/spar"), "Homebrew");
    assert_eq!(origin("/usr/bin/git"), "system");
    let p = public(&inv()).to_string();
    assert!(!p.contains("/Users/secret") && !p.contains("plutil"), "{p}");
    assert!(p.contains(r#""origin":"cargo install""#) && p.contains(r#""not_read":1"#));
    let r = report("mac", inv());
    let api = warte::server::api::public_report(&r).to_string();
    assert!(!api.contains("/Users/secret"), "{api}");
}

//! Per-mount disk alert levels (REQ-ALERT-004).

use warte::config::{AlertsConfig, DiskLevel, DiskLevels};
use warte::probe::{findings_with, Seen};

const RUNNERS: &str = "/var/lib/runners";

fn levels(toml_src: &str) -> Result<DiskLevels, String> {
    let c: warte::config::Config = toml::from_str(toml_src).map_err(|e| e.to_string())?;
    c.alerts.unwrap_or_default().check()
}

fn seen(now: i64, disks: &[(&str, f64)]) -> Seen {
    let disks: Vec<serde_json::Value> = disks
        .iter()
        .map(|(m, p)| {
            serde_json::json!({"mount": m, "fs": "ext4", "size_gb": 100.0,
                "used_gb": p, "used_pct": p, "removable": false})
        })
        .collect();
    let report = serde_json::from_value(serde_json::json!({
        "probe_version": "t", "host": "ci-host", "taken_at": now,
        "os": {"name": "Linux", "kernel": "6.12.1", "arch": "x86_64"},
        "uptime_s": 60, "cpu": {"ncpu": 4, "load": [0.1, 0.1, 0.1]},
        "memory": {"total_mb": 1000, "avail_mb": 900, "swap_total_mb": 0, "swap_used_mb": 0},
        "disks": disks, "tools": [], "sessions": []
    }))
    .unwrap();
    Seen {
        report,
        received_at: now,
        via: "probe".into(),
        cert_until: None,
    }
}

fn disk_sev(l: &DiskLevels, mount: &str, pct: f64) -> Option<String> {
    let now = 1_800_000_000;
    findings_with(&[seen(now, &[(mount, pct)])], now, l)
        .into_iter()
        .find(|f| f.text.starts_with("disk "))
        .map(|f| f.severity.to_string())
}

// rivet: verifies REQ-ALERT-004
#[test]
fn a_mount_override_alerts_earlier_and_other_mounts_keep_85_and_95() {
    let l = levels("[alerts.disk]\n\"/var/lib/runners\" = { amber = 75, red = 85 }\n").unwrap();
    assert_eq!(l.of(RUNNERS).amber, 75.0);
    assert_eq!(disk_sev(&l, RUNNERS, 74.0), None);
    assert_eq!(disk_sev(&l, RUNNERS, 76.0).as_deref(), Some("amber"));
    assert_eq!(disk_sev(&l, RUNNERS, 86.0).as_deref(), Some("red"));
    // any other mount, and the defaults, are unchanged
    assert_eq!(disk_sev(&l, "/", 80.0), None);
    assert_eq!(disk_sev(&l, "/", 90.0).as_deref(), Some("amber"));
    let d = DiskLevels::default();
    assert_eq!(disk_sev(&d, RUNNERS, 80.0), None);
    assert_eq!(disk_sev(&d, RUNNERS, 96.0).as_deref(), Some("red"));
}

// rivet: verifies REQ-ALERT-004
#[test]
fn levels_that_could_never_alert_or_clear_are_refused() {
    for bad in [
        "[alerts.disk]\n\"/x\" = { amber = 90, red = 80 }\n",
        "[alerts.disk]\n\"/x\" = { amber = 80, red = 80 }\n",
        "[alerts.disk]\n\"/x\" = { amber = 0, red = 80 }\n",
        "[alerts.disk]\n\"/x\" = { amber = 80, red = 101 }\n",
        "[alerts.disk]\n\"x\" = { amber = 70, red = 80 }\n",
        "[alerts.disk]\n\"/x\" = { amber = 70, red = 80, crit = 90 }\n",
        "[alerts]\nmemory = 1\n",
    ] {
        assert!(levels(bad).is_err(), "{bad}");
    }
    let ok = AlertsConfig {
        disk: [(
            RUNNERS.to_string(),
            DiskLevel {
                amber: 75.0,
                red: 85.0,
            },
        )]
        .into(),
    };
    assert!(ok.check().is_ok());
}

// rivet: verifies REQ-ALERT-004
#[tokio::test]
async fn the_server_applies_the_levels_and_refuses_bad_ones() {
    let dir = tempfile::tempdir().unwrap();
    let hub_dir = dir.path().join("hub");
    std::fs::create_dir_all(&hub_dir).unwrap();
    let mk = |alerts: AlertsConfig| warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: Some(warte::config::HubConfig {
            udp: "127.0.0.1:0".into(),
            allow: vec![std::net::Ipv4Addr::LOCALHOST.into()],
            data_dir: hub_dir.to_str().unwrap().into(),
            org: "pulseengine".into(),
            probe_self: false,
            costs_file: None,
            skill_usage_file: None,
            quic: None,
            quic_names: vec![],
            require_quic: false,
        }),
        alerts: Some(alerts),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&dir.path().join("state.db"))
        .await
        .unwrap();
    let bad = AlertsConfig {
        disk: [(
            RUNNERS.to_string(),
            DiskLevel {
                amber: 90.0,
                red: 85.0,
            },
        )]
        .into(),
    };
    let e = warte::server::build_state(mk(bad), db.clone())
        .err()
        .expect("refused");
    assert!(e.to_string().contains(RUNNERS), "{e}");

    let good = AlertsConfig {
        disk: [(
            RUNNERS.to_string(),
            DiskLevel {
                amber: 75.0,
                red: 85.0,
            },
        )]
        .into(),
    };
    let state = warte::server::build_state(mk(good), db).unwrap();
    let m = state.machines.as_ref().expect("a machine store with [hub]");
    let now = chrono::Utc::now().timestamp();
    let s = seen(now, &[(RUNNERS, 88.0)]);
    m.record(s.report, "probe", now);
    let f = m.findings(now);
    assert!(
        f.iter()
            .any(|f| f.severity == "red" && f.text.contains("88%")),
        "{f:?}"
    );
}

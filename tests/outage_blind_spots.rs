//! What went unnoticed on a host for days now reaches Outages and alerts
//! (REQ-TEMPER-003, REQ-PROBE-015, REQ-PROBE-016).

use std::net::Ipv4Addr;
use warte::dashboard::fleet::host_detected;
use warte::probe::health::{Health, Reboot, Restarting};
use warte::probe::{Cpu, Memory, Os, Report};

fn report(host: &str, health: Health) -> Report {
    Report {
        probe_version: "t".into(),
        host: host.into(),
        taken_at: warte::hub::now(),
        os: Os {
            name: "L".into(),
            kernel: "6.12.69+deb13-amd64".into(),
            arch: "x86_64".into(),
        },
        uptime_s: 1,
        cpu: Cpu {
            ncpu: 1,
            load: [0.0; 3],
        },
        memory: Memory {
            total_mb: 1,
            avail_mb: 1,
            swap_total_mb: 0,
            swap_used_mb: 0,
        },
        disks: vec![],
        tools: vec![],
        sessions: vec![],
        io: None,
        inventory: None,
        health: Some(health),
        hardware: None,
        pressure: None,
    }
}

async fn state(temper: bool) -> (tempfile::TempDir, warte::server::AppState) {
    let dir = tempfile::tempdir().unwrap();
    let hub_dir = dir.path().join("hub");
    std::fs::create_dir_all(&hub_dir).unwrap();
    let now = warte::hub::now();
    let m = warte::probe::Machines::open(hub_dir.to_str().unwrap());
    m.record(
        report(
            "vps",
            Health {
                restarting: vec![Restarting {
                    unit: "temper.service".into(),
                    restarts: 1101,
                    delta: 11,
                    state: "auto-restart".into(),
                }],
                reboot_required_since: Some(now - 22 * 86_400),
                reboot: Some(Reboot {
                    reason: "kernel".into(),
                    pkgs: None,
                    newest_kernel: Some("6.12.115+deb13.1-amd64".into()),
                }),
                // drift, not an outage
                failed_units: Some(vec!["run-u1.service".into()]),
                ..Default::default()
            },
        ),
        "probe",
        now,
    );
    m.record(report("quiet", Health::default()), "probe", now);
    drop(m);
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: Some(warte::config::HubConfig {
            udp: "127.0.0.1:0".into(),
            allow: vec![Ipv4Addr::LOCALHOST.into()],
            data_dir: hub_dir.to_str().unwrap().into(),
            org: "pulseengine".into(),
            probe_self: false,
            costs_file: None,
            skill_usage_file: None,
        }),
        temper: temper.then(|| warte::config::TemperFeedConfig {
            host: "vps".into(),
            stale_after_s: 13 * 3600,
        }),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let s = warte::server::build_state(config, db).unwrap();
    (dir, s)
}

// rivet: verifies REQ-PROBE-015
// rivet: verifies REQ-PROBE-016
#[tokio::test]
async fn red_machine_findings_are_outages_and_amber_drift_is_not() {
    let (_d, s) = state(false).await;
    let now = warte::hub::now();
    let found = host_detected(&s, now);
    let texts: Vec<&str> = found.iter().map(|d| d.text.as_str()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("vps: temper.service is crash-looping")),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("vps: reboot required for 22 day(s)")),
        "{texts:?}"
    );
    assert!(found.iter().all(|d| d.sev == 2));
    // amber drift (a failed transient unit) and a quiet host: not outages
    assert!(
        !texts.iter().any(|t| t.contains("failed systemd")),
        "{texts:?}"
    );
    assert!(!texts.iter().any(|t| t.starts_with("quiet")), "{texts:?}");
    // keys hold while the counts change, so an open outage does not re-open
    let later = host_detected(&s, now + 3 * 86_400);
    let key = |l: &[warte::dashboard::fleet::Detected], p: &str| {
        l.iter().find(|d| d.text.contains(p)).map(|d| d.key.clone())
    };
    assert_eq!(key(&found, "reboot"), key(&later, "reboot"));
}

// rivet: verifies REQ-TEMPER-003
#[tokio::test]
async fn a_temper_report_that_never_comes_is_an_outage_after_its_window() {
    let (_d, s) = state(true).await;
    let opened = s.temper.as_ref().unwrap().opened_at;
    let within = host_detected(&s, opened + 3600);
    assert!(!within.iter().any(|d| d.key.starts_with("temper")));
    let late = host_detected(&s, opened + 14 * 3600);
    let t = late
        .iter()
        .find(|d| d.key == "temper-missing")
        .expect("missing");
    assert_eq!(t.sev, 2);
    assert!(t.text.contains("no temper report from vps"), "{}", t.text);
    // without [temper] there is nothing to expect
    let (_d2, s2) = state(false).await;
    assert!(!host_detected(&s2, opened + 14 * 3600)
        .iter()
        .any(|d| d.key.starts_with("temper")));
}

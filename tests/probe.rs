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
        let keys: Vec<&String> = s.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["agent", "dir", "pid", "started_at"], "{s}");
    }
}

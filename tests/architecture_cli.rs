//! `warte arch check` exits 0, 1 or 2 (REQ-ARCHVIEW-001). The checks themselves
//! are tested in crates/warte-check, natively and as a WASI component.

use serde_json::{json, Value};

const EXAMPLE: &str = include_str!("fixtures/architecture.example.json");

fn example() -> Value {
    serde_json::from_str(EXAMPLE).unwrap()
}

// rivet: verifies REQ-ARCHVIEW-001
#[test]
fn the_cli_exits_0_1_or_2() {
    let bin = env!("CARGO_BIN_EXE_warte");
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good.json");
    std::fs::write(&good, EXAMPLE).unwrap();
    let mut broken = example();
    broken["schema"] = json!("nope");
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, broken.to_string()).unwrap();
    let run = |p: &std::path::Path| {
        std::process::Command::new(bin)
            .args(["arch", "check"])
            .arg(p)
            .output()
            .unwrap()
    };
    let o = run(&good);
    assert_eq!(o.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&o.stdout).contains("finding: e-sensor"));
    assert_eq!(run(&bad).status.code(), Some(1));
    assert_eq!(run(&dir.path().join("missing.json")).status.code(), Some(2));
}

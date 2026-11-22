//! The architecture view's contract, architecture.v1 (REQ-ARCHVIEW-001).

use serde_json::{json, Value};
use warte::architecture::check;

const EXAMPLE: &str = include_str!("fixtures/architecture.example.json");

/// A planted change to the example, and the error it must cause.
type Case = (Box<dyn Fn(&mut Value)>, &'static str);

fn example() -> Value {
    serde_json::from_str(EXAMPLE).unwrap()
}

fn errors(v: &Value) -> Vec<String> {
    check(&v.to_string()).err().unwrap_or_default()
}

// rivet: verifies REQ-ARCHVIEW-001
#[test]
fn the_example_checks_with_its_crossings_and_one_finding() {
    let c = check(EXAMPLE).unwrap();
    assert_eq!(c.crossings.values().filter(|x| **x).count(), 3);
    assert!(!c.crossings["e-session"]);
    assert_eq!(
        c.findings,
        ["e-sensor crosses from z-home to z-ci without authentication and encryption"]
    );
}

// rivet: verifies REQ-ARCHVIEW-001
#[test]
fn every_broken_reference_or_shape_is_refused_by_name() {
    let cases: Vec<Case> = vec![
        (
            Box::new(|v| v["schema"] = json!("architecture.v2")),
            "schema: must be architecture.v1",
        ),
        (
            Box::new(|v| v["zones"][0]["colour"] = json!("red")),
            "schema:",
        ),
        (
            Box::new(|v| v["components"][0]["kind"] = json!("router")),
            "schema:",
        ),
        (
            Box::new(|v| v["components"][1]["id"] = json!("c-server")),
            "c-server: declared more than once",
        ),
        (
            Box::new(|v| v["components"][1]["id"] = json!("Bad_Id")),
            "an id is [a-z0-9-]",
        ),
        (
            Box::new(|v| v["components"][1]["zone"] = json!("z-nowhere")),
            "c-ground.zone: no zone z-nowhere",
        ),
        (
            Box::new(|v| v["components"][0]["host"] = json!("c-ci-host")),
            "c-server.host: a host runs on no host",
        ),
        (
            Box::new(|v| v["components"][1]["host"] = json!("c-forge")),
            "c-ground.host: c-forge is not a host component",
        ),
        (
            Box::new(|v| v["components"][1]["agent"] = json!("someone")),
            "c-ground.agent: only a session runs as an agent",
        ),
        (
            Box::new(|v| v["connections"][0]["to"] = json!("c-missing")),
            "e-telemetry: from and to must name components",
        ),
        (
            Box::new(|v| v["connections"][0]["credential"] = json!("c-agent/no-such")),
            "e-telemetry.credential",
        ),
        (
            Box::new(|v| v["connections"][0]["zone_crossing"] = json!(false)),
            "e-telemetry.zone_crossing: disagrees",
        ),
    ];
    for (i, (plant, want)) in cases.iter().enumerate() {
        let mut v = example();
        plant(&mut v);
        let e = errors(&v);
        assert!(
            e.iter().any(|m| m.contains(want)),
            "case {i}: want {want:?}, got {e:?}"
        );
    }
}

// rivet: verifies REQ-ARCHVIEW-001
#[test]
fn a_value_that_looks_like_a_secret_is_refused_and_never_echoed() {
    for secret in [
        "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        "-----BEGIN PRIVATE KEY-----",
        "github_pat_11ABCDEFG0123456789",
        "AKIAIOSFODNN7EXAMPLE",
        "warte1.eyJhZ2VudCI6ImEifQ.c2ln",
        "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ1MSIsImV4cCI6MX0.c2lnbmF0dXJlLXZhbHVl",
    ] {
        let mut v = example();
        v["components"][1]["credentials"][0]["purpose"] = json!(secret);
        let e = errors(&v);
        assert_eq!(e.len(), 1, "{secret}: {e:?}");
        assert!(
            e[0].starts_with("$.components[1].credentials[0].purpose: looks like a secret"),
            "{e:?}"
        );
        assert!(!e[0].contains(&secret[..10]), "the value was echoed: {e:?}");
    }
    // an ordinary purpose passes
    let mut v = example();
    v["components"][1]["credentials"][0]["purpose"] = json!("signs agent tokens");
    assert!(check(&v.to_string()).is_ok());
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

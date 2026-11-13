//! sessions v1: the agent session list (REQ-SESS-001), against the contract
//! in docs/topics/sessions.md.

use warte::sessions::{check, SessionsFile};

const DOC: &str = include_str!("../docs/topics/sessions.md");

/// The example file in the contract doc.
fn example() -> serde_json::Value {
    let start = DOC.find("```json\n").expect("example") + 8;
    let end = start + DOC[start..].find("```").unwrap();
    serde_json::from_str(&DOC[start..end]).expect("the doc's example is JSON")
}

fn verdict(v: &serde_json::Value) -> Result<(), String> {
    let f: SessionsFile = warte::sessions::parse(&v.to_string())?;
    check(&f)
}

/// One way to change the example file.
type Edit = Box<dyn Fn(&mut serde_json::Value)>;

// rivet: verifies REQ-SESS-001
#[test]
fn the_contract_example_passes() {
    verdict(&example()).unwrap();
}

// rivet: verifies REQ-SESS-001
#[test]
fn every_rule_refuses_a_file_and_names_the_field() {
    let many: Vec<serde_json::Value> = (0..65)
        .map(|i| {
            let mut s = example()["sessions"][0].clone();
            s["id"] = format!("s{i}").into();
            s["name"] = format!("s{i}").into();
            s
        })
        .collect();
    let s = |k: &'static str, v: serde_json::Value| -> Edit {
        Box::new(move |f| f["sessions"][1][k] = v.clone())
    };
    let cases: Vec<(&str, Edit, &str)> = vec![
        (
            "version 2",
            Box::new(|f| f["version"] = 2.into()),
            "version",
        ),
        (
            "unknown top key",
            Box::new(|f| f["hosts"] = 1.into()),
            "hosts",
        ),
        ("unknown session key", s("host", "a".into()), "host"),
        (
            "missing enabled",
            Box::new(|f| {
                f["sessions"][1].as_object_mut().unwrap().remove("enabled");
            }),
            "enabled",
        ),
        (
            "missing mode",
            Box::new(|f| {
                f["sessions"][1].as_object_mut().unwrap().remove("mode");
            }),
            "mode",
        ),
        ("enabled as string", s("enabled", "yes".into()), "enabled"),
        (
            "no sessions",
            Box::new(|f| f["sessions"] = serde_json::json!([])),
            "sessions",
        ),
        (
            "65 sessions",
            Box::new(move |f| f["sessions"] = many.clone().into()),
            "sessions",
        ),
        ("uppercase id", s("id", "Docs".into()), "sessions[1].id"),
        ("empty id", s("id", "".into()), "sessions[1].id"),
        ("long id", s("id", "a".repeat(33).into()), "sessions[1].id"),
        ("duplicate id", s("id", "warte".into()), "sessions[1].id"),
        ("blank name", s("name", "  ".into()), "sessions[1].name"),
        (
            "long name",
            s("name", "n".repeat(65).into()),
            "sessions[1].name",
        ),
        (
            "control in name",
            s("name", "a\nb".into()),
            "sessions[1].name",
        ),
        (
            "duplicate name by case",
            s("name", "WARTE".into()),
            "sessions[1].name",
        ),
        (
            "relative dir",
            s("dir", "git/site".into()),
            "sessions[1].dir",
        ),
        (
            "dotdot dir",
            s("dir", "/Users/a/../b".into()),
            "sessions[1].dir",
        ),
        (
            "control in dir",
            s("dir", "/a\u{7}b".into()),
            "sessions[1].dir",
        ),
        (
            "long dir",
            s("dir", format!("/{}", "d".repeat(1024)).into()),
            "sessions[1].dir",
        ),
        ("empty model", s("model", "".into()), "sessions[1].model"),
        (
            "model with space",
            s("model", "opus 5".into()),
            "sessions[1].model",
        ),
        (
            "model with flag",
            s("model", "--x;rm".into()),
            "sessions[1].model",
        ),
        (
            "unknown effort",
            s("effort", "huge".into()),
            "sessions[1].effort",
        ),
        ("unknown mode", s("mode", "yolo".into()), "sessions[1].mode"),
        (
            "mode case",
            s("mode", "bypasspermissions".into()),
            "sessions[1].mode",
        ),
        (
            "hostname as role",
            s("role", "mini.local".into()),
            "sessions[1].role",
        ),
        ("empty role", s("role", "".into()), "sessions[1].role"),
    ];
    for (what, edit, field) in cases {
        let mut v = example();
        edit(&mut v);
        let err = verdict(&v).expect_err(what);
        assert!(err.contains(field), "{what}: {err:?} does not name {field}");
    }
}

// rivet: verifies REQ-SESS-001
#[test]
fn the_limits_themselves_pass() {
    let mut v = example();
    v["sessions"][1]["id"] = "a".repeat(32).into();
    v["sessions"][1]["name"] = "n".repeat(64).into();
    v["sessions"][1]["dir"] = format!("/{}", "d".repeat(1023)).into();
    v["sessions"][1]["model"] = "claude-opus-5-5[1m]".into();
    v["sessions"][1]["role"] = "b".repeat(32).into();
    for e in warte::sessions::EFFORTS {
        v["sessions"][1]["effort"] = (*e).into();
        for m in warte::sessions::MODES {
            v["sessions"][1]["mode"] = (*m).into();
            verdict(&v).unwrap_or_else(|e| panic!("{m}/{e}"));
        }
    }
}

// rivet: verifies REQ-SESS-001
#[test]
fn the_documented_choices_are_the_checked_ones() {
    for e in warte::sessions::EFFORTS {
        assert!(DOC.contains(&format!("`{e}`")), "effort {e} undocumented");
    }
    for m in warte::sessions::MODES {
        assert!(DOC.contains(&format!("`{m}`")), "mode {m} undocumented");
    }
}

fn run(args: &[&str]) -> (i32, String, String) {
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_warte"))
        .args(args)
        .env_remove("WARTE_CONFIG")
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into(),
        String::from_utf8_lossy(&o.stderr).into(),
    )
}

// rivet: verifies REQ-SESS-001
#[test]
fn the_check_command_exits_by_the_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good.json");
    std::fs::write(&good, example().to_string()).unwrap();
    let (code, out, _) = run(&["sessions", "check", good.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "ok: sessions v1, 2 sessions, 1 enabled");
    let (code, out, _) = run(&[
        "--format",
        "json",
        "sessions",
        "check",
        good.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    let j: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["enabled"], 1);

    let bad = dir.path().join("bad.json");
    let mut v = example();
    v["sessions"][0]["mode"] = "yolo".into();
    std::fs::write(&bad, v.to_string()).unwrap();
    let (code, _, err) = run(&["sessions", "check", bad.to_str().unwrap()]);
    assert_eq!(code, 1, "{err}");
    assert!(
        err.contains("[sessions]") && err.contains("bad.json") && err.contains("sessions[0].mode"),
        "{err}"
    );

    let (code, _, err) = run(&[
        "sessions",
        "check",
        dir.path().join("absent.json").to_str().unwrap(),
    ]);
    assert_eq!(code, 1);
    assert!(err.contains("absent.json"), "{err}");
}

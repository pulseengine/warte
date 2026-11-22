//! `warte probe focus` (REQ-FOCUS-001, warte#180). Fixtures follow the
//! shape of macOS's ModeConfigurations.json as read on 2026-11-22.

use serde_json::{json, Value};
use warte::probe::focus::{night, run, with_available, Night, DEFAULT_MODE};

fn focus_file(triggers: Value) -> String {
    json!({"data": [{"modeConfigurations": {
        "com.apple.sleep.sleep-mode": {"mode": {"name": "Schlafen"}, "triggers": {"triggers": triggers}},
        "com.apple.donotdisturb.mode.default": {"mode": {"name": "Do Not Disturb"},
            "triggers": {"triggers": [{"class": "DNDModeConfigurationScheduleTrigger", "enabledSetting": 0,
                "timePeriodStartTimeHour": 9, "timePeriodStartTimeMinute": 0,
                "timePeriodEndTimeHour": 17, "timePeriodEndTimeMinute": 0, "timePeriodWeekdays": 127}]}}
    }}]})
    .to_string()
}

fn schedule(sh: u64, sm: u64, eh: u64, em: u64, enabled: u64, days: u64) -> Value {
    json!({"class": "DNDModeConfigurationScheduleTrigger", "enabledSetting": enabled,
           "timePeriodStartTimeHour": sh, "timePeriodStartTimeMinute": sm,
           "timePeriodEndTimeHour": eh, "timePeriodEndTimeMinute": em, "timePeriodWeekdays": days})
}

// rivet: verifies REQ-FOCUS-001
#[test]
fn a_sleep_schedule_22_30_to_06_30_gives_available_06_30_to_22_30() {
    let f = focus_file(json!([schedule(22, 30, 6, 30, 2, 127)]));
    let n = night(&f, DEFAULT_MODE).unwrap();
    assert_eq!(
        n,
        Night {
            start: (22, 30),
            end: (6, 30)
        }
    );
    assert_eq!(n.available(), json!([{"from": "06:30", "to": "22:30"}]));
    // the mode can also be named, in any case, as macOS localizes it
    assert_eq!(night(&f, "schlafen").unwrap(), n);
    // a night that does not cross midnight gives a span that does
    let early = night(
        &focus_file(json!([schedule(1, 0, 5, 0, 2, 127)])),
        DEFAULT_MODE,
    )
    .unwrap();
    assert_eq!(early.available(), json!([{"from": "05:00", "to": "01:00"}]));
}

// rivet: verifies REQ-FOCUS-001
#[test]
fn anything_in_doubt_is_refused_with_a_reason() {
    for (file, want) in [
        (
            focus_file(json!([schedule(22, 30, 6, 30, 0, 127)])),
            "no enabled schedule",
        ),
        (
            focus_file(json!([schedule(22, 30, 6, 30, 2, 62)])),
            "some days only",
        ),
        (
            focus_file(
                json!([{"class": "DNDModeConfigurationSleepingTrigger", "enabledSetting": 2}]),
            ),
            "not on a time schedule",
        ),
        (
            focus_file(json!([
                schedule(22, 0, 6, 0, 2, 127),
                schedule(13, 0, 14, 0, 2, 127)
            ])),
            "several schedules",
        ),
        (focus_file(json!([schedule(22, 0, 22, 0, 2, 127)])), "empty"),
        (
            focus_file(json!([schedule(25, 0, 6, 0, 2, 127)])),
            "format changed",
        ),
        (json!({"modes": []}).to_string(), "format changed"),
        ("not json".to_string(), "not JSON"),
    ] {
        let e = night(&file, DEFAULT_MODE).unwrap_err();
        assert!(e.contains(want), "{want}: {e}");
    }
    let f = focus_file(json!([schedule(22, 30, 6, 30, 2, 127)]));
    assert!(night(&f, "Work").unwrap_err().contains("no Focus mode"));
}

// rivet: verifies REQ-FOCUS-001
#[test]
fn writing_replaces_only_available_and_a_doubt_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let att = dir.path().join("attention.json");
    let before = r#"{"schema_version":1,"windows":[{"from":"09:00","to":"11:00"}],"off":[{"from":"2026-12-24","to":"2026-12-27","note":"holiday"}],"available":["10:00-12:00"],"timezone":"Europe/Berlin"}"#;
    std::fs::write(&att, before).unwrap();
    let good = dir.path().join("modes.json");
    std::fs::write(&good, focus_file(json!([schedule(22, 30, 6, 30, 2, 127)]))).unwrap();
    run(&good, DEFAULT_MODE, Some(&att)).unwrap();
    let after: Value = serde_json::from_str(&std::fs::read_to_string(&att).unwrap()).unwrap();
    let old: Value = serde_json::from_str(before).unwrap();
    assert_eq!(
        after["available"],
        json!([{"from": "06:30", "to": "22:30"}])
    );
    for k in ["schema_version", "windows", "off", "timezone"] {
        assert_eq!(after[k], old[k], "{k} changed");
    }
    // the helper never writes off, and keeps a value's bytes
    let text = std::fs::read_to_string(&att).unwrap();
    assert!(
        text.contains(r#"{"from":"2026-12-24","to":"2026-12-27","note":"holiday"}"#),
        "{text}"
    );

    // planted: a disabled schedule, an unreadable file; nothing is written
    let snapshot = std::fs::read_to_string(&att).unwrap();
    let off = dir.path().join("off.json");
    std::fs::write(&off, focus_file(json!([schedule(22, 30, 6, 30, 0, 127)]))).unwrap();
    assert!(run(&off, DEFAULT_MODE, Some(&att)).is_err());
    let e = run(&dir.path().join("missing.json"), DEFAULT_MODE, Some(&att)).unwrap_err();
    assert!(e.contains("Full Disk Access"), "{e}");
    assert_eq!(
        std::fs::read_to_string(&att).unwrap(),
        snapshot,
        "a doubt wrote something"
    );
    // an attention file that is not an object is refused, not replaced
    std::fs::write(&att, "[1,2]").unwrap();
    assert!(run(&good, DEFAULT_MODE, Some(&att)).is_err());
    assert_eq!(std::fs::read_to_string(&att).unwrap(), "[1,2]");
    assert!(with_available(
        "{}",
        &Night {
            start: (22, 0),
            end: (6, 0)
        }
    )
    .unwrap()
    .contains("06:00"));
}

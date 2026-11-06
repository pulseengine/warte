//! Oracles for the probe version and certificate findings (REQ-PROBE-013).

use warte::probe::version::{behind, can_renew, cert_finding, clean, parse};
use warte::probe::{findings, Seen};

const D: i64 = 86400;

// rivet: verifies REQ-PROBE-013
#[test]
fn versions_parse_strictly_and_compare_by_release() {
    assert_eq!(parse("1.5.1"), Some((1, 5, 1)));
    assert_eq!(parse("1.6.0-rc.1"), Some((1, 6, 0)));
    assert_eq!(parse("1.6.0+abc"), Some((1, 6, 0)));
    for bad in [
        "",
        "t",
        "1.5",
        "1.5.1.2",
        "1..1",
        "v1.5.1",
        "1.5.x",
        "<b>1.5.1</b>",
    ] {
        assert_eq!(parse(bad), None, "{bad:?}");
    }
    assert_eq!(clean("1.5.1"), Some("1.5.1"));
    assert_eq!(clean("<script>"), None);
    assert_eq!(clean(&"1".repeat(41)), None);
    assert!(behind("1.5.1", "1.6.0"));
    assert!(behind("1.9.9", "2.0.0"));
    assert!(!behind("1.6.0", "1.6.0"));
    assert!(!behind("1.7.0", "1.6.0"));
    assert!(
        !behind("t", "1.6.0"),
        "an unknown version is not called behind"
    );
    assert!(!can_renew("1.4.9"));
    assert!(can_renew("1.5.0") && can_renew("2.0.0") && can_renew("t"));
}

// rivet: verifies REQ-PROBE-013
#[test]
fn a_probe_that_cannot_renew_is_warned_two_weeks_ahead() {
    let now = 1_800_000_000;
    assert_eq!(cert_finding("1.4.0", now + 15 * D, now), None);
    let (sev, text) = cert_finding("1.4.0", now + 10 * D, now).unwrap();
    assert_eq!(sev, "amber");
    assert_eq!(
        text,
        "probe 1.4.0 cannot renew its certificate, which ends in 10 d; it needs 1.5.0 or newer"
    );
    assert_eq!(cert_finding("1.4.0", now + 3 * D, now).unwrap().0, "red");
    assert!(cert_finding("1.4.0", now + 20 * 3600, now)
        .unwrap()
        .1
        .contains("ends in 20 h"));
}

// rivet: verifies REQ-PROBE-013
#[test]
fn a_probe_that_can_renew_is_warned_only_when_it_did_not() {
    let now = 1_800_000_000;
    // renewal starts at 10 days left: 8 left is still normal
    assert_eq!(cert_finding("1.5.1", now + 8 * D, now), None);
    let (sev, text) = cert_finding("1.5.1", now + 6 * D, now).unwrap();
    assert_eq!(sev, "amber");
    assert_eq!(
        text,
        "probe certificate ends in 6 d and has not been renewed"
    );
    assert_eq!(cert_finding("1.5.1", now + D, now).unwrap().0, "red");
    // an unreadable version is treated as able to renew, not as a known old one
    assert!(cert_finding("t", now + 6 * D, now)
        .unwrap()
        .1
        .contains("not been renewed"));
}

// rivet: verifies REQ-PROBE-013
#[test]
fn findings_carry_the_certificate_rule_per_machine() {
    let now = 1_800_000_000;
    let report = |host: &str, v: &str| -> warte::probe::Report {
        serde_json::from_value(serde_json::json!({
            "probe_version": v, "host": host, "taken_at": now, "uptime_s": 1,
            "os": {"name": "x", "kernel": "x", "arch": "x"},
            "cpu": {"ncpu": 1, "load": [0.0, 0.0, 0.0]},
            "memory": {"total_mb": 1, "avail_mb": 1, "swap_total_mb": 0, "swap_used_mb": 0},
            "disks": [], "tools": [], "sessions": []
        }))
        .unwrap()
    };
    let seen = |host, v, until| Seen {
        report: report(host, v),
        received_at: now,
        via: "probe".into(),
        cert_until: until,
    };
    let f = findings(
        &[
            seen("old", "1.4.0", Some(now + 5 * D)),
            seen("new", "1.6.0", Some(now + 25 * D)),
            seen("local", "1.4.0", None),
        ],
        now,
    );
    let mine = |h: &str| f.iter().filter(|x| x.host == h).count();
    assert_eq!(mine("old"), 1, "{f:?}");
    assert_eq!(mine("new"), 0);
    assert_eq!(mine("local"), 0, "no certificate known, no finding");
}

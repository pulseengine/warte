//! Bench devices (REQ-PROBE-019, #230): which registered debug probes, boards
//! and bus interfaces are attached to this host now.
//!
//! The registry is the deployment's `bench-devices.yaml` (the file the
//! device claims use): `devices:` maps a name to an entry with `serial`,
//! `usb` (`vvvv:pppp`) or `iface`, and optionally `missing-alert-minutes`.
//! Entries with none of the three (pseudo-devices) are not hardware and are
//! skipped.
//!
//! Presence comes from sysfs only: `/sys/bus/usb/devices/*/{serial,idVendor,
//! idProduct}` and `/sys/class/net/*/operstate`. The probe never opens a tty,
//! a CAN socket or any device node: those need a claim, and the probe holds
//! none. Only a device's name, present or missing, how it matched, and since
//! when it is missing leave the host; serials and USB ids stay here.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A missing device alerts after this many minutes unless its entry says
/// otherwise; `missing-alert-minutes: 0` never alerts.
pub const DEFAULT_MISSING_ALERT_MIN: u32 = 15;
/// Most devices reported.
pub const MAX_DEVICES: usize = 40;
/// A registry larger than this is refused unread.
pub const MAX_REGISTRY_BYTES: u64 = 64 * 1024;
/// Longest device name reported.
pub const MAX_NAME: usize = 48;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Bench {
    /// `ok`, `invalid` (registry refused: too large or not parseable),
    /// `unsupported` (no sysfs on this host, e.g. macOS).
    pub state: String,
    #[serde(default)]
    pub devices: Vec<Device>,
    /// Entries skipped because their name is outside the allowed charset.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub skipped: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Device {
    pub name: String,
    pub present: bool,
    /// `serial`, `usb` or `iface`: the registry key that decided.
    pub matched_by: String,
    /// When it was first seen missing (unix seconds), while it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_since: Option<i64>,
    /// Minutes missing before it alerts; 0 never.
    pub alert_after_min: u32,
}

#[derive(Deserialize, Debug, Default)]
struct Registry {
    #[serde(default)]
    devices: BTreeMap<String, Entry>,
}

#[derive(Deserialize, Debug, Default)]
struct Entry {
    #[serde(default)]
    serial: Option<String>,
    #[serde(default)]
    usb: Option<String>,
    #[serde(default)]
    iface: Option<String>,
    #[serde(default, rename = "missing-alert-minutes")]
    missing_alert_minutes: Option<u32>,
}

/// What is attached now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Live {
    /// (serial, `vvvv:pppp`) per USB device; serial empty when it has none.
    pub usb: Vec<(String, String)>,
    /// Interface name → whether its operational state is not `down`.
    pub ifaces: BTreeMap<String, bool>,
}

/// A name the read API may carry: `[a-z0-9][a-z0-9._-]*`, at most `MAX_NAME`.
pub fn name_ok(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= MAX_NAME
        && n.as_bytes()[0].is_ascii_alphanumeric()
        && n.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

/// Match the registry against what is attached. `prev` maps a name to the
/// time it was first seen missing; the result's second half is the next one.
pub fn evaluate(
    registry: &str,
    live: &Live,
    prev: &BTreeMap<String, i64>,
    now: i64,
) -> (Bench, BTreeMap<String, i64>) {
    let Ok(reg) = serde_yaml::from_str::<Registry>(registry) else {
        return (
            Bench {
                state: "invalid".into(),
                ..Default::default()
            },
            BTreeMap::new(),
        );
    };
    let mut b = Bench {
        state: "ok".into(),
        ..Default::default()
    };
    let mut next = BTreeMap::new();
    for (name, e) in reg.devices {
        let (present, by) = if let Some(s) = &e.serial {
            (live.usb.iter().any(|(ser, _)| ser == s.trim()), "serial")
        } else if let Some(id) = &e.usb {
            let id = id.trim().to_ascii_lowercase();
            (live.usb.iter().any(|(_, i)| *i == id), "usb")
        } else if let Some(i) = &e.iface {
            (live.ifaces.get(i.trim()).copied().unwrap_or(false), "iface")
        } else {
            continue; // a pseudo-device, not hardware
        };
        if !name_ok(&name) {
            b.skipped += 1;
            continue;
        }
        if b.devices.len() == MAX_DEVICES {
            break;
        }
        let missing_since = (!present).then(|| *prev.get(&name).unwrap_or(&now));
        if let Some(t) = missing_since {
            next.insert(name.clone(), t);
        }
        b.devices.push(Device {
            name,
            present,
            matched_by: by.into(),
            missing_since,
            alert_after_min: e.missing_alert_minutes.unwrap_or(DEFAULT_MISSING_ALERT_MIN),
        });
    }
    (b, next)
}

/// Findings: a device missing past its threshold is red (it pages), one
/// missing for less, or whose threshold is 0, amber; a refused registry amber.
pub fn findings(b: &Bench, taken_at: i64) -> Vec<(&'static str, String)> {
    let mut f = Vec::new();
    if b.state == "invalid" {
        f.push((
            "amber",
            "bench device registry refused (too large or not YAML)".to_string(),
        ));
    }
    // re-checked here: the report comes from the machine, not from this code
    for d in b.devices.iter().filter(|d| !d.present && name_ok(&d.name)) {
        let since = d.missing_since.unwrap_or(taken_at);
        let min = (taken_at - since).max(0) / 60;
        if d.alert_after_min > 0 && min >= i64::from(d.alert_after_min) {
            f.push((
                "red",
                format!("bench device {} missing for {min} min", d.name),
            ));
        } else {
            f.push(("amber", format!("bench device {} missing", d.name)));
        }
    }
    f
}

fn read_trim(p: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
}

/// USB devices and network interfaces from sysfs; `None` without sysfs.
fn linux_live() -> Option<Live> {
    let rd = std::fs::read_dir("/sys/bus/usb/devices").ok()?;
    let mut live = Live::default();
    for e in rd.flatten() {
        let p = e.path();
        let (Some(v), Some(pr)) = (
            read_trim(&p.join("idVendor")),
            read_trim(&p.join("idProduct")),
        ) else {
            continue;
        };
        let serial = read_trim(&p.join("serial")).unwrap_or_default();
        live.usb
            .push((serial, format!("{}:{}", v, pr).to_ascii_lowercase()));
    }
    if let Ok(rd) = std::fs::read_dir("/sys/class/net") {
        for e in rd.flatten() {
            let up = read_trim(&e.path().join("operstate")).is_some_and(|s| s != "down");
            live.ifaces
                .insert(e.file_name().to_string_lossy().into_owned(), up);
        }
    }
    Some(live)
}

/// The registry path: `WARTE_BENCH_REGISTRY`, else
/// `$HOME/.config/pulseengine/bench-devices.yaml`.
pub fn registry_path() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("WARTE_BENCH_REGISTRY") {
        return Some(p.into());
    }
    std::env::var_os("HOME")
        .map(|h| std::path::Path::new(&h).join(".config/pulseengine/bench-devices.yaml"))
}

/// Collect: `None` when there is no registry (not a bench host).
pub fn collect(
    registry: Option<&std::path::Path>,
    prev_path: Option<&std::path::Path>,
    now: i64,
) -> Option<Bench> {
    let reg = registry?;
    let meta = std::fs::metadata(reg).ok()?;
    let invalid = || Bench {
        state: "invalid".into(),
        ..Default::default()
    };
    if meta.len() > MAX_REGISTRY_BYTES {
        return Some(invalid());
    }
    let Ok(text) = std::fs::read_to_string(reg) else {
        return Some(invalid());
    };
    let Some(live) = linux_live() else {
        return Some(Bench {
            state: "unsupported".into(),
            ..Default::default()
        });
    };
    let prev: BTreeMap<String, i64> = prev_path
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let (b, next) = evaluate(&text, &live, &prev, now);
    if let Some(p) = prev_path {
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, serde_json::to_vec(&next).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, p);
        }
    }
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REG: &str = r#"
devices:
  stlink-v3:
    what: a debug probe
    serial: 003B001A0000000000000001
  stlink-v1:
    usb: 0483:3744
  logic-analyser:
    serial: E665D0C500000001
  can0:
    iface: can0
  esp-jtag:
    usb: 303a:1001
    missing-alert-minutes: 30
  disk-io:
    what: a pseudo-device, no hardware
  "Bad Name<x>":
    usb: 1234:5678
"#;

    fn live() -> Live {
        Live {
            usb: vec![
                ("003B001A0000000000000001".into(), "0483:374f".into()),
                (String::new(), "0483:3744".into()),
                ("E665D0C500000001".into(), "2e8a:000a".into()),
            ],
            ifaces: [("can0".to_string(), true), ("lo".to_string(), true)].into(),
        }
    }

    // rivet: verifies REQ-PROBE-019
    #[test]
    fn four_present_one_missing_and_the_missing_one_alerts_after_its_threshold() {
        let (b, next) = evaluate(REG, &live(), &BTreeMap::new(), 1_000);
        assert_eq!(b.state, "ok");
        assert_eq!(b.skipped, 1, "the name outside the charset is skipped");
        let by: BTreeMap<_, _> = b
            .devices
            .iter()
            .map(|d| (d.name.as_str(), (d.present, d.matched_by.as_str())))
            .collect();
        assert_eq!(by.len(), 5, "disk-io is not hardware: {by:?}");
        assert_eq!(by["stlink-v3"], (true, "serial"));
        assert_eq!(by["stlink-v1"], (true, "usb"));
        assert_eq!(by["logic-analyser"], (true, "serial"));
        assert_eq!(by["can0"], (true, "iface"));
        assert_eq!(by["esp-jtag"], (false, "usb"));
        assert_eq!(next.get("esp-jtag"), Some(&1_000));

        // missing since 1000: amber at first, red once 30 min have passed
        assert_eq!(
            findings(&b, 1_000),
            vec![("amber", "bench device esp-jtag missing".into())]
        );
        let (b2, next2) = evaluate(REG, &live(), &next, 1_000 + 30 * 60);
        assert_eq!(next2.get("esp-jtag"), Some(&1_000), "missing_since is kept");
        assert_eq!(
            findings(&b2, 1_000 + 30 * 60),
            vec![("red", "bench device esp-jtag missing for 30 min".into())]
        );

        // it comes back: no finding and no missing_since
        let mut l = live();
        l.usb.push((String::new(), "303a:1001".into()));
        let (b3, next3) = evaluate(REG, &l, &next2, 1_000 + 31 * 60);
        assert!(findings(&b3, 1_000 + 31 * 60).is_empty());
        assert!(next3.is_empty());
    }

    // rivet: verifies REQ-PROBE-019
    #[test]
    fn no_serial_or_usb_id_is_reported_and_bad_input_is_refused() {
        let (b, _) = evaluate(REG, &live(), &BTreeMap::new(), 0);
        let json = serde_json::to_string(&b).unwrap();
        for secret in ["003B001A", "E665D0C5", "0483", "303a", "Bad Name"] {
            assert!(!json.contains(secret), "{secret} leaked: {json}");
        }
        // a threshold of 0 never pages
        let reg = "devices:\n  board:\n    usb: 1111:2222\n    missing-alert-minutes: 0\n";
        let (b, _) = evaluate(reg, &live(), &[("board".to_string(), 0)].into(), 86_400);
        assert_eq!(
            findings(&b, 86_400),
            vec![("amber", "bench device board missing".into())]
        );
        // not YAML: invalid, an amber finding, no devices
        let (b, next) = evaluate("devices: [unclosed", &live(), &BTreeMap::new(), 0);
        assert_eq!(b.state, "invalid");
        assert!(b.devices.is_empty() && next.is_empty());
        assert_eq!(findings(&b, 0).len(), 1);
        // an interface that is down is missing
        let mut l = live();
        l.ifaces.insert("can0".into(), false);
        let (b, _) = evaluate(REG, &l, &BTreeMap::new(), 0);
        assert!(b.devices.iter().any(|d| d.name == "can0" && !d.present));
    }

    #[test]
    fn names_outside_the_charset_are_refused() {
        for ok in ["stlink-v3", "can0", "a.b_c-d", "0x"] {
            assert!(name_ok(ok), "{ok}");
        }
        for bad in [
            "",
            "-x",
            "A",
            "a b",
            "<img>",
            "é",
            &"a".repeat(MAX_NAME + 1),
        ] {
            assert!(!name_ok(bad), "{bad}");
        }
    }
}

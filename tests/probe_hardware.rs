//! Oracles for the probe's hardware block and the privileged collector
//! split (REQ-PROBE-012).

use std::collections::BTreeMap;
use warte::probe::hardware::*;

// rivet: verifies REQ-PROBE-012
#[test]
fn debug_probes_are_recognised_hubs_are_not_devices() {
    assert_eq!(
        probe_kind("0483", "374E", None).as_deref(),
        Some("ST-LINK V3")
    );
    assert_eq!(probe_kind("1366", "0105", None).as_deref(), Some("J-Link"));
    assert_eq!(
        probe_kind("c0de", "0001", Some("Foo CMSIS-DAP v2")).as_deref(),
        Some("CMSIS-DAP")
    );
    assert_eq!(probe_kind("05ac", "8104", Some("Keyboard")), None);
    assert!(usb_device("05e3", "0625", Some("USB3.2 Hub"), None, None).is_none());
    assert!(usb_device(
        "1d6b",
        "0002",
        Some("xHCI Host Controller"),
        None,
        Some("hub")
    )
    .is_none());
    let u = usb_device(
        "0x0483",
        "0x374e",
        Some("STLINK-V3"),
        Some("STMicroelectronics"),
        Some("cdc_acm"),
    )
    .unwrap();
    assert_eq!(
        (u.id.as_str(), u.probe.as_deref(), u.driver.as_deref()),
        ("0483:374e", Some("ST-LINK V3"), Some("cdc_acm"))
    );
    // a product string outside the whitelist is dropped, the device stays
    let u = usb_device("1234", "5678", Some("evil<script>"), None, None).unwrap();
    assert_eq!(u.product, None);
}

// rivet: verifies REQ-PROBE-012
#[test]
fn macos_usb_tree_parses_without_serial_numbers() {
    let ioreg = r#"+-o Root  <class IORegistryEntry>
  +-o USB3 Gen2 Hub@01100000  <class IOUSBHostDevice>
  | |   "USB Product Name" = "USB3 Gen2 Hub"
  | |   "idVendor" = 1452
  | |   "idProduct" = 32780
  | +-o STLINK-V3@01130000  <class IOUSBHostDevice>
  |     "USB Serial Number" = "003A00123456789"
  |     "USB Product Name" = "STLINK-V3"
  |     "USB Vendor Name" = "STMicroelectronics"
  |     "idVendor" = 1155
  |     "idProduct" = 14158
  +-o YubiKey@01200000  <class IOUSBHostDevice>
        "USB Product Name" = "Yubikey NEO OTP+U2F"
        "idVendor" = 4176
        "idProduct" = 276
"#;
    let u = parse_ioreg_usb(ioreg);
    assert_eq!(u.len(), 2, "{u:?}");
    assert_eq!(u[0].probe.as_deref(), Some("ST-LINK V3"));
    assert_eq!(u[0].id, "0483:374e");
    assert_eq!(u[1].id, "1050:0114");
    assert!(!format!("{u:?}").contains("003A00123456789"));
}

const NVME: &str = r#"{"model_name":"Samsung SSD 990 PRO 2TB","serial_number":"S6Z2NJ0SECRET","smart_status":{"passed":true},
"nvme_smart_health_information_log":{"percentage_used":83,"media_errors":0,"power_on_hours":12000,"temperature":41},
"temperature":{"current":41},"power_on_time":{"hours":12000}}"#;

const ATA: &str = r#"{"model_name":"WDC WD40EFRX","rotation_rate":5400,"smart_status":{"passed":false},
"ata_smart_attributes":{"table":[{"id":5,"value":100,"raw":{"value":8}},{"id":197,"value":100,"raw":{"value":2}}]},
"power_on_time":{"hours":40000},"temperature":{"current":38}}"#;

const SSD: &str = r#"{"model_name":"CT1000MX500","rotation_rate":0,"smart_status":{"passed":true},
"ata_smart_attributes":{"table":[{"id":177,"value":90,"raw":{"value":120}}]}}"#;

// rivet: verifies REQ-PROBE-012
#[test]
fn smartctl_json_becomes_disk_health_without_the_serial() {
    let n = parse_smartctl("nvme0n1", NVME, 7).unwrap();
    assert_eq!(
        (
            n.wear_pct,
            n.media_errors,
            n.power_on_h,
            n.temp_c,
            n.smart_ok
        ),
        (Some(83.0), Some(0), Some(12000), Some(41.0), Some(true))
    );
    assert_eq!(n.model.as_deref(), Some("Samsung SSD 990 PRO 2TB"));
    assert!(!format!("{n:?}").contains("SECRET"));
    let a = parse_smartctl("sda", ATA, 7).unwrap();
    assert_eq!(
        (a.smart_ok, a.media_errors, a.rotational, a.wear_pct),
        (Some(false), Some(10), Some(true), None)
    );
    let s = parse_smartctl("sdb", SSD, 7).unwrap();
    assert_eq!((s.wear_pct, s.rotational), (Some(10.0), Some(false)));
    assert!(parse_smartctl("sda", "not json", 7).is_none());
    assert!(
        parse_smartctl("../etc", NVME, 7).is_none(),
        "a device name outside the whitelist is dropped"
    );
    assert_eq!(
        parse_diskutil_smart("   SMART Status:              Verified\n"),
        Some(true)
    );
    assert_eq!(
        parse_diskutil_smart("   SMART Status:              Failing\n"),
        Some(false)
    );
    assert_eq!(
        parse_diskutil_smart("   SMART Status:              Not Supported\n"),
        None
    );
    assert_eq!(
        parse_diskutil_model("   Device / Media Name:       APPLE SSD AP0256Z\n").as_deref(),
        Some("APPLE SSD AP0256Z")
    );
}

// rivet: verifies REQ-PROBE-012
#[test]
fn gpus_parse() {
    let g = parse_sp_displays(
        r#"{"SPDisplaysDataType":[{"sppci_model":"Apple M4","sppci_cores":"10"}]}"#,
    );
    assert_eq!((g[0].name.as_str(), g[0].cores), ("Apple M4", Some(10)));
    let n = parse_nvidia_smi("NVIDIA GeForce RTX 4090, 37, 6000, 24000, 61\n");
    assert_eq!(n[0].name, "NVIDIA GeForce RTX 4090");
    assert_eq!(
        (n[0].util_pct, n[0].mem_used_pct, n[0].temp_c),
        (Some(37.0), Some(25.0), Some(61.0))
    );
}

// rivet: verifies REQ-PROBE-012
#[test]
fn a_probe_attached_for_a_while_and_gone_is_named_by_kind() {
    let now = 1_800_000_000;
    let mut before = BTreeMap::new();
    // attached an hour, last seen 5 minutes ago: gone
    before.insert("ST-LINK V3".to_string(), (now - 3600, now - 300));
    // plugged in for one flash (2 minutes): not a fixture, not alarmed
    before.insert("J-Link".to_string(), (now - 600, now - 480));
    // last seen 8 days ago: forgotten
    before.insert(
        "TI XDS110".to_string(),
        (now - 9 * 86_400, now - 8 * 86_400),
    );
    let (gone, next) = probes_gone(&before, &["Raspberry Pi Debug Probe".to_string()], now);
    assert_eq!(
        gone.iter().map(|g| g.probe.as_str()).collect::<Vec<_>>(),
        vec!["ST-LINK V3"]
    );
    assert!(!next.contains_key("TI XDS110"));
    assert_eq!(next["Raspberry Pi Debug Probe"], (now, now));
    // reflashing changes the product id, not the kind: still attached, not gone
    let (gone, _) = probes_gone(
        &next,
        &[
            "Raspberry Pi Debug Probe".to_string(),
            "ST-LINK V3".to_string(),
        ],
        now + 60,
    );
    assert!(gone.is_empty(), "{gone:?}");
    // boards and adapters are shown but are not probes
    let b = usb_device("1915", "c00a", Some("nRF52 DK"), None, None).unwrap();
    assert_eq!(
        (b.probe, b.board.as_deref(), b.product.as_deref()),
        (None, Some("Nordic board"), Some("nRF52 DK"))
    );
    let f = usb_device("0403", "6010", Some("Dual RS232-HS"), None, None).unwrap();
    assert_eq!(
        (f.probe, f.board.as_deref()),
        (None, Some("FTDI dual adapter"))
    );
    // an unknown device's firmware-set names are not published
    let u = usb_device(
        "abcd",
        "1234",
        Some("ACME ProjectX ECU Rev3"),
        Some("ACME"),
        None,
    )
    .unwrap();
    assert_eq!((u.product, u.vendor), (None, None));
}

// rivet: verifies REQ-PROBE-012
#[test]
fn media_errors_alarm_when_they_rise_not_forever() {
    let now = 1_800_000_000;
    let disk = |n| DiskHealth {
        dev: "sda".into(),
        media_errors: Some(n),
        ..Default::default()
    };
    let (r, m) = media_rising(&BTreeMap::new(), &[disk(8)], now);
    assert!(r.is_empty(), "a first reading is a baseline");
    let (r, m) = media_rising(&m, &[disk(8)], now + 3600);
    assert!(r.is_empty(), "constant is not news");
    let (r, m) = media_rising(&m, &[disk(9)], now + 7200);
    assert_eq!(r, vec!["sda".to_string()]);
    let (r, m) = media_rising(&m, &[disk(9)], now + 7200 + 3600);
    assert_eq!(r, vec!["sda".to_string()], "for a day after the rise");
    let (r, _) = media_rising(&m, &[disk(9)], now + 7200 + 86_401);
    assert!(r.is_empty());
}

fn write(dir: &std::path::Path, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(PRIVILEGED_FILE), body).unwrap();
}

// rivet: verifies REQ-PROBE-012
#[test]
fn the_privileged_output_is_untrusted_data() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path().join("priv");
    let now = 1_800_000_000;
    assert_eq!(read_privileged(&d, now).1, "missing");

    // a round trip through the collector's own writer
    let p = Privileged {
        schema: PRIVILEGED_SCHEMA.into(),
        taken_at: now - 600,
        disks: vec![PrivDisk {
            dev: "nvme0n1".into(),
            model: Some("Samsung SSD".into()),
            rotational: Some(false),
            smart_ok: Some(true),
            wear_pct: Some(12.0),
            media_errors: Some(0),
            power_on_h: Some(100),
            temp_c: Some(40.0),
            asleep: None,
        }],
        errors: 0,
        candidates: 1,
        smartctl: true,
    };
    write_privileged(&d, &p).unwrap();
    let (disks, st) = read_privileged(&d, now);
    assert_eq!(st, "ok");
    assert_eq!(
        (disks[0].dev.as_str(), disks[0].wear_pct),
        ("nvme0n1", Some(12.0))
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(d.join(PRIVILEGED_FILE))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o644);

    // stale or from the future: no numbers, a status
    assert_eq!(
        read_privileged(&d, now + PRIVILEGED_STALE_S + 601),
        (vec![], "stale")
    );
    assert_eq!(read_privileged(&d, now - 3600), (vec![], "stale"));

    // unknown fields, a wrong schema, an oversized file: refused unread
    let base = format!(
        r#"{{"schema":"{PRIVILEGED_SCHEMA}","taken_at":{now},"disks":[],"errors":0,"smartctl":true"#
    );
    write(&d, &(base.clone() + r#","cmd":"rm -rf /"}"#));
    assert_eq!(read_privileged(&d, now).1, "invalid");
    write(
        &d,
        &format!(r#"{{"schema":"other","taken_at":{now},"disks":[]}}"#),
    );
    assert_eq!(read_privileged(&d, now).1, "invalid");
    write(
        &d,
        &(base.clone() + "}" + &" ".repeat(PRIVILEGED_MAX_BYTES as usize)),
    );
    assert_eq!(read_privileged(&d, now).1, "invalid");

    // names and values are checked again on the probe's side
    write(
        &d,
        &format!(
            r#"{{"schema":"{PRIVILEGED_SCHEMA}","taken_at":{now},"disks":[
            {{"dev":"../../etc/passwd","model":null,"rotational":null,"smart_ok":true,"wear_pct":1,"media_errors":0,"power_on_h":1,"temp_c":30,"asleep":null}},
            {{"dev":"sda","model":"<b>x</b>","rotational":true,"smart_ok":true,"wear_pct":9999,"media_errors":0,"power_on_h":1,"temp_c":999,"asleep":null}}],"errors":0,"smartctl":true}}"#
        ),
    );
    let (disks, st) = read_privileged(&d, now);
    assert_eq!(st, "ok");
    assert_eq!(disks.len(), 1, "a path as device name is dropped");
    assert_eq!(
        (disks[0].model.as_ref(), disks[0].wear_pct, disks[0].temp_c),
        (None, None, None)
    );
}

// rivet: verifies REQ-PROBE-012
#[test]
fn hardware_findings_series_and_publication() {
    let mut h = Hardware {
        disks: vec![
            DiskHealth {
                dev: "sda".into(),
                smart_ok: Some(false),
                media_errors: Some(10),
                ..Default::default()
            },
            DiskHealth {
                dev: "nvme0n1".into(),
                wear_pct: Some(83.0),
                temp_c: Some(41.0),
                ..Default::default()
            },
            DiskHealth {
                dev: "nvme1n1".into(),
                wear_pct: Some(96.0),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    h.usb = vec![Usb {
        id: "0483:374e".into(),
        probe: Some("ST-LINK V3".into()),
        ..Default::default()
    }];
    h.media_rising = vec!["sda".into()];
    h.probes_gone = vec![Gone {
        probe: "J-Link (1366:0105)".into(),
        last_seen: 1,
    }];
    h.privileged = Some("stale".into());
    h.errors = vec!["sda: SECRET-ERROR /dev/sda".into()];
    let f: Vec<String> = findings(&h)
        .into_iter()
        .map(|(s, t)| format!("{s}: {t}"))
        .collect();
    let all = f.join("\n");
    for want in [
        "red: disk sda SMART reports failing",
        "amber: disk sda: media errors or bad sectors rising (now 10)",
        "amber: disk nvme0n1 is 83% worn",
        "red: disk nvme1n1 is 96% worn",
        "amber: debug probe J-Link (1366:0105) no longer attached",
        "amber: privileged collector not running: no fresh SMART data",
    ] {
        assert!(all.contains(want), "missing {want:?} in\n{all}");
    }
    h.privileged = Some("missing".into());
    assert!(
        !format!("{:?}", findings(&h)).contains("collector"),
        "not deployed is shown, not alarmed"
    );

    let s = warte::history::project::from_hardware(&h);
    let get = |n: &str| s.iter().find(|(k, _)| k == n).map(|(_, v)| *v);
    assert_eq!(get("disk.wear_pct{dev=nvme0n1}"), Some(83.0));
    assert_eq!(get("disk.media_errors{dev=sda}"), Some(10.0));
    assert_eq!(get("usb.probes"), Some(1.0));

    let p = public(&h).to_string();
    assert!(
        !p.contains("SECRET") && p.contains(r#""not_read":1"#),
        "{p}"
    );
}

// rivet: verifies REQ-PROBE-012
#[test]
fn attached_now_lists_probes_first() {
    let t = tempfile::tempdir().unwrap();
    let h = warte::history::History::open(t.path().to_str().unwrap()).unwrap();
    let hw = Hardware {
        usb: vec![
            Usb {
                id: "1050:0114".into(),
                product: Some("Yubikey".into()),
                ..Default::default()
            },
            Usb {
                id: "0483:374e".into(),
                product: Some("STLINK-V3".into()),
                driver: Some("cdc_acm".into()),
                probe: Some("ST-LINK V3".into()),
                ..Default::default()
            },
        ],
        gpus: vec![Gpu {
            name: "Apple M4".into(),
            cores: Some(10),
            ..Default::default()
        }],
        ..Default::default()
    };
    let html = warte::dashboard::machines::history_panel_full(&h, "box", None, None, Some(&hw));
    let st = html.find("STLINK-V3").expect(&html);
    let yk = html.find("Yubikey").expect(&html);
    assert!(st < yk, "probes first");
    assert!(
        html.contains(r#"<span class="badge green">ST-LINK V3</span>"#),
        "{html}"
    );
    assert!(html.contains("GPU <b>Apple M4</b> · 10 cores"), "{html}");
}

// rivet: verifies REQ-PROBE-012
#[test]
fn the_root_writer_refuses_unsafe_directories_and_says_when_it_read_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let now = 1_800_000_000;
    let p = |disks: Vec<PrivDisk>, errors: u32, smartctl: bool| Privileged {
        schema: PRIVILEGED_SCHEMA.into(),
        taken_at: now,
        disks,
        errors,
        candidates: 2,
        smartctl,
    };
    // writable by others: refused
    let open = t.path().join("open");
    std::fs::create_dir(&open).unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(write_privileged(&open, &p(vec![], 0, true)).is_err());
    // a symlink to a directory: refused
    let real = t.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
    let link = t.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(write_privileged(&link, &p(vec![], 0, true)).is_err());
    // a planted symlink at the final name is replaced, not followed
    let victim = t.path().join("victim");
    std::fs::write(&victim, "keep").unwrap();
    std::os::unix::fs::symlink(&victim, real.join(PRIVILEGED_FILE)).unwrap();
    write_privileged(&real, &p(vec![], 0, true)).unwrap();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
    // read nothing: the probe says failing, not ok
    write_privileged(&real, &p(vec![], 3, true)).unwrap();
    assert_eq!(read_privileged(&real, now), (vec![], "failing"));
    write_privileged(&real, &p(vec![], 1, false)).unwrap();
    assert_eq!(read_privileged(&real, now).1, "failing");
    // a host with no SMART-capable disk (virtio, SD card, USB sticks) is not
    // failing: its unit succeeds and the probe says so
    let none = Privileged {
        candidates: 0,
        ..p(vec![], 0, false)
    };
    assert!(!privileged_failed(&none));
    write_privileged(&real, &none).unwrap();
    assert_eq!(read_privileged(&real, now), (vec![], "none"));
    assert!(privileged_failed(&p(vec![], 3, true)));
    assert!(!smart_capable("vda") && !smart_capable("mmcblk0") && smart_capable("nvme0n1"));
}

// rivet: verifies REQ-PROBE-012
#[test]
fn only_known_collector_states_reach_the_page() {
    let mut m = warte::probe::Machines::open(tempfile::tempdir().unwrap().path().to_str().unwrap());
    let _ = &mut m;
    let mut r: warte::probe::Report = serde_json::from_value(serde_json::json!({
        "probe_version": "t", "host": "box", "taken_at": 0,
        "os": {"name": "Linux", "kernel": "6", "arch": "x86_64"},
        "uptime_s": 1, "cpu": {"ncpu": 1, "load": [0.0, 0.0, 0.0]},
        "memory": {"total_mb": 1, "avail_mb": 1, "swap_total_mb": 0, "swap_used_mb": 0},
        "disks": [], "tools": [], "sessions": []
    }))
    .unwrap();
    r.hardware = Some(Hardware {
        privileged: Some(r#"x"><script>alert(1)</script>"#.into()),
        ..Default::default()
    });
    m.record(r.clone(), "probe", 0);
    let html = warte::dashboard::machines::view(Some(&m), None);
    assert!(!html.contains("<script>alert"), "{html}");
    assert!(!html.contains("collector not running"));
    r.hardware.as_mut().unwrap().privileged = Some("stale".into());
    m.record(r, "probe", 1);
    assert!(
        warte::dashboard::machines::view(Some(&m), None).contains("SMART: collector not running")
    );
}

// rivet: verifies REQ-PROBE-012
#[test]
fn smartctl_is_found_outside_launchds_path_on_macos_only() {
    use warte::probe::hardware::smartctl_path;
    let d = tempfile::tempdir().unwrap();
    let brew = d.path().join("smartctl");
    std::fs::write(&brew, "").unwrap();
    let brew_s = brew.to_str().unwrap();
    let missing = d.path().join("nope");
    let fallbacks = [missing.to_str().unwrap(), brew_s];
    // on PATH wins everywhere
    let on_path = std::path::PathBuf::from("/usr/sbin/smartctl");
    assert_eq!(
        smartctl_path(Some(on_path.clone()), true, &fallbacks),
        on_path
    );
    assert_eq!(
        smartctl_path(Some(on_path.clone()), false, &fallbacks),
        on_path
    );
    // macOS: the first fallback that exists
    assert_eq!(smartctl_path(None, true, &fallbacks), brew);
    // Linux: never a fallback; the bare name, so a missing one reads "not installed"
    assert_eq!(
        smartctl_path(None, false, &fallbacks),
        std::path::PathBuf::from("smartctl")
    );
    assert_eq!(
        smartctl_path(None, true, &fallbacks[..1]),
        std::path::PathBuf::from("smartctl")
    );
}

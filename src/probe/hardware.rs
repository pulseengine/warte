//! Hardware: disk health (SMART), USB devices with debug probes and boards,
//! and GPUs (REQ-PROBE-012).
//!
//! No serial numbers, device paths beyond a device's own name, or user-set
//! names leave the machine; product strings pass the same whitelist as the
//! health block (`health::clean_label`) or are dropped.
//!
//! SMART needs root on Linux. The probe never gets it (owner, 2026-11-06):
//! `warte probe privileged`, a separate root-run oneshot with no network,
//! reads SMART with `smartctl --json -a -n standby` (a sleeping disk is not
//! woken) and writes one JSON file; the unprivileged probe reads that file as
//! untrusted data (size cap, strict schema, names re-checked, values bounded)
//! and reports "privileged collector not running" when it is missing or stale,
//! never old numbers. On macOS smartctl works unprivileged, so the probe runs
//! it itself when it is installed.

use super::health::{clean, clean_label};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// SMART is read at most this often per disk.
pub const SMART_EVERY_S: i64 = 3600;
/// A debug probe seen within this window and gone now is a finding.
pub const PROBE_GONE_WINDOW_S: i64 = 7 * 86_400;
/// Most USB devices reported.
pub const MAX_USB: usize = 40;
/// Where `warte probe privileged` writes and the probe reads.
pub const PRIVILEGED_DIR: &str = "/var/lib/warte-privileged";
/// The collector's output file in `PRIVILEGED_DIR`.
pub const PRIVILEGED_FILE: &str = "smart.json";
/// The collector's output schema.
pub const PRIVILEGED_SCHEMA: &str = "warte.privileged.v1";
/// A collector output larger than this is refused unread.
pub const PRIVILEGED_MAX_BYTES: u64 = 256 * 1024;
/// Older than this, the collector counts as not running (it runs hourly).
pub const PRIVILEGED_STALE_S: i64 = 2 * 3600 + 600;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct DiskHealth {
    /// Device name only: `nvme0n1`, `sda`, `disk0`.
    pub dev: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotational: Option<bool>,
    /// SMART overall assessment: true passed / verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smart_ok: Option<bool>,
    /// Life used, percent (NVMe percentage used; SATA SSD wear attributes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wear_pct: Option<f64>,
    /// NVMe media errors, or SATA reallocated + pending sectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_errors: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_on_h: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp_c: Option<f64>,
    /// When SMART was last read (unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smart_at: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Usb {
    /// `vvvv:pppp`, lower-case hex.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    /// Kernel driver bound to it (Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
    /// Debug probe kind, when it is one (e.g. "ST-LINK").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Gpu {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cores: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub util_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_used_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp_c: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Gone {
    pub probe: String,
    pub last_seen: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Hardware {
    #[serde(default)]
    pub disks: Vec<DiskHealth>,
    #[serde(default)]
    pub usb: Vec<Usb>,
    #[serde(default)]
    pub gpus: Vec<Gpu>,
    /// Debug probes attached in the last 7 days and not now.
    #[serde(default)]
    pub probes_gone: Vec<Gone>,
    /// The privileged collector's state (Linux): `ok`, `missing` (not
    /// deployed or never ran), `stale` (stopped), `invalid` (output refused).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privileged: Option<String>,
    /// What could not be read; published only as a count.
    #[serde(default)]
    pub errors: Vec<String>,
}

/// Carried between runs.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Prev {
    /// dev → the last SMART reading
    #[serde(default)]
    pub smart: BTreeMap<String, DiskHealth>,
    /// probe kind + id → last seen (unix seconds)
    #[serde(default)]
    pub probes_seen: BTreeMap<String, i64>,
}

/// Debug probes and board interfaces by USB vendor:product (product `*` = any).
pub const PROBES: [(&str, &str, &str); 22] = [
    ("1366", "*", "J-Link"),
    ("0483", "3748", "ST-LINK V2"),
    ("0483", "374b", "ST-LINK V2-1"),
    ("0483", "374e", "ST-LINK V3"),
    ("0483", "374f", "ST-LINK V3"),
    ("0483", "3753", "ST-LINK V3"),
    ("0483", "3754", "ST-LINK V3"),
    ("0483", "3757", "ST-LINK V3PWR"),
    ("2e8a", "000c", "Raspberry Pi Debug Probe"),
    ("1d50", "6018", "Black Magic Probe"),
    ("0403", "6010", "FTDI dual (JTAG/UART)"),
    ("0403", "6011", "FTDI quad (JTAG/UART)"),
    ("0403", "6014", "FTDI FT232H (JTAG)"),
    ("1fc9", "0090", "LPC-Link2"),
    ("1fc9", "0143", "MCU-Link"),
    ("0451", "bef3", "TI XDS110"),
    ("303a", "1001", "Espressif USB-JTAG"),
    ("03eb", "2111", "Microchip EDBG"),
    ("03eb", "2141", "Microchip ICE"),
    ("1915", "*", "Nordic"),
    ("c251", "f002", "Keil ULINK"),
    ("15ba", "002a", "Olimex ARM-USB-TINY-H"),
];

/// The debug probe kind for a USB device, from its id or (CMSIS-DAP) its product name.
pub fn probe_kind(vid: &str, pid: &str, product: Option<&str>) -> Option<String> {
    let (vid, pid) = (vid.to_ascii_lowercase(), pid.to_ascii_lowercase());
    PROBES
        .iter()
        .find(|(v, p, _)| *v == vid && (*p == "*" || *p == pid))
        .map(|(_, _, k)| k.to_string())
        .or_else(|| {
            product
                .filter(|p| p.to_ascii_uppercase().contains("CMSIS-DAP"))
                .map(|_| "CMSIS-DAP".to_string())
        })
}

fn hex4(s: &str) -> Option<String> {
    let s = s.trim().trim_start_matches("0x").to_ascii_lowercase();
    (s.len() <= 4 && !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("{s:0>4}"))
}

/// A USB device from its raw fields; hubs and root hubs are left out.
pub fn usb_device(
    vid: &str,
    pid: &str,
    product: Option<&str>,
    vendor: Option<&str>,
    driver: Option<&str>,
) -> Option<Usb> {
    let (v, p) = (hex4(vid)?, hex4(pid)?);
    let product = product.and_then(clean_label);
    if product
        .as_deref()
        .is_some_and(|p| p.to_ascii_lowercase().contains("hub"))
        || driver == Some("hub")
    {
        return None;
    }
    Some(Usb {
        probe: probe_kind(&v, &p, product.as_deref()),
        id: format!("{v}:{p}"),
        product,
        vendor: vendor.and_then(clean_label),
        driver: driver.and_then(clean),
    })
}

/// USB devices from `ioreg -p IOUSB -l -w0` (macOS): one per `+-o` entry.
pub fn parse_ioreg_usb(text: &str) -> Vec<Usb> {
    let mut out = Vec::new();
    let mut cur: BTreeMap<&str, String> = BTreeMap::new();
    let flush = |cur: &mut BTreeMap<&str, String>, out: &mut Vec<Usb>| {
        if let (Some(v), Some(p)) = (cur.get("idVendor"), cur.get("idProduct")) {
            let (v, p) = (
                v.parse::<u32>()
                    .map(|x| format!("{x:04x}"))
                    .unwrap_or_default(),
                p.parse::<u32>()
                    .map(|x| format!("{x:04x}"))
                    .unwrap_or_default(),
            );
            if let Some(u) = usb_device(
                &v,
                &p,
                cur.get("USB Product Name").map(String::as_str),
                cur.get("USB Vendor Name").map(String::as_str),
                None,
            ) {
                out.push(u);
            }
        }
        cur.clear();
    };
    for line in text.lines() {
        if line.contains("+-o ") {
            flush(&mut cur, &mut out);
            continue;
        }
        let l = line.trim_start_matches([' ', '|']).trim();
        for k in [
            "idVendor",
            "idProduct",
            "USB Product Name",
            "USB Vendor Name",
        ] {
            if let Some(v) = l.strip_prefix(&format!("\"{k}\" = ")) {
                cur.insert(k, v.trim().trim_matches('"').to_string());
            }
        }
    }
    flush(&mut cur, &mut out);
    out.truncate(MAX_USB);
    out
}

/// A disk's health from `smartctl --json -a` output.
pub fn parse_smartctl(dev: &str, json: &str, now: i64) -> Option<DiskHealth> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let u = |p: &str| v.pointer(p).and_then(serde_json::Value::as_u64);
    let nvme = v.pointer("/nvme_smart_health_information_log");
    let attr = |id: u64| {
        v.pointer("/ata_smart_attributes/table")?
            .as_array()?
            .iter()
            .find(|a| a.get("id").and_then(serde_json::Value::as_u64) == Some(id))
            .cloned()
    };
    let raw = |id: u64| attr(id).and_then(|a| a.pointer("/raw/value")?.as_u64());
    // SATA SSD wear: 231 SSD life left, 233 media wearout, 177 wear levelling (normalised, 100 = new)
    let ata_wear = [231u64, 233, 177]
        .iter()
        .find_map(|id| attr(*id).and_then(|a| a.get("value")?.as_u64()))
        .filter(|x| *x <= 100)
        .map(|x| 100.0 - x as f64);
    let wear_pct = nvme
        .and_then(|n| n.get("percentage_used")?.as_u64())
        .map(|x| x as f64)
        .or(ata_wear);
    let media_errors = nvme
        .and_then(|n| n.get("media_errors")?.as_u64())
        .or_else(|| match (raw(5), raw(197)) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        });
    let rotational = u("/rotation_rate").map(|r| r > 0);
    Some(DiskHealth {
        dev: clean(dev)?,
        model: v
            .get("model_name")
            .and_then(serde_json::Value::as_str)
            .and_then(clean_label),
        rotational,
        smart_ok: v
            .pointer("/smart_status/passed")
            .and_then(serde_json::Value::as_bool),
        wear_pct,
        media_errors,
        power_on_h: u("/power_on_time/hours"),
        temp_c: u("/temperature/current").map(|t| t as f64),
        smart_at: Some(now),
    })
}

/// `SMART Status:` from `diskutil info` (macOS): Verified → true, Failing → false.
pub fn parse_diskutil_smart(text: &str) -> Option<bool> {
    let l = text
        .lines()
        .find(|l| l.trim_start().starts_with("SMART Status:"))?;
    let v = l.split(':').nth(1)?.trim();
    match v {
        "Verified" => Some(true),
        "Failing" => Some(false),
        _ => None,
    }
}

/// `Device / Media Name:` from `diskutil info` (macOS), e.g. "APPLE SSD AP0256Z".
pub fn parse_diskutil_model(text: &str) -> Option<String> {
    let l = text
        .lines()
        .find(|l| l.trim_start().starts_with("Device / Media Name:"))?;
    clean_label(l.split_once(':')?.1)
}

/// GPUs from `system_profiler SPDisplaysDataType -json` (macOS).
pub fn parse_sp_displays(json: &str) -> Vec<Gpu> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    v.get("SPDisplaysDataType")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|g| {
                    Some(Gpu {
                        name: g.get("sppci_model")?.as_str().and_then(clean_label)?,
                        cores: g.get("sppci_cores").and_then(|c| c.as_str()?.parse().ok()),
                        ..Default::default()
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// GPUs from `nvidia-smi --query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu --format=csv,noheader,nounits`.
pub fn parse_nvidia_smi(csv: &str) -> Vec<Gpu> {
    csv.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').map(str::trim).collect();
            if f.len() < 5 {
                return None;
            }
            let n = |i: usize| f[i].parse::<f64>().ok();
            Some(Gpu {
                name: clean_label(f[0])?,
                driver: Some("nvidia".into()),
                util_pct: n(1),
                mem_used_pct: match (n(2), n(3)) {
                    (Some(u), Some(t)) if t > 0.0 => Some(100.0 * u / t),
                    _ => None,
                },
                temp_c: n(4),
                cores: None,
            })
        })
        .collect()
}

/// Which debug probes are gone, and the updated last-seen map.
pub fn probes_gone(
    seen_before: &BTreeMap<String, i64>,
    now_attached: &[String],
    now: i64,
) -> (Vec<Gone>, BTreeMap<String, i64>) {
    let mut seen: BTreeMap<String, i64> = seen_before
        .iter()
        .filter(|(_, t)| now - **t <= PROBE_GONE_WINDOW_S)
        .map(|(k, t)| (k.clone(), *t))
        .collect();
    for p in now_attached {
        seen.insert(p.clone(), now);
    }
    let gone = seen
        .iter()
        .filter(|(k, _)| !now_attached.contains(k))
        .map(|(k, t)| Gone {
            probe: k.clone(),
            last_seen: *t,
        })
        .collect();
    (gone, seen)
}

/// What `warte probe privileged` writes: SMART per disk, already reduced to
/// `DiskHealth` (no raw smartctl output, no serial numbers).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Privileged {
    pub schema: String,
    pub taken_at: i64,
    pub disks: Vec<PrivDisk>,
}

/// One disk in the collector's output; strict, so a changed or tampered
/// file is refused rather than half-read.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PrivDisk {
    pub dev: String,
    pub model: Option<String>,
    pub rotational: Option<bool>,
    pub smart_ok: Option<bool>,
    pub wear_pct: Option<f64>,
    pub media_errors: Option<u64>,
    pub power_on_h: Option<u64>,
    pub temp_c: Option<f64>,
}

/// The root-only collection (`warte probe privileged`): SMART for every whole
/// disk. No network I/O; the caller writes the result with `write_privileged`.
pub async fn privileged_collect(now: i64) -> (Privileged, Vec<String>) {
    let mut errors = Vec::new();
    let mut disks = Vec::new();
    let devs: Vec<String> = if cfg!(target_os = "linux") {
        linux_disks().into_iter().map(|(d, _, _)| d).collect()
    } else {
        vec!["disk0".to_string()]
    };
    for dev in devs {
        let path = if cfg!(target_os = "linux") {
            format!("/dev/{dev}")
        } else {
            dev.clone()
        };
        match smartctl(&path).await {
            Ok(Some(j)) => {
                if let Some(d) = parse_smartctl(&dev, &j, now) {
                    disks.push(PrivDisk {
                        dev: d.dev,
                        model: d.model,
                        rotational: d.rotational,
                        smart_ok: d.smart_ok,
                        wear_pct: d.wear_pct,
                        media_errors: d.media_errors,
                        power_on_h: d.power_on_h,
                        temp_c: d.temp_c,
                    });
                }
            }
            Ok(None) => errors.push("smartctl is not installed".into()),
            Err(e) => errors.push(format!("{dev}: {e}")),
        }
    }
    (
        Privileged {
            schema: PRIVILEGED_SCHEMA.into(),
            taken_at: now,
            disks,
        },
        errors,
    )
}

/// Write the collector's output atomically, world-readable (it holds no secret).
pub fn write_privileged(dir: &std::path::Path, p: &Privileged) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
    let tmp = dir.join(format!("{PRIVILEGED_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(p).map_err(std::io::Error::other)?)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
    std::fs::rename(&tmp, dir.join(PRIVILEGED_FILE))
}

/// Read the collector's output as untrusted data: at most
/// `PRIVILEGED_MAX_BYTES`, the exact schema, names through the whitelist,
/// values in range. Returns the disks and a status: `ok`, `missing`,
/// `stale` (older than `PRIVILEGED_STALE_S`, or from the future) or
/// `invalid`. Only `ok` returns disks: no stale numbers.
pub fn read_privileged(dir: &std::path::Path, now: i64) -> (Vec<DiskHealth>, &'static str) {
    let path = dir.join(PRIVILEGED_FILE);
    let Ok(meta) = std::fs::metadata(&path) else {
        return (Vec::new(), "missing");
    };
    if !meta.is_file() || meta.len() > PRIVILEGED_MAX_BYTES {
        return (Vec::new(), "invalid");
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return (Vec::new(), "missing");
    };
    let Ok(p) = serde_json::from_slice::<Privileged>(&bytes) else {
        return (Vec::new(), "invalid");
    };
    if p.schema != PRIVILEGED_SCHEMA || p.disks.len() > 64 {
        return (Vec::new(), "invalid");
    }
    if now - p.taken_at > PRIVILEGED_STALE_S || p.taken_at - now > 300 {
        return (Vec::new(), "stale");
    }
    let in_range =
        |x: Option<f64>, lo: f64, hi: f64| x.filter(|v| v.is_finite() && *v >= lo && *v <= hi);
    let disks = p
        .disks
        .into_iter()
        .filter_map(|d| {
            Some(DiskHealth {
                dev: clean(&d.dev)?,
                model: d.model.as_deref().and_then(clean_label),
                rotational: d.rotational,
                smart_ok: d.smart_ok,
                wear_pct: in_range(d.wear_pct, 0.0, 255.0),
                media_errors: d.media_errors,
                power_on_h: d.power_on_h.filter(|h| *h < 1_000_000),
                temp_c: in_range(d.temp_c, -40.0, 200.0),
                smart_at: Some(p.taken_at),
            })
        })
        .collect();
    (disks, "ok")
}

async fn run(cmd: &str, args: &[&str], secs: u64) -> Result<String, String> {
    super::inventory::run_for(cmd, args, Duration::from_secs(secs)).await
}

/// smartctl exits non-zero on SMART warnings too (bit flags); JSON on stdout
/// is what counts. Ok(None): not run (no permission, standby, not installed).
async fn smartctl(dev_path: &str) -> Result<Option<String>, String> {
    let out = tokio::process::Command::new("smartctl")
        .args(["--json", "-a", "-n", "standby", dev_path])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(Duration::from_secs(15), out).await {
        Err(_) => Err("smartctl timed out".into()),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Ok(Err(e)) => Err(format!("smartctl: {e}")),
        Ok(Ok(o)) => {
            let code = o.status.code().unwrap_or(-1);
            // bit 1: the device could not be opened (no permission, or asleep with -n standby)
            if code & 0b10 != 0 {
                return Err("smartctl could not open the device".into());
            }
            Ok(Some(String::from_utf8_lossy(&o.stdout).into_owned()))
        }
    }
}

/// Linux whole disks worth reading (no loop, ram, zram, device-mapper, partitions).
fn linux_disks() -> Vec<(String, Option<bool>, Option<String>)> {
    let Ok(rd) = std::fs::read_dir("/sys/block") else {
        return Vec::new();
    };
    let mut v: Vec<_> = rd
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let keep = n.starts_with("sd")
                || n.starts_with("nvme")
                || n.starts_with("mmcblk")
                || n.starts_with("vd");
            if !keep || n.contains("boot") || n.contains("rpmb") {
                return None;
            }
            let p = e.path();
            let rot = std::fs::read_to_string(p.join("queue/rotational"))
                .ok()
                .map(|s| s.trim() == "1");
            let model = std::fs::read_to_string(p.join("device/model"))
                .ok()
                .and_then(|m| clean_label(&m));
            Some((n, rot, model))
        })
        .collect();
    v.sort();
    v
}

fn linux_usb() -> Vec<Usb> {
    let Ok(rd) = std::fs::read_dir("/sys/bus/usb/devices") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let read = |f: &str| {
            std::fs::read_to_string(p.join(f))
                .ok()
                .map(|s| s.trim().to_string())
        };
        let (Some(vid), Some(pid)) = (read("idVendor"), read("idProduct")) else {
            continue;
        };
        // the driver of the device's first interface (e.g. 1-1.2:1.0 → ftdi_sio)
        let name = e.file_name().to_string_lossy().into_owned();
        let driver = std::fs::read_link(p.join(format!("{name}:1.0/driver")))
            .ok()
            .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()));
        if read("bDeviceClass").as_deref() == Some("09") {
            continue; // hub
        }
        if let Some(u) = usb_device(
            &vid,
            &pid,
            read("product").as_deref(),
            read("manufacturer").as_deref(),
            driver.as_deref(),
        ) {
            out.push(u);
        }
    }
    out.truncate(MAX_USB);
    out
}

fn linux_gpus() -> Vec<Gpu> {
    let Ok(rd) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if !n.starts_with("card") || n.contains('-') {
            continue;
        }
        let dev = e.path().join("device");
        let read = |f: &str| {
            std::fs::read_to_string(dev.join(f))
                .ok()
                .map(|s| s.trim().to_string())
        };
        let driver = std::fs::read_link(dev.join("driver"))
            .ok()
            .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()));
        let name = match (read("vendor"), read("device")) {
            (Some(v), Some(d)) => format!(
                "{}:{}",
                v.trim_start_matches("0x"),
                d.trim_start_matches("0x")
            ),
            _ => driver.clone().unwrap_or_else(|| n.clone()),
        };
        if let Some(name) = clean(&name) {
            out.push(Gpu {
                name,
                driver: driver.and_then(|d| clean(&d)),
                ..Default::default()
            });
        }
    }
    out
}

fn privileged_dir_or_default(p: Option<&std::path::Path>) -> &std::path::Path {
    p.unwrap_or(std::path::Path::new(PRIVILEGED_DIR))
}

/// Collect once. `state`: where macOS SMART readings and probe sightings
/// persist; `privileged`: the collector's directory (default `PRIVILEGED_DIR`).
pub async fn collect(
    state: Option<&std::path::Path>,
    privileged: Option<&std::path::Path>,
    now: i64,
) -> Hardware {
    let prev: Prev = state
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut hw = Hardware::default();
    let mut next = Prev::default();
    let linux = cfg!(target_os = "linux");
    let macos = cfg!(target_os = "macos");

    // disks: Linux from the privileged collector's file; macOS runs smartctl
    // itself (unprivileged there), at most hourly
    if linux {
        let (disks, status) = read_privileged(privileged_dir_or_default(privileged), now);
        let mut by: BTreeMap<String, DiskHealth> =
            disks.into_iter().map(|d| (d.dev.clone(), d)).collect();
        for (dev, rot, model) in linux_disks() {
            let mut d = by.remove(&dev).unwrap_or(DiskHealth {
                dev: dev.clone(),
                ..Default::default()
            });
            d.rotational = d.rotational.or(rot);
            d.model = d.model.or(model);
            hw.disks.push(d);
        }
        hw.privileged = Some(status.to_string());
    } else if macos {
        let dev = "disk0".to_string();
        let cached = prev.smart.get(&dev).cloned();
        let fresh = cached
            .as_ref()
            .and_then(|c| c.smart_at)
            .is_some_and(|t| now - t < SMART_EVERY_S);
        let mut d = match (fresh || state.is_none(), cached) {
            (true, Some(c)) => c,
            (_, cached) => match smartctl(&dev).await {
                Ok(Some(j)) => parse_smartctl(&dev, &j, now).unwrap_or_default(),
                Ok(None) => DiskHealth::default(),
                Err(e) => {
                    hw.errors.push(e);
                    cached.unwrap_or_default()
                }
            },
        };
        d.dev = dev.clone();
        d.smart_at.get_or_insert(now);
        if d.smart_ok.is_none() || d.model.is_none() {
            if let Ok(t) = run("diskutil", &["info", &dev], 5).await {
                d.smart_ok = d.smart_ok.or(parse_diskutil_smart(&t));
                d.model = d.model.or(parse_diskutil_model(&t));
            }
        }
        next.smart.insert(dev, d.clone());
        hw.disks.push(d);
    }

    // USB devices and debug probes
    hw.usb = if linux {
        linux_usb()
    } else if macos {
        match run("ioreg", &["-p", "IOUSB", "-l", "-w0"], 5).await {
            Ok(t) => parse_ioreg_usb(&t),
            Err(e) => {
                hw.errors.push(e);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let attached: Vec<String> = hw
        .usb
        .iter()
        .filter_map(|u| u.probe.as_ref().map(|p| format!("{p} ({})", u.id)))
        .collect();
    let (gone, seen) = probes_gone(&prev.probes_seen, &attached, now);
    hw.probes_gone = gone;
    next.probes_seen = seen;

    // GPUs
    if macos {
        if let Ok(j) = run("system_profiler", &["SPDisplaysDataType", "-json"], 10).await {
            hw.gpus = parse_sp_displays(&j);
        }
    } else if linux {
        hw.gpus = linux_gpus();
        if let Ok(csv) = run(
            "nvidia-smi",
            &[
                "--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu",
                "--format=csv,noheader,nounits",
            ],
            5,
        )
        .await
        {
            let nv = parse_nvidia_smi(&csv);
            if !nv.is_empty() {
                hw.gpus.retain(|g| g.driver.as_deref() != Some("nvidia"));
                hw.gpus.extend(nv);
            }
        }
    }

    if let Some(p) = state {
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec(&next).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, p);
        }
    }
    hw
}

/// What the read API publishes: everything but the error text (a count instead).
pub fn public(h: &Hardware) -> serde_json::Value {
    let mut v = serde_json::to_value(h).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.remove("errors");
        o.insert("not_read".into(), h.errors.len().into());
    }
    v
}

/// Findings from one machine's hardware: (severity, text).
pub fn findings(h: &Hardware) -> Vec<(&'static str, String)> {
    let mut f = Vec::new();
    for d in &h.disks {
        if d.smart_ok == Some(false) {
            f.push(("red", format!("disk {} SMART reports failing", d.dev)));
        }
        if let Some(w) = d.wear_pct {
            if w >= 95.0 {
                f.push(("red", format!("disk {} is {w:.0}% worn", d.dev)));
            } else if w >= 80.0 {
                f.push(("amber", format!("disk {} is {w:.0}% worn", d.dev)));
            }
        }
        if d.media_errors.is_some_and(|e| e > 0) {
            f.push((
                "amber",
                format!(
                    "disk {} has {} media error(s) or bad sector(s)",
                    d.dev,
                    d.media_errors.unwrap_or(0)
                ),
            ));
        }
    }
    match h.privileged.as_deref() {
        Some("stale") => f.push((
            "amber",
            "privileged collector not running: no fresh SMART data".to_string(),
        )),
        Some("invalid") => f.push((
            "amber",
            "privileged collector output refused (schema or size)".to_string(),
        )),
        _ => {}
    }
    for g in &h.probes_gone {
        f.push((
            "amber",
            format!("debug probe {} no longer attached", g.probe),
        ));
    }
    f
}

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
/// A probe counts as gone only after it was attached at least this long
/// (a probe plugged in for one flash is not a fixture).
pub const PROBE_MIN_ATTACHED_S: i64 = 30 * 60;
/// Media errors that rose within this window are a finding.
pub const MEDIA_RISE_WINDOW_S: i64 = 86_400;
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
    /// The disk was asleep, so SMART was not read (it is not woken).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asleep: Option<bool>,
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
    /// Board or adapter kind (a development board's own USB, a serial adapter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
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
    /// Disks whose media errors rose in the last 24 h.
    #[serde(default)]
    pub media_rising: Vec<String>,
    /// The privileged collector's state (Linux): `ok`, `missing` (not
    /// deployed or never ran), `stale` (stopped), `invalid` (output
    /// refused), `failing` (it ran but read no disk).
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
    /// probe kind → (first seen, last seen) of its current or last attachment
    #[serde(default)]
    pub probes: BTreeMap<String, (i64, i64)>,
    /// dev → (media errors, when they last rose)
    #[serde(default)]
    pub media: MediaCounts,
    /// GPUs from slow sources (system_profiler), and when they were read
    #[serde(default)]
    pub gpus: Option<(i64, Vec<Gpu>)>,
}

/// Debug probes by USB vendor:product (product `*` = any). These are fixtures:
/// one that disappears is a finding.
pub const PROBES: [(&str, &str, &str); 17] = [
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
    ("1fc9", "0090", "LPC-Link2"),
    ("1fc9", "0143", "MCU-Link"),
    ("0451", "bef3", "TI XDS110"),
    ("03eb", "2111", "Microchip EDBG"),
    ("03eb", "2141", "Microchip ICE"),
    ("c251", "f002", "Keil ULINK"),
    ("15ba", "002a", "Olimex ARM-USB-TINY-H"),
];

/// Boards and adapters: the board under development's own USB, or a generic
/// serial/JTAG adapter. Shown, never alarmed about (they come and go with
/// every flash).
pub const BOARDS: [(&str, &str, &str); 6] = [
    ("1915", "*", "Nordic board"),
    ("303a", "*", "Espressif board"),
    ("0403", "6010", "FTDI dual adapter"),
    ("0403", "6011", "FTDI quad adapter"),
    ("0403", "6014", "FTDI FT232H adapter"),
    ("2e8a", "0003", "RP2040/RP2350 board"),
];

fn table_kind(t: &[(&str, &str, &str)], vid: &str, pid: &str) -> Option<String> {
    t.iter()
        .find(|(v, p, _)| *v == vid && (*p == "*" || *p == pid))
        .map(|(_, _, k)| k.to_string())
}

/// The debug probe kind for a USB device, from its id or (CMSIS-DAP) its product name.
pub fn probe_kind(vid: &str, pid: &str, product: Option<&str>) -> Option<String> {
    let (vid, pid) = (vid.to_ascii_lowercase(), pid.to_ascii_lowercase());
    table_kind(&PROBES, &vid, &pid).or_else(|| {
        product
            .filter(|p| p.to_ascii_uppercase().contains("CMSIS-DAP"))
            .map(|_| "CMSIS-DAP".to_string())
    })
}

/// The board or adapter kind for a USB device.
pub fn board_kind(vid: &str, pid: &str) -> Option<String> {
    table_kind(
        &BOARDS,
        &vid.to_ascii_lowercase(),
        &pid.to_ascii_lowercase(),
    )
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
    let probe = probe_kind(&v, &p, product.as_deref());
    let board = if probe.is_none() {
        board_kind(&v, &p)
    } else {
        None
    };
    // firmware sets product and vendor strings (a board's own name, a
    // project): published only for known probes and boards, else the id alone
    let known = probe.is_some() || board.is_some();
    Some(Usb {
        id: format!("{v}:{p}"),
        product: product.filter(|_| known),
        vendor: vendor.and_then(clean_label).filter(|_| known),
        driver: driver.and_then(clean),
        probe,
        board,
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
        .find_map(|id| {
            attr(*id)
                .and_then(|a| a.get("value")?.as_u64())
                .filter(|x| *x <= 100)
        })
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
        asleep: None,
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

/// Which debug probe kinds are gone, and the updated attachment map. Keyed by
/// kind, so reflashing a probe (its product id changes) is not "gone"; a kind
/// counts only after `PROBE_MIN_ATTACHED_S` attached, and is reported until
/// `PROBE_GONE_WINDOW_S` after it was last seen.
pub fn probes_gone(
    before: &BTreeMap<String, (i64, i64)>,
    attached_kinds: &[String],
    now: i64,
) -> (Vec<Gone>, BTreeMap<String, (i64, i64)>) {
    let mut next: BTreeMap<String, (i64, i64)> = before
        .iter()
        .filter(|(_, (_, last))| now - last <= PROBE_GONE_WINDOW_S)
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    for k in attached_kinds {
        let e = next.entry(k.clone()).or_insert((now, now));
        // re-attached after a gap: a new attachment starts
        if now - e.1 > 10 * 60 {
            e.0 = now;
        }
        e.1 = now;
    }
    let gone = next
        .iter()
        .filter(|(k, (first, last))| {
            !attached_kinds.contains(k) && last - first >= PROBE_MIN_ATTACHED_S
        })
        .map(|(k, (_, last))| Gone {
            probe: k.clone(),
            last_seen: *last,
        })
        .collect();
    (gone, next)
}

/// dev → (media errors, when they last rose).
pub type MediaCounts = BTreeMap<String, (u64, Option<i64>)>;

/// Disks whose media errors rose in the last day, and the updated counters.
/// A first reading is a baseline, never a finding: these counters never fall.
pub fn media_rising(
    before: &MediaCounts,
    disks: &[DiskHealth],
    now: i64,
) -> (Vec<String>, MediaCounts) {
    let mut next = BTreeMap::new();
    let mut rising = Vec::new();
    for d in disks {
        let Some(n) = d.media_errors else { continue };
        let rose_at = match before.get(&d.dev) {
            Some((old, at)) if n > *old => Some(now),
            Some((_, at)) => *at,
            None => None,
        };
        if rose_at.is_some_and(|t| now - t <= MEDIA_RISE_WINDOW_S) {
            rising.push(d.dev.clone());
        }
        next.insert(d.dev.clone(), (n, rose_at));
    }
    (rising, next)
}

/// What `warte probe privileged` writes: SMART per disk, already reduced to
/// `DiskHealth` (no raw smartctl output, no serial numbers).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Privileged {
    pub schema: String,
    pub taken_at: i64,
    pub disks: Vec<PrivDisk>,
    /// Disks the collector could not read (the reasons stay in its own log).
    pub errors: u32,
    /// Whether smartctl was found.
    pub smartctl: bool,
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
    pub asleep: Option<bool>,
}

/// The root-only collection (`warte probe privileged`): SMART for every whole
/// disk. No network I/O; the caller writes the result with `write_privileged`.
pub async fn privileged_collect(now: i64) -> (Privileged, Vec<String>) {
    let mut errors = Vec::new();
    let mut disks = Vec::new();
    let mut found = true;
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
        let reduce = |d: DiskHealth| PrivDisk {
            dev: d.dev,
            model: d.model,
            rotational: d.rotational,
            smart_ok: d.smart_ok,
            wear_pct: d.wear_pct,
            media_errors: d.media_errors,
            power_on_h: d.power_on_h,
            temp_c: d.temp_c,
            asleep: d.asleep,
        };
        match smartctl(&path).await {
            Ok(Smart::Json(j)) => match parse_smartctl(&dev, &j, now) {
                Some(d) => disks.push(reduce(d)),
                None => errors.push(format!("{dev}: unreadable smartctl output")),
            },
            Ok(Smart::Asleep) => disks.push(reduce(DiskHealth {
                dev: dev.clone(),
                asleep: Some(true),
                ..Default::default()
            })),
            Ok(Smart::NotInstalled) => {
                found = false;
                errors.push("smartctl is not installed".into());
                break;
            }
            Err(e) => errors.push(format!("{dev}: {e}")),
        }
    }
    (
        Privileged {
            schema: PRIVILEGED_SCHEMA.into(),
            taken_at: now,
            disks,
            errors: errors.len() as u32,
            smartctl: found,
        },
        errors,
    )
}

/// Write the collector's output atomically, world-readable (it holds no
/// secret). Runs as root, so the directory must be safe: a real directory
/// (not a symlink), owned by the writing user, writable by nobody else. The
/// temporary file is created exclusively under a random name (`create_new`
/// never follows a symlink), its mode set on the open handle, then renamed.
pub fn write_privileged(dir: &std::path::Path, p: &Privileged) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    if std::fs::symlink_metadata(dir).is_err() {
        std::fs::DirBuilder::new().mode(0o755).create(dir)?;
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.file_type().is_dir() {
        return Err(std::io::Error::other(format!(
            "{} is not a directory",
            dir.display()
        )));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(std::io::Error::other(format!(
            "{} is writable by others; refusing to write there",
            dir.display()
        )));
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{PRIVILEGED_FILE}.{}.{nonce}", std::process::id()));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&tmp)?;
    // the directory belongs to whoever writes here (root, from the unit)
    if f.metadata()?.uid() != meta.uid() {
        let _ = std::fs::remove_file(&tmp);
        return Err(std::io::Error::other(format!(
            "{} is owned by another user; refusing to write there",
            dir.display()
        )));
    }
    f.set_permissions(std::fs::Permissions::from_mode(0o644))?;
    f.write_all(&serde_json::to_vec(p).map_err(std::io::Error::other)?)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, dir.join(PRIVILEGED_FILE))
}

/// Read the collector's output as untrusted data: at most
/// `PRIVILEGED_MAX_BYTES`, the exact schema, names through the whitelist,
/// values in range. Returns the disks and a status: `ok`, `missing`,
/// `stale` (older than `PRIVILEGED_STALE_S`, or from the future) or
/// `invalid`. Only `ok` returns disks: no stale numbers.
pub fn read_privileged(dir: &std::path::Path, now: i64) -> (Vec<DiskHealth>, &'static str) {
    use std::io::Read;
    let path = dir.join(PRIVILEGED_FILE);
    // one open, the checks on the handle, a bounded read: no swap between
    // a check and the read, no FIFO, no unbounded file
    let Ok(f) = std::fs::File::open(&path) else {
        return (Vec::new(), "missing");
    };
    match f.metadata() {
        Ok(m) if m.is_file() && m.len() <= PRIVILEGED_MAX_BYTES => {}
        _ => return (Vec::new(), "invalid"),
    }
    let mut bytes = Vec::new();
    if f.take(PRIVILEGED_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > PRIVILEGED_MAX_BYTES
    {
        return (Vec::new(), "invalid");
    }
    let Ok(p) = serde_json::from_slice::<Privileged>(&bytes) else {
        return (Vec::new(), "invalid");
    };
    if p.schema != PRIVILEGED_SCHEMA || p.disks.len() > 64 {
        return (Vec::new(), "invalid");
    }
    if now - p.taken_at > PRIVILEGED_STALE_S || p.taken_at - now > 300 {
        return (Vec::new(), "stale");
    }
    if !p.smartctl || (p.disks.is_empty() && p.errors > 0) {
        return (Vec::new(), "failing");
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
                asleep: d.asleep,
            })
        })
        .collect();
    (disks, "ok")
}

async fn run(cmd: &str, args: &[&str], secs: u64) -> Result<String, String> {
    super::inventory::run_for(cmd, args, Duration::from_secs(secs)).await
}

/// What one smartctl run gave.
enum Smart {
    Json(String),
    /// `-n standby,7`: the disk sleeps and was not woken.
    Asleep,
    NotInstalled,
}

/// smartctl exits non-zero on SMART warnings too (bit flags); JSON on stdout
/// is what counts. A sleeping disk is left asleep and exits 7.
async fn smartctl(dev_path: &str) -> Result<Smart, String> {
    let out = tokio::process::Command::new("smartctl")
        .args(["--json", "-a", "-n", "standby,7", dev_path])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(Duration::from_secs(15), out).await {
        Err(_) => Err("smartctl timed out".into()),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(Smart::NotInstalled),
        Ok(Err(e)) => Err(format!("smartctl: {e}")),
        Ok(Ok(o)) => {
            let code = o.status.code().unwrap_or(-1);
            if code == 7 {
                return Ok(Smart::Asleep);
            }
            // bit 1: the device could not be opened
            if code >= 0 && code & 0b10 != 0 {
                return Err("smartctl could not open the device".into());
            }
            Ok(Smart::Json(String::from_utf8_lossy(&o.stdout).into_owned()))
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
            let attr = |f: &str| {
                std::fs::read_to_string(p.join(f))
                    .ok()
                    .map(|s| s.trim().to_string())
            };
            // hidden NVMe multipath nodes and empty card readers
            if attr("hidden").as_deref() == Some("1") || attr("size").as_deref() == Some("0") {
                return None;
            }
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

/// The NVIDIA driver is loaded and none of its devices is runtime-suspended.
fn nvidia_awake() -> bool {
    if !std::path::Path::new("/proc/driver/nvidia").exists() {
        return false;
    }
    let Ok(rd) = std::fs::read_dir("/sys/bus/pci/drivers/nvidia") else {
        return false;
    };
    !rd.flatten().any(|e| {
        std::fs::read_to_string(e.path().join("power/runtime_status"))
            .is_ok_and(|s| s.trim() == "suspended")
    })
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
        // a failed read is retried after the hour too, and never carries
        // the old numbers forward
        let mut d = match (fresh || state.is_none(), cached) {
            (true, Some(c)) => c,
            _ => match smartctl(&dev).await {
                Ok(Smart::Json(j)) => parse_smartctl(&dev, &j, now).unwrap_or_default(),
                Ok(Smart::Asleep) => DiskHealth {
                    asleep: Some(true),
                    ..Default::default()
                },
                Ok(Smart::NotInstalled) => DiskHealth::default(),
                Err(e) => {
                    hw.errors.push(e);
                    DiskHealth::default()
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
    let mut kinds: Vec<String> = hw.usb.iter().filter_map(|u| u.probe.clone()).collect();
    kinds.sort();
    kinds.dedup();
    let (gone, probes) = probes_gone(&prev.probes, &kinds, now);
    hw.probes_gone = gone;
    next.probes = probes;
    let (rising, media) = media_rising(&prev.media, &hw.disks, now);
    hw.media_rising = rising;
    next.media = media;

    // GPUs; system_profiler is slow and its answer does not change: hourly
    if macos {
        match prev.gpus.clone().filter(|(t, _)| now - t < SMART_EVERY_S) {
            Some((t, g)) => {
                hw.gpus = g.clone();
                next.gpus = Some((t, g));
            }
            None => {
                if let Ok(j) = run("system_profiler", &["SPDisplaysDataType", "-json"], 10).await {
                    hw.gpus = parse_sp_displays(&j);
                    next.gpus = Some((now, hw.gpus.clone()));
                }
            }
        }
    } else if linux {
        hw.gpus = linux_gpus();
        // nvidia-smi would wake a sleeping laptop GPU: only when the driver is
        // loaded and no NVIDIA device is runtime-suspended
        if nvidia_awake() {
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
        // these counters never fall: a rise is news, a constant count is not
        if h.media_rising.contains(&d.dev) {
            f.push((
                "amber",
                format!(
                    "disk {}: media errors or bad sectors rising (now {})",
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
        Some("failing") => f.push((
            "amber",
            "privileged collector reads no disk (smartctl missing or failing)".to_string(),
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

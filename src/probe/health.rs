//! System health, network, temperatures, battery and top processes
//! (REQ-PROBE-011). Everything here becomes a series in machine history
//! (REQ-MHIST-001) except the process list, which is a current snapshot.
//!
//! What leaves the machine: numbers, interface and sensor names, failed unit
//! names and process names, each through a strict character whitelist
//! (`clean`). Never a command line, an argument, an environment value, a path
//! or a user. No root is needed: macOS temperatures come through IOKit as an
//! ordinary user (sysinfo's components), updates from `softwareupdate --list`.
//!
//! Counters (network bytes, OOM kills) and slow checks (updates) keep their
//! previous values in a state file next to the IO state.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// Linux update check cadence (apt simulation, a few seconds).
pub const UPDATES_EVERY_LINUX_S: i64 = 6 * 3600;
/// macOS update check cadence (`softwareupdate --list` asks Apple, tens of seconds).
pub const UPDATES_EVERY_MACOS_S: i64 = 24 * 3600;
/// Most sensors (after grouping) and interfaces reported.
pub const MAX_ITEMS: usize = 16;
/// Processes per top list.
pub const TOP: usize = 5;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Iface {
    pub name: String,
    pub rx_kbs: f64,
    pub tx_kbs: f64,
    /// Receive and transmit errors per minute.
    pub errors: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Temp {
    /// A sensor group, e.g. "PMU tdie" (hottest of its members).
    pub sensor: String,
    pub c: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crit: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Battery {
    pub pct: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycles: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Proc {
    /// Process name only (summed over processes of that name).
    pub name: String,
    pub cpu_pct: f64,
    pub mem_mb: f64,
    pub count: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Updates {
    pub count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<u32>,
    pub checked_at: i64,
    /// Since when security updates have been pending, if they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security_since: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Health {
    /// PSI `some avg60` for CPU and memory, `full avg60` for memory (Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psi_cpu: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psi_mem: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psi_mem_full: Option<f64>,
    /// Processes killed for memory since the previous report (Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oom_kills: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_oom_at: Option<i64>,
    #[serde(default)]
    pub net: Vec<Iface>,
    #[serde(default)]
    pub temps: Vec<Temp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<Battery>,
    #[serde(default)]
    pub top_cpu: Vec<Proc>,
    #[serde(default)]
    pub top_mem: Vec<Proc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_units: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reboot_required_since: Option<i64>,
    /// Why a reboot is needed (REQ-PROBE-015).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reboot: Option<Reboot>,
    /// ext4 filesystems with errors recorded (REQ-PROBE-017).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fs_errors: Vec<FsErrors>,
    /// Services that restarted since the previous report, or are waiting to
    /// restart (REQ-PROBE-016): a Restart= crash loop never reaches failed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restarting: Vec<Restarting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<Updates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ntp_synced: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_offset_ms: Option<f64>,
    /// The smaller of this and the previous run's offset: what findings use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_offset_sustained_ms: Option<f64>,
    /// What could not be read; published only as a count.
    #[serde(default)]
    pub errors: Vec<String>,
}

/// Counters and slow results carried between runs.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Prev {
    pub at_ms: i64,
    /// interface → (rx bytes, tx bytes, errors)
    #[serde(default)]
    pub net: BTreeMap<String, (u64, u64, u64)>,
    #[serde(default)]
    pub oom: Option<u64>,
    #[serde(default)]
    pub last_oom_at: Option<i64>,
    #[serde(default)]
    pub updates: Option<Updates>,
    #[serde(default)]
    pub reboot_required_since: Option<i64>,
    /// Each service's restart count (NRestarts) at the previous run.
    #[serde(default)]
    pub restarts: BTreeMap<String, u64>,
    /// Each ext4 device's error count at the previous run.
    #[serde(default)]
    pub fs_errors: BTreeMap<String, u64>,
    /// Last update-check attempt, successful or not (retries at most hourly).
    #[serde(default)]
    pub updates_tried_at: Option<i64>,
    /// No update tool on this host: checked again daily, not every hour.
    #[serde(default)]
    pub updates_unavailable: bool,
    /// The previous run's clock offset and NTP state, so findings need two runs.
    #[serde(default)]
    pub clock_offset_ms: Option<f64>,
    #[serde(default)]
    pub ntp_unsynced: bool,
}

/// Retry a failed update check after this long.
pub const UPDATES_RETRY_S: i64 = 3600;

/// A process, unit or interface name safe to publish: `[A-Za-z0-9._@:+-]`,
/// 1..=40 characters, not starting with `.` or `-`. Anything else is
/// dropped, never repaired.
pub fn clean(s: &str) -> Option<String> {
    let s = s.trim();
    let ok = !s.is_empty()
        && s.len() <= 40
        && !s.starts_with('.')
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._@:+-".contains(c));
    ok.then(|| s.to_string())
}

/// A sensor label safe to publish: as `clean`, but words may be separated by
/// single spaces ("PMU tdie").
pub fn clean_label(s: &str) -> Option<String> {
    let words: Vec<String> = s.split_whitespace().map(clean).collect::<Option<_>>()?;
    let j = words.join(" ");
    (!j.is_empty() && j.len() <= 40).then_some(j)
}

/// A failed unit as published: services, timers and sockets only, the
/// instance cut (`ssh-tunnel@db.example.service` → `ssh-tunnel@.service`), so
/// no path (mount units), user id or host name can leave the machine.
pub fn unit_name(u: &str) -> Option<String> {
    let (stem, kind) = u.rsplit_once('.')?;
    if !["service", "timer", "socket"].contains(&kind) {
        return None;
    }
    let stem = match stem.split_once('@') {
        Some((t, _)) => format!("{t}@"),
        None => stem.to_string(),
    };
    clean(&format!("{stem}.{kind}"))
}

/// A sensor's group: its label without indices ("PMU tdie4" → "PMU tdie",
/// "NAND CH0 temp" → "NAND CH temp", "Core 3" → "Core").
pub fn temp_group(label: &str) -> String {
    let words: Vec<String> = label
        .split_whitespace()
        .map(|w| {
            let t = w.trim_end_matches(|c: char| c.is_ascii_digit());
            if t.is_empty() {
                String::new()
            } else {
                t.to_string()
            }
        })
        .filter(|w| !w.is_empty())
        .collect();
    words.join(" ")
}

/// The hottest reading per sensor group, hottest groups first, at most `MAX_ITEMS`.
pub fn temps_from(readings: &[(String, f64, Option<f64>)]) -> Vec<Temp> {
    let mut g: BTreeMap<String, Temp> = BTreeMap::new();
    for (label, c, crit) in readings {
        if !c.is_finite() || *c <= -40.0 || *c >= 200.0 {
            continue;
        }
        let Some(sensor) = clean_label(&temp_group(label)) else {
            continue;
        };
        // some drivers report 0, 20 or 255 as critical: only plausible values count
        let crit = crit.filter(|x| (50.0..=150.0).contains(x));
        let e = g.entry(sensor.clone()).or_insert(Temp {
            sensor,
            c: *c,
            crit,
        });
        if *c > e.c {
            // the threshold goes with the reading that is the maximum
            e.c = *c;
            e.crit = crit;
        }
    }
    let mut v: Vec<Temp> = g.into_values().collect();
    v.sort_by(|a, b| b.c.total_cmp(&a.c));
    v.truncate(MAX_ITEMS);
    v
}

/// Interfaces worth charting: not loopback or virtual plumbing.
pub fn keep_iface(name: &str) -> bool {
    const SKIP: [&str; 14] = [
        "lo",
        "utun",
        "awdl",
        "llw",
        "bridge",
        "veth",
        "docker",
        "br-",
        "anpi",
        "gif",
        "stf",
        "ap",
        "virbr",
        "tailscale",
    ];
    // enx…/wlx… names carry the MAC address
    !SKIP.iter().any(|p| name == *p || name.starts_with(p))
        && !name.starts_with("enx")
        && !name.starts_with("wlx")
        && clean(name).is_some()
}

/// Per-interface rates from two counter readings `dt_ms` apart. An interface
/// whose counters went backwards (reset) is left out of this round.
pub fn net_rates(
    prev: &BTreeMap<String, (u64, u64, u64)>,
    cur: &BTreeMap<String, (u64, u64, u64)>,
    dt_ms: i64,
) -> Vec<Iface> {
    if dt_ms <= 0 {
        return Vec::new();
    }
    let s = dt_ms as f64 / 1000.0;
    let mut v: Vec<Iface> = cur
        .iter()
        .filter(|(n, _)| keep_iface(n))
        .filter_map(|(n, (rx, tx, er))| {
            let (prx, ptx, per) = prev.get(n)?;
            if rx < prx || tx < ptx || er < per {
                return None;
            }
            Some(Iface {
                name: n.clone(),
                rx_kbs: (rx - prx) as f64 / 1024.0 / s,
                tx_kbs: (tx - ptx) as f64 / 1024.0 / s,
                errors: (er - per) as f64 * 60.0 / s,
            })
        })
        .filter(|i| i.rx_kbs + i.tx_kbs > 0.0 || i.errors > 0.0)
        .collect();
    v.sort_by(|a, b| (b.rx_kbs + b.tx_kbs).total_cmp(&(a.rx_kbs + a.tx_kbs)));
    v.truncate(MAX_ITEMS);
    v
}

/// Top processes by CPU and by memory, summed per name; unpublishable names dropped.
pub fn top_procs(procs: &[(String, f64, u64)]) -> (Vec<Proc>, Vec<Proc>) {
    let mut by: BTreeMap<String, Proc> = BTreeMap::new();
    for (name, cpu, mem) in procs {
        // a full path is reduced to its last part; a name with '/' that is not
        // a path (kernel threads such as kworker/u8:2) is dropped
        let base = if name.starts_with('/') {
            name.rsplit('/').next().unwrap_or(name)
        } else {
            name.as_str()
        };
        let Some(n) = clean(base) else { continue };
        let p = by.entry(n.clone()).or_insert(Proc {
            name: n,
            ..Default::default()
        });
        p.cpu_pct += cpu;
        p.mem_mb += *mem as f64 / 1_048_576.0;
        p.count += 1;
    }
    let all: Vec<Proc> = by.into_values().collect();
    let mut cpu = all.clone();
    cpu.sort_by(|a, b| b.cpu_pct.total_cmp(&a.cpu_pct));
    cpu.retain(|p| p.cpu_pct >= 0.5);
    cpu.truncate(TOP);
    let mut mem = all;
    mem.sort_by(|a, b| b.mem_mb.total_cmp(&a.mem_mb));
    mem.truncate(TOP);
    (cpu, mem)
}

/// `oom_kill` from `/proc/vmstat`.
pub fn parse_vmstat_oom(text: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        l.strip_prefix("oom_kill ")
            .and_then(|v| v.trim().parse().ok())
    })
}

/// Unit names from `systemctl --failed --output=json`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Reboot {
    /// `packages`: Debian's reboot-required file; `kernel`: a newer kernel
    /// is installed than the one running.
    pub reason: String,
    /// Packages listed in reboot-required.pkgs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pkgs: Option<u32>,
    /// The newest installed kernel, when it is not the running one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest_kernel: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FsErrors {
    /// The block device, e.g. `mmcblk0p2`.
    pub dev: String,
    /// ext4's errors_count: errors recorded since the last fsck cleared them.
    pub errors: u64,
    /// New since the previous report.
    pub delta: u64,
}

/// ext4 devices with a non-zero error count, with the change since `prev`
/// (REQ-PROBE-017). `now`: (device, errors_count) per mounted ext4 device.
pub fn fs_errors(now: &[(String, u64)], prev: &BTreeMap<String, u64>) -> Vec<FsErrors> {
    now.iter()
        .filter(|(_, n)| *n > 0)
        .filter_map(|(d, n)| {
            Some(FsErrors {
                dev: clean(d)?,
                errors: *n,
                delta: n.saturating_sub(prev.get(d).copied().unwrap_or(0)),
            })
        })
        .collect()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Restarting {
    pub unit: String,
    /// systemd's NRestarts since the unit was started by hand or boot.
    pub restarts: u64,
    /// Restarts since the previous report.
    pub delta: u64,
    /// SubState, e.g. `auto-restart` while waiting for the next try.
    pub state: String,
}

/// Is a reboot needed, why, and since when (REQ-PROBE-015)? `running`: the
/// running kernel's release; `images`: installed `/boot/vmlinuz-<release>`
/// files with their modification times; `file`: Debian's
/// `/var/run/reboot-required` (modification time, packages listed). The age
/// comes from those times, so it holds without any state kept between runs.
pub fn reboot_status(
    running: &str,
    images: &[(String, i64)],
    file: Option<(i64, Option<u32>)>,
) -> Option<(Reboot, i64)> {
    let flavour = super::kernel_flavour(running);
    let newest = images
        .iter()
        .filter(|(k, _)| super::kernel_flavour(k) == flavour)
        .max_by_key(|(k, _)| super::vkey(k));
    let kernel = newest.filter(|(k, _)| super::vkey(k) > super::vkey(running));
    match (file, kernel) {
        (None, None) => None,
        (Some((at, pkgs)), k) => Some((
            Reboot {
                reason: "packages".into(),
                pkgs,
                newest_kernel: k.and_then(|(k, _)| clean(k)),
            },
            k.map(|(_, t)| (*t).min(at)).unwrap_or(at),
        )),
        (None, Some((k, at))) => Some((
            Reboot {
                reason: "kernel".into(),
                pkgs: None,
                newest_kernel: clean(k),
            },
            *at,
        )),
    }
}

/// `systemctl show -p Id -p NRestarts -p SubState` output for many units:
/// blocks separated by blank lines.
pub fn parse_restarts(text: &str) -> Vec<(String, u64, String)> {
    text.split("\n\n")
        .filter_map(|b| {
            let mut id = None;
            let mut n = None;
            let mut sub = String::new();
            for l in b.lines() {
                match l.split_once('=') {
                    Some(("Id", v)) => id = unit_name(v),
                    Some(("NRestarts", v)) => n = v.trim().parse::<u64>().ok(),
                    Some(("SubState", v)) => sub = v.trim().to_string(),
                    _ => {}
                }
            }
            Some((id?, n?, sub))
        })
        .collect()
}

/// Units that restarted since the previous run, or wait to restart. A
/// counter that went down (the unit was restarted by hand or the machine
/// rebooted) counts from zero.
pub fn restarting(now: &[(String, u64, String)], prev: &BTreeMap<String, u64>) -> Vec<Restarting> {
    let mut out: Vec<Restarting> = now
        .iter()
        .filter_map(|(u, n, sub)| {
            let before = prev.get(u).copied().unwrap_or(0);
            let delta = if *n >= before { n - before } else { *n };
            (delta > 0 || sub == "auto-restart").then(|| Restarting {
                unit: u.clone(),
                restarts: *n,
                delta,
                state: clean(sub).unwrap_or_default(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.delta.cmp(&a.delta).then(a.unit.cmp(&b.unit)));
    out.truncate(20);
    out
}

pub fn parse_failed_units(json: &str) -> Option<Vec<String>> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let mut u: Vec<String> = v
        .as_array()?
        .iter()
        .filter_map(|x| x.get("unit")?.as_str().and_then(unit_name))
        .collect();
    u.truncate(20);
    Some(u)
}

/// Pending upgrades from `apt-get -s upgrade`: (all, from a security pocket).
pub fn parse_apt_sim(text: &str) -> (u32, u32) {
    let inst: Vec<&str> = text.lines().filter(|l| l.starts_with("Inst ")).collect();
    let sec = inst.iter().filter(|l| l.contains("-security")).count();
    (inst.len() as u32, sec as u32)
}

/// Updates listed by `softwareupdate --list` (one `* Label:` line each).
pub fn parse_softwareupdate(text: &str) -> u32 {
    text.lines()
        .filter(|l| l.trim_start().starts_with("* Label:"))
        .count() as u32
}

/// Charge and state from `pmset -g batt`.
pub fn parse_pmset(text: &str) -> Option<(f64, bool)> {
    let line = text.lines().find(|l| l.contains("InternalBattery"))?;
    let pct = line.split(['\t', ';']).find_map(|p| {
        p.trim()
            .strip_suffix('%')
            .and_then(|n| n.trim().parse().ok())
    })?;
    Some((
        pct,
        line.contains("charging") && !line.contains("discharging"),
    ))
}

/// `"CycleCount" = N` from `ioreg -rn AppleSmartBattery`.
pub fn parse_ioreg_cycles(text: &str) -> Option<u32> {
    text.lines().find_map(|l| {
        let l = l.trim();
        l.strip_prefix("\"CycleCount\" = ")
            .and_then(|v| v.trim().parse().ok())
    })
}

/// `NTPSynchronized=yes` from `timedatectl show`; `None` when NTP is not
/// turned on (`NTP=no`: a VM taking time from its host, or on purpose).
pub fn parse_timedatectl(text: &str) -> Option<bool> {
    let get = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k))
            .map(|v| v.trim() == "yes")
    };
    if get("NTP=") != Some(true) {
        return None;
    }
    get("NTPSynchronized=")
}

/// Offset from `chronyc tracking`: "System time : 0.000012345 seconds slow of NTP time".
pub fn parse_chrony_offset_ms(text: &str) -> Option<f64> {
    let l = text.lines().find(|l| l.starts_with("System time"))?;
    let v: f64 = l
        .split(':')
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let sign = if l.contains("fast") { 1.0 } else { -1.0 };
    Some(sign * v * 1000.0)
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

async fn run(cmd: &str, args: &[&str], secs: u64) -> Result<String, String> {
    super::inventory::run_for(cmd, args, Duration::from_secs(secs)).await
}

/// Collect once. `state`: where counters and slow results persist between runs.
pub async fn collect(state: Option<&std::path::Path>, now: i64) -> Health {
    let now_ms = now * 1000;
    let prev: Prev = state
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut h = Health::default();
    let mut next = Prev {
        at_ms: now_ms,
        ..Default::default()
    };

    // pressure and OOM kills (Linux)
    if let Some(t) = read("/proc/pressure/cpu") {
        h.psi_cpu = super::io::parse_psi(&t).0;
    }
    if let Some(t) = read("/proc/pressure/memory") {
        let (some, full) = super::io::parse_psi(&t);
        h.psi_mem = some;
        h.psi_mem_full = full;
    }
    if let Some(n) = read("/proc/vmstat").as_deref().and_then(parse_vmstat_oom) {
        next.oom = Some(n);
        next.last_oom_at = prev.last_oom_at;
        if let Some(p) = prev.oom.filter(|p| n >= *p) {
            h.oom_kills = Some(n - p);
            if n > p {
                next.last_oom_at = Some(now);
            }
        }
        h.last_oom_at = next.last_oom_at;
    }

    // network
    let nets = sysinfo::Networks::new_with_refreshed_list();
    next.net = nets
        .list()
        .iter()
        .map(|(n, d)| {
            (
                n.clone(),
                (
                    d.total_received(),
                    d.total_transmitted(),
                    d.total_errors_on_received() + d.total_errors_on_transmitted(),
                ),
            )
        })
        .collect();
    h.net = net_rates(&prev.net, &next.net, now_ms - prev.at_ms);

    // temperatures (no root: IOKit on macOS, hwmon on Linux)
    let comps = sysinfo::Components::new_with_refreshed_list();
    let readings: Vec<(String, f64, Option<f64>)> = comps
        .list()
        .iter()
        .filter_map(|c| {
            Some((
                c.label().to_string(),
                f64::from(c.temperature()?),
                c.critical().map(f64::from),
            ))
        })
        .collect();
    h.temps = temps_from(&readings);

    // top processes: CPU over one second, names only
    {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
        // processes only: with tasks, every Linux thread would be an entry
        // carrying its process's memory and an app-chosen thread name
        let kind = ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .without_tasks();
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        tokio::time::sleep(Duration::from_millis(1000)).await;
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        let procs: Vec<(String, f64, u64)> = sys
            .processes()
            .values()
            .filter(|p| p.thread_kind().is_none())
            .map(|p| {
                (
                    p.name().to_string_lossy().into_owned(),
                    f64::from(p.cpu_usage()),
                    p.memory(),
                )
            })
            .collect();
        (h.top_cpu, h.top_mem) = top_procs(&procs);
    }

    let linux = cfg!(target_os = "linux");
    let macos = cfg!(target_os = "macos");

    // battery
    if linux {
        if let Ok(rd) = std::fs::read_dir("/sys/class/power_supply") {
            for e in rd.flatten() {
                let p = e.path();
                if !e.file_name().to_string_lossy().starts_with("BAT") {
                    continue;
                }
                let num = |f: &str| {
                    std::fs::read_to_string(p.join(f))
                        .ok()
                        .and_then(|s| s.trim().parse::<f64>().ok())
                };
                if let Some(pct) = num("capacity") {
                    let status = std::fs::read_to_string(p.join("status")).unwrap_or_default();
                    h.battery = Some(Battery {
                        pct,
                        cycles: num("cycle_count").map(|c| c as u32).filter(|c| *c > 0),
                        charging: Some(status.trim() == "Charging"),
                    });
                    break;
                }
            }
        }
    } else if macos {
        if let Ok(t) = run("pmset", &["-g", "batt"], 5).await {
            if let Some((pct, charging)) = parse_pmset(&t) {
                let cycles = run("ioreg", &["-rn", "AppleSmartBattery"], 5)
                    .await
                    .ok()
                    .and_then(|t| parse_ioreg_cycles(&t));
                h.battery = Some(Battery {
                    pct,
                    cycles,
                    charging: Some(charging),
                });
            }
        }
    }

    // system health
    if linux {
        match run("systemctl", &["--failed", "--output=json", "--no-pager"], 5).await {
            Ok(j) => h.failed_units = parse_failed_units(&j),
            Err(e) => h.errors.push(e),
        }
        // REQ-PROBE-015: the file and the kernel images carry their own age
        let mtime = |p: &std::path::Path| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
        };
        let file = mtime(std::path::Path::new("/var/run/reboot-required")).map(|at| {
            let pkgs = std::fs::read_to_string("/var/run/reboot-required.pkgs")
                .ok()
                .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count() as u32);
            (at, pkgs)
        });
        let images: Vec<(String, i64)> = std::fs::read_dir("/boot")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let k = name.strip_prefix("vmlinuz-")?.to_string();
                Some((k, mtime(&e.path())?))
            })
            .collect();
        let running = sysinfo::System::kernel_version().unwrap_or_default();
        let status = reboot_status(&running, &images, file);
        next.reboot_required_since = status
            .as_ref()
            .map(|(_, at)| prev.reboot_required_since.map_or(*at, |p| p.min(*at)));
        h.reboot_required_since = next.reboot_required_since;
        h.reboot = status.map(|(r, _)| r);
        // REQ-PROBE-017: ext4 errors, readable without root
        let counts: Vec<(String, u64)> = std::fs::read_dir("/sys/fs/ext4")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = std::fs::read_to_string(e.path().join("errors_count")).ok()?;
                Some((
                    e.file_name().to_string_lossy().into_owned(),
                    n.trim().parse().ok()?,
                ))
            })
            .collect();
        h.fs_errors = fs_errors(&counts, &prev.fs_errors);
        next.fs_errors = counts.into_iter().collect();
        // REQ-PROBE-016: crash loops never reach "failed"
        match run(
            "systemctl",
            &[
                "show",
                "--type=service",
                "-p",
                "Id",
                "-p",
                "NRestarts",
                "-p",
                "SubState",
                "*.service",
            ],
            5,
        )
        .await
        {
            Ok(t) => {
                let now_units = parse_restarts(&t);
                h.restarting = restarting(&now_units, &prev.restarts);
                next.restarts = now_units.into_iter().map(|(u, n, _)| (u, n)).collect();
            }
            Err(e) => h.errors.push(e),
        }
        if let Ok(t) = run("timedatectl", &["show"], 5).await {
            let synced = parse_timedatectl(&t);
            next.ntp_unsynced = synced == Some(false);
            // unsynchronised only when this run and the previous one say so
            // (it is briefly false after every boot and resume)
            h.ntp_synced = synced.map(|s| s || !prev.ntp_unsynced);
        }
        if let Ok(t) = run("chronyc", &["tracking"], 5).await {
            let off = parse_chrony_offset_ms(&t);
            next.clock_offset_ms = off;
            h.clock_offset_ms = off;
            // the finding uses the smaller of two consecutive offsets: a
            // slew after resume does not flap it
            h.clock_offset_sustained_ms = match (off, prev.clock_offset_ms) {
                (Some(a), Some(b)) => Some(if a.abs() < b.abs() { a } else { b }),
                _ => None,
            };
        }
    }
    let every = if macos {
        UPDATES_EVERY_MACOS_S
    } else {
        UPDATES_EVERY_LINUX_S
    };
    let tried = prev.updates_tried_at;
    let retry = if prev.updates_unavailable {
        86_400
    } else {
        UPDATES_RETRY_S
    };
    // `probe report` (no state) never runs the slow check
    let due = state.is_some()
        && prev
            .updates
            .as_ref()
            .is_none_or(|u| now - u.checked_at >= every)
        && tried.is_none_or(|t| now - t >= retry);
    next.updates = prev.updates.clone();
    next.updates_tried_at = tried;
    next.updates_unavailable = prev.updates_unavailable;
    if due && (linux || macos) {
        next.updates_tried_at = Some(now);
        let got = if linux {
            run(
                "apt-get",
                &["-s", "-o", "Debug::NoLocking=1", "upgrade"],
                30,
            )
            .await
            .map(|t| {
                let (n, s) = parse_apt_sim(&t);
                (n, Some(s))
            })
        } else {
            run("softwareupdate", &["--list"], 45)
                .await
                .map(|t| (parse_softwareupdate(&t), None))
        };
        match got {
            Ok((count, security)) => {
                let since = prev.updates.as_ref().and_then(|u| u.security_since);
                next.updates = Some(Updates {
                    count,
                    security,
                    checked_at: now,
                    security_since: security.filter(|s| *s > 0).map(|_| since.unwrap_or(now)),
                });
            }
            Err(e) => {
                // no such tool (not apt, not macOS): stop asking every hour
                next.updates_unavailable = e.contains("No such file") || e.contains("not found");
                if !next.updates_unavailable {
                    h.errors.push(e);
                }
            }
        }
    }
    h.updates = next.updates.clone();

    if let Some(p) = state {
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec(&next).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, p);
        }
    }
    h
}

/// What the read API publishes: everything but the error text (a count instead).
pub fn public(h: &Health) -> serde_json::Value {
    let mut v = serde_json::to_value(h).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.remove("errors");
        o.insert("not_read".into(), h.errors.len().into());
    }
    v
}

/// Drift findings from one machine's health: (severity, text).
pub fn findings(h: &Health, now: i64) -> Vec<(&'static str, String)> {
    let mut f = Vec::new();
    if let Some(u) = h.failed_units.as_ref().filter(|u| !u.is_empty()) {
        let names = u.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
        let more = if u.len() > 3 {
            format!(" and {} more", u.len() - 3)
        } else {
            String::new()
        };
        f.push((
            "amber",
            format!("{} failed systemd unit(s): {names}{more}", u.len()),
        ));
    }
    // REQ-PROBE-015: at once, red after 7 days
    if let Some(s) = h.reboot_required_since {
        let days = (now - s).max(0) / 86_400;
        let why = match &h.reboot {
            Some(Reboot {
                newest_kernel: Some(k),
                ..
            }) => format!(": kernel {k} is installed, not running"),
            Some(Reboot { pkgs: Some(n), .. }) => format!(": {n} package(s) wait for it"),
            _ => String::new(),
        };
        f.push((
            if days >= 7 { "red" } else { "amber" },
            if days >= 1 {
                format!("reboot required for {days} day(s){why}")
            } else {
                format!("reboot required{why}")
            },
        ));
    }
    // REQ-PROBE-017: a filesystem recording errors
    for e in &h.fs_errors {
        f.push(if e.delta > 0 {
            (
                "red",
                format!(
                    "filesystem {} recorded {} new error(s) ({} since the last fsck): the disk or SD card may be failing",
                    e.dev, e.delta, e.errors
                ),
            )
        } else {
            (
                "amber",
                format!("filesystem {} has {} error(s) recorded: run fsck", e.dev, e.errors),
            )
        });
    }
    // REQ-PROBE-016: restarting services
    for r in &h.restarting {
        let looping = r.delta >= 3 || r.state == "auto-restart";
        f.push((
            if looping { "red" } else { "amber" },
            if looping {
                format!(
                    "{} is crash-looping: {} restart(s) since the last report, {} in all",
                    r.unit, r.delta, r.restarts
                )
            } else {
                format!("{} restarted ({} in all)", r.unit, r.restarts)
            },
        ));
    }
    if let Some(u) = &h.updates {
        if let Some(s) = u.security_since.filter(|s| now - s > 7 * 86_400) {
            f.push((
                "amber",
                format!(
                    "{} security update(s) pending for {} days",
                    u.security.unwrap_or(0),
                    (now - s) / 86_400
                ),
            ));
        }
    }
    if let Some(ms) = h.clock_offset_sustained_ms {
        if ms.abs() > 5000.0 {
            f.push(("red", format!("clock off by {:.1} s", ms / 1000.0)));
        } else if ms.abs() > 500.0 {
            f.push(("amber", format!("clock off by {ms:.0} ms")));
        }
    }
    if h.ntp_synced == Some(false) {
        f.push(("amber", "clock not synchronised (NTP)".to_string()));
    }
    if h.last_oom_at.is_some_and(|t| now - t < 3600) {
        f.push((
            "amber",
            "memory exhausted: a process was killed (OOM) in the last hour".to_string(),
        ));
    }
    for t in &h.temps {
        if let Some(c) = t.crit {
            if t.c >= c {
                f.push((
                    "red",
                    format!(
                        "{} at {:.0} °C, at or over critical {c:.0} °C",
                        t.sensor, t.c
                    ),
                ));
            } else if t.c >= c - 5.0 {
                f.push((
                    "amber",
                    format!("{} at {:.0} °C, near critical {c:.0} °C", t.sensor, t.c),
                ));
            }
        }
    }
    f
}

//! What of a report or ground-station point becomes history: numbers only.
//!
//! This is the whitelist (warte#46): host comes from the caller, everything
//! here is a series name and a number. Mount points and device names appear in
//! series labels, as they already do on the Machines view; nothing else from a
//! report (paths, sessions' directories, tool paths, errors) is read.

use crate::hub::model::Point;
use crate::probe::Report;

/// Accept the probe's clock unless it is more than this far from ours.
pub const CLOCK_SKEW_S: i64 = 300;

/// The minute a report belongs to: when it was taken, unless the probe's clock
/// is off by more than 5 minutes, then when it arrived.
pub fn minute(taken_at: i64, received_at: i64) -> i64 {
    let t = if (taken_at - received_at).abs() > CLOCK_SKEW_S {
        received_at
    } else {
        taken_at
    };
    t - t.rem_euclid(60)
}

pub fn label(v: &str) -> String {
    v.chars()
        .filter(|c| !c.is_control() && *c != '{' && *c != '}' && *c != ',')
        .take(120)
        .collect()
}

/// The series of one probe report.
pub fn from_report(r: &Report) -> Vec<(String, f64)> {
    let mut v: Vec<(String, f64)> = vec![
        ("load.1".into(), r.cpu.load[0]),
        ("load.5".into(), r.cpu.load[1]),
        ("load.15".into(), r.cpu.load[2]),
        ("uptime".into(), r.uptime_s as f64),
        ("sessions".into(), r.sessions.len() as f64),
        ("swap.used_mb".into(), r.memory.swap_used_mb as f64),
    ];
    if r.cpu.ncpu > 0 {
        v.push(("load.per_core".into(), r.cpu.load[1] / r.cpu.ncpu as f64));
    }
    if r.memory.total_mb > 0 {
        let used = r.memory.total_mb.saturating_sub(r.memory.avail_mb) as f64;
        v.push((
            "mem.used_pct".into(),
            100.0 * used / r.memory.total_mb as f64,
        ));
    }
    for d in &r.disks {
        v.push((
            format!("disk.used_pct{{mount={}}}", label(&d.mount)),
            d.used_pct,
        ));
    }
    if let Some(io) = &r.io {
        if let Some(x) = io.psi_some_avg60 {
            v.push(("psi.io.some".into(), x));
        }
        if let Some(x) = io.psi_full_avg60 {
            v.push(("psi.io.full".into(), x));
        }
        for d in &io.devices {
            let dev = label(&d.name);
            for (name, x) in [
                ("io.busy_pct", d.busy_pct),
                ("io.await_ms", d.await_ms),
                ("io.read_kbs", d.read_kbs),
                ("io.write_kbs", d.write_kbs),
            ] {
                if let Some(x) = x {
                    v.push((format!("{name}{{dev={dev}}}"), x));
                }
            }
        }
    }
    if let Some(h) = &r.health {
        v.extend(from_health(h));
    }
    v.retain(|(_, x)| x.is_finite());
    v
}

/// The series of a report's health block (REQ-PROBE-011). Process lists are
/// not series: they stay a snapshot of the latest report.
pub fn from_health(h: &crate::probe::health::Health) -> Vec<(String, f64)> {
    let mut v: Vec<(String, f64)> = Vec::new();
    let opt = [
        // the probe's own names: the ground station writes psi.cpu/psi.mem for
        // the CI host from its agent, and a host may run both
        ("psi.cpu.host", h.psi_cpu),
        ("psi.mem.host", h.psi_mem),
        ("psi.mem.full", h.psi_mem_full),
        ("oom.kills", h.oom_kills.map(|x| x as f64)),
        ("health.clock_offset_ms", h.clock_offset_ms),
        (
            "health.ntp_synced",
            h.ntp_synced.map(|b| f64::from(u8::from(b))),
        ),
        (
            "health.failed_units",
            h.failed_units.as_ref().map(|u| u.len() as f64),
        ),
        (
            "health.reboot_required",
            // only where it is checked (systemd hosts report failed units)
            h.failed_units
                .is_some()
                .then(|| f64::from(u8::from(h.reboot_required_since.is_some()))),
        ),
        (
            "health.updates",
            h.updates.as_ref().map(|u| f64::from(u.count)),
        ),
        (
            "health.updates_security",
            h.updates.as_ref().and_then(|u| u.security).map(f64::from),
        ),
        ("battery.pct", h.battery.as_ref().map(|b| b.pct)),
        (
            "battery.cycles",
            h.battery.as_ref().and_then(|b| b.cycles).map(f64::from),
        ),
    ];
    for (n, x) in opt {
        if let Some(x) = x {
            v.push((n.into(), x));
        }
    }
    for i in &h.net {
        let n = label(&i.name);
        v.push((format!("net.rx_kbs{{if={n}}}"), i.rx_kbs));
        v.push((format!("net.tx_kbs{{if={n}}}"), i.tx_kbs));
        v.push((format!("net.errors{{if={n}}}"), i.errors));
    }
    for t in &h.temps {
        v.push((format!("temp.c{{sensor={}}}", label(&t.sensor)), t.c));
    }
    v
}

/// The series of one ground-station point (the CI host).
pub fn from_point(p: &Point) -> Vec<(String, f64)> {
    let mut v: Vec<(String, f64)> = vec![
        ("load.1".into(), p.load1),
        ("psi.cpu".into(), p.psi_cpu),
        ("psi.mem".into(), p.psi_mem),
        ("runners.online".into(), f64::from(p.online)),
        ("runners.busy".into(), f64::from(p.local_busy + p.gh_busy)),
        (
            "runners.offline".into(),
            f64::from(p.runners.saturating_sub(p.online)),
        ),
    ];
    let opt = [
        ("queue.self_hosted", p.sh_queued.map(f64::from)),
        ("queue.hosted", p.hosted_queued.map(f64::from)),
        ("queue.oldest_s", p.sh_oldest_s.map(|x| x as f64)),
        ("disk.used_pct{mount=runners}", p.disk_runners_pct),
    ];
    for (n, x) in opt {
        if let Some(x) = x {
            v.push((n.into(), x));
        }
    }
    for (id, x) in &p.runner_cpu {
        v.push((format!("runner.cpu{{id={id}}}"), *x));
    }
    for (id, x) in &p.runner_mem {
        v.push((format!("runner.mem_mb{{id={id}}}"), f64::from(*x)));
    }
    v.retain(|(_, x)| x.is_finite());
    v
}

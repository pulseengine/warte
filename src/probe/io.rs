//! Disk IO and IO pressure (REQ-PROBE-008), Linux only.
//!
//! Rates are averaged since the previous report: the probe keeps the last
//! `/proc/diskstats` sample in a small file (`io-prev.json` in its state
//! directory) and compares. The first report after a start has no rates.
//! Pressure is `/proc/pressure/io` (PSI): the share of time some (or all)
//! tasks stalled on IO, averaged over 60 s by the kernel.
//!
//! Every field is optional on the wire, so old probes and old ground
//! stations keep working.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// IO of one machine.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Io {
    /// PSI `some avg60`, percent: some task stalled on IO.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psi_some_avg60: Option<f64>,
    /// PSI `full avg60`, percent: every non-idle task stalled on IO.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psi_full_avg60: Option<f64>,
    #[serde(default)]
    pub devices: Vec<Device>,
}

/// A block device backing one or more reported mounts.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Device {
    /// Kernel name, e.g. `sda`, `mmcblk0`, `nvme0n1`.
    pub name: String,
    /// The reported mounts it backs.
    pub mounts: Vec<String>,
    /// A spinning disk (`/sys/block/<dev>/queue/rotational`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotational: Option<bool>,
    /// Share of the interval the device had IO in flight, percent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_pct: Option<f64>,
    /// Mean time per completed request, ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub await_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_kbs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_kbs: Option<f64>,
}

/// The counters of one `/proc/diskstats` line that the rates need.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
pub struct Stat {
    pub reads: u64,
    pub sectors_read: u64,
    pub ms_reading: u64,
    pub writes: u64,
    pub sectors_written: u64,
    pub ms_writing: u64,
    pub ms_io: u64,
}

/// A sample kept between reports.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Sample {
    /// Milliseconds since the epoch.
    pub at_ms: i64,
    pub stats: BTreeMap<String, Stat>,
}

/// `/proc/diskstats`: device name → counters. Short or odd lines are skipped.
pub fn parse_diskstats(text: &str) -> BTreeMap<String, Stat> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 13 {
                return None;
            }
            let n = |i: usize| f[i].parse::<u64>().ok();
            Some((
                f[2].to_string(),
                Stat {
                    reads: n(3)?,
                    sectors_read: n(5)?,
                    ms_reading: n(6)?,
                    writes: n(7)?,
                    sectors_written: n(9)?,
                    ms_writing: n(10)?,
                    ms_io: n(12)?,
                },
            ))
        })
        .collect()
}

/// `/proc/pressure/io`: (some avg60, full avg60).
pub fn parse_psi(text: &str) -> (Option<f64>, Option<f64>) {
    let avg60 = |kind: &str| {
        text.lines()
            .find(|l| l.starts_with(kind))?
            .split_whitespace()
            .find_map(|t| t.strip_prefix("avg60="))?
            .parse()
            .ok()
    };
    (avg60("some"), avg60("full"))
}

/// Rates of one device between two samples `dt_ms` apart. A counter that went
/// backwards (a reboot, a re-plugged device) gives no rates.
pub fn rates(prev: &Stat, cur: &Stat, dt_ms: i64) -> Option<(f64, Option<f64>, f64, f64)> {
    if dt_ms <= 0 {
        return None;
    }
    let d = |a: u64, b: u64| b.checked_sub(a);
    let ios = d(prev.reads, cur.reads)? + d(prev.writes, cur.writes)?;
    let ms = d(prev.ms_reading, cur.ms_reading)? + d(prev.ms_writing, cur.ms_writing)?;
    let busy = (d(prev.ms_io, cur.ms_io)? as f64 * 100.0 / dt_ms as f64).min(100.0);
    let secs = dt_ms as f64 / 1000.0;
    let kbs = |s: u64| (s as f64 * 512.0 / 1024.0 / secs * 10.0).round() / 10.0;
    Some((
        (busy * 10.0).round() / 10.0,
        (ios > 0).then(|| (ms as f64 / ios as f64 * 10.0).round() / 10.0),
        kbs(d(prev.sectors_read, cur.sectors_read)?),
        kbs(d(prev.sectors_written, cur.sectors_written)?),
    ))
}

/// The whole-disk name for a partition name, read from sysfs (`sda1` → `sda`,
/// `mmcblk0p2` → `mmcblk0`); a whole disk or a mapper device stays itself.
fn whole_disk(sys: &std::path::Path, name: &str) -> String {
    let class = sys.join("class/block").join(name);
    if class.join("partition").exists() {
        if let Ok(target) = std::fs::read_link(&class) {
            if let Some(parent) = target.parent().and_then(|p| p.file_name()) {
                return parent.to_string_lossy().into_owned();
            }
        }
    }
    name.to_string()
}

/// Mount point → kernel device name, from `/proc/self/mounts`.
pub fn mount_devices(mounts: &str) -> BTreeMap<String, String> {
    mounts
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let dev = f.next()?;
            let mnt = f.next()?.replace("\\040", " ");
            let name = dev.strip_prefix("/dev/")?;
            Some((mnt, name.rsplit('/').next().unwrap_or(name).to_string()))
        })
        .collect()
}

/// Read IO for the reported `mounts`, from the files under `root` (`/` in
/// production; a fixture tree in tests), against the previous sample in
/// `prev`, which is replaced with this one. `None` when there is no
/// `/proc/diskstats` (macOS).
pub fn collect_at(
    root: &std::path::Path,
    mounts: &[String],
    prev: Option<&std::path::Path>,
    now_ms: i64,
) -> Option<Io> {
    let stats = parse_diskstats(&std::fs::read_to_string(root.join("proc/diskstats")).ok()?);
    let (psi_some_avg60, psi_full_avg60) = std::fs::read_to_string(root.join("proc/pressure/io"))
        .map(|t| parse_psi(&t))
        .unwrap_or((None, None));
    let by_mount =
        mount_devices(&std::fs::read_to_string(root.join("proc/self/mounts")).unwrap_or_default());
    let sys = root.join("sys");
    let mut devices: BTreeMap<String, Device> = BTreeMap::new();
    for m in mounts {
        let Some(dev) = by_mount.get(m) else { continue };
        let disk = whole_disk(&sys, dev);
        let d = devices.entry(disk.clone()).or_insert_with(|| Device {
            name: disk.clone(),
            rotational: std::fs::read_to_string(
                sys.join("block").join(&disk).join("queue/rotational"),
            )
            .ok()
            .map(|v| v.trim() == "1"),
            ..Default::default()
        });
        d.mounts.push(m.clone());
    }
    let before: Option<Sample> = prev
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok());
    if let Some(before) = &before {
        for d in devices.values_mut() {
            if let (Some(a), Some(b)) = (before.stats.get(&d.name), stats.get(&d.name)) {
                if let Some((busy, wait, r, w)) = rates(a, b, now_ms - before.at_ms) {
                    d.busy_pct = Some(busy);
                    d.await_ms = wait;
                    d.read_kbs = Some(r);
                    d.write_kbs = Some(w);
                }
            }
        }
    }
    if let Some(p) = prev {
        let keep: BTreeMap<String, Stat> = devices
            .keys()
            .filter_map(|k| stats.get(k).map(|s| (k.clone(), *s)))
            .collect();
        let sample = Sample {
            at_ms: now_ms,
            stats: keep,
        };
        if let Ok(b) = serde_json::to_vec(&sample) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, p);
            }
        }
    }
    Some(Io {
        psi_some_avg60,
        psi_full_avg60,
        devices: devices.into_values().collect(),
    })
}

/// Findings on one machine's IO: `(severity, text)`.
pub fn findings(io: &Io) -> Vec<(&'static str, String)> {
    let mut out = vec![];
    for d in &io.devices {
        if d.rotational != Some(true) {
            continue;
        }
        let busy = d.busy_pct.unwrap_or(0.0);
        let wait = d.await_ms.unwrap_or(0.0);
        let what = format!("spinning disk {} ({})", d.name, d.mounts.join(", "));
        if busy >= 90.0 {
            out.push(("red", format!("{what} is {busy:.0}% busy")));
        } else if busy >= 60.0 || wait >= 100.0 {
            out.push((
                "amber",
                format!("{what} is {busy:.0}% busy, {wait:.0} ms per request"),
            ));
        }
    }
    if let Some(p) = io.psi_some_avg60 {
        if p >= 50.0 {
            out.push((
                "red",
                format!("tasks stalled on IO {p:.0}% of the last minute"),
            ));
        } else if p >= 20.0 {
            out.push((
                "amber",
                format!("tasks stalled on IO {p:.0}% of the last minute"),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATS_A: &str =
        "   8       0 sda 1000 0 80000 5000 2000 0 160000 20000 0 30000 25000 0 0 0 0
   8       1 sda1 900 0 70000 4000 1900 0 150000 19000 0 29000 23000 0 0 0 0
 179       0 mmcblk0 10 0 100 10 10 0 100 10 0 20 20
   7       0 loop0 1 0
";
    const STATS_B: &str =
        "   8       0 sda 1100 0 100480 7000 2100 0 180480 26000 0 75000 33000 0 0 0 0
 179       0 mmcblk0 10 0 100 10 10 0 100 10 0 20 20
";

    // rivet: verifies REQ-PROBE-008
    #[test]
    fn diskstats_and_psi_are_read_strictly() {
        let s = parse_diskstats(STATS_A);
        assert_eq!(s.len(), 3, "short lines are skipped");
        assert_eq!(s["sda"].ms_io, 30000);
        assert_eq!(s["sda"].sectors_written, 160000);
        let psi = "some avg10=1.50 avg60=23.40 avg300=5.00 total=1\nfull avg10=0.00 avg60=4.10 avg300=0.00 total=2\n";
        assert_eq!(parse_psi(psi), (Some(23.4), Some(4.1)));
        assert_eq!(parse_psi("garbage"), (None, None));
    }

    // rivet: verifies REQ-PROBE-008
    #[test]
    fn rates_since_the_previous_sample() {
        let a = parse_diskstats(STATS_A)["sda"];
        let b = parse_diskstats(STATS_B)["sda"];
        // 60 s apart: 45000 ms busy of 60000 = 75 %; 200 requests took 8000 ms = 40 ms
        let (busy, wait, r, w) = rates(&a, &b, 60_000).unwrap();
        assert_eq!(busy, 75.0);
        assert_eq!(wait, Some(40.0));
        // 20480 sectors * 512 B / 1024 / 60 s = 170.7 KB/s
        assert_eq!((r, w), (170.7, 170.7));
        // counters that went backwards (reboot): no rates
        assert!(rates(&b, &a, 60_000).is_none());
        assert!(rates(&a, &b, 0).is_none());
        // no requests: no await
        assert_eq!(rates(&a, &a, 1000).unwrap().1, None);
    }

    // rivet: verifies REQ-PROBE-008
    #[test]
    fn spinning_disks_and_pressure_become_findings() {
        let dev = |rot: bool, busy: f64, wait: f64| Device {
            name: "sda".into(),
            mounts: vec!["/srv".into()],
            rotational: Some(rot),
            busy_pct: Some(busy),
            await_ms: Some(wait),
            ..Default::default()
        };
        let sev = |io: Io| findings(&io).into_iter().map(|f| f.0).collect::<Vec<_>>();
        let io = |d: Device, psi: Option<f64>| Io {
            psi_some_avg60: psi,
            devices: vec![d],
            ..Default::default()
        };
        assert_eq!(sev(io(dev(true, 59.0, 99.0), None)), Vec::<&str>::new());
        assert_eq!(sev(io(dev(true, 60.0, 0.0), None)), ["amber"]);
        assert_eq!(sev(io(dev(true, 0.0, 100.0), None)), ["amber"]);
        assert_eq!(sev(io(dev(true, 90.0, 0.0), None)), ["red"]);
        // an SSD at 100 % is not a spinning-disk finding
        assert_eq!(sev(io(dev(false, 100.0, 500.0), None)), Vec::<&str>::new());
        assert_eq!(
            sev(io(dev(false, 0.0, 0.0), Some(19.9))),
            Vec::<&str>::new()
        );
        assert_eq!(sev(io(dev(false, 0.0, 0.0), Some(20.0))), ["amber"]);
        assert_eq!(sev(io(dev(false, 0.0, 0.0), Some(50.0))), ["red"]);
        let t = &findings(&io(dev(true, 95.0, 0.0), None))[0].1;
        assert!(
            t.contains("sda") && t.contains("/srv") && t.contains("95%"),
            "{t}"
        );
    }

    // rivet: verifies REQ-PROBE-008
    #[test]
    fn a_fixture_tree_gives_devices_mounts_and_rates_on_the_second_run() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let w = |p: &str, s: &str| {
            let p = r.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, s).unwrap();
        };
        w("proc/diskstats", STATS_A);
        w(
            "proc/pressure/io",
            "some avg10=0 avg60=2.00 avg300=0 total=0\n",
        );
        w(
            "proc/self/mounts",
            "/dev/sda1 /srv ext4 rw 0 0\n/dev/mmcblk0p2 / ext4 rw 0 0\nproc /proc proc rw 0 0\n",
        );
        w("sys/block/sda/queue/rotational", "1\n");
        w("sys/block/mmcblk0/queue/rotational", "0\n");
        // sysfs: partitions link into their disk's directory
        for (part, disk) in [("sda1", "sda"), ("mmcblk0p2", "mmcblk0")] {
            let dir = r.join("sys/devices/x/block").join(disk).join(part);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("partition"), "1").unwrap();
            std::fs::create_dir_all(r.join("sys/class/block")).unwrap();
            std::os::unix::fs::symlink(&dir, r.join("sys/class/block").join(part)).unwrap();
        }
        let prev = r.join("io-prev.json");
        let mounts = vec!["/".to_string(), "/srv".to_string()];
        let first = collect_at(r, &mounts, Some(&prev), 1_000_000).unwrap();
        assert_eq!(first.psi_some_avg60, Some(2.0));
        let names: Vec<&str> = first.devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["mmcblk0", "sda"]);
        assert_eq!(first.devices[1].rotational, Some(true));
        assert_eq!(first.devices[1].mounts, ["/srv"]);
        assert!(
            first.devices.iter().all(|d| d.busy_pct.is_none()),
            "no rates yet"
        );
        w("proc/diskstats", STATS_B);
        let second = collect_at(r, &mounts, Some(&prev), 1_060_000).unwrap();
        assert_eq!(second.devices[1].busy_pct, Some(75.0));
        assert_eq!(second.devices[0].busy_pct, Some(0.0));
        // no diskstats (macOS): no IO at all
        assert!(collect_at(&r.join("nothing"), &mounts, None, 0).is_none());
    }
}

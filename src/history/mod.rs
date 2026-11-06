//! Machine history (REQ-MHIST-001): every machine's numbers over time.
//!
//! One redb file next to `machines.json`. A sample is one row per host and
//! minute holding all of that host's series; finished hours are rolled up to
//! min/avg/max. Minute rows are kept 7 days, hour rows 90 days by default.
//!
//! Writes are buffered for the SD card the hub host runs from: each sample
//! commits with `Durability::None`, and `flush` (every 5 minutes and at
//! shutdown) makes them durable. A crash loses at most those minutes. A store
//! error is counted and logged, never returned to the report path: history
//! must not reject or delay a report.
//!
//! Only host names, series names and numbers are stored (warte#46 rules); the
//! projection in `project` is the whitelist.

pub mod project;

use redb::{Database, Durability, ReadableDatabase, ReadableTable, TableDefinition};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// (host, minute) → encoded `[(series id u32, value f64)]`.
const MINUTE: TableDefinition<(&str, i64), &[u8]> = TableDefinition::new("minute");
/// (host, hour) → encoded `[(series id u32, min f64, avg f64, max f64)]`.
const HOUR: TableDefinition<(&str, i64), &[u8]> = TableDefinition::new("hour");
/// series name → id.
const NAMES: TableDefinition<&str, u32> = TableDefinition::new("names");
/// (host, series id) → () : which series a host has sent.
const HOST_SERIES: TableDefinition<(&str, u32), ()> = TableDefinition::new("host_series");
/// small counters, e.g. `rolled_to`, `hub_imported`.
const META: TableDefinition<&str, i64> = TableDefinition::new("meta");

pub const MINUTE_KEEP_S: i64 = 7 * 86_400;
pub const HOUR_KEEP_S: i64 = 90 * 86_400;
/// Ranges up to this long are answered from minute rows.
pub const MINUTE_RANGE_S: i64 = 7 * 86_400;
/// The longest range the API answers.
pub const MAX_RANGE_S: i64 = 400 * 86_400;
/// `flush` cadence; also the most a crash loses.
pub const FLUSH_EVERY_S: u64 = 300;

/// One series over a range, as the read API returns it. A gap is `None`
/// (JSON `null`), so charts break the line instead of drawing across it.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Series {
    pub metric: String,
    pub unit: &'static str,
    pub t0: i64,
    pub step: i64,
    pub values: Vec<Option<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<Vec<Option<f64>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<Vec<Option<f64>>>,
}

/// The unit of a series, from its name.
pub fn unit(metric: &str) -> &'static str {
    let base = metric.split('{').next().unwrap_or(metric);
    if base.ends_with("_pct") || base.starts_with("psi.") {
        "%"
    } else if base.ends_with("_mb") {
        "MB"
    } else if base.ends_with("_kbs") {
        "KB/s"
    } else if base.ends_with("_ms") {
        "ms"
    } else if base.starts_with("temp.") || base.ends_with("temp_c") {
        "°C"
    } else if base == "battery.pct" {
        "%"
    } else if base.ends_with("_h") {
        "h"
    } else if base.ends_with("_s") || base == "uptime" {
        "s"
    } else {
        ""
    }
}

struct Names {
    by_name: HashMap<String, u32>,
    by_id: HashMap<u32, String>,
    host_series: BTreeSet<(String, u32)>,
    /// The next free series id: the persisted maximum plus one.
    next_id: u32,
}

/// What the writer thread does, in order.
enum Cmd {
    Record(String, i64, Vec<(String, f64)>),
    Maintain(i64, std::sync::mpsc::SyncSender<()>),
    Flush(std::sync::mpsc::SyncSender<()>),
    Meta(String, i64, std::sync::mpsc::SyncSender<()>),
    Sync(std::sync::mpsc::SyncSender<()>),
}

/// Samples waiting for the writer; beyond this a sample is dropped and counted.
pub const QUEUE: usize = 4096;

struct Inner {
    db: Database,
    names: Mutex<Names>,
    errors: AtomicU64,
    last_error_at: AtomicI64,
    keep_hours_s: AtomicI64,
    tx: std::sync::mpsc::SyncSender<Cmd>,
}

/// The history store. Cheap to clone.
#[derive(Clone)]
pub struct History(Arc<Inner>);

type Res<T> = Result<T, String>;

/// Per series id: (min, sum, max, count) over one hour.
type Acc = BTreeMap<u32, (f64, f64, f64, u32)>;

fn e<E: std::fmt::Display>(x: E) -> String {
    x.to_string()
}

fn enc_min(v: &[(u32, f64)]) -> Vec<u8> {
    let mut b = Vec::with_capacity(v.len() * 12);
    for (id, x) in v {
        b.extend_from_slice(&id.to_le_bytes());
        b.extend_from_slice(&x.to_le_bytes());
    }
    b
}

fn dec_min(b: &[u8]) -> Vec<(u32, f64)> {
    b.as_chunks::<12>()
        .0
        .iter()
        .map(|c| {
            (
                u32::from_le_bytes(c[0..4].try_into().unwrap()),
                f64::from_le_bytes(c[4..12].try_into().unwrap()),
            )
        })
        .collect()
}

fn enc_hour(v: &[(u32, f64, f64, f64)]) -> Vec<u8> {
    let mut b = Vec::with_capacity(v.len() * 28);
    for (id, lo, avg, hi) in v {
        b.extend_from_slice(&id.to_le_bytes());
        for x in [lo, avg, hi] {
            b.extend_from_slice(&x.to_le_bytes());
        }
    }
    b
}

fn dec_hour(b: &[u8]) -> Vec<(u32, f64, f64, f64)> {
    let f = |c: &[u8]| f64::from_le_bytes(c.try_into().unwrap());
    b.as_chunks::<28>()
        .0
        .iter()
        .map(|c| {
            (
                u32::from_le_bytes(c[0..4].try_into().unwrap()),
                f(&c[4..12]),
                f(&c[12..20]),
                f(&c[20..28]),
            )
        })
        .collect()
}

impl History {
    /// Open or create `history.redb` in `data_dir` and start its writer thread.
    pub fn open(data_dir: &str) -> Res<History> {
        let path = std::path::Path::new(data_dir).join("history.redb");
        let db = Database::create(&path).map_err(|x| format!("{}: {x}", path.display()))?;
        // create the tables and load the name index
        let w = db.begin_write().map_err(e)?;
        {
            w.open_table(MINUTE).map_err(e)?;
            w.open_table(HOUR).map_err(e)?;
            w.open_table(META).map_err(e)?;
            w.open_table(NAMES).map_err(e)?;
            w.open_table(HOST_SERIES).map_err(e)?;
        }
        w.commit().map_err(e)?;
        let mut names = Names {
            by_name: HashMap::new(),
            by_id: HashMap::new(),
            host_series: BTreeSet::new(),
            next_id: 0,
        };
        {
            let r = db.begin_read().map_err(e)?;
            for row in r.open_table(NAMES).map_err(e)?.iter().map_err(e)? {
                let (k, v) = row.map_err(e)?;
                names.by_name.insert(k.value().to_string(), v.value());
                names.by_id.insert(v.value(), k.value().to_string());
                names.next_id = names.next_id.max(v.value() + 1);
            }
            for row in r.open_table(HOST_SERIES).map_err(e)?.iter().map_err(e)? {
                let (k, _) = row.map_err(e)?;
                let (h, id) = k.value();
                names.host_series.insert((h.to_string(), id));
            }
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(QUEUE);
        let inner = Arc::new(Inner {
            db,
            names: Mutex::new(names),
            errors: AtomicU64::new(0),
            last_error_at: AtomicI64::new(0),
            keep_hours_s: AtomicI64::new(HOUR_KEEP_S),
            tx,
        });
        // The writer holds only a weak reference: when the last History is
        // dropped, the sender goes with it and the thread ends, closing the file.
        let weak = Arc::downgrade(&inner);
        std::thread::Builder::new()
            .name("warte-history".into())
            .spawn(move || {
                while let Ok(cmd) = rx.recv() {
                    let Some(inner) = weak.upgrade() else { break };
                    let h = History(inner);
                    let reply = match cmd {
                        Cmd::Record(host, t, series) => {
                            if let Err(x) = h.write_sample(&host, t, &series) {
                                h.fail("record", x);
                            }
                            None
                        }
                        Cmd::Maintain(now, r) => {
                            if let Err(x) = h.try_maintain(now) {
                                h.fail("maintain", x);
                            }
                            Some(r)
                        }
                        Cmd::Flush(r) => {
                            if let Err(x) = h.try_flush() {
                                h.fail("flush", x);
                            }
                            Some(r)
                        }
                        Cmd::Meta(k, v, r) => {
                            if let Err(x) = h.try_set_meta(&k, v) {
                                h.fail("meta", x);
                            }
                            Some(r)
                        }
                        Cmd::Sync(r) => Some(r),
                    };
                    // let go of the store before answering, so a caller that
                    // drops its last handle next really closes the file
                    drop(h);
                    if let Some(r) = reply {
                        let _ = r.send(());
                    }
                }
            })
            .map_err(e)?;
        Ok(History(inner))
    }

    /// Send `cmd` and wait until the writer has done it and everything before it.
    fn ask(&self, cmd: impl FnOnce(std::sync::mpsc::SyncSender<()>) -> Cmd) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        if self.0.tx.send(cmd(tx)).is_ok() {
            let _ = rx.recv();
        }
    }

    /// Keep hourly rows this many days (at least 8, so they outlive minutes).
    pub fn set_keep_days(&self, days: u32) {
        self.0
            .keep_hours_s
            .store(i64::from(days.max(8)) * 86_400, Ordering::Relaxed);
    }

    fn fail(&self, what: &str, err: String) {
        let n = self.0.errors.fetch_add(1, Ordering::Relaxed) + 1;
        self.0
            .last_error_at
            .store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
        if n <= 10 || n.is_power_of_two() {
            tracing::warn!(error = %err, so_far = n, "machine history: {what} failed");
        }
    }

    /// Store errors since start (dropped samples included).
    pub fn errors(&self) -> u64 {
        self.0.errors.load(Ordering::Relaxed)
    }

    /// When the last store error happened (unix seconds), 0 if never.
    pub fn last_error_at(&self) -> i64 {
        self.0.last_error_at.load(Ordering::Relaxed)
    }

    /// One amber finding while the store has failed within the last hour.
    pub fn finding(&self, now: i64) -> Option<crate::probe::Finding> {
        let n = self.errors();
        (n > 0 && now - self.last_error_at() < 3600).then(|| crate::probe::Finding {
            host: "warte".into(),
            severity: "amber",
            text: format!(
                "machine history is not being written ({n} store error(s)); see the server log"
            ),
        })
    }

    /// Queue one sample and return at once: the report path never waits for
    /// the store. `t` is bucketed to the minute; per series, a later sample in
    /// the same minute replaces an earlier one, other series in that minute
    /// are kept. A full queue drops the sample and counts it as an error.
    pub fn record(&self, host: &str, t: i64, series: &[(String, f64)]) {
        match self
            .0
            .tx
            .try_send(Cmd::Record(host.to_string(), t, series.to_vec()))
        {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                self.fail("record", "queue full, sample dropped".into())
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                self.fail("record", "writer stopped".into())
            }
        }
    }

    /// Queue one sample, waiting for room: for bulk imports, never for reports.
    pub fn record_waiting(&self, host: &str, t: i64, series: &[(String, f64)]) {
        if self
            .0
            .tx
            .send(Cmd::Record(host.to_string(), t, series.to_vec()))
            .is_err()
        {
            self.fail("record", "writer stopped".into());
        }
    }

    /// Wait until every sample queued so far is written (not yet durable).
    pub fn sync(&self) {
        self.ask(Cmd::Sync);
    }

    fn write_sample(&self, host: &str, t: i64, series: &[(String, f64)]) -> Res<()> {
        let t = t - t.rem_euclid(60);
        let mut w = self.0.db.begin_write().map_err(e)?;
        w.set_durability(Durability::None).map_err(e)?;
        // names new to the store; applied to memory only after the commit
        let mut new_names: Vec<(String, u32)> = Vec::new();
        let mut new_host_series: Vec<u32> = Vec::new();
        {
            let names = self.0.names.lock().unwrap_or_else(|p| p.into_inner());
            let mut nt = w.open_table(NAMES).map_err(e)?;
            let mut hs = w.open_table(HOST_SERIES).map_err(e)?;
            let mut row = Vec::with_capacity(series.len());
            for (name, v) in series {
                if !v.is_finite() {
                    continue;
                }
                let id = match names.by_name.get(name) {
                    Some(id) => *id,
                    None => match new_names.iter().find(|(n, _)| n == name) {
                        Some((_, id)) => *id,
                        None => {
                            let id = names.next_id + new_names.len() as u32;
                            nt.insert(name.as_str(), id).map_err(e)?;
                            new_names.push((name.clone(), id));
                            id
                        }
                    },
                };
                if !names.host_series.contains(&(host.to_string(), id))
                    && !new_host_series.contains(&id)
                {
                    hs.insert((host, id), ()).map_err(e)?;
                    new_host_series.push(id);
                }
                row.push((id, *v));
            }
            drop(names);
            // merge per series: a probe and the ground station may both
            // report the same host in the same minute
            let mut table = w.open_table(MINUTE).map_err(e)?;
            let old = table
                .get((host, t))
                .map_err(e)?
                .map(|g| dec_min(g.value()))
                .unwrap_or_default();
            for (id, x) in old {
                if !row.iter().any(|(i, _)| *i == id) {
                    row.push((id, x));
                }
            }
            table
                .insert((host, t), enc_min(&row).as_slice())
                .map_err(e)?;
            // a minute in an hour that is already rolled up and will not be
            // re-rolled (an import, a late report): mark it for the next run
            let mut meta = w.open_table(META).map_err(e)?;
            let rolled_to = meta.get("rolled_to").map_err(e)?.map(|g| g.value());
            if rolled_to.is_some_and(|r| t < r - 3600) {
                let dirty = meta.get("dirty_from").map_err(e)?.map(|g| g.value());
                if dirty.is_none_or(|d| t < d) {
                    meta.insert("dirty_from", t).map_err(e)?;
                }
            }
        }
        w.commit().map_err(e)?;
        let mut names = self.0.names.lock().unwrap_or_else(|p| p.into_inner());
        for (n, id) in new_names {
            names.by_id.insert(id, n.clone());
            names.by_name.insert(n, id);
            names.next_id = names.next_id.max(id + 1);
        }
        for id in new_host_series {
            names.host_series.insert((host.to_string(), id));
        }
        Ok(())
    }

    /// Make every sample queued so far durable.
    pub fn flush(&self) {
        self.ask(Cmd::Flush);
    }

    fn try_flush(&self) -> Res<()> {
        let w = self.0.db.begin_write().map_err(e)?;
        w.commit().map_err(e)
    }

    /// Roll finished hours up to min/avg/max and drop rows past retention.
    /// Idempotent. Re-rolls the hour before the last rolled one, and every
    /// hour from the earliest minute written into an already-rolled hour.
    /// Waits for the samples queued before it.
    pub fn maintain(&self, now: i64) {
        self.ask(|r| Cmd::Maintain(now, r));
    }

    fn try_maintain(&self, now: i64) -> Res<()> {
        let end = now - now.rem_euclid(3600); // hours before this are finished
        let mut w = self.0.db.begin_write().map_err(e)?;
        w.set_durability(Durability::None).map_err(e)?;
        {
            let mut meta = w.open_table(META).map_err(e)?;
            let rolled_to = meta.get("rolled_to").map_err(e)?.map(|g| g.value());
            let dirty = meta.get("dirty_from").map_err(e)?.map(|g| g.value());
            let mut minute = w.open_table(MINUTE).map_err(e)?;
            let mut hour = w.open_table(HOUR).map_err(e)?;
            let mut from = rolled_to.map(|r| r - 3600).unwrap_or(i64::MIN);
            if let Some(d) = dirty {
                from = from.min(d - d.rem_euclid(3600));
            }
            let mut acc: BTreeMap<(String, i64), Acc> = BTreeMap::new();
            for row in minute.iter().map_err(e)? {
                let (k, v) = row.map_err(e)?;
                let (h, t) = k.value();
                if t < from || t >= end {
                    continue;
                }
                let slot = acc
                    .entry((h.to_string(), t - t.rem_euclid(3600)))
                    .or_default();
                for (id, x) in dec_min(v.value()) {
                    let a = slot
                        .entry(id)
                        .or_insert((f64::INFINITY, 0.0, f64::NEG_INFINITY, 0));
                    a.0 = a.0.min(x);
                    a.1 += x;
                    a.2 = a.2.max(x);
                    a.3 += 1;
                }
            }
            for ((h, t), ids) in acc {
                let row: Vec<_> = ids
                    .into_iter()
                    .map(|(id, (lo, sum, hi, n))| (id, lo, sum / f64::from(n), hi))
                    .collect();
                hour.insert((h.as_str(), t), enc_hour(&row).as_slice())
                    .map_err(e)?;
            }
            if end > rolled_to.unwrap_or(i64::MIN) {
                meta.insert("rolled_to", end).map_err(e)?;
            }
            if dirty.is_some() {
                meta.remove("dirty_from").map_err(e)?;
            }
            let keep_h = self.0.keep_hours_s.load(Ordering::Relaxed);
            minute
                .retain(|(_, t), _| t >= now - MINUTE_KEEP_S && t <= now + 86_400)
                .map_err(e)?;
            hour.retain(|(_, t), _| t >= now - keep_h && t <= now + 86_400)
                .map_err(e)?;
        }
        w.commit().map_err(e)
    }

    /// A one-off marker, e.g. that the ground station's points were imported.
    pub fn meta(&self, key: &str) -> Option<i64> {
        let r = self.0.db.begin_read().ok()?;
        let t = r.open_table(META).ok()?;
        t.get(key).ok().flatten().map(|g| g.value())
    }

    /// Set a marker, after every sample queued before it.
    pub fn set_meta(&self, key: &str, value: i64) {
        self.ask(|r| Cmd::Meta(key.to_string(), value, r));
    }

    fn try_set_meta(&self, key: &str, value: i64) -> Res<()> {
        let w = self.0.db.begin_write().map_err(e)?;
        w.open_table(META)
            .map_err(e)?
            .insert(key, value)
            .map_err(e)?;
        w.commit().map_err(e)
    }

    /// Hosts with any series, sorted.
    pub fn hosts(&self) -> Vec<String> {
        let names = self.0.names.lock().unwrap_or_else(|p| p.into_inner());
        let set: BTreeSet<&String> = names.host_series.iter().map(|(h, _)| h).collect();
        set.into_iter().cloned().collect()
    }

    /// Series names a host has sent, sorted.
    pub fn metrics(&self, host: &str) -> Vec<String> {
        let names = self.0.names.lock().unwrap_or_else(|p| p.into_inner());
        let mut v: Vec<String> = names
            .host_series
            .iter()
            .filter(|(h, _)| h == host)
            .filter_map(|(_, id)| names.by_id.get(id).cloned())
            .collect();
        v.sort();
        v
    }

    /// The given series of `host` over `[from, to]`: per minute when the range
    /// is at most 7 days, per hour (with min and max) beyond. Times outside
    /// 0 .. 2^40 s, an inverted range or one over 400 days answer nothing.
    pub fn series(&self, host: &str, metrics: &[String], from: i64, to: i64) -> Vec<Series> {
        match self.try_series(host, metrics, from, to) {
            Ok(s) => s,
            Err(x) => {
                self.fail("read", x);
                Vec::new()
            }
        }
    }

    fn try_series(&self, host: &str, metrics: &[String], from: i64, to: i64) -> Res<Vec<Series>> {
        const T_MAX: i64 = 1 << 40;
        if !(0..=T_MAX).contains(&from) || !(0..=T_MAX).contains(&to) {
            return Ok(Vec::new());
        }
        if to < from || to - from > MAX_RANGE_S {
            return Ok(Vec::new());
        }
        let hourly = to - from > MINUTE_RANGE_S;
        let step = if hourly { 3600 } else { 60 };
        let t0 = from - from.rem_euclid(step);
        let n = ((to - t0) / step + 1).max(0) as usize;
        let ids: Vec<Option<u32>> = {
            let names = self.0.names.lock().unwrap_or_else(|p| p.into_inner());
            metrics
                .iter()
                .map(|m| names.by_name.get(m).copied())
                .collect()
        };
        let mut vals = vec![vec![None; n]; metrics.len()];
        let mut mins = vec![vec![None; n]; metrics.len()];
        let mut maxs = vec![vec![None; n]; metrics.len()];
        let r = self.0.db.begin_read().map_err(e)?;
        let table = r
            .open_table(if hourly { HOUR } else { MINUTE })
            .map_err(e)?;
        for row in table.range((host, t0)..=(host, to)).map_err(e)? {
            let (k, v) = row.map_err(e)?;
            let i = ((k.value().1 - t0) / step) as usize;
            if i >= n {
                continue;
            }
            if hourly {
                for (id, lo, avg, hi) in dec_hour(v.value()) {
                    for (j, want) in ids.iter().enumerate() {
                        if *want == Some(id) {
                            vals[j][i] = Some(avg);
                            mins[j][i] = Some(lo);
                            maxs[j][i] = Some(hi);
                        }
                    }
                }
            } else {
                for (id, x) in dec_min(v.value()) {
                    for (j, want) in ids.iter().enumerate() {
                        if *want == Some(id) {
                            vals[j][i] = Some(x);
                        }
                    }
                }
            }
        }
        Ok(metrics
            .iter()
            .enumerate()
            .map(|(j, m)| Series {
                metric: m.clone(),
                unit: unit(m),
                t0,
                step,
                values: std::mem::take(&mut vals[j]),
                min: hourly.then(|| std::mem::take(&mut mins[j])),
                max: hourly.then(|| std::mem::take(&mut maxs[j])),
            })
            .collect())
    }

    /// Roll up at start and just after every full hour, flush every
    /// `FLUSH_EVERY_S`, until the process ends.
    pub fn spawn_upkeep(&self) {
        let h = self.clone();
        tokio::spawn(async move {
            let mut rolled_hour = i64::MIN;
            loop {
                let now = chrono::Utc::now().timestamp();
                let hour = now.div_euclid(3600);
                let h2 = h.clone();
                let roll = hour != rolled_hour;
                let _ = tokio::task::spawn_blocking(move || {
                    if roll {
                        h2.maintain(now);
                    }
                    h2.flush();
                })
                .await;
                rolled_hour = hour;
                // wake at the next 5-minute mark or just after the hour, whichever is first
                let to_hour = 3600 - now.rem_euclid(3600) + 30;
                let wait = (FLUSH_EVERY_S as i64).min(to_hour).max(1) as u64;
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            }
        });
    }
}

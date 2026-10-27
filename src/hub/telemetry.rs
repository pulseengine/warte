//! Agent -> hub telemetry as CCSDS space packets, using pulseengine/relay's
//! `relay-ccsds` codec. The CI host is the spacecraft, fourpi the ground
//! station.
//!
//! One APID per stream, each with its own 14-bit sequence counter, so the
//! hub can count LOST packets per stream rather than just noticing silence.
//! Packet = 6-byte CCSDS primary header + payload + 1-byte XOR checksum
//! (relay's `compute_checksum`). relay's `validate_packet` only decodes the
//! header, so `open` additionally checks exact length and checksum.
//!
//! "Real but quick": fixed binary layouts where they are trivial (host and
//! runner housekeeping), compact JSON inside its own packet for the
//! variable-shaped streams (queue, automation log lines, frame metadata).
use crate::hub::model::*;
use relay_ccsds::engine::{
    compute_checksum, decode_header, encode_header, CcsdsHeader, PacketType,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const APID_HOST: u16 = 0x100;
pub const APID_RUNNER_BASE: u16 = 0x110; // + runner id (1..=0x6F)
pub const APID_QUEUE: u16 = 0x200;
/// Queue detail (biggest queued runs + hosted groups): its own packet so the
/// headline queue numbers never get squeezed out by lists.
pub const APID_QUEUE_DETAIL: u16 = 0x201;
/// Hosted-job list, split over chunk packets 0x210..=0x21F (index = apid - base).
pub const APID_HOSTED_BASE: u16 = 0x210;
pub const HOSTED_CHUNKS: u16 = 16;
/// Queued-job list (what /api/eta searches), chunk packets 0x220..=0x22F.
pub const APID_QUEUED_BASE: u16 = 0x220;
pub const QUEUED_CHUNKS: u16 = 16;
/// Conveyor-belt ledger: finished-job records (each packet a standalone JSON
/// array, so a lost packet loses only its own records) and per-day manifests.
pub const APID_LEDGER: u16 = 0x230;
pub const APID_MANIFEST: u16 = 0x231;
pub const APID_TIMERS: u16 = 0x300;
pub const APID_DISPATCHER: u16 = 0x301;
pub const APID_WATCHDOG: u16 = 0x302;
pub const APID_BAZEL: u16 = 0x303;
pub const APID_TRACE: u16 = 0x304;
/// Sent LAST in every burst: tells the hub a snapshot is complete.
pub const APID_FRAME_END: u16 = 0x3F0;
/// Stay under the WireGuard MTU so no packet is IP-fragmented.
pub const MAX_PACKET: usize = 1300;
const MAX_PAYLOAD: usize = MAX_PACKET - 7;
const SEQ_MOD: u32 = 1 << 14;

pub fn apid_name(apid: u16) -> String {
    match apid {
        APID_HOST => "host".into(),
        APID_QUEUE => "queue".into(),
        APID_QUEUE_DETAIL => "queue-detail".into(),
        APID_LEDGER => "ledger".into(),
        APID_MANIFEST => "ledger-manifest".into(),
        APID_TIMERS => "timers".into(),
        APID_DISPATCHER => "dispatcher".into(),
        APID_WATCHDOG => "watchdog".into(),
        APID_BAZEL => "bazel-sweep".into(),
        APID_TRACE => "trace-rollout".into(),
        APID_FRAME_END => "frame-end".into(),
        a if (APID_HOSTED_BASE..APID_HOSTED_BASE + HOSTED_CHUNKS).contains(&a) => {
            format!("hosted{}", a - APID_HOSTED_BASE)
        }
        a if (APID_QUEUED_BASE..APID_QUEUED_BASE + QUEUED_CHUNKS).contains(&a) => {
            format!("queued{}", a - APID_QUEUED_BASE)
        }
        a if a > APID_RUNNER_BASE && a < APID_QUEUE => format!("runner{}", a - APID_RUNNER_BASE),
        a => format!("0x{a:03x}"),
    }
}

/// Wrap a payload as one standalone CCSDS telemetry packet.
pub fn packet(apid: u16, seq: u16, payload: &[u8]) -> Vec<u8> {
    let hdr = CcsdsHeader {
        version: 0,
        packet_type: PacketType::Telemetry,
        sec_header_flag: false,
        apid,
        sequence_flags: 0b11,
        sequence_count: seq & 0x3FFF,
        // CCSDS length field = (data field octets) - 1; data field = payload + checksum.
        data_length: payload.len() as u16,
    };
    let mut h = [0u8; 6];
    encode_header(&hdr, &mut h);
    let mut v = Vec::with_capacity(7 + payload.len());
    v.extend_from_slice(&h);
    v.extend_from_slice(payload);
    let c = compute_checksum(&v);
    v.push(c);
    v
}

/// Validate a received packet; returns header and payload.
pub fn open(buf: &[u8]) -> Result<(CcsdsHeader, &[u8]), String> {
    let h = decode_header(buf).map_err(|e| format!("header: {e:?}"))?;
    if h.packet_type != PacketType::Telemetry {
        return Err("not a telemetry packet".into());
    }
    let want = 6 + h.data_length as usize + 1;
    if buf.len() != want {
        return Err(format!(
            "length mismatch: header says {want}, got {}",
            buf.len()
        ));
    }
    if compute_checksum(&buf[..want - 1]) != buf[want - 1] {
        return Err("checksum mismatch".into());
    }
    Ok((h, &buf[6..want - 1]))
}

#[derive(Default)]
pub struct SeqCounters(HashMap<u16, u16>);
impl SeqCounters {
    pub fn next(&mut self, apid: u16) -> u16 {
        let c = self.0.entry(apid).or_insert(0);
        let v = *c;
        *c = ((*c as u32 + 1) % SEQ_MOD) as u16;
        v
    }
}

// ---- tiny binary writer/reader -------------------------------------------------
struct W(Vec<u8>);
impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v)
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_be_bytes())
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes())
    }
    fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_be_bytes())
    }
    fn s8(&mut self, s: &str) {
        let b = trunc(s, 255);
        self.u8(b.len() as u8);
        self.0.extend_from_slice(b)
    }
    /// length-prefixed string capped at `cap` bytes (keeps every packet under MAX_PACKET)
    fn sc(&mut self, s: &str, cap: usize) {
        let b = trunc(s, cap.min(255));
        self.u8(b.len() as u8);
        self.0.extend_from_slice(b)
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes())
    }
    fn c100(&mut self, f: f64) {
        self.u32((f.max(0.0) * 100.0).round().min(u32::MAX as f64) as u32)
    }
}
/// Truncate to at most `n` bytes on a UTF-8 char boundary.
fn trunc(s: &str, n: usize) -> &[u8] {
    let mut end = s.len().min(n);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s.as_bytes()[..end]
}
struct R<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.i..self.i + n)?;
        self.i += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }
    fn s8(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        Some(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn c100(&mut self) -> Option<f64> {
        Some(self.u32()? as f64 / 100.0)
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }
}

// ---- host housekeeping (APID 0x100) --------------------------------------------
pub fn enc_host(t: i64, m: &Machine) -> Vec<u8> {
    let mut w = W(vec![]);
    w.i64(t);
    w.u16(m.ncpu as u16);
    for l in m.load {
        w.c100(l)
    }
    for v in [
        m.mem_total_mb,
        m.mem_avail_mb,
        m.swap_total_mb,
        m.swap_used_mb,
    ] {
        w.u32(v as u32)
    }
    for p in [m.psi.cpu, m.psi.memory, m.psi.io] {
        w.c100(p.avg10);
        w.c100(p.avg60);
        w.c100(p.avg300)
    }
    w.u8(m.disks.len().min(8) as u8);
    for d in m.disks.iter().take(8) {
        w.s8(&d.mount);
        w.c100(d.used_pct);
        w.c100(d.size_gb);
        w.c100(d.used_gb)
    }
    w.0
}
pub fn dec_host(b: &[u8]) -> Option<(i64, Machine)> {
    let mut r = R { b, i: 0 };
    let t = r.i64()?;
    let mut m = Machine {
        ncpu: r.u16()? as u32,
        ..Default::default()
    };
    m.load = [r.c100()?, r.c100()?, r.c100()?];
    m.mem_total_mb = r.u32()? as u64;
    m.mem_avail_mb = r.u32()? as u64;
    m.swap_total_mb = r.u32()? as u64;
    m.swap_used_mb = r.u32()? as u64;
    let mut psi = [PsiLine::default(); 3];
    for p in psi.iter_mut() {
        *p = PsiLine {
            avg10: r.c100()?,
            avg60: r.c100()?,
            avg300: r.c100()?,
        }
    }
    m.psi = Psi {
        cpu: psi[0],
        memory: psi[1],
        io: psi[2],
    };
    for _ in 0..r.u8()? {
        m.disks.push(Disk {
            mount: r.s8()?,
            used_pct: r.c100()?,
            size_gb: r.c100()?,
            used_gb: r.c100()?,
        });
    }
    Some((t, m))
}

// ---- runner housekeeping (APID 0x110 + id) -------------------------------------
const F_ACTIVE: u8 = 1;
const F_BUSY: u8 = 2;
const F_GH: u8 = 4;
const F_GH_ONLINE: u8 = 8;
const F_GH_BUSY: u8 = 16;
pub fn enc_runner(t: i64, r: &Runner) -> Vec<u8> {
    let mut f = 0u8;
    if r.active {
        f |= F_ACTIVE
    }
    if r.local_busy {
        f |= F_BUSY
    }
    if let Some(s) = &r.gh_status {
        f |= F_GH;
        if s == "online" {
            f |= F_GH_ONLINE
        }
    }
    if r.gh_busy == Some(true) {
        f |= F_GH_BUSY
    }
    let mut w = W(vec![]);
    w.i64(t);
    w.u16(r.id as u16);
    w.u8(f);
    w.u32(r.pid);
    w.i64(r.job_started.unwrap_or(0));
    // Per-field caps guarantee the worst case still fits one packet (see test).
    w.sc(r.job.as_deref().unwrap_or(""), 200);
    w.sc(r.last_result.as_deref().unwrap_or(""), 32);
    w.sc(r.trace.as_deref().unwrap_or(""), 16);
    w.u8(r.labels.len().min(10) as u8);
    for l in r.labels.iter().take(10) {
        w.sc(l, 24)
    }
    match &r.ctx {
        None => w.u8(0),
        Some(c) => {
            w.u8(1);
            w.sc(&c.repo, 80);
            w.sc(&c.workflow, 80);
            w.u64(c.run_id);
            w.u32(c.run_number);
            w.u8(c.run_attempt.min(255) as u8);
            w.sc(&c.event, 32);
            w.sc(&c.branch, 100);
            w.sc(&c.base, 50);
            w.sc(&c.sha, 40);
            w.sc(&c.actor, 40);
            w.u32(c.pr.unwrap_or(0));
            w.sc(c.pr_title.as_deref().unwrap_or(""), 160);
        }
    }
    // cgroup stats, appended LAST so a hub without them still decodes the rest.
    // cpu in centi-cores (u16::MAX = unknown), pressure in centi-percent.
    match &r.stats {
        None => w.u8(0),
        Some(s) => {
            w.u8(1);
            w.u16(
                s.cpu_cores
                    .map(|c| (c * 100.0).round().clamp(0.0, 65534.0) as u16)
                    .unwrap_or(u16::MAX),
            );
            w.u32(s.mem_mb);
            w.u32(s.mem_peak_mb);
            w.u32(s.mem_max_mb.unwrap_or(u32::MAX));
            w.u32(s.pids);
            w.u16((s.psi_cpu * 100.0).round().clamp(0.0, 10000.0) as u16);
            w.u16((s.psi_mem * 100.0).round().clamp(0.0, 10000.0) as u16);
            w.u32(s.oom_kills);
        }
    }
    w.0
}
pub fn dec_runner(b: &[u8]) -> Option<(i64, Runner)> {
    let mut r = R { b, i: 0 };
    let t = r.i64()?;
    let id = r.u16()? as u32;
    let f = r.u8()?;
    let pid = r.u32()?;
    let js = r.i64()?;
    let (job, last, trace) = (r.s8()?, r.s8()?, r.s8()?);
    let mut labels = vec![];
    for _ in 0..r.u8()? {
        labels.push(r.s8()?)
    }
    let opt = |s: String| if s.is_empty() { None } else { Some(s) };
    let ctx = if r.u8()? == 1 {
        let (repo, workflow, run_id, run_number, run_attempt) =
            (r.s8()?, r.s8()?, r.u64()?, r.u32()?, r.u8()? as u32);
        let (event, branch, base, sha, actor) = (r.s8()?, r.s8()?, r.s8()?, r.s8()?, r.s8()?);
        let pr = r.u32()?;
        let title = r.s8()?;
        Some(JobContext {
            repo,
            workflow,
            run_id,
            run_number,
            run_attempt,
            event,
            branch,
            base,
            sha,
            actor,
            pr: if pr == 0 { None } else { Some(pr) },
            pr_title: opt(title),
        })
    } else {
        None
    };
    // optional trailer: an older agent simply has no stats
    let stats = (|| {
        if r.u8()? != 1 {
            return None;
        }
        let cpu = r.u16()?;
        let (mem_mb, mem_peak_mb, max, pids) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
        let (pc, pm, oom_kills) = (r.u16()?, r.u16()?, r.u32()?);
        Some(CgStats {
            cpu_cores: if cpu == u16::MAX {
                None
            } else {
                Some(cpu as f64 / 100.0)
            },
            mem_mb,
            mem_peak_mb,
            mem_max_mb: if max == u32::MAX { None } else { Some(max) },
            pids,
            psi_cpu: pc as f64 / 100.0,
            psi_mem: pm as f64 / 100.0,
            oom_kills,
        })
    })();
    Some((
        t,
        Runner {
            id,
            unit: format!("github-runner@runner{id}.service"),
            active: f & F_ACTIVE != 0,
            pid,
            local_busy: f & F_BUSY != 0,
            job: opt(job),
            job_started: if js == 0 { None } else { Some(js) },
            last_result: opt(last),
            trace: opt(trace),
            gh_status: if f & F_GH != 0 {
                Some(
                    if f & F_GH_ONLINE != 0 {
                        "online"
                    } else {
                        "offline"
                    }
                    .into(),
                )
            } else {
                None
            },
            gh_busy: if f & F_GH != 0 {
                Some(f & F_GH_BUSY != 0)
            } else {
                None
            },
            labels,
            ctx,
            stats,
        },
    ))
}

// ---- JSON streams --------------------------------------------------------------
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct FrameMeta {
    pub agent_version: String,
    pub host: String,
    pub taken_at: i64,
    pub errors: Vec<String>,
}

/// Serialise; if too big, let `shrink` drop content until it fits.
fn fit<T: Serialize>(v: &mut T, mut shrink: impl FnMut(&mut T) -> bool) -> Vec<u8> {
    loop {
        let b = serde_json::to_vec(v).unwrap_or_default();
        if b.len() <= MAX_PAYLOAD || !shrink(v) {
            return b.into_iter().take(MAX_PAYLOAD).collect();
        }
    }
}
/// The queue packet carries everything EXCEPT the hosted list (that goes in chunks).
pub fn enc_queue(q: &Queue) -> Vec<u8> {
    let mut q = q.clone();
    q.hosted.clear();
    q.queued.clear();
    q.top_runs.clear();
    q.hosted_groups.clear();
    // only pools/waits remain; drop the smallest-named extras if ever needed
    fit(&mut q, |q| {
        (q.waits.len() > 1 && {
            let k = q.waits.keys().last().cloned().unwrap();
            q.waits.remove(&k);
            true
        }) || (q.pools.len() > 1 && {
            let k = q.pools.keys().last().cloned().unwrap();
            q.pools.remove(&k);
            true
        })
    })
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct QueueDetail {
    pub top_runs: Vec<QueuedRun>,
    pub hosted_groups: Vec<HostedGroup>,
}

/// Lists are sorted biggest-first, so shrinking pops the least important rows.
pub fn enc_queue_detail(q: &Queue) -> Vec<u8> {
    let mut d = QueueDetail {
        top_runs: q
            .top_runs
            .iter()
            .map(|r| QueuedRun {
                repo: cap(&r.repo, 60),
                workflow: cap(&r.workflow, 60),
                event: cap(&r.event, 24),
                ..r.clone()
            })
            .collect(),
        hosted_groups: q
            .hosted_groups
            .iter()
            .map(|g| HostedGroup {
                repo: cap(&g.repo, 60),
                labels: cap(&g.labels, 40),
                ..g.clone()
            })
            .collect(),
    };
    fit(&mut d, |d| {
        if d.hosted_groups.len() > d.top_runs.len() {
            d.hosted_groups.pop().is_some()
        } else {
            d.top_runs.pop().is_some() || d.hosted_groups.pop().is_some()
        }
    })
}
#[derive(Serialize, Deserialize)]
struct Chunk<T> {
    i: u16,
    n: u16,
    items: Vec<T>,
}

/// Greedily pack items into as few packets as fit, at most `max_chunks`; items
/// that do not fit are dropped (the totals travel separately).
fn chunks<T: Serialize + Clone>(items: &[T], max_chunks: u16) -> (Vec<Vec<u8>>, usize) {
    let mut cs: Vec<Vec<T>> = vec![vec![]];
    for it in items {
        let last = cs.last_mut().unwrap();
        last.push(it.clone());
        if serde_json::to_vec(&Chunk {
            i: 0,
            n: 0,
            items: last.clone(),
        })
        .unwrap_or_default()
        .len()
            > MAX_PAYLOAD - 16
        {
            let it = last.pop().unwrap();
            if cs.len() == max_chunks as usize {
                break;
            }
            cs.push(vec![it]);
        }
    }
    let n = cs.len() as u16;
    let carried = cs.iter().map(Vec::len).sum();
    (
        cs.into_iter()
            .enumerate()
            .map(|(i, items)| {
                serde_json::to_vec(&Chunk {
                    i: i as u16,
                    n,
                    items,
                })
                .unwrap_or_default()
            })
            .collect(),
        carried,
    )
}

fn cap(s: &str, n: usize) -> String {
    String::from_utf8_lossy(trunc(s, n)).into_owned()
}

/// Greedily pack items into standalone JSON-array payloads, each <= MAX_PAYLOAD.
fn pack<T: Serialize + Clone>(items: &[T]) -> Vec<Vec<u8>> {
    let mut out = vec![];
    let mut cur: Vec<T> = vec![];
    for it in items {
        cur.push(it.clone());
        if serde_json::to_vec(&cur).unwrap_or_default().len() > MAX_PAYLOAD {
            let it = cur.pop().unwrap();
            if !cur.is_empty() {
                out.push(serde_json::to_vec(&cur).unwrap_or_default())
            }
            cur = vec![it];
        }
    }
    if !cur.is_empty() {
        out.push(serde_json::to_vec(&cur).unwrap_or_default())
    }
    out
}

/// Ledger records + manifests as packets (sent before the snapshot burst).
pub fn encode_ledger(
    recs: &[JobRecord],
    manifests: &[DayManifest],
    seq: &mut SeqCounters,
) -> Vec<Vec<u8>> {
    let recs: Vec<JobRecord> = recs
        .iter()
        .map(|r| JobRecord {
            d: cap(&r.d, 10),
            repo: cap(&r.repo, 60),
            wf: cap(&r.wf, 60),
            job: cap(&r.job, 80),
            pool: cap(&r.pool, 40),
            runner: cap(&r.runner, 40),
            ev: cap(&r.ev, 24),
            r: cap(&r.r, 16),
            ..r.clone()
        })
        .collect();
    let mut out = vec![];
    for (apid, payloads) in [(APID_LEDGER, pack(&recs)), (APID_MANIFEST, pack(manifests))] {
        for p in payloads {
            out.push(packet(apid, seq.next(apid), &p));
        }
    }
    out
}

/// Long strings are capped first so a single item can never exceed a packet.
pub fn enc_hosted(jobs: &[HostedJob]) -> (Vec<Vec<u8>>, usize) {
    let jobs: Vec<HostedJob> = jobs
        .iter()
        .map(|j| HostedJob {
            repo: cap(&j.repo, 80),
            workflow: cap(&j.workflow, 60),
            job: cap(&j.job, 80),
            status: cap(&j.status, 16),
            runner: cap(&j.runner, 40),
            labels: cap(&j.labels, 60),
            branch: cap(&j.branch, 60),
            event: cap(&j.event, 24),
            ..j.clone()
        })
        .collect();
    chunks(&jobs, HOSTED_CHUNKS)
}
pub fn enc_queued(q: &[QueuedEntry]) -> (Vec<Vec<u8>>, usize) {
    let q: Vec<QueuedEntry> = q
        .iter()
        .map(|e| QueuedEntry {
            repo: cap(&e.repo, 60),
            workflow: cap(&e.workflow, 60),
            event: cap(&e.event, 24),
            branch: cap(&e.branch, 80),
            pool: cap(&e.pool, 40),
            ..e.clone()
        })
        .collect();
    chunks(&q, QUEUED_CHUNKS)
}
pub fn enc_lines(lines: &[String]) -> Vec<u8> {
    let mut v: Vec<String> = lines
        .iter()
        .map(|l| String::from_utf8_lossy(trunc(l, 300)).into_owned())
        .collect();
    fit(&mut v, |v| {
        if v.len() > 1 {
            v.remove(0);
            true
        } else {
            false
        }
    })
}
pub fn enc_timers(t: &[TimerState]) -> Vec<u8> {
    let mut t = t.to_vec();
    fit(&mut t, |t| t.pop().is_some())
}
pub fn enc_meta(m: &FrameMeta) -> Vec<u8> {
    let mut m = m.clone();
    fit(&mut m, |m| m.errors.pop().is_some())
}

/// Every packet for one snapshot, frame-end LAST.
pub fn encode_snapshot(s: &Snapshot, seq: &mut SeqCounters) -> Vec<Vec<u8>> {
    let t = s.taken_at;
    let mut out = vec![packet(
        APID_HOST,
        seq.next(APID_HOST),
        &enc_host(t, &s.machine),
    )];
    for r in &s.runners {
        let apid = APID_RUNNER_BASE + r.id as u16;
        out.push(packet(apid, seq.next(apid), &enc_runner(t, r)));
    }
    if let Some(q) = &s.queue {
        let (hc, _) = enc_hosted(&q.hosted);
        let (qc, carried) = enc_queued(&q.queued);
        // items that did not fit the chunk budget make the list incomplete
        let mut head = q.clone();
        if carried < q.queued.len() {
            head.queued_complete = false
        }
        out.push(packet(APID_QUEUE, seq.next(APID_QUEUE), &enc_queue(&head)));
        out.push(packet(
            APID_QUEUE_DETAIL,
            seq.next(APID_QUEUE_DETAIL),
            &enc_queue_detail(q),
        ));
        for (base, cs) in [(APID_HOSTED_BASE, hc), (APID_QUEUED_BASE, qc)] {
            for (i, c) in cs.iter().enumerate() {
                let apid = base + i as u16;
                out.push(packet(apid, seq.next(apid), c));
            }
        }
    }
    let a = &s.automation;
    out.push(packet(
        APID_TIMERS,
        seq.next(APID_TIMERS),
        &enc_timers(&a.timers),
    ));
    for (apid, lines) in [
        (APID_DISPATCHER, &a.dispatcher),
        (APID_WATCHDOG, &a.watchdog),
        (APID_BAZEL, &a.bazel_sweep),
        (APID_TRACE, &a.trace_rollout),
    ] {
        out.push(packet(apid, seq.next(apid), &enc_lines(lines)));
    }
    let meta = FrameMeta {
        agent_version: s.agent_version.clone(),
        host: s.host.clone(),
        taken_at: t,
        errors: s.errors.clone(),
    };
    out.push(packet(
        APID_FRAME_END,
        seq.next(APID_FRAME_END),
        &enc_meta(&meta),
    ));
    out
}

// ---- hub-side assembly + link accounting ---------------------------------------
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct LinkStat {
    pub received: u64,
    pub lost: u64,
    pub restarts: u64,
    pub last_seq: Option<u16>,
    pub last_seen: i64,
}

impl LinkStat {
    /// Account one packet. A sequence jump counts as LOST packets — unless the
    /// stream was silent for `restart_after` seconds (agent restarted and its
    /// counters reset), which is recorded as a restart instead.
    pub fn account(&mut self, seq: u16, now: i64, restart_after: i64) {
        if let Some(last) = self.last_seq {
            let expected = (last as u32 + 1) % SEQ_MOD;
            if seq as u32 != expected {
                let jump = (seq as u32 + SEQ_MOD - expected) % SEQ_MOD;
                if now - self.last_seen >= restart_after || jump > 1000 {
                    self.restarts += 1
                } else {
                    self.lost += jump as u64
                }
            }
        }
        self.received += 1;
        self.last_seq = Some(seq);
        self.last_seen = now;
    }
}

#[derive(Default)]
pub struct Assembler {
    host: Option<Machine>,
    runners: BTreeMap<u32, (i64, Runner)>,
    queue: Option<Queue>,
    /// Hosted chunks of the CURRENT frame, keyed by chunk index; `n` = expected count.
    hosted: BTreeMap<u16, Vec<HostedJob>>,
    queued: BTreeMap<u16, Vec<QueuedEntry>>,
    /// Ledger records received since the hub last drained them.
    pub ledger_in: Vec<JobRecord>,
    /// Latest manifest per ledger day.
    pub manifests: BTreeMap<String, DayManifest>,
    detail: Option<QueueDetail>,
    /// Chunk count the agent announced (0 = no queued chunk arrived at all).
    queued_n: u16,
    auto: Automation,
    pub link: BTreeMap<u16, LinkStat>,
    pub rejected: u64,
}

impl Assembler {
    /// Feed one packet; returns a complete Snapshot when a frame-end arrives.
    pub fn ingest(&mut self, buf: &[u8], now: i64) -> Option<Snapshot> {
        let (h, p) = match open(buf) {
            Ok(x) => x,
            Err(_) => {
                self.rejected += 1;
                return None;
            }
        };
        self.link
            .entry(h.apid)
            .or_default()
            .account(h.sequence_count, now, 300);
        match h.apid {
            APID_HOST => self.host = dec_host(p).map(|(_, m)| m),
            a if a > APID_RUNNER_BASE && a < APID_QUEUE => {
                if let Some((_, r)) = dec_runner(p) {
                    self.runners.insert(r.id, (now, r));
                }
            }
            APID_QUEUE => {
                self.queue = serde_json::from_slice(p).ok();
                self.hosted.clear();
                self.queued.clear();
                self.queued_n = 0;
                self.detail = None
            }
            APID_QUEUE_DETAIL => self.detail = serde_json::from_slice(p).ok(),
            APID_LEDGER => {
                if let Ok(v) = serde_json::from_slice::<Vec<JobRecord>>(p) {
                    self.ledger_in.extend(v)
                }
            }
            APID_MANIFEST => {
                if let Ok(v) = serde_json::from_slice::<Vec<DayManifest>>(p) {
                    for m in v {
                        self.manifests.insert(m.day.clone(), m);
                    }
                }
            }
            a if (APID_HOSTED_BASE..APID_HOSTED_BASE + HOSTED_CHUNKS).contains(&a) => {
                if let Ok(c) = serde_json::from_slice::<Chunk<HostedJob>>(p) {
                    self.hosted.insert(c.i, c.items);
                }
            }
            a if (APID_QUEUED_BASE..APID_QUEUED_BASE + QUEUED_CHUNKS).contains(&a) => {
                if let Ok(c) = serde_json::from_slice::<Chunk<QueuedEntry>>(p) {
                    self.queued_n = c.n;
                    self.queued.insert(c.i, c.items);
                }
            }
            APID_TIMERS => self.auto.timers = serde_json::from_slice(p).unwrap_or_default(),
            APID_DISPATCHER => self.auto.dispatcher = serde_json::from_slice(p).unwrap_or_default(),
            APID_WATCHDOG => self.auto.watchdog = serde_json::from_slice(p).unwrap_or_default(),
            APID_BAZEL => self.auto.bazel_sweep = serde_json::from_slice(p).unwrap_or_default(),
            APID_TRACE => self.auto.trace_rollout = serde_json::from_slice(p).unwrap_or_default(),
            APID_FRAME_END => {
                let m: FrameMeta = serde_json::from_slice(p).ok()?;
                // runners silent for 5 min are dropped rather than shown with stale state
                self.runners.retain(|_, (seen, _)| now - *seen < 300);
                // a lost chunk shows as fewer jobs (hosted_total still says how many exist)
                if let Some(q) = self.queue.as_mut() {
                    q.hosted = self.hosted.values().flatten().cloned().collect();
                    if let Some(d) = &self.detail {
                        q.top_runs = d.top_runs.clone();
                        q.hosted_groups = d.hosted_groups.clone();
                    }
                    q.queued = self.queued.values().flatten().cloned().collect();
                    // a lost chunk means the list is incomplete: never let /api/eta say "not queued" then
                    if self.queued_n == 0
                        || (0..self.queued_n).any(|i| !self.queued.contains_key(&i))
                    {
                        q.queued_complete = false
                    }
                }
                return Some(Snapshot {
                    agent_version: m.agent_version,
                    host: m.host,
                    taken_at: m.taken_at,
                    machine: self.host.clone().unwrap_or_default(),
                    runners: self.runners.values().map(|(_, r)| r.clone()).collect(),
                    queue: self.queue.clone(),
                    automation: self.auto.clone(),
                    errors: m.errors,
                });
            }
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(id: u32) -> Runner {
        Runner {
            id,
            unit: format!("github-runner@runner{id}.service"),
            active: true,
            pid: 4242,
            local_busy: true,
            job: Some("Mutation Testing (rivet-core 21-of-64) — ünïcode".into()),
            job_started: Some(1792653039),
            last_result: Some("Succeeded".into()),
            trace: Some("relatime".into()),
            gh_status: Some("online".into()),
            gh_busy: Some(false),
            labels: vec!["self-hosted".into(), "rust-cpu".into(), "light".into()],
            ctx: Some(JobContext {
                repo: "pulseengine/spar".into(),
                workflow: "CI".into(),
                run_id: 39739849534,
                run_number: 1106,
                run_attempt: 1,
                event: "pull_request".into(),
                branch: "fix/binding-consumers-codegen".into(),
                base: "main".into(),
                sha: "ce7e0ac49009ce57dc120104003d0feda16fee68".into(),
                actor: "avrabe".into(),
                pr: Some(511),
                pr_title: Some("fix(codegen): bind consumers".into()),
            }),
            stats: Some(CgStats {
                cpu_cores: Some(3.27),
                mem_mb: 7317,
                mem_peak_mb: 36791,
                mem_max_mb: Some(49152),
                pids: 350,
                psi_cpu: 12.0,
                psi_mem: 0.5,
                oom_kills: 1,
            }),
        }
    }

    fn hosted(i: u32, long: bool) -> HostedJob {
        let l = |s: &str| if long { s.repeat(40) } else { s.to_string() };
        HostedJob {
            repo: l("pulseengine/rivet"),
            workflow: l("Release"),
            job: l(&format!("build (macos-14) #{i}")),
            status: "in_progress".into(),
            runner: l("GitHub Actions 12"),
            labels: l("macos-14"),
            since: 1792653039 + i as i64,
            run_id: 39739849534 + i as u64,
            event: "push".into(),
            branch: l("main"),
            pr: if i.is_multiple_of(2) { Some(i) } else { None },
        }
    }

    #[test]
    fn hosted_list_spans_chunks_and_reassembles() {
        let q = Queue {
            hosted: (0..40).map(|i| hosted(i, false)).collect(),
            hosted_total: 40,
            ..Default::default()
        };
        let s = Snapshot {
            queue: Some(q.clone()),
            taken_at: 1,
            ..Default::default()
        };
        let pkts = encode_snapshot(&s, &mut SeqCounters::default());
        let chunks = pkts
            .iter()
            .filter(|p| {
                p.len() > 6 && {
                    let a = u16::from_be_bytes([p[0], p[1]]) & 0x7FF;
                    (APID_HOSTED_BASE..APID_HOSTED_BASE + HOSTED_CHUNKS).contains(&a)
                }
            })
            .count();
        assert!(
            chunks > 1,
            "40 jobs must need several packets, got {chunks}"
        );
        assert!(pkts.iter().all(|p| p.len() <= MAX_PACKET));
        let mut a = Assembler::default();
        let snap = pkts.iter().filter_map(|p| a.ingest(p, 1)).last().unwrap();
        assert_eq!(snap.queue.unwrap().hosted, q.hosted);
    }

    #[test]
    fn queue_packet_keeps_the_biggest_hosted_groups() {
        let g = |i: u32| HostedGroup {
            repo: format!("pulseengine/repo-with-a-long-name-{i}"),
            labels: "ubuntu-latest,ubuntu-22.04".into(),
            running: 100 - i,
            queued: i,
            since: 1,
        };
        let q = Queue {
            hosted_groups: (0..60).map(g).collect(),
            top_runs: vec![QueuedRun::default(); 10],
            ..Default::default()
        };
        let b = enc_queue_detail(&q);
        assert!(b.len() <= MAX_PAYLOAD);
        let d: QueueDetail = serde_json::from_slice(&b).unwrap();
        assert!(!d.top_runs.is_empty(), "top runs survive alongside groups");
        assert!(
            !d.hosted_groups.is_empty() && d.hosted_groups[0] == q.hosted_groups[0],
            "largest group survives shrinking"
        );
    }

    fn qsnap(n: u32) -> Snapshot {
        let e = |i: u32| QueuedEntry {
            repo: "spar".into(),
            run_id: i as u64,
            workflow: "CI".into(),
            event: "pull_request".into(),
            branch: format!("fix/branch-{i}"),
            pr: Some(i),
            pool: "rust-cpu".into(),
            jobs: 2,
            since: 5,
        };
        Snapshot {
            taken_at: 1,
            queue: Some(Queue {
                queued: (0..n).map(e).collect(),
                queued_complete: true,
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    fn is_queued_chunk(p: &[u8]) -> bool {
        let a = u16::from_be_bytes([p[0], p[1]]) & 0x7FF;
        (APID_QUEUED_BASE..APID_QUEUED_BASE + QUEUED_CHUNKS).contains(&a)
    }

    #[test]
    fn queued_list_complete_only_when_every_chunk_arrives() {
        let s = qsnap(60);
        let pkts = encode_snapshot(&s, &mut SeqCounters::default());
        assert!(pkts.iter().filter(|p| is_queued_chunk(p)).count() > 1);
        let mut a = Assembler::default();
        let q = pkts
            .iter()
            .filter_map(|p| a.ingest(p, 1))
            .last()
            .unwrap()
            .queue
            .unwrap();
        assert!(q.queued_complete && q.queued.len() == 60);
        // drop ONE queued chunk on the way: the hub must stop claiming completeness
        let mut a = Assembler::default();
        let mut dropped = false;
        let q = pkts
            .iter()
            .filter(|p| {
                !(is_queued_chunk(p) && !dropped && {
                    dropped = true;
                    true
                })
            })
            .filter_map(|p| a.ingest(p, 1))
            .last()
            .unwrap()
            .queue
            .unwrap();
        assert!(!q.queued_complete, "lost chunk must clear queued_complete");
        // drop ALL queued chunks
        let mut a = Assembler::default();
        let q = pkts
            .iter()
            .filter(|p| !is_queued_chunk(p))
            .filter_map(|p| a.ingest(p, 1))
            .last()
            .unwrap()
            .queue
            .unwrap();
        assert!(!q.queued_complete && q.queued.is_empty());
    }

    #[test]
    fn overflowing_queued_list_is_marked_incomplete() {
        let pkts = encode_snapshot(&qsnap(2000), &mut SeqCounters::default());
        let mut a = Assembler::default();
        let q = pkts
            .iter()
            .filter_map(|p| a.ingest(p, 1))
            .last()
            .unwrap()
            .queue
            .unwrap();
        assert!(!q.queued_complete && q.queued.len() < 2000);
    }

    #[test]
    fn ledger_records_and_manifests_cross_the_wire() {
        let rec = |i: u64| JobRecord {
            d: "2026-10-21".into(),
            id: i,
            run: 1,
            att: 1,
            repo: "spar".into(),
            wf: "CI".into(),
            job: "ä".repeat(200),
            pool: "rust-cpu".into(),
            runner: "pulseengine-ci-01-3".into(),
            ev: "pull_request".into(),
            c: 1,
            s: Some(2),
            e: Some(3),
            r: "success".into(),
        };
        let recs: Vec<JobRecord> = (0..50).map(rec).collect();
        let ms = vec![DayManifest {
            day: "2026-10-21".into(),
            n: 50,
            x: (0..50).fold(0, |a, b| a ^ b),
            closed: false,
        }];
        let pkts = encode_ledger(&recs, &ms, &mut SeqCounters::default());
        assert!(pkts.len() > 2 && pkts.iter().all(|p| p.len() <= MAX_PACKET));
        let mut a = Assembler::default();
        for p in &pkts {
            a.ingest(p, 1);
        }
        assert_eq!(a.ledger_in.len(), 50, "every record arrives");
        assert_eq!(a.ledger_in[3].id, 3);
        assert_eq!(a.manifests["2026-10-21"].n, 50);
    }

    #[test]
    fn worst_case_hosted_jobs_stay_under_mtu_and_bounded() {
        let jobs: Vec<HostedJob> = (0..64).map(|i| hosted(i, true)).collect();
        let (c, _) = enc_hosted(&jobs);
        assert!(c.len() <= HOSTED_CHUNKS as usize);
        assert!(c.iter().all(|b| b.len() <= MAX_PAYLOAD));
    }

    #[test]
    fn runner_context_survives_the_wire() {
        let r = runner(1);
        let (_, d) = dec_runner(&enc_runner(1, &r)).unwrap();
        assert_eq!(d.ctx, r.ctx);
        assert_eq!(d.stats, r.stats);
        let none = Runner {
            stats: Some(CgStats {
                cpu_cores: None,
                mem_max_mb: None,
                ..Default::default()
            }),
            ..r.clone()
        };
        assert_eq!(
            dec_runner(&enc_runner(1, &none)).unwrap().1.stats,
            none.stats,
            "unknown cpu / unlimited memory survive"
        );
        let mut idle = runner(2);
        idle.ctx = None;
        assert_eq!(dec_runner(&enc_runner(1, &idle)).unwrap().1.ctx, None);
    }

    #[test]
    fn worst_case_runner_packet_still_fits() {
        let big = "ä".repeat(400); // multi-byte, far over every cap
        let mut r = runner(14);
        r.job = Some(big.clone());
        r.last_result = Some(big.clone());
        r.trace = Some(big.clone());
        r.labels = vec![big.clone(); 40];
        r.ctx = Some(JobContext {
            repo: big.clone(),
            workflow: big.clone(),
            run_id: u64::MAX,
            run_number: u32::MAX,
            run_attempt: 999,
            event: big.clone(),
            branch: big.clone(),
            base: big.clone(),
            sha: big.clone(),
            actor: big.clone(),
            pr: Some(u32::MAX),
            pr_title: Some(big),
        });
        let p = packet(APID_RUNNER_BASE + 14, 0, &enc_runner(1, &r));
        assert!(
            p.len() <= MAX_PACKET,
            "worst-case runner packet is {} bytes",
            p.len()
        );
        let (_, d) = dec_runner(open(&p).unwrap().1).unwrap(); // and it still decodes
        assert!(d.ctx.is_some() && d.stats.is_some());
    }

    #[test]
    fn packet_header_follows_relay_convention() {
        let p = packet(APID_HOST, 5, b"abc");
        let h = decode_header(&p).unwrap();
        assert_eq!(h.apid, APID_HOST);
        assert_eq!(h.sequence_count, 5);
        assert_eq!(h.sequence_flags, 0b11);
        assert_eq!(h.packet_type, PacketType::Telemetry);
        assert_eq!(h.data_length as usize, 3); // (payload 3 + checksum 1) - 1
        assert_eq!(p.len(), 6 + 3 + 1);
    }

    #[test]
    fn open_rejects_corruption_and_truncation() {
        let p = packet(APID_QUEUE, 1, b"{\"x\":1}");
        assert!(open(&p).is_ok());
        let mut bad = p.clone();
        bad[8] ^= 0x01;
        assert_eq!(open(&bad).unwrap_err(), "checksum mismatch");
        assert!(open(&p[..p.len() - 1])
            .unwrap_err()
            .starts_with("length mismatch"));
        let mut extra = p.clone();
        extra.push(0);
        assert!(open(&extra).unwrap_err().starts_with("length mismatch"));
    }

    #[test]
    fn host_roundtrip() {
        let m = Machine {
            ncpu: 32,
            load: [98.11, 85.65, 80.75],
            mem_total_mb: 128000,
            mem_avail_mb: 91000,
            swap_total_mb: 16384,
            swap_used_mb: 11000,
            psi: Psi {
                cpu: PsiLine {
                    avg10: 69.87,
                    avg60: 66.62,
                    avg300: 59.49,
                },
                memory: PsiLine::default(),
                io: PsiLine {
                    avg10: 0.5,
                    avg60: 0.25,
                    avg300: 0.1,
                },
            },
            disks: vec![Disk {
                mount: "/var/lib/runners".into(),
                size_gb: 984.25,
                used_gb: 556.5,
                used_pct: 60.0,
            }],
        };
        let (t, d) = dec_host(&enc_host(1792653039, &m)).unwrap();
        assert_eq!(t, 1792653039);
        assert_eq!(d.load, m.load);
        assert_eq!(d.psi.cpu, m.psi.cpu);
        assert_eq!(d.disks, m.disks);
        assert_eq!(d.swap_used_mb, 11000);
    }

    #[test]
    fn runner_roundtrip_keeps_both_busy_views_and_labels() {
        let r = runner(10);
        let (_, d) = dec_runner(&enc_runner(1, &r)).unwrap();
        assert_eq!(d.id, 10);
        assert!(d.local_busy);
        assert_eq!(d.gh_busy, Some(false)); // disagreement must survive the wire
        assert_eq!(d.gh_status.as_deref(), Some("online"));
        assert_eq!(d.job, r.job);
        assert_eq!(d.labels, r.labels);
        assert_eq!(d.trace.as_deref(), Some("relatime"));
    }

    #[test]
    fn runner_without_github_view_stays_unknown() {
        let mut r = runner(3);
        r.gh_status = None;
        r.gh_busy = None;
        let (_, d) = dec_runner(&enc_runner(1, &r)).unwrap();
        assert_eq!((d.gh_status, d.gh_busy), (None, None));
    }

    #[test]
    fn oversized_streams_still_fit_one_packet() {
        let lines: Vec<String> = (0..200)
            .map(|i| format!("{i:04} {}", "x".repeat(250)))
            .collect();
        let p = packet(APID_DISPATCHER, 0, &enc_lines(&lines));
        assert!(p.len() <= MAX_PACKET);
        let back: Vec<String> = serde_json::from_slice(open(&p).unwrap().1).unwrap();
        assert!(back.last().unwrap().starts_with("0199")); // newest lines are the ones kept
    }

    #[test]
    fn sequence_wraps_at_14_bits() {
        let mut s = SeqCounters::default();
        for _ in 0..16383 {
            s.next(APID_HOST);
        }
        assert_eq!(s.next(APID_HOST), 16383);
        assert_eq!(s.next(APID_HOST), 0);
    }

    #[test]
    fn link_counts_lost_packets_across_the_wrap() {
        let mut l = LinkStat::default();
        l.account(16382, 100, 300);
        l.account(16383, 160, 300);
        l.account(2, 220, 300); // 0 and 1 lost
        assert_eq!((l.received, l.lost, l.restarts), (3, 2, 0));
    }

    #[test]
    fn agent_restart_is_not_counted_as_loss() {
        let mut l = LinkStat::default();
        l.account(900, 100, 300);
        l.account(0, 100 + 400, 300); // silent 400 s, counters reset
        assert_eq!((l.lost, l.restarts), (0, 1));
    }

    #[test]
    fn full_snapshot_survives_the_link() {
        let s = Snapshot {
            agent_version: "0.1.0".into(),
            host: "pulseengine-ci-01".into(),
            taken_at: 1792653039,
            runners: vec![runner(1), runner(13)],
            queue: Some(Queue {
                scanned_at: 1792653000,
                jobs_queued: 312,
                runs_queued: 48,
                pools: BTreeMap::from([(
                    "rust-cpu".into(),
                    PoolQueue {
                        jobs_queued: 282,
                        oldest_s: 43200,
                        median_s: 6840,
                    },
                )]),
                ..Default::default()
            }),
            errors: vec!["queue scan: example".into()],
            ..Default::default()
        };
        let mut seq = SeqCounters::default();
        let pkts = encode_snapshot(&s, &mut seq);
        assert!(pkts.iter().all(|p| p.len() <= MAX_PACKET));
        let mut a = Assembler::default();
        let mut got = None;
        for p in &pkts {
            if let Some(snap) = a.ingest(p, 1792653040) {
                got = Some(snap)
            }
        }
        let g = got.expect("frame-end must complete a snapshot");
        assert_eq!(g.runners.len(), 2);
        assert_eq!(g.queue.unwrap().pools["rust-cpu"].jobs_queued, 282);
        assert_eq!(g.errors, s.errors);
        assert_eq!(a.rejected, 0);
        assert!(a.link.values().all(|l| l.lost == 0));
    }
}

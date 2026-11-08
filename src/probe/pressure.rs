//! Disk pressure: how full each volume is, and what agents hold on it
//! (REQ-PRESSURE-001, REQ-PRESSURE-002, DD-PRESSURE-001).
//!
//! The probe sizes what agents hold (Claude Code session directories with
//! their scratchpads, `target/` in live sessions' checkouts, and those
//! checkouts' git worktrees) only while a volume is low, at most every
//! [`MEASURE_EVERY_S`] and within a time budget. It never follows a symlink,
//! never crosses onto another device and never deletes anything.
//!
//! Who holds what (paths, session ids, pids, repos) stays on the machine, in
//! the probe's state directory, for `warte probe pressure`. The report sent to
//! the ground station carries only GB totals per kind per volume: the read API
//! is unauthenticated (warte#46).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Defaults for `[pressure]` and the probe's own trigger.
pub const DEFAULT_LOW_GB: f64 = 20.0;
pub const DEFAULT_CRITICAL_GB: f64 = 10.0;
/// The probe sizes again at most this often while a volume is low.
pub const MEASURE_EVERY_S: i64 = 10 * 60;
/// A stored measurement older than this is not reported.
pub const STORED_FRESH_S: i64 = 60 * 60;
/// How long one background sizing may take.
pub const BUDGET: Duration = Duration::from_secs(60);
/// The file in the probe's state directory.
pub const STORED_FILE: &str = "pressure.json";

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Ok,
    Low,
    Critical,
    /// No fresh report: nothing is known.
    Unknown,
}

impl Verdict {
    /// How bad, for picking the worst volume: unknown ranks below ok.
    fn rank(self) -> u8 {
        match self {
            Verdict::Unknown => 0,
            Verdict::Ok => 1,
            Verdict::Low => 2,
            Verdict::Critical => 3,
        }
    }
    /// When to ask again.
    pub fn recheck_s(self) -> u64 {
        match self {
            Verdict::Ok => 600,
            Verdict::Low => 120,
            Verdict::Critical => 60,
            Verdict::Unknown => 300,
        }
    }
}

/// `[pressure]` in warte's config: `low_gb = 20`, `critical_gb = 10` (GB
/// free on a volume). A missing key keeps its default.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(default)]
pub struct Thresholds {
    pub low_gb: f64,
    pub critical_gb: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            low_gb: DEFAULT_LOW_GB,
            critical_gb: DEFAULT_CRITICAL_GB,
        }
    }
}

impl Thresholds {
    /// From `WARTE_PRESSURE_LOW_GB` / `WARTE_PRESSURE_CRITICAL_GB`, else the
    /// defaults (the probe's trigger and the local command).
    pub fn from_env() -> Self {
        let get = |k: &str, d: f64| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0.0)
                .unwrap_or(d)
        };
        Thresholds {
            low_gb: get("WARTE_PRESSURE_LOW_GB", DEFAULT_LOW_GB),
            critical_gb: get("WARTE_PRESSURE_CRITICAL_GB", DEFAULT_CRITICAL_GB),
        }
    }

    pub fn verdict(&self, free_gb: f64) -> Verdict {
        if free_gb < self.critical_gb {
            Verdict::Critical
        } else if free_gb < self.low_gb {
            Verdict::Low
        } else {
            Verdict::Ok
        }
    }
}

/// The worst of several verdicts; [`Verdict::Unknown`] when there are none.
pub fn worst(vs: impl IntoIterator<Item = Verdict>) -> Verdict {
    vs.into_iter()
        .max_by_key(|v| v.rank())
        .unwrap_or(Verdict::Unknown)
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A Claude Code session's directory under `/tmp/claude-<uid>/`, with its
    /// scratchpad and task outputs.
    Scratchpad,
    /// `target/` in a live session's checkout.
    Target,
    /// A git worktree of a live session's checkout.
    Worktree,
}

/// A live agent session in a holder's directory.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Agent {
    pub agent: String,
    pub pid: u32,
}

/// One holder, local only: never sent to the ground station.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Holder {
    pub kind: Kind,
    /// The volume it is on, as the report names it.
    pub mount: String,
    pub gb: f64,
    pub path: String,
    /// The Claude Code session id, for a scratchpad.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The checkout's directory name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Live agent sessions working in that directory. Empty: none is (a
    /// scratchpad whose session ended, for example).
    pub agents: Vec<Agent>,
    /// A live agent session works in that directory (`agents` not empty).
    pub live: bool,
    /// The session's pid when exactly one live session works there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// False when the time budget ran out while sizing it: `gb` is a floor.
    pub complete: bool,
}

/// What the probe stored: the holders, local only.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Stored {
    pub measured_at: i64,
    pub holders: Vec<Holder>,
}

/// What the report carries to the ground station: GB per kind per volume.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Held {
    pub measured_at: i64,
    pub volumes: Vec<HeldOn>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct HeldOn {
    pub mount: String,
    pub scratchpad_gb: f64,
    pub target_gb: f64,
    pub worktree_gb: f64,
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Totals per kind per volume: the only part of the holders that leaves the
/// machine.
pub fn held(s: &Stored) -> Held {
    let mut by: BTreeMap<&str, HeldOn> = BTreeMap::new();
    for h in &s.holders {
        let e = by.entry(h.mount.as_str()).or_insert_with(|| HeldOn {
            mount: h.mount.clone(),
            ..Default::default()
        });
        match h.kind {
            Kind::Scratchpad => e.scratchpad_gb += h.gb,
            Kind::Target => e.target_gb += h.gb,
            Kind::Worktree => e.worktree_gb += h.gb,
        }
    }
    Held {
        measured_at: s.measured_at,
        volumes: by
            .into_values()
            .map(|mut v| {
                v.scratchpad_gb = round1(v.scratchpad_gb);
                v.target_gb = round1(v.target_gb);
                v.worktree_gb = round1(v.worktree_gb);
                v
            })
            .collect(),
    }
}

/// Claude Code's directory name for a working directory: every character
/// that is not ASCII alphanumeric becomes `-`.
pub fn slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Bytes on disk under `root` (allocated blocks), without following symlinks
/// or leaving `root`'s device. Stops at `deadline`; the bool says whether it
/// saw everything.
pub fn size_bytes(root: &Path, deadline: Instant) -> (u64, bool) {
    let Ok(meta) = std::fs::symlink_metadata(root) else {
        return (0, true);
    };
    if meta.file_type().is_symlink() {
        return (0, true);
    }
    let dev = meta.dev();
    let mut total = meta.blocks() * 512;
    let mut stack = vec![root.to_path_buf()];
    let mut n = 0u32;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            n = n.wrapping_add(1);
            if n.is_multiple_of(1024) && Instant::now() >= deadline {
                return (total, false);
            }
            let Ok(m) = e.metadata() else { continue };
            if m.file_type().is_symlink() || m.dev() != dev {
                continue;
            }
            total += m.blocks() * 512;
            if m.is_dir() {
                stack.push(e.path());
            }
        }
    }
    (total, true)
}

/// A live agent session: what [`measure`] needs from the process table.
#[derive(Debug, Clone, PartialEq)]
pub struct Live {
    pub agent: String,
    pub pid: u32,
    pub cwd: PathBuf,
}

/// Live agent sessions with their working directories (local only).
pub fn live_sessions() -> Vec<Live> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_cwd(UpdateKind::Always),
    );
    let mut cwd: BTreeMap<u32, PathBuf> = BTreeMap::new();
    let found = sys
        .processes()
        .values()
        .filter_map(|p| {
            let cmd: Vec<String> = p
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
            let agent = super::agent_of(&p.name().to_string_lossy(), &cmd)?;
            if let Some(c) = p.cwd() {
                cwd.insert(p.pid().as_u32(), c.to_path_buf());
            }
            Some((
                super::Session {
                    agent: agent.into(),
                    pid: p.pid().as_u32(),
                    started_at: 0,
                    dir: None,
                    activity: None,
                    activity_at: None,
                },
                p.parent().map(|q| q.as_u32()),
            ))
        })
        .collect();
    super::innermost(found)
        .into_iter()
        .filter_map(|s| {
            cwd.get(&s.pid).map(|c| Live {
                agent: s.agent,
                pid: s.pid,
                cwd: c.clone(),
            })
        })
        .collect()
}

/// Where Claude Code keeps per-session directories: `<root>/claude-<uid>/`.
pub fn scratch_roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for base in ["/private/tmp", "/tmp"] {
        let Ok(rd) = std::fs::read_dir(base) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let ok = name
                .strip_prefix("claude-")
                .is_some_and(|u| !u.is_empty() && u.chars().all(|c| c.is_ascii_digit()));
            if !ok {
                continue;
            }
            // /tmp is /private/tmp on macOS: count each directory once
            let p = std::fs::canonicalize(e.path()).unwrap_or_else(|_| e.path());
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// The volume a path is on: the mount with the same device, else the longest
/// mount that prefixes it.
pub fn mount_of(path: &Path, mounts: &[String]) -> Option<String> {
    if let Ok(m) = std::fs::metadata(path) {
        if let Some(found) = mounts.iter().find(|mnt| {
            std::fs::metadata(mnt)
                .map(|x| x.dev() == m.dev())
                .unwrap_or(false)
        }) {
            return Some(found.clone());
        }
    }
    mounts
        .iter()
        .filter(|m| path.starts_with(m.as_str()))
        .max_by_key(|m| m.len())
        .cloned()
}

/// The git checkout a directory is in: the nearest ancestor with `.git`.
fn checkout_of(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|a| a.join(".git").exists())
        .map(Path::to_path_buf)
}

/// Linked worktrees git lists for a checkout. git lists the main worktree
/// first, whichever worktree it is asked from: that one is a checkout, not a
/// worktree.
fn worktrees_of(checkout: &Path) -> Vec<PathBuf> {
    let Ok(out) = std::process::Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(["worktree", "list", "--porcelain"])
        .output()
    else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .skip(1)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .collect()
}

/// A Claude Code session id: a UUID, `8-4-4-4-12` hex digits.
pub fn is_session_id(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// What to size, before sizing: (kind, path, session id, repo, agents).
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub kind: Kind,
    pub path: PathBuf,
    pub session_id: Option<String>,
    pub repo: Option<String>,
    pub agents: Vec<Agent>,
}

/// Find the holders, without sizing them. Scratchpads are
/// `<root>/<cwd-slug>/<session-id>/`; their agents are the live sessions
/// whose working directory has that slug.
pub fn candidates(live: &[Live], roots: &[PathBuf]) -> Vec<Candidate> {
    let agents_in = |pred: &dyn Fn(&Path) -> bool| -> Vec<Agent> {
        let mut a: Vec<Agent> = live
            .iter()
            .filter(|l| pred(&l.cwd))
            .map(|l| Agent {
                agent: l.agent.clone(),
                pid: l.pid,
            })
            .collect();
        a.sort_by_key(|x| x.pid);
        a
    };
    let mut out = Vec::new();
    for root in roots {
        let Ok(projects) = std::fs::read_dir(root) else {
            continue;
        };
        for p in projects.flatten() {
            if !p.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let project = p.file_name().to_string_lossy().into_owned();
            // the slug of an absolute path starts with '-'; other directories
            // there (bash-edit-diff, ...) are not sessions
            if !project.starts_with('-') {
                continue;
            }
            let Ok(sessions) = std::fs::read_dir(p.path()) else {
                continue;
            };
            for s in sessions.flatten() {
                if !s.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let sid = s.file_name().to_string_lossy().into_owned();
                if !is_session_id(&sid) {
                    continue;
                }
                let repo = live
                    .iter()
                    .find(|l| slug(&l.cwd) == project)
                    .and_then(|l| l.cwd.file_name())
                    .map(|n| n.to_string_lossy().into_owned());
                out.push(Candidate {
                    kind: Kind::Scratchpad,
                    path: s.path(),
                    session_id: Some(sid),
                    repo,
                    agents: agents_in(&|c: &Path| slug(c) == project),
                });
            }
        }
    }
    let mut checkouts: Vec<PathBuf> = live.iter().filter_map(|l| checkout_of(&l.cwd)).collect();
    checkouts.sort();
    checkouts.dedup();
    // a worktree is itself a checkout: list it once, as a worktree
    let mut worktrees: Vec<PathBuf> = checkouts.iter().flat_map(|c| worktrees_of(c)).collect();
    worktrees.sort();
    worktrees.dedup();
    for c in &checkouts {
        if worktrees.contains(c) {
            continue;
        }
        let t = c.join("target");
        if t.is_dir() {
            out.push(Candidate {
                kind: Kind::Target,
                path: t,
                session_id: None,
                repo: c.file_name().map(|n| n.to_string_lossy().into_owned()),
                agents: agents_in(&|w: &Path| {
                    w.starts_with(c) && !worktrees.iter().any(|x| w.starts_with(x))
                }),
            });
        }
    }
    for w in &worktrees {
        out.push(Candidate {
            kind: Kind::Worktree,
            path: w.clone(),
            session_id: None,
            repo: w.file_name().map(|n| n.to_string_lossy().into_owned()),
            agents: agents_in(&|x: &Path| x.starts_with(w)),
        });
    }
    out
}

/// Size each candidate within `budget`, largest first in the result.
/// Holders on a volume outside `wanted` are skipped (the probe sizes only
/// what is on a low volume).
pub fn measure(
    cands: Vec<Candidate>,
    mounts: &[String],
    wanted: &[String],
    budget: Duration,
    now: i64,
) -> Stored {
    let deadline = Instant::now() + budget;
    let mut holders: Vec<Holder> = cands
        .into_iter()
        .filter_map(|c| {
            let mount = mount_of(&c.path, mounts).filter(|m| wanted.contains(m))?;
            let (bytes, complete) = if Instant::now() >= deadline {
                (0, false)
            } else {
                size_bytes(&c.path, deadline)
            };
            Some(Holder {
                kind: c.kind,
                mount,
                gb: round1(bytes as f64 / 1e9),
                path: c.path.display().to_string(),
                session_id: c.session_id,
                repo: c.repo,
                live: !c.agents.is_empty(),
                pid: match c.agents.as_slice() {
                    [one] => Some(one.pid),
                    _ => None,
                },
                agents: c.agents,
                complete,
            })
        })
        .collect();
    holders.sort_by(|a, b| b.gb.total_cmp(&a.gb));
    Stored {
        measured_at: now,
        holders,
    }
}

pub fn read_stored(state_dir: &Path) -> Option<Stored> {
    let b = std::fs::read(state_dir.join(STORED_FILE)).ok()?;
    serde_json::from_slice(&b).ok()
}

/// Write the holders for this machine's user only (they carry paths).
pub fn write_stored(state_dir: &Path, s: &Stored) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = state_dir.join(format!("{STORED_FILE}.tmp"));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(&serde_json::to_vec(s)?)?;
    f.sync_all()?;
    std::fs::rename(&tmp, state_dir.join(STORED_FILE))?;
    Ok(())
}

/// Free GB per published volume of a report.
pub fn free_gb(d: &super::Disk) -> f64 {
    round1((d.size_gb - d.used_gb).max(0.0))
}

/// The probe's part, after collecting a report: size again when a volume is
/// low and the last sizing is older than [`MEASURE_EVERY_S`], then attach the
/// totals of a fresh stored sizing to the report.
pub fn attach(report: &mut super::Report, state_dir: &Path, t: &Thresholds, now: i64) {
    let low_mounts: Vec<String> = report
        .disks
        .iter()
        .filter(|d| t.verdict(free_gb(d)) != Verdict::Ok)
        .map(|d| d.mount.clone())
        .collect();
    let low = !low_mounts.is_empty();
    let mut stored = read_stored(state_dir);
    let due = stored
        .as_ref()
        .is_none_or(|s| now - s.measured_at >= MEASURE_EVERY_S);
    if low && due {
        let mounts: Vec<String> = report.disks.iter().map(|d| d.mount.clone()).collect();
        let s = measure(
            candidates(&live_sessions(), &scratch_roots()),
            &mounts,
            &low_mounts,
            BUDGET,
            now,
        );
        if let Err(e) = write_stored(state_dir, &s) {
            eprintln!("warte probe: storing the disk holders failed: {e:#}");
        }
        stored = Some(s);
    }
    report.pressure = stored
        .filter(|s| low && now - s.measured_at < STORED_FRESH_S)
        .map(|s| held(&s));
}

/// The probe's state directory when none is given: `$HOME/Library/Application
/// Support/warte-probe` on macOS when it exists, else `/var/lib/warte-probe`.
pub fn default_state_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        if let Some(h) = std::env::var_os("HOME") {
            let p = PathBuf::from(h).join("Library/Application Support/warte-probe");
            if p.is_dir() {
                return p;
            }
        }
    }
    PathBuf::from("/var/lib/warte-probe")
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Volume {
    pub mount: String,
    pub free_gb: f64,
    pub total_gb: f64,
    pub verdict: Verdict,
}

/// What `warte probe pressure` prints.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Local {
    /// The host as enrolled (the probe certificate's name), else this
    /// machine's host name.
    pub host: String,
    pub verdict: Verdict,
    pub thresholds: Thresholds,
    /// When the volumes were read (now).
    pub volumes_at: i64,
    pub volumes: Vec<Volume>,
    /// When the holders were sized; none before the first sizing.
    pub measured_at: Option<i64>,
    /// Largest first.
    pub holders: Vec<Holder>,
    /// Why there are no holders, when there are none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

pub fn local(state_dir: &Path, measure_now: bool, t: &Thresholds, now: i64) -> Local {
    let disks = super::disks();
    let volumes: Vec<Volume> = disks
        .iter()
        .map(|d| Volume {
            mount: d.mount.clone(),
            free_gb: free_gb(d),
            total_gb: d.size_gb,
            verdict: t.verdict(free_gb(d)),
        })
        .collect();
    let host = std::fs::read_to_string(state_dir.join("probe-cert.pem"))
        .ok()
        .and_then(|c| super::send::cert_host(&c).ok().flatten())
        .or_else(sysinfo::System::host_name)
        .unwrap_or_default();
    let (stored, note) = if measure_now {
        let mounts: Vec<String> = disks.iter().map(|d| d.mount.clone()).collect();
        // asked for explicitly: every volume
        let s = measure(
            candidates(&live_sessions(), &scratch_roots()),
            &mounts,
            &mounts,
            BUDGET,
            now,
        );
        // keep it for the next caller when this user may write there
        let _ = write_stored(state_dir, &s);
        (Some(s), None)
    } else {
        match read_stored(state_dir) {
            Some(s) => (Some(s), None),
            None => (
                None,
                Some(
                    "not sized yet: the probe sizes holders only while a volume is low; --measure sizes them now"
                        .to_string(),
                ),
            ),
        }
    };
    Local {
        host,
        verdict: worst(volumes.iter().map(|v| v.verdict)),
        thresholds: *t,
        volumes_at: now,
        volumes,
        measured_at: stored.as_ref().map(|s| s.measured_at),
        holders: stored.map(|s| s.holders).unwrap_or_default(),
        note,
    }
}

/// `warte probe pressure` for a terminal.
pub fn text(l: &Local) -> String {
    use std::fmt::Write;
    let v = |v: Verdict| {
        serde_json::to_value(v)
            .ok()
            .and_then(|x| x.as_str().map(String::from))
            .unwrap_or_default()
    };
    let mut o = String::new();
    let _ = writeln!(o, "{}: {}", l.host, v(l.verdict));
    for x in &l.volumes {
        let _ = writeln!(
            o,
            "  {:<28} {:>7.1} GB free of {:>7.1}  {}",
            x.mount,
            x.free_gb,
            x.total_gb,
            v(x.verdict)
        );
    }
    if let Some(n) = &l.note {
        let _ = writeln!(o, "{n}");
    }
    if let Some(at) = l.measured_at {
        let _ = writeln!(
            o,
            "held (sized {} min ago):",
            (l.volumes_at - at).max(0) / 60
        );
    }
    for h in &l.holders {
        let who = if h.agents.is_empty() {
            "no live session".to_string()
        } else {
            h.agents
                .iter()
                .map(|a| format!("{} {}", a.agent, a.pid))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let kind = serde_json::to_value(h.kind)
            .ok()
            .and_then(|x| x.as_str().map(String::from))
            .unwrap_or_default();
        let _ = writeln!(
            o,
            "  {:>7.1} GB{} {:<10} {}  ({})",
            h.gb,
            if h.complete { " " } else { "+" },
            kind,
            h.path,
            who
        );
    }
    o
}

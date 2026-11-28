//! Local, read-only collection on the CI host (runs as root to read the
//! journal and every runner's /proc entries). Nothing here changes state.
use crate::ciagent::model::*;
use crate::ciagent::parse;
use crate::ciagent::CiAgentConfig;
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Previous `cpu.stat usage_usec` per runner, so CPU can be reported as a rate.
static CPU_PREV: Mutex<Option<HashMap<u32, (Instant, u64)>>> = Mutex::new(None);

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let o = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if !o.status.success() {
        return Err(format!("{cmd} {}: {}", args.join(" "), o.status));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

pub fn hostname() -> String {
    fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

pub fn machine(cfg: &CiAgentConfig, errors: &mut Vec<String>) -> Machine {
    let mut m = Machine {
        ncpu: std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(0),
        ..Default::default()
    };
    match fs::read_to_string("/proc/loadavg") {
        Ok(s) => m.load = parse::loadavg(&s).unwrap_or_default(),
        Err(e) => errors.push(format!("loadavg: {e}")),
    }
    if let Ok(s) = fs::read_to_string("/proc/meminfo") {
        let (t, a, st, sf) = parse::meminfo(&s);
        m.mem_total_mb = t;
        m.mem_avail_mb = a;
        m.swap_total_mb = st;
        m.swap_used_mb = st.saturating_sub(sf);
    }
    let rd = |r: &str| {
        fs::read_to_string(format!("/proc/pressure/{r}"))
            .map(|s| parse::psi(&s))
            .unwrap_or_default()
    };
    m.psi = Psi {
        cpu: rd("cpu"),
        memory: rd("memory"),
        io: rd("io"),
    };
    let mut df_args = vec!["-P", "-B1"];
    df_args.extend(cfg.df_mounts.iter().map(String::as_str));
    match run("df", &df_args) {
        Ok(s) => m.disks = parse::df(&s),
        Err(e) => errors.push(format!("df: {e}")),
    }
    m
}

/// uid -> number of Runner.Worker processes owned by it.
fn workers_by_uid() -> HashMap<u32, u32> {
    let mut out = HashMap::new();
    let Ok(rd) = fs::read_dir("/proc") else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if !p
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.bytes().all(|b| b.is_ascii_digit()))
            .unwrap_or(false)
        {
            continue;
        }
        let Ok(cmd) = fs::read(p.join("cmdline")) else {
            continue;
        };
        if !String::from_utf8_lossy(&cmd).contains("Runner.Worker") {
            continue;
        }
        if let Ok(st) = fs::read_to_string(p.join("status")) {
            if let Some(uid) = st
                .lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|u| u.parse::<u32>().ok())
            {
                *out.entry(uid).or_insert(0) += 1;
            }
        }
    }
    out
}

fn passwd_uids() -> HashMap<String, u32> {
    fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            Some((f.first()?.to_string(), f.get(2)?.parse().ok()?))
        })
        .collect()
}

pub fn runners(cfg: &CiAgentConfig, errors: &mut Vec<String>) -> Vec<Runner> {
    let glob = cfg.unit_glob();
    let units = match run(
        "systemctl",
        &[
            "list-units",
            glob.as_str(),
            "--all",
            "--plain",
            "--no-legend",
            "--type=service",
        ],
    ) {
        Ok(s) => s,
        Err(e) => {
            errors.push(format!("list runners: {e}"));
            return vec![];
        }
    };
    let workers = workers_by_uid();
    let uids = passwd_uids();
    let mut out = vec![];
    for unit in units.lines().filter_map(|l| l.split_whitespace().next()) {
        // github-runner@runner12.service -> 12
        let Some(id) = cfg.unit_id(unit) else {
            continue;
        };
        let show = run(
            "systemctl",
            &[
                "show",
                "-p",
                "ActiveState",
                "-p",
                "MainPID",
                "-p",
                "ControlGroup",
                unit,
            ],
        )
        .map(|s| parse::systemctl_show(&s))
        .unwrap_or_default();
        let pid: u32 = show
            .get("MainPID")
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        let journal = run(
            "journalctl",
            &["-u", unit, "-n", "80", "--no-pager", "-o", "short-unix"],
        )
        .unwrap_or_default();
        let js = parse::job_state(journal.lines());
        let user = cfg.runner(id);
        let local_busy = uids
            .get(&user)
            .map(|u| workers.get(u).copied().unwrap_or(0) > 0)
            .unwrap_or(false);
        let trace = if pid > 0 {
            fs::read_to_string(format!("/proc/{pid}/mountinfo"))
                .ok()
                .and_then(|mi| parse::mount_atime(&mi, &format!("{}/.rustup", cfg.runner_dir(id))))
        } else {
            None
        };
        let ctx = if local_busy {
            worker_context(&cfg.runner_dir(id))
        } else {
            None
        };
        let stats = show
            .get("ControlGroup")
            .filter(|g| g.starts_with('/'))
            .map(|g| cg_stats(id, &format!("/sys/fs/cgroup{g}")));
        out.push(Runner {
            ctx,
            stats,
            id,
            unit: unit.to_string(),
            active: show
                .get("ActiveState")
                .map(|s| s == "active")
                .unwrap_or(false),
            pid,
            local_busy,
            job: js.job,
            job_started: js.started,
            last_result: js.last_result,
            trace,
            ..Default::default()
        });
    }
    out.sort_by_key(|r| r.id);
    out
}

/// Read one runner's cgroup. Missing files read as zero rather than failing the snapshot.
fn cg_stats(id: u32, dir: &str) -> CgStats {
    let rd = |f: &str| fs::read_to_string(format!("{dir}/{f}")).unwrap_or_default();
    let usage = parse::cg_key(&rd("cpu.stat"), "usage_usec");
    let now = Instant::now();
    let cpu_cores = usage.and_then(|u| {
        let mut g = CPU_PREV.lock().unwrap_or_else(|e| e.into_inner());
        let prev = g.get_or_insert_with(HashMap::new).insert(id, (now, u));
        prev.and_then(|(t, p)| parse::cpu_cores(p, u, now.duration_since(t).as_secs_f64()))
    });
    CgStats {
        cpu_cores,
        mem_mb: parse::cg_mb(&rd("memory.current")).unwrap_or(0),
        mem_peak_mb: parse::cg_mb(&rd("memory.peak")).unwrap_or(0),
        mem_max_mb: parse::cg_mb(&rd("memory.max")),
        pids: rd("pids.current").trim().parse().unwrap_or(0),
        psi_cpu: parse::psi(&rd("cpu.pressure")).avg60,
        psi_mem: parse::psi(&rd("memory.pressure")).avg60,
        oom_kills: parse::cg_key(&rd("memory.events"), "oom_kill").unwrap_or(0) as u32,
    }
}

/// Job context of a busy runner from its newest `_diag/Worker_*.log`. The job
/// message (with GitHub's contextData) sits near the top, so only a prefix is read.
fn worker_context(runner_dir: &str) -> Option<crate::ciagent::model::JobContext> {
    use std::io::Read;
    let dir = format!("{runner_dir}/_diag");
    let newest = fs::read_dir(&dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("Worker_"))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())?;
    let mut buf = Vec::new();
    fs::File::open(newest.path())
        .ok()?
        .take(512 * 1024)
        .read_to_end(&mut buf)
        .ok()?;
    parse::job_context(&String::from_utf8_lossy(&buf))
}

fn tail_file(path: &str, n: usize) -> Vec<String> {
    let s = fs::read_to_string(path).unwrap_or_default();
    let v: Vec<String> = s.lines().map(|l| l.to_string()).collect();
    v[v.len().saturating_sub(n)..].to_vec()
}

fn tail_unit(unit: &str, n: &str, keep: impl Fn(&str) -> bool) -> Vec<String> {
    run(
        "journalctl",
        &["-u", unit, "-n", n, "--no-pager", "-o", "short-iso"],
    )
    .unwrap_or_default()
    .lines()
    .filter(|l| keep(l))
    .map(|l| l.to_string())
    .collect()
}

pub fn automation(cfg: &CiAgentConfig) -> Automation {
    let timers = cfg
        .timers
        .iter()
        .map(|u| {
            let s = run(
                "systemctl",
                &[
                    "show",
                    "-p",
                    "ActiveState",
                    "-p",
                    "NextElapseUSecRealtime",
                    u.as_str(),
                ],
            )
            .map(|s| parse::systemctl_show(&s))
            .unwrap_or_default();
            TimerState {
                unit: u.to_string(),
                active: s.get("ActiveState").map(|v| v == "active").unwrap_or(false),
                next: s
                    .get("NextElapseUSecRealtime")
                    .filter(|v| !v.is_empty() && *v != "n/a")
                    .cloned(),
            }
        })
        .collect();
    Automation {
        dispatcher: tail_file(&cfg.dispatcher_log, 8),
        watchdog: tail_unit(&cfg.watchdog_unit, "12", |l| {
            l.contains("[runner-watchdog]")
        })
        .into_iter()
        .rev()
        .take(4)
        .rev()
        .collect(),
        bazel_sweep: tail_unit(&cfg.bazel_sweep_unit, "8", |l| {
            l.contains("swept") || l.contains("dead")
        })
        .into_iter()
        .rev()
        .take(2)
        .rev()
        .collect(),
        trace_rollout: tail_file(&cfg.trace_rollout_log, 6),
        timers,
    }
}

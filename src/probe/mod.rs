//! `warte probe`: what a machine is and runs, for the ground station
//! (REQ-PROBE-001, DD-PROBE-001..003).
//!
//! This part collects and reports locally (`warte probe report`). Sending
//! goes over QUIC with mutual TLS and relay-sec (DD-TRANSPORT-001) once the
//! probe is enrolled (DD-PROBE-002); agent state comes from hooks the user
//! installs (DD-PROBE-003). Both build on this report.
//!
//! The probe is read-only. From agent processes it reports the agent, pid,
//! start time and working-directory name, never command lines: those can
//! carry prompts and tokens.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Agent CLIs and tools whose presence and version the probe reports.
pub const TOOLS: [&str; 5] = ["claude", "copilot", "codex", "gh", "varve"];
/// A `--version` that takes longer than this is reported as an error.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
/// Mounts smaller than this are pseudo or recovery volumes.
const MIN_DISK_BYTES: u64 = 1 << 30;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Report {
    pub probe_version: String,
    pub host: String,
    pub taken_at: i64,
    pub os: Os,
    pub uptime_s: u64,
    pub cpu: Cpu,
    pub memory: Memory,
    pub disks: Vec<Disk>,
    pub tools: Vec<Tool>,
    pub sessions: Vec<Session>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Os {
    /// e.g. "macOS 26.0" or "Linux (Debian GNU/Linux 12)".
    pub name: String,
    pub kernel: String,
    pub arch: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Cpu {
    pub ncpu: usize,
    pub load: [f64; 3],
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Memory {
    pub total_mb: u64,
    pub avail_mb: u64,
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Disk {
    pub mount: String,
    pub fs: String,
    pub size_gb: f64,
    pub used_gb: f64,
    pub used_pct: f64,
    pub removable: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Tool {
    pub name: String,
    /// `None`: not on PATH.
    pub path: Option<String>,
    pub version: Option<String>,
    /// Why there is no version although the tool was found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Session {
    pub agent: String,
    pub pid: u32,
    pub started_at: u64,
    /// The working directory's last component, e.g. the repo.
    pub dir: Option<String>,
}

/// The first `x.y` or `x.y.z` token, without a leading `v`:
/// "2.1.4 (Claude Code)" → "2.1.4", "gh version 2.81.0 (2025-..)" → "2.81.0".
pub fn parse_version(out: &str) -> Option<String> {
    out.split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
        .map(|t| t.trim_start_matches('v'))
        .find(|t| {
            let parts: Vec<&str> = t.split('.').collect();
            parts.len() >= 2
                && parts[0].chars().all(|c| c.is_ascii_digit())
                && !parts[0].is_empty()
                && parts[1].chars().next().is_some_and(|c| c.is_ascii_digit())
        })
        .map(String::from)
}

/// Which agent a process is, from its name and command line. Agents that run
/// on node (Copilot CLI, older Claude Code) show as `node`; their script path
/// names them. Only used for matching: the command line is never reported.
pub fn agent_of(name: &str, cmd: &[String]) -> Option<&'static str> {
    let base = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    let named = |a: &str| base(name) == a;
    for agent in ["claude", "copilot", "codex"] {
        if named(agent) {
            return Some(agent);
        }
    }
    if base(name).starts_with("node") {
        let script = cmd.get(1).map(String::as_str).unwrap_or_default();
        if script.contains("@anthropic-ai/claude-code") || base(script) == "claude" {
            return Some("claude");
        }
        if script.contains("@github/copilot") || base(script) == "copilot" {
            return Some("copilot");
        }
        if script.contains("@openai/codex") || base(script) == "codex" {
            return Some("codex");
        }
    }
    None
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

async fn tool(name: &str) -> Tool {
    let Some(path) = which(name) else {
        return Tool {
            name: name.into(),
            path: None,
            version: None,
            error: None,
        };
    };
    let run = tokio::process::Command::new(&path)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let (version, error) = match tokio::time::timeout(VERSION_TIMEOUT, run).await {
        Err(_) => (None, Some("--version timed out".into())),
        Ok(Err(e)) => (None, Some(format!("--version: {e}"))),
        Ok(Ok(o)) => {
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            match parse_version(&text) {
                Some(v) => (Some(v), None),
                // e.g. a varve shim outside a pinned project: say so, briefly
                None => (
                    None,
                    Some(format!(
                        "--version exited {}: {}",
                        o.status.code().unwrap_or(-1),
                        text.trim()
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .chars()
                            .take(120)
                            .collect::<String>()
                    )),
                ),
            }
        }
    };
    Tool {
        name: name.into(),
        path: Some(path.display().to_string()),
        version,
        error,
    }
}

/// Collect a report of this machine.
pub async fn collect() -> Report {
    use sysinfo::{Disks, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_cwd(UpdateKind::Always),
    );
    let mb = |b: u64| b / (1024 * 1024);
    let gb = |b: u64| (b as f64 / 1e9 * 10.0).round() / 10.0;
    let la = System::load_average();

    let mut disks: Vec<Disk> = Disks::new_with_refreshed_list()
        .list()
        .iter()
        .filter(|d| d.total_space() >= MIN_DISK_BYTES && !d.is_read_only())
        .map(|d| {
            let used = d.total_space().saturating_sub(d.available_space());
            Disk {
                mount: d.mount_point().display().to_string(),
                fs: d.file_system().to_string_lossy().into(),
                size_gb: gb(d.total_space()),
                used_gb: gb(used),
                used_pct: (used as f64 * 1000.0 / d.total_space() as f64).round() / 10.0,
                removable: d.is_removable(),
            }
        })
        .collect();
    disks.sort_by(|a, b| a.mount.cmp(&b.mount));
    disks.dedup_by(|a, b| a.mount == b.mount);

    let mut sessions: Vec<Session> = sys
        .processes()
        .values()
        .filter_map(|p| {
            let cmd: Vec<String> = p
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
            agent_of(&p.name().to_string_lossy(), &cmd).map(|agent| Session {
                agent: agent.into(),
                pid: p.pid().as_u32(),
                started_at: p.start_time(),
                dir: p
                    .cwd()
                    .and_then(|c| c.file_name())
                    .map(|n| n.to_string_lossy().into_owned()),
            })
        })
        .collect();
    sessions.sort_by_key(|s| s.pid);

    let mut tools = Vec::new();
    for name in TOOLS {
        tools.push(tool(name).await);
    }

    Report {
        probe_version: env!("CARGO_PKG_VERSION").into(),
        host: System::host_name().unwrap_or_default(),
        taken_at: chrono::Utc::now().timestamp(),
        os: Os {
            name: System::long_os_version().unwrap_or_default(),
            kernel: System::kernel_version().unwrap_or_default(),
            arch: System::cpu_arch(),
        },
        uptime_s: System::uptime(),
        cpu: Cpu {
            ncpu: sys.cpus().len(),
            load: [la.one, la.five, la.fifteen],
        },
        memory: Memory {
            total_mb: mb(sys.total_memory()),
            avail_mb: mb(sys.available_memory()),
            swap_total_mb: mb(sys.total_swap()),
            swap_used_mb: mb(sys.used_swap()),
        },
        disks,
        tools,
        sessions,
    }
}

/// The report for a terminal.
pub fn text(r: &Report) -> String {
    use std::fmt::Write;
    let mut o = String::new();
    let _ = writeln!(
        o,
        "{}  {} · {} · {}",
        r.host, r.os.name, r.os.kernel, r.os.arch
    );
    let _ = writeln!(
        o,
        "cpu   {} cores, load {:.1} {:.1} {:.1}",
        r.cpu.ncpu, r.cpu.load[0], r.cpu.load[1], r.cpu.load[2]
    );
    let _ = writeln!(
        o,
        "mem   {} MB free of {} MB, swap {} of {} MB",
        r.memory.avail_mb, r.memory.total_mb, r.memory.swap_used_mb, r.memory.swap_total_mb
    );
    for d in &r.disks {
        let _ = writeln!(
            o,
            "disk  {} {:.0}% of {} GB ({})",
            d.mount, d.used_pct, d.size_gb, d.fs
        );
    }
    for t in &r.tools {
        let v = match (&t.path, &t.version, &t.error) {
            (None, _, _) => "not installed".to_string(),
            (_, Some(v), _) => v.clone(),
            (_, None, Some(e)) => e.clone(),
            (_, None, None) => "?".into(),
        };
        let _ = writeln!(o, "tool  {:<8} {v}", t.name);
    }
    for s in &r.sessions {
        let _ = writeln!(
            o,
            "agent {:<8} pid {} in {}",
            s.agent,
            s.pid,
            s.dir.as_deref().unwrap_or("?")
        );
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_from_the_tools_own_words() {
        for (out, want) in [
            ("2.1.4 (Claude Code)", Some("2.1.4")),
            (
                "gh version 2.81.0 (2025-10-01)\nhttps://github.com/cli/cli",
                Some("2.81.0"),
            ),
            ("codex-cli 0.48.0", Some("0.48.0")),
            ("varve v0.14.0", Some("0.14.0")),
            (
                "GitHub Copilot CLI 0.0.354.\nRun 'copilot --help'",
                Some("0.0.354."),
            ),
            ("error: no varve.toml here", None),
            ("", None),
        ] {
            assert_eq!(parse_version(out).as_deref(), want, "{out}");
        }
    }

    #[test]
    fn agents_are_recognised_by_binary_or_node_script() {
        let cmd = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(agent_of("claude", &cmd(&["claude"])), Some("claude"));
        assert_eq!(
            agent_of(
                "node",
                &cmd(&["node", "/usr/lib/node_modules/@github/copilot/index.js"])
            ),
            Some("copilot")
        );
        assert_eq!(
            agent_of("node", &cmd(&["node", "/opt/bin/codex"])),
            Some("codex")
        );
        // a file or repo merely named after an agent is not the agent
        assert_eq!(agent_of("vim", &cmd(&["vim", "claude.md"])), None);
        assert_eq!(
            agent_of("node", &cmd(&["node", "server.js", "claude"])),
            None
        );
        assert_eq!(agent_of("claude-helper", &cmd(&[])), None);
    }
}

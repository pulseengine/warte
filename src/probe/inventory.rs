//! Agent and toolchain inventory (REQ-PROBE-010): what agents on this machine
//! actually run, which plugins they carry, and what runs on a schedule.
//!
//! - **varve shadowing** is checked in the agent's own environment: the
//!   session-start hook (`warte probe hook`) compares, for every tool varve
//!   has a shim for, the first match on the agent's PATH with the shim, and
//!   records only the tool names and the shadowing binaries' paths. The
//!   probe runs as a service with another PATH, so it cannot see this itself.
//! - **Plugins** come from Claude Code's `installed_plugins.json`.
//! - **Scheduled jobs**: the user's LaunchAgents on macOS, systemd timers on
//!   Linux, and the crontab's schedule and program name. Never arguments,
//!   environment or command lines: those can carry tokens.
//!
//! Read-only, and every external command is bounded by a timeout.

use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A command the inventory runs that takes longer is reported as an error.
const RUN_TIMEOUT: Duration = Duration::from_secs(5);
/// At most this many scheduled jobs are reported.
pub const JOBS_MAX: usize = 60;
/// A shadowing record older than this is dropped: no agent started lately.
pub const SHADOWING_FRESH_S: i64 = 14 * 24 * 3600;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Inventory {
    /// varve shims shadowed on the PATH agents run with; `None`: varve is
    /// not set up here, or no agent session started since the probe has
    /// had the hook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadowing: Option<Shadowing>,
    #[serde(default)]
    pub plugins: Vec<Plugin>,
    #[serde(default)]
    pub scheduled: Vec<Job>,
    /// What could not be read, said rather than left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Shadowing {
    /// When an agent session last started and the check ran.
    pub at: i64,
    /// varve's shim directory is on the agents' PATH at all.
    pub shims_on_path: bool,
    pub tools: Vec<Shadowed>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Shadowed {
    pub tool: String,
    /// The binary that runs instead of the shim, home shown as `~`.
    pub path: String,
    /// Its `--version`, read by the probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Plugin {
    pub name: String,
    pub marketplace: String,
    pub version: String,
    /// `user` or `project`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Job {
    /// launchd label, systemd timer unit, or `cron: <program>`.
    pub name: String,
    /// `launchd`, `systemd`, `systemd-user` or `cron`.
    pub via: String,
    /// When it runs, e.g. "every 3600 s", "Day=1 Hour=3", "at login".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub schedule: String,
    /// Exit status of the last run; negative is the signal that ended it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_exit: Option<i64>,
    /// The last run failed (systemd's `Result` is not `success`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub failed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub running: bool,
}

/// `~` for the home directory, so paths do not carry the user name around.
pub fn tilde(p: &Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

/// Where a binary comes from, coarsely: what the views show instead of its
/// path, which the read API does not publish (warte#46).
pub fn origin(path: &str) -> &'static str {
    let p = path;
    if p.contains("/.cargo/bin/") {
        "cargo install"
    } else if p.starts_with("/opt/homebrew/")
        || p.starts_with("/usr/local/Cellar/")
        || p.starts_with("/home/linuxbrew/")
    {
        "Homebrew"
    } else if p.contains("/.local/bin/") {
        "~/.local/bin"
    } else if p.contains("/.varve/") {
        "varve"
    } else if ["/usr/bin/", "/bin/", "/usr/local/bin/", "/usr/sbin/"]
        .iter()
        .any(|d| p.starts_with(d))
    {
        "system"
    } else if p.starts_with('~') || p.starts_with("/Users/") || p.starts_with("/home/") {
        "home directory"
    } else {
        "elsewhere"
    }
}

/// What the read API publishes of an inventory: no paths, no error output.
pub fn public(i: &Inventory) -> serde_json::Value {
    serde_json::json!({
        "shadowing": i.shadowing.as_ref().map(|s| serde_json::json!({
            "at": s.at,
            "shims_on_path": s.shims_on_path,
            "tools": s.tools.iter().map(|t| serde_json::json!({
                "tool": t.tool,
                "version": t.version,
                "origin": origin(&t.path),
            })).collect::<Vec<_>>(),
        })),
        "plugins": i.plugins,
        "scheduled": i.scheduled,
        "not_read": i.errors.len(),
    })
}

fn varve_shims() -> Option<PathBuf> {
    let root = std::env::var_os("VARVE_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".varve")))?;
    let d = root.join("shims");
    d.is_dir().then_some(d)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// For every shim in `shims`, the binary that runs first on `path_var` when it
/// is not that shim. Also whether `shims` is on the PATH at all. Pure apart
/// from reading the file system.
pub fn shadowed(path_var: &OsStr, shims: &Path) -> (bool, Vec<(String, PathBuf)>) {
    let dirs: Vec<PathBuf> = std::env::split_paths(path_var).collect();
    let on_path = dirs.iter().any(|d| same_dir(d, shims));
    let mut names: Vec<String> = std::fs::read_dir(shims)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    let mut out = Vec::new();
    for n in names {
        let first = dirs.iter().find(|d| d.join(&n).is_file());
        if let Some(d) = first {
            if !same_dir(d, shims) {
                let p = d.join(&n);
                out.push((n, p));
            }
        }
    }
    (on_path, out)
}

fn state_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state/warte"))
}

/// Run by the hook at session start, in the agent's environment: record
/// which varve tools the agent's PATH shadows. Never fails.
pub fn record_shadowing(now: i64) {
    let (Some(shims), Some(dir), Some(path)) =
        (varve_shims(), state_dir(), std::env::var_os("PATH"))
    else {
        return;
    };
    let (on_path, found) = shadowed(&path, &shims);
    let rec = Shadowing {
        at: now,
        shims_on_path: on_path,
        // absolute here; shown with `~` once the probe reads it
        tools: found
            .into_iter()
            .map(|(tool, p)| Shadowed {
                tool,
                path: p.display().to_string(),
                version: None,
            })
            .collect(),
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let tmp = dir.join("toolchain.json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec(&rec).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join("toolchain.json"));
    }
}

async fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    run_for(cmd, args, RUN_TIMEOUT).await
}

/// Run `cmd` with no stdin, at most `limit`; stdout on success, else a short reason.
pub(crate) async fn run_for(cmd: &str, args: &[&str], limit: Duration) -> Result<String, String> {
    let out = tokio::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(limit, out).await {
        Err(_) => Err(format!("{cmd} timed out")),
        Ok(Err(e)) => Err(format!("{cmd}: {e}")),
        Ok(Ok(o)) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(Ok(o)) => Err(format!(
            "{cmd} exited {}: {}",
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stderr)
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(120)
                .collect::<String>()
        )),
    }
}

/// The recorded shadowing, with each shadowing binary's version read now.
async fn shadowing(now: i64) -> Option<Shadowing> {
    let p = state_dir()?.join("toolchain.json");
    let mut s: Shadowing = serde_json::from_slice(&std::fs::read(p).ok()?).ok()?;
    if now - s.at > SHADOWING_FRESH_S {
        return None;
    }
    for t in &mut s.tools {
        let path = PathBuf::from(&t.path);
        if path.is_file() {
            t.version = run(&t.path, &["--version"])
                .await
                .ok()
                .and_then(|o| super::parse_version(&o));
        }
        t.path = tilde(&path);
    }
    Some(s)
}

/// Plugins from Claude Code's `installed_plugins.json` (version 2 layout:
/// `plugins: { "name@marketplace": [ { version, scope, .. } ] }`).
pub fn plugins_from(json: &str) -> Result<Vec<Plugin>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("installed_plugins.json: {e}"))?;
    let map = v
        .get("plugins")
        .and_then(|p| p.as_object())
        .ok_or("installed_plugins.json: no plugins map")?;
    let mut out = Vec::new();
    for (key, entries) in map {
        let (name, market) = key.split_once('@').unwrap_or((key.as_str(), ""));
        for e in entries.as_array().into_iter().flatten() {
            out.push(Plugin {
                name: name.into(),
                marketplace: market.into(),
                version: e["version"].as_str().unwrap_or("unknown").into(),
                scope: e["scope"].as_str().unwrap_or_default().into(),
            });
        }
    }
    out.sort_by(|a, b| (&a.name, &a.scope).cmp(&(&b.name, &b.scope)));
    Ok(out)
}

fn plugins() -> Result<Vec<Plugin>, String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(vec![]);
    };
    let p = PathBuf::from(home).join(".claude/plugins/installed_plugins.json");
    match std::fs::read_to_string(&p) {
        Ok(s) => plugins_from(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(format!("installed_plugins.json: {e}")),
    }
}

/// `launchctl list` → label → (running, last exit).
pub fn launchctl_status(out: &str) -> std::collections::HashMap<String, (bool, Option<i64>)> {
    out.lines()
        .skip(1)
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let (pid, st, label) = (f.next()?, f.next()?, f.next()?);
            Some((label.to_string(), (pid != "-", st.parse().ok())))
        })
        .collect()
}

/// A launchd plist (as JSON, from `plutil`) → when it runs.
pub fn launchd_schedule(plist: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    if let Some(n) = plist["StartInterval"].as_i64() {
        parts.push(format!("every {n} s"));
    }
    let cal = &plist["StartCalendarInterval"];
    let one = |d: &serde_json::Value| {
        d.as_object()
            .map(|m| {
                let mut kv: Vec<String> = m.iter().map(|(k, v)| format!("{k}={v}")).collect();
                kv.sort();
                kv.join(" ")
            })
            .unwrap_or_default()
    };
    match cal {
        serde_json::Value::Array(a) => {
            let s: Vec<String> = a.iter().map(one).collect();
            parts.push(s.join("; "));
        }
        serde_json::Value::Object(_) => parts.push(one(cal)),
        _ => {}
    }
    if plist["KeepAlive"].as_bool() == Some(true) || plist["KeepAlive"].is_object() {
        parts.push("kept alive".into());
    } else if plist["RunAtLoad"].as_bool() == Some(true) {
        parts.push("at login".into());
    }
    parts.retain(|p| !p.is_empty());
    parts.join(", ")
}

async fn launchd(errors: &mut Vec<String>) -> Vec<Job> {
    let Some(home) = std::env::var_os("HOME") else {
        return vec![];
    };
    let dir = PathBuf::from(home).join("Library/LaunchAgents");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let status = match run("launchctl", &["list"]).await {
        Ok(o) => launchctl_status(&o),
        Err(e) => {
            errors.push(e);
            Default::default()
        }
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "plist"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for f in files.into_iter().take(JOBS_MAX) {
        let path = f.display().to_string();
        let plist: serde_json::Value = run("plutil", &["-convert", "json", "-o", "-", &path])
            .await
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let label = plist["Label"]
            .as_str()
            .map(String::from)
            .or_else(|| f.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let (running, last_exit) = status.get(&label).cloned().unwrap_or((false, None));
        out.push(Job {
            name: label,
            via: "launchd".into(),
            schedule: launchd_schedule(&plist),
            last_exit,
            failed: last_exit.is_some_and(|x| x != 0),
            running,
        });
    }
    out
}

/// `systemctl list-timers --all --output=json` → (timer, service it starts).
pub fn timers_from(json: &str) -> Vec<(String, String)> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|t| {
            Some((
                t["unit"].as_str()?.to_string(),
                t["activates"].as_str()?.to_string(),
            ))
        })
        .collect()
}

async fn systemd(user: bool, errors: &mut Vec<String>) -> Vec<Job> {
    let mut base: Vec<&str> = vec![];
    if user {
        base.push("--user");
    }
    let mut args = base.clone();
    args.extend(["list-timers", "--all", "--output=json", "--no-pager"]);
    let timers = match run("systemctl", &args).await {
        Ok(o) => timers_from(&o),
        Err(e) => {
            // no user manager (a service account) is normal, not an error
            if !user {
                errors.push(e);
            }
            return vec![];
        }
    };
    let mut out = Vec::new();
    for (timer, service) in timers.into_iter().take(JOBS_MAX) {
        let mut a = base.clone();
        a.extend([
            "show",
            "--no-pager",
            "-p",
            "Result,ExecMainStatus,ActiveState",
            service.as_str(),
        ]);
        let props = run("systemctl", &a).await.unwrap_or_default();
        let get = |k: &str| {
            props
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{k}=")))
                .unwrap_or_default()
                .to_string()
        };
        let result = get("Result");
        let mut sa = base.clone();
        sa.extend([
            "show",
            "--no-pager",
            "-p",
            "TimersCalendar,TimersMonotonic",
            "--value",
            timer.as_str(),
        ]);
        let schedule = run("systemctl", &sa)
            .await
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                // "{ OnCalendar=*-*-* 03:00:00 ; next_elapse=... }"
                let l = l.trim().trim_start_matches('{').trim();
                let first = l.split(';').next()?.trim();
                (!first.is_empty()).then(|| first.to_string())
            })
            .collect::<Vec<_>>()
            .join(", ");
        out.push(Job {
            name: timer,
            via: if user { "systemd-user" } else { "systemd" }.into(),
            schedule,
            last_exit: get("ExecMainStatus").parse().ok(),
            failed: !result.is_empty() && result != "success",
            running: get("ActiveState") == "active",
        });
    }
    out
}

/// Crontab lines → (schedule, program name). Arguments are dropped: they can
/// carry tokens.
pub fn cron_from(crontab: &str) -> Vec<(String, String)> {
    crontab
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        // environment lines such as MAILTO=... or PATH=...
        .filter(|l| {
            let first = l.split_whitespace().next().unwrap_or_default();
            !first.contains('=')
        })
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let (sched, cmd) = if f.first()?.starts_with('@') {
                (f[0].to_string(), *f.get(1)?)
            } else {
                (f.get(..5)?.join(" "), *f.get(5)?)
            };
            let prog = Path::new(cmd)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())?;
            Some((sched, prog))
        })
        .collect()
}

async fn cron() -> Vec<Job> {
    // no crontab exits non-zero: nothing scheduled, not an error
    let Ok(t) = run("crontab", &["-l"]).await else {
        return vec![];
    };
    cron_from(&t)
        .into_iter()
        .map(|(schedule, prog)| Job {
            name: format!("cron: {prog}"),
            via: "cron".into(),
            schedule,
            last_exit: None,
            failed: false,
            running: false,
        })
        .collect()
}

pub async fn collect(now: i64) -> Inventory {
    let mut errors = Vec::new();
    let plugins = plugins().unwrap_or_else(|e| {
        errors.push(e);
        vec![]
    });
    let mut scheduled = Vec::new();
    if cfg!(target_os = "macos") {
        scheduled.extend(launchd(&mut errors).await);
    } else if cfg!(target_os = "linux") {
        scheduled.extend(systemd(false, &mut errors).await);
        scheduled.extend(systemd(true, &mut errors).await);
    }
    scheduled.extend(cron().await);
    scheduled.truncate(JOBS_MAX);
    Inventory {
        shadowing: shadowing(now).await,
        plugins,
        scheduled,
        errors,
    }
}

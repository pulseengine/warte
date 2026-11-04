//! What an agent is doing right now, as a verb (DD-PROBE-003).
//!
//! Claude Code runs `warte probe hook` on its hook events and passes the
//! event as JSON on stdin. The hook maps it to a verb ("editing", "running
//! tests", "waiting for you") and writes only that verb, keyed by the agent's
//! process id, to `~/.local/state/warte/activity/<pid>`. No prompt, file name,
//! command or path is written; the command is looked at locally to choose a
//! verb and then dropped. The probe joins the verb onto the session with that
//! pid.
//!
//! The hook never fails the agent: every error is swallowed and it prints
//! nothing.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A verb older than this is dropped (the agent moved on without a hook),
/// except "waiting for you", which holds until the next event.
pub const FRESH_S: i64 = 10 * 60;
pub const WAITING: &str = "waiting for you";

/// Every verb [`verb_of`] can produce: nothing else is ever written.
pub const VERBS: [&str; 15] = [
    "thinking",
    WAITING,
    "starting",
    "editing",
    "reading",
    "searching the web",
    "asking a helper",
    "planning",
    "running tests",
    "building",
    "using git",
    "talking to GitHub",
    "visiting another machine",
    "running a command",
    "working",
];

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Activity {
    pub verb: String,
    pub at: i64,
}

#[derive(Debug, Deserialize, Default)]
struct HookEvent {
    #[serde(default)]
    hook_event_name: String,
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    tool_input: serde_json::Value,
}

/// The verb for a hook event, or None for events that change nothing.
pub fn verb_of(event_json: &str) -> Option<&'static str> {
    let e: HookEvent = serde_json::from_str(event_json).ok()?;
    match e.hook_event_name.as_str() {
        "UserPromptSubmit" => Some("thinking"),
        "Stop" | "Notification" => Some(WAITING),
        "SessionStart" => Some("starting"),
        "PreToolUse" => Some(match e.tool_name.as_str() {
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "editing",
            "Read" | "Grep" | "Glob" | "LS" | "NotebookRead" => "reading",
            "WebSearch" | "WebFetch" => "searching the web",
            "Agent" | "Task" => "asking a helper",
            "TodoWrite" => "planning",
            "Bash" => {
                let cmd = e.tool_input["command"].as_str().unwrap_or_default();
                let has = |w: &str| {
                    cmd.split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                        .any(|t| t == w)
                };
                if has("test") || has("pytest") || has("nextest") || has("playwright") {
                    "running tests"
                } else if has("build") || has("zigbuild") || has("clippy") {
                    "building"
                } else if has("git") {
                    "using git"
                } else if has("gh") {
                    "talking to GitHub"
                } else if has("ssh") || has("scp") {
                    "visiting another machine"
                } else {
                    "running a command"
                }
            }
            _ => "working",
        }),
        _ => None,
    }
}

pub fn dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state/warte/activity"))
}

/// The agent process this hook was started for: the nearest ancestor that is
/// an agent (the hook runs under a shell the agent started).
fn agent_pid() -> Option<u32> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    let mut pid = Pid::from_u32(std::os::unix::process::parent_id());
    for _ in 0..8 {
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
        );
        let p = sys.process(pid)?;
        let cmd: Vec<String> = p
            .cmd()
            .iter()
            .map(|c| c.to_string_lossy().into_owned())
            .collect();
        if super::agent_of(&p.name().to_string_lossy(), &cmd).is_some() {
            return Some(pid.as_u32());
        }
        pid = p.parent()?;
    }
    None
}

/// `warte probe hook`: read the event, write the verb. Never fails.
pub fn hook(stdin: &str, now: i64) {
    let Some(verb) = verb_of(stdin) else { return };
    let (Some(pid), Some(dir)) = (agent_pid(), dir()) else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let body = serde_json::to_vec(&Activity {
        verb: verb.into(),
        at: now,
    })
    .unwrap_or_default();
    let tmp = dir.join(format!("{pid}.tmp"));
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(pid.to_string()));
    }
}

/// The current verb for an agent pid, if fresh. Files of pids that are gone
/// are removed by [`sweep`].
pub fn current(dir: &std::path::Path, pid: u32, now: i64) -> Option<String> {
    let a: Activity =
        serde_json::from_slice(&std::fs::read(dir.join(pid.to_string())).ok()?).ok()?;
    (a.verb == WAITING || now - a.at <= FRESH_S).then_some(a.verb)
}

/// Remove activity files whose process is not among `live`.
pub fn sweep(dir: &std::path::Path, live: &[u32]) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name();
        if let Ok(pid) = name.to_string_lossy().parse::<u32>() {
            if !live.contains(&pid) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // rivet: verifies REQ-PROBE-009
    #[test]
    fn events_become_verbs_and_nothing_else() {
        let v = |j: serde_json::Value| verb_of(&j.to_string());
        assert_eq!(
            v(
                serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"Edit","tool_input":{"file_path":"/secret/x.rs"}})
            ),
            Some("editing")
        );
        assert_eq!(
            v(
                serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo test --release"}})
            ),
            Some("running tests")
        );
        assert_eq!(
            v(
                serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git push -u origin HEAD"}})
            ),
            Some("using git")
        );
        assert_eq!(
            v(
                serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"echo latest"}})
            ),
            Some("running a command")
        );
        assert_eq!(
            v(serde_json::json!({"hook_event_name":"Stop"})),
            Some(WAITING)
        );
        assert_eq!(
            v(serde_json::json!({"hook_event_name":"UserPromptSubmit","prompt":"my secret plan"})),
            Some("thinking")
        );
        assert_eq!(
            v(serde_json::json!({"hook_event_name":"PostToolUse","tool_name":"Edit"})),
            None
        );
        assert_eq!(verb_of("not json"), None);
        // every verb produced is in VERBS
        for (ev, tool, cmd) in [
            ("UserPromptSubmit", "", ""),
            ("Stop", "", ""),
            ("SessionStart", "", ""),
            ("PreToolUse", "Edit", ""),
            ("PreToolUse", "Read", ""),
            ("PreToolUse", "WebFetch", ""),
            ("PreToolUse", "Agent", ""),
            ("PreToolUse", "TodoWrite", ""),
            ("PreToolUse", "Bash", "cargo test"),
            ("PreToolUse", "Bash", "cargo build"),
            ("PreToolUse", "Bash", "git log"),
            ("PreToolUse", "Bash", "gh pr list"),
            ("PreToolUse", "Bash", "ssh host"),
            ("PreToolUse", "Bash", "ls"),
            ("PreToolUse", "Other", ""),
        ] {
            let v = v(serde_json::json!({"hook_event_name": ev, "tool_name": tool,
                "tool_input": {"command": cmd}}))
            .unwrap();
            assert!(VERBS.contains(&v), "{v} not in VERBS");
        }
    }

    // rivet: verifies REQ-PROBE-009
    #[test]
    fn a_verb_goes_stale_but_waiting_holds_and_dead_pids_are_swept() {
        let d = tempfile::tempdir().unwrap();
        let w = |pid: u32, verb: &str, at: i64| {
            std::fs::write(
                d.path().join(pid.to_string()),
                serde_json::to_vec(&Activity {
                    verb: verb.into(),
                    at,
                })
                .unwrap(),
            )
            .unwrap()
        };
        w(1, "editing", 1_000);
        w(2, WAITING, 1_000);
        assert_eq!(
            current(d.path(), 1, 1_000 + FRESH_S),
            Some("editing".into())
        );
        assert_eq!(current(d.path(), 1, 1_001 + FRESH_S), None);
        assert_eq!(
            current(d.path(), 2, 1_000 + 10 * FRESH_S),
            Some(WAITING.into())
        );
        sweep(d.path(), &[2]);
        assert!(!d.path().join("1").exists() && d.path().join("2").exists());
    }
}

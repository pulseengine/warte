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
/// except "waiting for you" and "done", which hold until the next event.
pub const FRESH_S: i64 = 10 * 60;
/// The agent asked the user something: a permission prompt, a question, a
/// form. Only this is "waiting for you" (REQ-PROBE-009).
pub const WAITING: &str = "waiting for you";
/// The agent finished its turn; nobody is being asked anything.
pub const DONE: &str = "done";
/// [`DONE`] for longer than [`IDLE_AFTER_S`]. Never written by the hook.
pub const IDLE: &str = "idle";
pub const IDLE_AFTER_S: i64 = 30 * 60;
/// Activity files written by a hook that tells waiting from done.
const FORMAT: u8 = 2;

/// Every verb [`verb_of`] and [`current`] can produce: nothing else is ever
/// written or reported.
pub const VERBS: [&str; 17] = [
    "thinking",
    WAITING,
    DONE,
    IDLE,
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
    /// [`FORMAT`]; 0 for files of older hooks, whose "waiting for you" also
    /// meant a finished turn.
    #[serde(default)]
    pub v: u8,
}

#[derive(Debug, Deserialize, Default)]
struct HookEvent {
    /// Claude Code, Vibe and Copilot's PascalCase hooks: `hook_event_name`.
    #[serde(default, alias = "hookEventName")]
    hook_event_name: String,
    /// Claude `Edit`, Copilot `edit`, Vibe `file_system.bash`.
    #[serde(default, alias = "toolName")]
    tool_name: String,
    /// Copilot's camelCase hooks send `toolArgs`, sometimes as a JSON string.
    #[serde(default, alias = "toolArgs")]
    tool_input: serde_json::Value,
    #[serde(default)]
    notification_type: String,
    #[serde(default)]
    message: String,
}

/// The verb for a hook event, or None for events that change nothing.
pub fn verb_of(event_json: &str) -> Option<&'static str> {
    verb_of_as(event_json, None)
}

/// [`verb_of`], with the event named by the hook's own config
/// (`warte probe hook --event agentStop`) when the payload may not name it.
/// Claude Code, Copilot CLI (either casing) and Mistral Vibe.
pub fn verb_of_as(event_json: &str, event: Option<&str>) -> Option<&'static str> {
    let e: HookEvent = serde_json::from_str(event_json).ok()?;
    let name = event.unwrap_or(&e.hook_event_name);
    match name {
        "UserPromptSubmit" | "userPromptSubmitted" => Some("thinking"),
        // a finished turn: Claude's and Copilot's Stop, Vibe's post_agent
        "Stop" | "agentStop" | "post_agent" => Some(DONE),
        "Notification" | "notification" => match e.notification_type.as_str() {
            "permission_prompt"
            | "elicitation_dialog"
            | "elicitation_url_dialog"
            | "agent_needs_input" => Some(WAITING),
            "idle_prompt" | "agent_idle" | "agent_completed" => Some(DONE),
            // older Claude Code sends no type: only a permission prompt asks
            "" if e.message.to_ascii_lowercase().contains("permission") => Some(WAITING),
            "" => Some(DONE),
            _ => None,
        },
        "SessionStart" | "sessionStart" => Some("starting"),
        "PreToolUse" | "preToolUse" | "pre_tool" => Some(tool_verb(&e)),
        _ => None,
    }
}

/// The verb for a tool call, from the tool's bare name in any of the three
/// agents' spellings.
fn tool_verb(e: &HookEvent) -> &'static str {
    let leaf = e.tool_name.rsplit('.').next().unwrap_or_default();
    let tool = leaf.to_ascii_lowercase().replace('_', "");
    match tool.as_str() {
        "askuserquestion" | "askuser" => WAITING,
        "edit" | "write" | "multiedit" | "notebookedit" | "create" | "strreplaceeditor"
        | "applypatch" | "searchreplace" | "writefile" => "editing",
        "read" | "grep" | "glob" | "ls" | "notebookread" | "view" | "rg" | "readfile" => "reading",
        "websearch" | "webfetch" => "searching the web",
        "agent" | "task" => "asking a helper",
        "todowrite" | "updatetodo" | "todo" => "planning",
        "bash" | "shell" | "powershell" | "processstart" | "start" => {
            // Copilot's camelCase hooks pass the arguments as a JSON string
            let args = match &e.tool_input {
                serde_json::Value::String(s) => serde_json::from_str(s).unwrap_or_default(),
                v => v.clone(),
            };
            command_verb(args["command"].as_str().unwrap_or_default())
        }
        _ => "working",
    }
}

fn command_verb(cmd: &str) -> &'static str {
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
    hook_as(stdin, None, now)
}

/// [`hook`] for an event the hook's config names (`--event`).
pub fn hook_as(stdin: &str, event: Option<&str>, now: i64) {
    let Some(verb) = verb_of_as(stdin, event) else {
        return;
    };
    // the toolchain as this agent sees it, once per session (REQ-PROBE-010)
    if verb == "starting" {
        super::inventory::record_shadowing(now);
    }
    let (Some(pid), Some(dir)) = (agent_pid(), dir()) else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let body = serde_json::to_vec(&Activity {
        verb: verb.into(),
        at: now,
        v: FORMAT,
    })
    .unwrap_or_default();
    let tmp = dir.join(format!("{pid}.tmp"));
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(pid.to_string()));
    }
}

/// The current verb for an agent pid and since when, if it still holds.
/// "waiting for you" holds until the next event; "done" turns "idle" after
/// [`IDLE_AFTER_S`]; any other verb is dropped after [`FRESH_S`]. Files of
/// pids that are gone are removed by [`sweep`].
pub fn current(dir: &std::path::Path, pid: u32, now: i64) -> Option<(String, i64)> {
    let a: Activity =
        serde_json::from_slice(&std::fs::read(dir.join(pid.to_string())).ok()?).ok()?;
    let verb = match a.verb.as_str() {
        // an older hook wrote this for a finished turn too
        WAITING if a.v < FORMAT => DONE,
        WAITING => WAITING,
        DONE => DONE,
        v => VERBS.iter().copied().find(|x| *x == v)?,
    };
    match verb {
        WAITING => Some((verb.into(), a.at)),
        DONE if now - a.at > IDLE_AFTER_S => Some((IDLE.into(), a.at + IDLE_AFTER_S)),
        DONE => Some((verb.into(), a.at)),
        _ => (now - a.at <= FRESH_S).then(|| (verb.into(), a.at)),
    }
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
        // a finished turn is done; only a question to the user is waiting
        assert_eq!(v(serde_json::json!({"hook_event_name":"Stop"})), Some(DONE));
        let note = |t: &str, m: &str| {
            v(
                serde_json::json!({"hook_event_name":"Notification","notification_type":t,"message":m}),
            )
        };
        assert_eq!(note("permission_prompt", "x"), Some(WAITING));
        assert_eq!(note("elicitation_dialog", "x"), Some(WAITING));
        assert_eq!(note("agent_needs_input", "x"), Some(WAITING));
        assert_eq!(
            note("idle_prompt", "Claude is waiting for your input"),
            Some(DONE)
        );
        assert_eq!(note("auth_success", "x"), None);
        assert_eq!(
            note("", "Claude needs your permission to use Bash"),
            Some(WAITING)
        );
        assert_eq!(note("", "Claude is waiting for your input"), Some(DONE));
        // Copilot fires PermissionRequest even when no prompt follows: the
        // prompt itself comes as a Notification
        assert_eq!(
            v(serde_json::json!({"hook_event_name":"PermissionRequest","tool_name":"Bash"})),
            None
        );
        assert_eq!(
            v(serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"AskUserQuestion"})),
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
            ("Notification", "", ""),
            ("PreToolUse", "AskUserQuestion", ""),
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

    // rivet: verifies REQ-PROBE-014
    #[test]
    fn copilot_and_vibe_events_become_the_same_verbs() {
        let as_ = |j: serde_json::Value, ev: Option<&str>| verb_of_as(&j.to_string(), ev);
        // Copilot CLI, camelCase: the event comes from --event, args as a string
        assert_eq!(
            as_(
                serde_json::json!({"toolName":"bash","toolArgs":"{\"command\":\"cargo test\"}"}),
                Some("preToolUse")
            ),
            Some("running tests")
        );
        assert_eq!(
            as_(serde_json::json!({"toolName":"edit"}), Some("preToolUse")),
            Some("editing")
        );
        assert_eq!(
            as_(
                serde_json::json!({"toolName":"ask_user"}),
                Some("preToolUse")
            ),
            Some(WAITING)
        );
        assert_eq!(
            as_(
                serde_json::json!({"stopReason":"end_turn"}),
                Some("agentStop")
            ),
            Some(DONE)
        );
        assert_eq!(
            as_(
                serde_json::json!({"hook_event_name":"Notification","notification_type":"agent_idle"}),
                None
            ),
            Some(DONE)
        );
        assert_eq!(
            as_(
                serde_json::json!({"hook_event_name":"Notification","notification_type":"permission_prompt"}),
                None
            ),
            Some(WAITING)
        );
        assert_eq!(
            as_(serde_json::json!({}), Some("userPromptSubmitted")),
            Some("thinking")
        );
        // Mistral Vibe: hook_event_name in the payload, qualified tool names
        let vibe = |t: &str, input: serde_json::Value| {
            as_(
                serde_json::json!({"hook_event_name":"pre_tool","tool_name":t,"tool_input":input}),
                None,
            )
        };
        assert_eq!(
            vibe(
                "file_system.bash",
                serde_json::json!({"command":"git status"})
            ),
            Some("using git")
        );
        assert_eq!(
            vibe(
                "process.start",
                serde_json::json!({"command":"cargo build"})
            ),
            Some("building")
        );
        assert_eq!(
            vibe("file_system.search_replace", serde_json::json!({})),
            Some("editing")
        );
        assert_eq!(
            vibe("x.ask_user_question", serde_json::json!({})),
            Some(WAITING)
        );
        assert_eq!(
            vibe("mcp_github.list_prs", serde_json::json!({})),
            Some("working")
        );
        assert_eq!(
            as_(
                serde_json::json!({"hook_event_name":"post_agent","agent_name":"main"}),
                None
            ),
            Some(DONE)
        );
        assert_eq!(
            as_(serde_json::json!({"hook_event_name":"post_tool"}), None),
            None
        );
        // --event wins over the payload
        assert_eq!(
            as_(
                serde_json::json!({"hook_event_name":"Stop"}),
                Some("sessionStart")
            ),
            Some("starting")
        );
    }

    // rivet: verifies REQ-PROBE-009
    // rivet: verifies REQ-PROBE-014
    #[test]
    fn a_verb_goes_stale_but_waiting_holds_and_dead_pids_are_swept() {
        let d = tempfile::tempdir().unwrap();
        let w = |pid: u32, verb: &str, at: i64, v: u8| {
            std::fs::write(
                d.path().join(pid.to_string()),
                serde_json::to_vec(&Activity {
                    verb: verb.into(),
                    at,
                    v,
                })
                .unwrap(),
            )
            .unwrap()
        };
        let s = |v: &str, at: i64| Some((v.to_string(), at));
        w(1, "editing", 1_000, FORMAT);
        w(2, WAITING, 1_000, FORMAT);
        w(3, DONE, 1_000, FORMAT);
        // an older hook's "waiting for you" was also a finished turn
        w(4, WAITING, 1_000, 0);
        // a verb no hook writes is never reported
        w(5, "<b>x</b>", 1_000, FORMAT);
        assert_eq!(current(d.path(), 1, 1_000 + FRESH_S), s("editing", 1_000));
        assert_eq!(current(d.path(), 1, 1_001 + FRESH_S), None);
        assert_eq!(
            current(d.path(), 2, 1_000 + 100 * FRESH_S),
            s(WAITING, 1_000)
        );
        assert_eq!(current(d.path(), 3, 1_000 + IDLE_AFTER_S), s(DONE, 1_000));
        assert_eq!(
            current(d.path(), 3, 1_001 + IDLE_AFTER_S),
            s(IDLE, 1_000 + IDLE_AFTER_S)
        );
        assert_eq!(current(d.path(), 4, 1_060), s(DONE, 1_000));
        assert_eq!(
            current(d.path(), 4, 1_001 + IDLE_AFTER_S),
            s(IDLE, 1_000 + IDLE_AFTER_S)
        );
        assert_eq!(current(d.path(), 5, 1_000), None);
        sweep(d.path(), &[2]);
        assert!(!d.path().join("1").exists() && d.path().join("2").exists());
    }
}

//! Session lifecycle (REQ-SESS-002): contract lifecycle v1, in
//! `docs/topics/lifecycle.md`. Starts the sessions a sessions v1 file lists,
//! each in its own detached zellij session, and keeps per-session state on
//! the machine. It never kills a process and never deletes a transcript:
//! `rm` refuses while the session runs, and removes only warte's own state.

use super::{Session, SessionsFile};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 1;
/// A reason is required for every start, pause and resume, and kept.
pub const MAX_REASON: usize = 200;
/// zellij session names are `warte-<id>`.
pub const ZELLIJ_PREFIX: &str = "warte-";

/// What warte keeps per session on the machine: `<state>/<id>.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct State {
    #[serde(default)]
    pub paused: bool,
    /// The Claude Code session id last seen for the session's directory.
    #[serde(default)]
    pub last_session_id: Option<String>,
    #[serde(default)]
    pub last_start: Option<i64>,
    /// The remote-control name it was last started with, so `rm` can tell
    /// it runs even after the file no longer lists it.
    #[serde(default)]
    pub name: Option<String>,
}

/// A session's state as `status` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Its process runs.
    Running,
    /// Enabled and not paused, but no process: `start` starts it.
    Missing,
    Paused,
    Disabled,
}

impl Status {
    pub fn word(self) -> &'static str {
        match self {
            Status::Running => "running",
            Status::Missing => "missing",
            Status::Paused => "paused",
            Status::Disabled => "disabled",
        }
    }
}

pub fn status(s: &Session, st: &State, running: bool) -> Status {
    if running {
        Status::Running
    } else if !s.enabled {
        Status::Disabled
    } else if st.paused {
        Status::Paused
    } else {
        Status::Missing
    }
}

/// A reason: 1 to [`MAX_REASON`] characters after trimming, no control
/// characters (it goes into the event log as one line).
pub fn reason_ok(r: &str) -> Result<String, String> {
    let t = r.trim();
    if t.is_empty() || t.chars().count() > MAX_REASON || t.chars().any(char::is_control) {
        return Err(format!(
            "--reason must be 1 to {MAX_REASON} characters, no control characters"
        ));
    }
    Ok(t.to_string())
}

/// Claude Code's project directory name for a working directory: every
/// character that is not an ASCII letter or digit becomes `-`.
pub fn project_slug(dir: &str) -> String {
    dir.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The newest `<uuid>.jsonl` in the session's project directory, by
/// modification time; only file names and times are read, never content.
pub fn latest_session_id(claude_dir: &Path, dir: &str) -> Option<String> {
    let p = claude_dir.join("projects").join(project_slug(dir));
    std::fs::read_dir(p)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".jsonl")?.to_string();
            uuid_ok(&id).then_some(())?;
            Some((e.metadata().ok()?.modified().ok()?, id))
        })
        .max()
        .map(|(_, id)| id)
}

fn uuid_ok(s: &str) -> bool {
    s.len() == 36
        && s.chars().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

/// The argv that starts a session: a detached zellij session named
/// `warte-<id>` whose first pane runs Claude Code with the session's
/// remote-control name, model, effort and permission mode, resuming the
/// last session when one is known. Run with the session's `dir` as the
/// working directory.
pub fn start_argv(s: &Session, resume: Option<&str>) -> Vec<String> {
    let mut v: Vec<String> = vec![
        "zellij".into(),
        "attach".into(),
        "--create-background".into(),
        format!("{ZELLIJ_PREFIX}{}", s.id),
        "--".into(),
        "claude".into(),
        "--remote-control".into(),
        s.name.clone(),
        "--model".into(),
        s.model.clone(),
        "--effort".into(),
        s.effort.clone(),
        "--permission-mode".into(),
        s.mode.clone(),
    ];
    if let Some(id) = resume {
        v.push("--resume".into());
        v.push(id.into());
    }
    v
}

/// Whether a process's argv holds `--remote-control <name>` (or
/// `--remote-control=<name>`): the session is running.
pub fn argv_runs(argv: &[String], name: &str) -> bool {
    argv.iter().enumerate().any(|(i, a)| {
        (a == "--remote-control" && argv.get(i + 1).is_some_and(|n| n == name))
            || a.strip_prefix("--remote-control=") == Some(name)
    })
}

/// Names of the sessions whose process runs, from every process's argv.
pub fn running_names(f: &SessionsFile, argvs: &[Vec<String>]) -> Vec<String> {
    f.sessions
        .iter()
        .filter(|s| argvs.iter().any(|a| argv_runs(a, &s.name)))
        .map(|s| s.name.clone())
        .collect()
}

/// Every process's argv on this machine.
pub fn process_argvs() -> Vec<Vec<String>> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    sys.processes()
        .values()
        .filter(|p| p.thread_kind().is_none())
        .map(|p| {
            p.cmd()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        })
        .collect()
}

/// warte's lifecycle state for one machine.
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Store, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Store { dir: dir.into() })
    }

    fn path(&self, id: &str) -> PathBuf {
        // ids are [a-z0-9-] by the sessions v1 check: never a path
        self.dir.join(format!("{id}.json"))
    }

    pub fn get(&self, id: &str) -> State {
        std::fs::read(self.path(id))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn put(&self, id: &str, st: &State) -> Result<(), String> {
        let p = self.path(id);
        let tmp = p.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(st).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
    }

    /// One line per start, pause, resume and rm: `events.jsonl`.
    pub fn log(&self, at: i64, id: &str, action: &str, reason: &str) -> Result<(), String> {
        use std::io::Write;
        let line = serde_json::json!({"at": at, "id": id, "action": action, "reason": reason});
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("events.jsonl"))
            .map_err(|e| e.to_string())?;
        writeln!(f, "{line}").map_err(|e| e.to_string())
    }

    /// Remove the session's state; never anything else.
    pub fn remove(&self, id: &str) -> Result<bool, String> {
        match std::fs::remove_file(self.path(id)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// One session in `status`.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub status: Status,
    pub last_session_id: Option<String>,
}

pub fn rows(f: &SessionsFile, store: &Store, running: &[String], claude_dir: &Path) -> Vec<Row> {
    f.sessions
        .iter()
        .map(|s| {
            let st = store.get(&s.id);
            Row {
                id: s.id.clone(),
                name: s.name.clone(),
                status: status(s, &st, running.contains(&s.name)),
                last_session_id: latest_session_id(claude_dir, &s.dir).or(st.last_session_id),
            }
        })
        .collect()
}

/// The sessions `start` would start: missing ones, all or the one `only`
/// names. An `only` that names an unknown, disabled, paused or running
/// session is an error naming why.
pub fn to_start<'a>(
    f: &'a SessionsFile,
    store: &Store,
    running: &[String],
    only: Option<&str>,
) -> Result<Vec<&'a Session>, String> {
    let pick = |s: &&Session| only.is_none_or(|o| o == s.id);
    if let Some(o) = only {
        let Some(s) = f.sessions.iter().find(|s| s.id == o) else {
            return Err(format!("no session {o:?} in the file"));
        };
        let st = status(s, &store.get(o), running.contains(&s.name));
        if st != Status::Missing {
            return Err(format!("session {o} is {}, not missing", st.word()));
        }
    }
    Ok(f.sessions
        .iter()
        .filter(pick)
        .filter(|s| status(s, &store.get(&s.id), running.contains(&s.name)) == Status::Missing)
        .collect())
}

/// `rm`: refused while the session runs; removes warte's state for it
/// only, never the transcripts.
pub fn rm(
    store: &Store,
    f: &SessionsFile,
    id: &str,
    argvs: &[Vec<String>],
) -> Result<bool, String> {
    let listed = f
        .sessions
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.name.clone());
    let names = listed.into_iter().chain(store.get(id).name);
    for n in names {
        if argvs.iter().any(|a| argv_runs(a, &n)) {
            return Err(format!(
                "session {id} is running: stop it yourself first; warte never kills a session"
            ));
        }
    }
    store.remove(id)
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The lifecycle commands; returns the exit code.
pub fn run_cli(cmd: crate::cli::SessionsCommands, json: bool) -> Result<i32, String> {
    use crate::cli::SessionsCommands as C;
    let at = match &cmd {
        C::Status { at }
        | C::Start { at, .. }
        | C::Pause { at, .. }
        | C::Resume { at, .. }
        | C::Rm { at, .. } => at.clone(),
        C::Check { .. } => return Err("not a lifecycle command".into()),
    };
    let f = super::load(&at.config)?;
    let state_dir = match at.state {
        Some(p) => p,
        None => home()?.join(".local/state/warte/sessions"),
    };
    let claude_dir = match at.claude_dir {
        Some(p) => p,
        None => home()?.join(".claude"),
    };
    let store = Store::open(&state_dir)?;
    let argvs = process_argvs();
    let running = running_names(&f, &argvs);
    let known = |id: &str| -> Result<&Session, String> {
        f.sessions
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| format!("no session {id:?} in {}", at.config.display()))
    };
    match cmd {
        C::Check { .. } => unreachable!(),
        C::Status { .. } => {
            let rows = rows(&f, &store, &running, &claude_dir);
            if json {
                println!(
                    "{}",
                    serde_json::json!({"version": VERSION, "sessions": rows})
                );
            } else {
                for r in &rows {
                    println!(
                        "{:<32} {:<9} {}",
                        r.id,
                        r.status.word(),
                        r.last_session_id.as_deref().unwrap_or("-")
                    );
                }
            }
            Ok(0)
        }
        C::Start {
            id,
            reason,
            dry_run,
            ..
        } => {
            let reason = reason_ok(&reason)?;
            let list = to_start(&f, &store, &running, id.as_deref())?;
            let mut failed = 0;
            for s in list {
                let mut st = store.get(&s.id);
                let resume = latest_session_id(&claude_dir, &s.dir).or(st.last_session_id.clone());
                let argv = start_argv(s, resume.as_deref());
                if dry_run {
                    println!(
                        "{}",
                        serde_json::json!({"id": s.id, "dir": s.dir, "argv": argv})
                    );
                    continue;
                }
                let out = std::process::Command::new(&argv[0])
                    .args(&argv[1..])
                    .current_dir(&s.dir)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                match out {
                    Ok(x) if x.success() => {
                        st.last_session_id = resume;
                        st.last_start = Some(now());
                        st.name = Some(s.name.clone());
                        store.put(&s.id, &st)?;
                        store.log(now(), &s.id, "start", &reason)?;
                        if !json {
                            println!("started {}", s.id);
                        }
                    }
                    Ok(x) => {
                        failed += 1;
                        eprintln!(
                            "[sessions] {}: zellij exited {}",
                            s.id,
                            x.code().unwrap_or(-1)
                        );
                    }
                    Err(e) => {
                        failed += 1;
                        eprintln!("[sessions] {}: zellij: {}", s.id, e.kind());
                    }
                }
            }
            Ok(i32::from(failed > 0))
        }
        C::Pause { id, reason, .. } => set_paused(&store, known(&id)?, true, &reason, json),
        C::Resume { id, reason, .. } => set_paused(&store, known(&id)?, false, &reason, json),
        C::Rm { id, reason, .. } => {
            let reason = reason_ok(&reason)?;
            let removed = rm(&store, &f, &id, &argvs)?;
            store.log(now(), &id, "rm", &reason)?;
            if !json {
                println!(
                    "{} {id}; its transcripts are untouched",
                    if removed {
                        "forgot"
                    } else {
                        "nothing kept for"
                    }
                );
            }
            Ok(0)
        }
    }
}

fn set_paused(
    store: &Store,
    s: &Session,
    paused: bool,
    reason: &str,
    json: bool,
) -> Result<i32, String> {
    let reason = reason_ok(reason)?;
    let mut st = store.get(&s.id);
    st.paused = paused;
    store.put(&s.id, &st)?;
    let action = if paused { "pause" } else { "resume" };
    store.log(now(), &s.id, action, &reason)?;
    if !json {
        println!(
            "{} {}{}",
            if paused { "paused" } else { "resumed" },
            s.id,
            if paused {
                "; it keeps running if it runs: warte never stops a session"
            } else {
                ""
            }
        );
    }
    Ok(0)
}

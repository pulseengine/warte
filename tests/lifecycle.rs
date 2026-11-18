//! lifecycle v1: starting and keeping agent sessions (REQ-SESS-002), against
//! the contract in docs/topics/lifecycle.md. Every refusal is tested with
//! the bad input planted: a running session, a paused one, a bad reason, a
//! bad file.

use std::path::Path;
use std::process::{Child, Command, Output};
use warte::sessions::lifecycle::{argv_runs, latest_session_id, project_slug, start_argv};

const UUID_OLD: &str = "11111111-2222-3333-4444-555555555555";
const UUID_NEW: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

struct Fixture {
    dir: tempfile::TempDir,
    /// A unique remote-control name, so a parallel test never matches it.
    name: String,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let name = format!("lifecycle-test-{tag}-{}", std::process::id());
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let file = serde_json::json!({
            "version": 1,
            "sessions": [
                {"id": "one", "name": name, "dir": work, "model": "opus", "effort": "medium",
                 "mode": "bypassPermissions", "role": "workstation", "enabled": true},
                {"id": "off", "name": format!("{name}-off"), "dir": work, "model": "sonnet",
                 "effort": "low", "mode": "plan", "role": "workstation", "enabled": false}
            ]
        });
        std::fs::write(dir.path().join("sessions.json"), file.to_string()).unwrap();
        Fixture { dir, name }
    }

    fn p(&self, s: &str) -> std::path::PathBuf {
        self.dir.path().join(s)
    }

    fn run(&self, args: &[&str]) -> Output {
        let sub = args[0];
        let mut c = Command::new(env!("CARGO_BIN_EXE_warte"));
        c.args(["--format", "json", "sessions", sub])
            .arg("--config")
            .arg(self.p("sessions.json"))
            .arg("--state")
            .arg(self.p("state"))
            .arg("--claude-dir")
            .arg(self.p("claude"))
            .args(&args[1..]);
        c.output().unwrap()
    }

    /// A process that looks like the session running: its argv holds
    /// `--remote-control <name>`. Ours, killed by its own pid.
    fn fake_running(&self) -> Child {
        Command::new("sh")
            .args(["-c", "sleep 60; true", "sh", "--remote-control", &self.name])
            .spawn()
            .unwrap()
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn status_of(fx: &Fixture, id: &str) -> String {
    let o = fx.run(&["status"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    v["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_string()
}

fn transcript(fx: &Fixture, id: &str) -> std::path::PathBuf {
    let d = fx
        .p("claude/projects")
        .join(project_slug(fx.p("work").to_str().unwrap()));
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join(format!("{id}.jsonl"));
    std::fs::write(&f, "{}\n").unwrap();
    f
}

// rivet: verifies REQ-SESS-002
#[test]
fn start_starts_only_missing_sessions_with_the_listed_settings() {
    let fx = Fixture::new("start");
    let old = transcript(&fx, UUID_OLD);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    transcript(&fx, UUID_NEW);
    let o = fx.run(&["start", "--reason", "login", "--dry-run"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let lines: Vec<serde_json::Value> = stdout(&o)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // the disabled session is not started
    assert_eq!(lines.len(), 1, "{lines:?}");
    let argv: Vec<String> = serde_json::from_value(lines[0]["argv"].clone()).unwrap();
    assert_eq!(
        argv,
        [
            "zellij",
            "attach",
            "--create-background",
            "warte-one",
            "--",
            "claude",
            "--remote-control",
            &fx.name,
            "--model",
            "opus",
            "--effort",
            "medium",
            "--permission-mode",
            "bypassPermissions",
            "--resume",
            UUID_NEW,
        ]
    );
    assert_eq!(lines[0]["dir"], fx.p("work").to_str().unwrap());
    assert!(old.exists());

    // planted: the session already runs, so nothing starts
    let mut child = fx.fake_running();
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(status_of(&fx, "one"), "running");
    let o = fx.run(&["start", "--reason", "login", "--dry-run"]);
    assert!(o.status.success());
    assert_eq!(
        stdout(&o).trim(),
        "",
        "a running session is never started again"
    );
    let o = fx.run(&["start", "--id", "one", "--reason", "x", "--dry-run"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("is running, not missing"),
        "{}",
        stderr(&o)
    );
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(status_of(&fx, "one"), "missing");
    assert_eq!(status_of(&fx, "off"), "disabled");
}

// rivet: verifies REQ-SESS-002
#[test]
fn a_paused_session_is_not_started_until_resumed() {
    let fx = Fixture::new("pause");
    let o = fx.run(&["pause", "one", "--reason", "owner away"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(status_of(&fx, "one"), "paused");
    let o = fx.run(&["start", "--reason", "login", "--dry-run"]);
    assert_eq!(stdout(&o).trim(), "");
    let o = fx.run(&["start", "--id", "one", "--reason", "x", "--dry-run"]);
    assert!(stderr(&o).contains("is paused"), "{}", stderr(&o));
    fx.run(&["resume", "one", "--reason", "back"]);
    assert_eq!(status_of(&fx, "one"), "missing");
    let log = std::fs::read_to_string(fx.p("state/events.jsonl")).unwrap();
    assert!(
        log.contains("\"action\":\"pause\"") && log.contains("owner away"),
        "{log}"
    );
}

// rivet: verifies REQ-SESS-002
#[test]
fn rm_refuses_while_the_session_runs_and_never_touches_transcripts() {
    let fx = Fixture::new("rm");
    let t = transcript(&fx, UUID_OLD);
    fx.run(&["pause", "one", "--reason", "x"]);
    assert!(fx.p("state/one.json").exists());
    // planted: it runs
    let mut child = fx.fake_running();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let o = fx.run(&["rm", "one", "--reason", "retired"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("is running"), "{}", stderr(&o));
    assert!(
        fx.p("state/one.json").exists(),
        "a refused rm keeps the state"
    );
    let alive = child.try_wait().unwrap().is_none();
    assert!(alive, "rm never kills the session");
    child.kill().unwrap();
    child.wait().unwrap();
    let o = fx.run(&["rm", "one", "--reason", "retired"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!fx.p("state/one.json").exists());
    assert!(t.exists(), "transcripts are never deleted");
}

// rivet: verifies REQ-SESS-002
#[test]
fn bad_reasons_unknown_ids_and_bad_files_are_refused() {
    let fx = Fixture::new("bad");
    for bad in ["", "   ", "line\nbreak", &"x".repeat(201)] {
        let o = fx.run(&["start", "--reason", bad, "--dry-run"]);
        assert_eq!(o.status.code(), Some(1), "{bad:?}");
        assert!(stderr(&o).contains("--reason"), "{}", stderr(&o));
    }
    let o = fx.run(&["pause", "nope", "--reason", "x"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("no session \"nope\""), "{}", stderr(&o));
    // planted: a file sessions v1 refuses stops every lifecycle command
    std::fs::write(
        fx.p("sessions.json"),
        r#"{"version": 1, "sessions": [{"id": "BAD"}]}"#,
    )
    .unwrap();
    for sub in ["status", "start"] {
        let o = fx.run(&[sub, "--reason", "x", "--dry-run"][..if sub == "status" { 1 } else { 4 }]);
        assert_eq!(o.status.code(), Some(1), "{sub}");
        assert!(stderr(&o).contains("[sessions]"), "{sub}: {}", stderr(&o));
    }
}

// rivet: verifies REQ-SESS-002
#[test]
fn the_last_session_id_and_the_running_test_read_names_only() {
    assert_eq!(
        project_slug("/Volumes/Home/git/pulseengine/pulseengine.eu"),
        "-Volumes-Home-git-pulseengine-pulseengine-eu"
    );
    let d = tempfile::tempdir().unwrap();
    let proj = d.path().join("projects").join(project_slug("/w"));
    std::fs::create_dir_all(&proj).unwrap();
    assert_eq!(latest_session_id(d.path(), "/w"), None);
    // planted: files that are not session ids are ignored
    for n in [
        "notes.jsonl",
        "x.json",
        "11111111-2222-3333-4444-55555555555g.jsonl",
    ] {
        std::fs::write(proj.join(n), "").unwrap();
    }
    assert_eq!(latest_session_id(d.path(), "/w"), None);
    std::fs::write(proj.join(format!("{UUID_OLD}.jsonl")), "").unwrap();
    assert_eq!(latest_session_id(d.path(), "/w").as_deref(), Some(UUID_OLD));
    assert!(Path::new(d.path()).exists());

    let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(argv_runs(
        &a(&["claude", "--remote-control", "warte"]),
        "warte"
    ));
    assert!(argv_runs(
        &a(&["node", "cli.js", "--remote-control=warte"]),
        "warte"
    ));
    // planted near misses
    assert!(!argv_runs(
        &a(&["claude", "--remote-control", "warte2"]),
        "warte"
    ));
    assert!(!argv_runs(&a(&["claude", "--remote-control"]), "warte"));
    assert!(!argv_runs(
        &a(&["grep", "warte", "--remote-control-x", "warte"]),
        "warte"
    ));

    let s: warte::sessions::Session = serde_json::from_value(serde_json::json!({
        "id": "a", "name": "A b", "dir": "/w", "model": "m", "effort": "low",
        "mode": "plan", "role": "r", "enabled": true}))
    .unwrap();
    let v = start_argv(&s, None);
    assert!(!v.contains(&"--resume".to_string()));
    assert_eq!(v[7], "A b", "the name is one argument, never split");
}

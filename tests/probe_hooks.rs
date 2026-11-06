//! Oracles for `warte probe install-hooks` (DD-PROBE-003): each agent's
//! hook is offered, existing settings are kept exactly, a second run adds
//! nothing, and nothing is written without a yes.

use warte::probe::hooks::{self, CLAUDE_EVENTS, COPILOT_EVENTS};

const W: &str = "/opt/warte/bin/warte";

const SETTINGS: &str = r#"{
  "permissions": {"allow": ["Bash(ls:*)"],   "deny": []},
  "hooks": {
    "Stop": [{"hooks": [{"type": "command", "command": "/Users/x/.config/iterm2/cc-status"}]}],
    "SessionEnd": [{"hooks": [{"type": "command", "command": "/Users/x/.config/iterm2/cc-status"}]}]
  },
  "model": "opus"
}"#;

// rivet: verifies DD-PROBE-003
#[test]
fn claude_gets_every_event_and_keeps_everything_else_exactly() {
    let after = hooks::claude(Some(SETTINGS), W).unwrap().expect("a change");
    let v: serde_json::Value = serde_json::from_str(&after).unwrap();
    // the untouched key keeps its exact text, odd spacing included
    assert!(
        after.contains(r#"{"allow": ["Bash(ls:*)"],   "deny": []}"#),
        "{after}"
    );
    assert_eq!(v["model"], "opus");
    // key order kept: permissions, hooks, model
    let p = |k: &str| after.find(&format!("\"{k}\"")).unwrap();
    assert!(p("permissions") < p("hooks") && p("hooks") < p("model"));
    for ev in CLAUDE_EVENTS {
        let list = v["hooks"][ev].as_array().unwrap();
        let ours: Vec<_> = list
            .iter()
            .filter(|e| e.to_string().contains("probe hook"))
            .collect();
        assert_eq!(ours.len(), 1, "{ev}: {after}");
        assert_eq!(
            ours[0]["hooks"][0]["command"],
            format!("\"{W}\" probe hook >/dev/null 2>&1 || true")
        );
    }
    assert_eq!(
        v["hooks"]["PreToolUse"][0]["matcher"], "*",
        "PreToolUse matches every tool"
    );
    // iTerm2's hooks stay, ours comes after them
    assert!(v["hooks"]["Stop"][0].to_string().contains("cc-status"));
    assert!(v["hooks"]["SessionEnd"][0]
        .to_string()
        .contains("cc-status"));
    // a second run adds nothing
    assert_eq!(hooks::claude(Some(&after), W).unwrap(), None);
    // an older install (another path) counts as installed
    let old = after.replace(W, "$HOME/.local/bin/warte");
    assert_eq!(hooks::claude(Some(&old), W).unwrap(), None);
    // no file, an empty file: a new one; not JSON: an error, nothing written
    assert!(hooks::claude(None, W).unwrap().is_some());
    assert!(hooks::claude(Some(""), W).unwrap().is_some());
    assert!(hooks::claude(Some("{oops"), W).is_err());
    assert!(hooks::claude(Some(r#"{"hooks": 3}"#), W).is_err());
}

// rivet: verifies DD-PROBE-003
#[test]
fn copilot_gets_its_own_file_with_the_event_named() {
    let after = hooks::copilot(None, W).unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(v["version"], 1);
    for ev in COPILOT_EVENTS {
        let cmd = v["hooks"][ev][0]["bash"].as_str().unwrap();
        assert_eq!(
            cmd,
            format!("\"{W}\" probe hook --event {ev} >/dev/null 2>&1 || true")
        );
    }
    assert_eq!(hooks::copilot(Some(&after), W).unwrap(), None);
}

// rivet: verifies DD-PROBE-003
#[test]
fn vibe_gets_two_entries_appended_and_the_rest_untouched() {
    let mine =
        "# my guard\n[[hooks]]\nname = \"deny-rm-rf\"\ntype = \"pre_tool\"\ncommand = \"guard\"\n";
    let after = hooks::vibe(Some(mine), W).unwrap().unwrap();
    assert!(after.starts_with(mine), "{after}");
    let v: toml::Value = toml::from_str(&after).unwrap();
    let hs = v["hooks"].as_array().unwrap();
    assert_eq!(hs.len(), 3);
    let kinds: Vec<&str> = hs[1..]
        .iter()
        .map(|h| h["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["pre_tool", "post_agent"]);
    assert_eq!(
        hs[1]["command"].as_str().unwrap(),
        format!("\"{W}\" probe hook >/dev/null 2>&1 || true")
    );
    assert_eq!(hooks::vibe(Some(&after), W).unwrap(), None);
    assert!(hooks::vibe(None, W).unwrap().is_some());
    assert!(hooks::vibe(Some("[[hooks"), W).is_err());
}

// rivet: verifies DD-PROBE-003
#[test]
fn only_agents_present_are_offered_and_a_write_keeps_a_backup() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".vibe")).unwrap();
    std::fs::write(home.path().join(".vibe/hooks.toml"), "# mine\n").unwrap();
    let plan = hooks::plan(home.path(), W).unwrap();
    assert_eq!(plan.len(), 1, "only Vibe is here");
    assert_eq!(plan[0].agent, "Mistral Vibe");
    let d = hooks::diff(plan[0].before.as_deref(), &plan[0].after);
    assert!(
        d.lines().all(|l| l.starts_with("+ ")),
        "only additions: {d}"
    );
    let backup = hooks::apply(&plan[0], 1_800_000_000).unwrap().unwrap();
    assert_eq!(std::fs::read_to_string(backup).unwrap(), "# mine\n");
    assert!(hooks::plan(home.path(), W).unwrap().is_empty());
}

// rivet: verifies DD-PROBE-003
#[test]
fn the_command_writes_nothing_without_a_yes() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".copilot")).unwrap();
    let run = |args: &[&str], stdin: &str| {
        use std::io::Write;
        let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_warte"))
            .args(["probe", "install-hooks"])
            .args(args)
            .env("HOME", home.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        let o = c.wait_with_output().unwrap();
        assert!(o.status.success());
        String::from_utf8(o.stdout).unwrap()
    };
    let file = home.path().join(".copilot/hooks/warte.json");
    let out = run(&[], "n\n");
    assert!(
        out.contains("Copilot CLI") && out.contains("Nothing written."),
        "{out}"
    );
    assert!(!file.exists());
    let out = run(&["--yes"], "");
    assert!(out.contains("wrote"), "{out}");
    assert!(file.exists());
    assert!(run(&[], "").contains("already has the warte hook"));
}

//! `warte probe install-hooks` (DD-PROBE-003): offer the activity hook to
//! each agent on this machine, show the exact change, and write it only after
//! the user confirms. Existing hooks are kept, the file is backed up first,
//! and nothing is written for an agent whose hook is already there.
//!
//! - Claude Code: `~/.claude/settings.json`, edited in place. Every key the
//!   installer does not touch is kept byte for byte (raw JSON values), so the
//!   user's other settings and hooks (e.g. iTerm2's) are not reformatted.
//! - Copilot CLI: its own file, `~/.copilot/hooks/warte.json`, with the event
//!   named on the command line (`--event`), since its payloads differ by
//!   casing.
//! - Mistral Vibe: two `[[hooks]]` entries appended to `~/.vibe/hooks.toml`.
//!   Vibe has no session-start or prompt event; its sessions show their
//!   tools and "done".

use anyhow::Context;
use indexmap::IndexMap;
use serde_json::value::RawValue;
use std::path::{Path, PathBuf};

/// Marks the installer's own entries, so a second run adds nothing.
const MARK: &str = "probe hook";

/// The Claude Code events the hook listens to.
pub const CLAUDE_EVENTS: [&str; 5] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "Stop",
    "Notification",
];
/// Copilot CLI events (camelCase, named again with `--event`).
pub const COPILOT_EVENTS: [&str; 5] = [
    "sessionStart",
    "userPromptSubmitted",
    "preToolUse",
    "agentStop",
    "notification",
];
/// Vibe hook types and the names of the installer's entries.
pub const VIBE_HOOKS: [(&str, &str); 2] = [
    ("warte-activity-tool", "pre_tool"),
    ("warte-activity-done", "post_agent"),
];

/// One file the installer would write.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub agent: &'static str,
    pub path: PathBuf,
    /// The file as it is; `None` when it does not exist yet.
    pub before: Option<String>,
    pub after: String,
}

/// A command that runs `warte probe hook`, prints nothing and never fails the
/// agent. `warte` is the absolute path of this binary.
pub fn command(warte: &str, event: Option<&str>) -> String {
    let ev = event.map(|e| format!(" --event {e}")).unwrap_or_default();
    format!("\"{warte}\" {MARK}{ev} >/dev/null 2>&1 || true")
}

fn has_mark(raw: &str) -> bool {
    raw.contains(MARK) && raw.contains("warte")
}

/// Claude Code's settings with the hook added for every event that lacks it,
/// or `None` when nothing is missing. Keys and entries the installer does not
/// change keep their exact text.
pub fn claude(settings: Option<&str>, warte: &str) -> anyhow::Result<Option<String>> {
    let text = settings.filter(|s| !s.trim().is_empty()).unwrap_or("{}");
    let mut top: IndexMap<String, Box<RawValue>> =
        serde_json::from_str(text).context("~/.claude/settings.json is not a JSON object")?;
    let mut hooks: IndexMap<String, Box<RawValue>> = match top.get("hooks") {
        Some(h) => serde_json::from_str(h.get()).context("\"hooks\" is not an object")?,
        None => IndexMap::new(),
    };
    let entry = serde_json::json!({
        "hooks": [{"type": "command", "command": command(warte, None), "timeout": 5}]
    });
    let mut changed = false;
    for ev in CLAUDE_EVENTS {
        let mut list: Vec<Box<RawValue>> = match hooks.get(ev) {
            Some(l) => serde_json::from_str(l.get())
                .with_context(|| format!("hooks.{ev} is not a list"))?,
            None => Vec::new(),
        };
        if list.iter().any(|e| has_mark(e.get())) {
            continue;
        }
        let mut e = entry.clone();
        if ev == "PreToolUse" {
            e["matcher"] = "*".into();
        }
        list.push(RawValue::from_string(serde_json::to_string(&e)?)?);
        hooks.insert(
            ev.into(),
            RawValue::from_string(serde_json::to_string(&list)?)?,
        );
        changed = true;
    }
    if !changed {
        return Ok(None);
    }
    top.insert(
        "hooks".into(),
        RawValue::from_string(serde_json::to_string_pretty(&hooks)?)?,
    );
    Ok(Some(serde_json::to_string_pretty(&top)? + "\n"))
}

/// Copilot CLI's hook file, or `None` when it is already the one we write.
pub fn copilot(current: Option<&str>, warte: &str) -> anyhow::Result<Option<String>> {
    let mut hooks = serde_json::Map::new();
    for ev in COPILOT_EVENTS {
        hooks.insert(
            ev.into(),
            serde_json::json!([{"type": "command", "bash": command(warte, Some(ev)), "timeoutSec": 5}]),
        );
    }
    let after =
        serde_json::to_string_pretty(&serde_json::json!({"version": 1, "hooks": hooks}))? + "\n";
    Ok((current != Some(after.as_str())).then_some(after))
}

/// Vibe's hooks.toml with the installer's entries appended, or `None` when
/// they are there. The rest of the file is not touched.
pub fn vibe(current: Option<&str>, warte: &str) -> anyhow::Result<Option<String>> {
    let text = current.unwrap_or("");
    let parsed: toml::Value = toml::from_str(text).context("~/.vibe/hooks.toml is not TOML")?;
    let names: Vec<&str> = parsed
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|h| h.get("name").and_then(|n| n.as_str()))
                .collect()
        })
        .unwrap_or_default();
    let mut add = String::new();
    for (name, kind) in VIBE_HOOKS {
        if names.contains(&name) {
            continue;
        }
        let cmd = toml::Value::String(command(warte, None)).to_string();
        add += &format!(
            "\n[[hooks]]\nname = \"{name}\"\ntype = \"{kind}\"\ncommand = {cmd}\ntimeout = 5\ndescription = \"warte: what this agent is doing (REQ-PROBE-014)\"\n"
        );
    }
    if add.is_empty() {
        return Ok(None);
    }
    let mut after = text.to_string();
    if !after.is_empty() && !after.ends_with('\n') {
        after.push('\n');
    }
    after += if after.is_empty() {
        add.trim_start()
    } else {
        &add
    };
    // what we write must still be valid for Vibe
    toml::from_str::<toml::Value>(&after).context("the new hooks.toml would not parse")?;
    Ok(Some(after))
}

fn read(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// The changes for every agent found under `home`: an agent counts as
/// present when its config directory exists.
pub fn plan(home: &Path, warte: &str) -> anyhow::Result<Vec<Change>> {
    let mut out = Vec::new();
    type Edit = fn(Option<&str>, &str) -> anyhow::Result<Option<String>>;
    let agents: [(&'static str, &str, &str, Edit); 3] = [
        ("Claude Code", ".claude", ".claude/settings.json", claude),
        (
            "Copilot CLI",
            ".copilot",
            ".copilot/hooks/warte.json",
            copilot,
        ),
        ("Mistral Vibe", ".vibe", ".vibe/hooks.toml", vibe),
    ];
    for (agent, dir, file, edit) in agents {
        if !home.join(dir).is_dir() {
            continue;
        }
        let path = home.join(file);
        let before = read(&path)?;
        if let Some(after) = edit(before.as_deref(), warte)? {
            out.push(Change {
                agent,
                path,
                before,
                after,
            });
        }
    }
    Ok(out)
}

/// The lines that differ, for the confirmation (`-` removed, `+` added).
pub fn diff(before: Option<&str>, after: &str) -> String {
    let old: Vec<&str> = before.unwrap_or("").lines().collect();
    let new: Vec<&str> = after.lines().collect();
    // common prefix and suffix; what is between is the change
    let pre = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let max_suf = old.len().min(new.len()) - pre;
    let suf = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suf)
        .take_while(|(a, b)| a == b)
        .count();
    let mut s = String::new();
    for l in &old[pre..old.len() - suf] {
        s += &format!("- {l}\n");
    }
    for l in &new[pre..new.len() - suf] {
        s += &format!("+ {l}\n");
    }
    s
}

/// Write one change: back the old file up next to it, then replace it
/// atomically. Returns the backup's path, if there was a file.
pub fn apply(c: &Change, now: i64) -> anyhow::Result<Option<PathBuf>> {
    if let Some(dir) = c.path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let backup = match &c.before {
        Some(old) => {
            let b = c.path.with_extension(format!(
                "{}.warte-backup-{now}",
                c.path.extension().and_then(|e| e.to_str()).unwrap_or("")
            ));
            std::fs::write(&b, old).with_context(|| format!("backing up to {}", b.display()))?;
            Some(b)
        }
        None => None,
    };
    let tmp = c.path.with_extension("warte-tmp");
    std::fs::write(&tmp, &c.after).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &c.path).with_context(|| format!("replacing {}", c.path.display()))?;
    Ok(backup)
}

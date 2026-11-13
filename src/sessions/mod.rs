//! The agent session list (REQ-SESS-001): contract sessions v1, in
//! `docs/topics/sessions.md`. The operator's configuration renders one JSON
//! file per machine; `warte sessions check` refuses a file that breaks a rule
//! and names the field, so a bad list stops at render time.

use serde::{Deserialize, Serialize};

/// One agent session.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub dir: String,
    pub model: String,
    pub effort: String,
    pub mode: String,
    pub role: String,
    pub enabled: bool,
}

/// A sessions v1 file.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SessionsFile {
    pub version: u32,
    pub sessions: Vec<Session>,
}

pub const VERSION: u32 = 1;
pub const MAX_SESSIONS: usize = 64;
pub const MAX_ID: usize = 32;
pub const MAX_NAME: usize = 64;
pub const MAX_DIR: usize = 1024;
pub const MAX_MODEL: usize = 64;
pub const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
pub const MODES: &[&str] = &[
    "acceptEdits",
    "auto",
    "bypassPermissions",
    "manual",
    "dontAsk",
    "plan",
];

fn slug_ok(s: &str) -> bool {
    (1..=MAX_ID).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn dir_ok(d: &str) -> bool {
    d.starts_with('/')
        && d.len() <= MAX_DIR
        && !d.chars().any(|c| c.is_control())
        && !d.split('/').any(|p| p == "..")
}

/// Check a parsed file against the sessions v1 rules.
pub fn check(f: &SessionsFile) -> Result<(), String> {
    if f.version != VERSION {
        return Err(format!("version is {}, sessions v1 needs 1", f.version));
    }
    if f.sessions.is_empty() || f.sessions.len() > MAX_SESSIONS {
        return Err(format!(
            "sessions must hold 1 to {MAX_SESSIONS} entries, not {}",
            f.sessions.len()
        ));
    }
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for (i, s) in f.sessions.iter().enumerate() {
        let at = format!("sessions[{i}]");
        if !slug_ok(&s.id) {
            return Err(format!(
                "{at}.id must be [a-z0-9-], 1 to {MAX_ID} characters"
            ));
        }
        if !ids.insert(s.id.as_str()) {
            return Err(format!("{at}.id {:?} is not unique", s.id));
        }
        let n = s.name.trim();
        if !(1..=MAX_NAME).contains(&n.chars().count()) || n.chars().any(|c| c.is_control()) {
            return Err(format!(
                "{at}.name must be 1 to {MAX_NAME} characters, without control characters"
            ));
        }
        if !names.insert(n.to_lowercase()) {
            return Err(format!("{at}.name {n:?} is not unique"));
        }
        if !dir_ok(&s.dir) {
            return Err(format!(
                "{at}.dir must be an absolute path without \"..\" or control characters, at most {MAX_DIR} bytes"
            ));
        }
        let model_ok = (1..=MAX_MODEL).contains(&s.model.len())
            && s.model
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-[]".contains(c));
        if !model_ok {
            return Err(format!(
                "{at}.model must be 1 to {MAX_MODEL} characters of [A-Za-z0-9._-[]]"
            ));
        }
        if !EFFORTS.contains(&s.effort.as_str()) {
            return Err(format!("{at}.effort must be one of {}", EFFORTS.join(", ")));
        }
        if !MODES.contains(&s.mode.as_str()) {
            return Err(format!("{at}.mode must be one of {}", MODES.join(", ")));
        }
        if !slug_ok(&s.role) {
            return Err(format!(
                "{at}.role must be [a-z0-9-], 1 to {MAX_ID} characters (a machine role, not a hostname)"
            ));
        }
    }
    Ok(())
}

/// Parse a file's text; a type error names the field it is in. Objects must
/// be objects: serde alone also accepts a positional array for a struct.
pub fn parse(text: &str) -> Result<SessionsFile, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if !v.is_object() {
        return Err("the file must be a JSON object".into());
    }
    if let Some(list) = v["sessions"].as_array() {
        if let Some(i) = list.iter().position(|a| !a.is_object()) {
            return Err(format!("sessions[{i}] must be an object"));
        }
    }
    let de = &mut serde_json::Deserializer::from_str(text);
    serde_path_to_error::deserialize(de).map_err(|e| {
        let at = e.path().to_string();
        if at == "." {
            e.inner().to_string()
        } else {
            format!("{at}: {}", e.inner())
        }
    })
}

/// Read, parse and check a file; `Err` names the file and the rule.
pub fn load(path: &std::path::Path) -> Result<SessionsFile, String> {
    let at = path.display();
    let text = std::fs::read_to_string(path).map_err(|e| format!("[sessions] {at}: {e}"))?;
    let f = parse(&text).map_err(|e| format!("[sessions] {at}: {e}"))?;
    check(&f).map_err(|e| format!("[sessions] {at}: {e}"))?;
    Ok(f)
}

//! The home link and app switcher (REQ-NAV-001): contract nav v1, in
//! `docs/topics/nav.md`. One JSON file, written by the operator and shared by
//! every app on the host, is read and checked once at startup. A file that
//! breaks a rule stops startup with the rule named.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// `[nav]` in warte.toml.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NavConfig {
    /// The nav v1 file.
    pub file: PathBuf,
    /// This app's id in the file.
    #[serde(rename = "self")]
    pub self_id: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Home {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub desc: Option<String>,
}

/// A nav v1 file.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NavFile {
    pub version: u32,
    pub home: Home,
    pub apps: Vec<App>,
}

pub const VERSION: u32 = 1;
pub const MAX_APPS: usize = 20;
pub const MAX_NAME: usize = 40;
pub const MAX_DESC: usize = 120;
pub const MAX_ID: usize = 32;

fn url_ok(u: &str) -> bool {
    let shape = (u.starts_with('/') && !u.starts_with("//")) || u.starts_with("https://");
    shape
        && !u
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "\"'<>\\".contains(c))
}

fn name_ok(s: &str) -> bool {
    let n = s.trim().chars().count();
    (1..=MAX_NAME).contains(&n)
}

/// Check a parsed file against the nav v1 rules and this app's id.
pub fn check(f: &NavFile, self_id: &str) -> Result<(), String> {
    if f.version != VERSION {
        return Err(format!("version is {}, nav v1 needs 1", f.version));
    }
    if !name_ok(&f.home.name) {
        return Err(format!("home.name must be 1 to {MAX_NAME} characters"));
    }
    if !url_ok(&f.home.url) {
        return Err(
            "home.url must be \"/…\" (not \"//\") or \"https://…\", without spaces, quotes, <, > or \\"
                .into(),
        );
    }
    if f.apps.is_empty() || f.apps.len() > MAX_APPS {
        return Err(format!(
            "apps must hold 1 to {MAX_APPS} entries, not {}",
            f.apps.len()
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for (i, a) in f.apps.iter().enumerate() {
        let at = format!("apps[{i}]");
        let id_ok = (1..=MAX_ID).contains(&a.id.len())
            && a.id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !id_ok {
            return Err(format!(
                "{at}.id must be [a-z0-9-], 1 to {MAX_ID} characters"
            ));
        }
        if !seen.insert(a.id.as_str()) {
            return Err(format!("{at}.id {:?} is not unique", a.id));
        }
        if !name_ok(&a.name) {
            return Err(format!("{at}.name must be 1 to {MAX_NAME} characters"));
        }
        if !url_ok(&a.url) {
            return Err(format!(
                "{at}.url must be \"/…\" (not \"//\") or \"https://…\", without spaces, quotes, <, > or \\"
            ));
        }
        if a.desc
            .as_ref()
            .is_some_and(|d| d.chars().count() > MAX_DESC)
        {
            return Err(format!("{at}.desc must be at most {MAX_DESC} characters"));
        }
    }
    if !seen.contains(self_id) {
        return Err(format!("self {self_id:?} is not one of the apps' ids"));
    }
    Ok(())
}

/// Parse a file's text; a type error names the field it is in. Objects must
/// be objects: serde alone also accepts a positional array for a struct.
pub fn parse(text: &str) -> Result<NavFile, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if !v.is_object() {
        return Err("the file must be a JSON object".into());
    }
    if !v["home"].is_object() && v.get("home").is_some() {
        return Err("home must be an object".into());
    }
    if let Some(apps) = v["apps"].as_array() {
        if let Some(i) = apps.iter().position(|a| !a.is_object()) {
            return Err(format!("apps[{i}] must be an object"));
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

/// Read, parse and check the file; `Err` names the file and the rule.
pub fn load(cfg: &NavConfig) -> Result<NavFile, String> {
    let at = cfg.file.display();
    let text = std::fs::read_to_string(&cfg.file).map_err(|e| format!("[nav] {at}: {e}"))?;
    let f = parse(&text).map_err(|e| format!("[nav] {at}: {e}"))?;
    check(&f, &cfg.self_id).map_err(|e| format!("[nav] {at}: {e}"))?;
    Ok(f)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The header's nav: the home link, then every app in file order; this app's
/// entry carries `aria-current="page"`. Exactly `1 + apps.len()` links.
pub fn html(f: &NavFile, self_id: &str) -> String {
    let mut s = format!(
        "<nav class=\"apps\" aria-label=\"apps\"><a class=\"home\" href=\"{}\">← {}</a>",
        esc(&f.home.url),
        esc(f.home.name.trim())
    );
    for a in &f.apps {
        let title = a
            .desc
            .as_deref()
            .map(|d| format!(" title=\"{}\"", esc(d)))
            .unwrap_or_default();
        let current = if a.id == self_id {
            " aria-current=\"page\""
        } else {
            ""
        };
        s.push_str(&format!(
            "<a href=\"{}\"{title}{current}>{}</a>",
            esc(&a.url),
            esc(a.name.trim())
        ));
    }
    s.push_str("</nav>");
    s
}

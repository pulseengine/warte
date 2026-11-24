//! Push alerts to an ntfy topic (REQ-ALERT-003). Every notice the alerts
//! loop records (an outage finding opening, escalating or clearing) is also
//! published to the `[notify]` topic, so it reaches a phone without anyone
//! reading the notices. The loop's hysteresis and its cap of
//! [`super::MAX_PER_TICK`] per evaluation apply unchanged.
//!
//! A push carries the finding's text with every path replaced by `[path]`,
//! the host when the text names it, and a priority from the severity. It
//! never carries the finding key, a token or an error body. Without
//! `[notify]` nothing is sent.

use serde::{Deserialize, Serialize};

/// `[notify]` in warte.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotifyConfig {
    /// The ntfy server, e.g. `https://ntfy.example/` (https, or http on
    /// loopback only).
    pub ntfy_url: String,
    /// The topic: 1 to 64 of A-Z, a-z, 0-9, `-` and `_`.
    pub topic: String,
    /// A file holding the ntfy access token (mode 600). Read on every push,
    /// so a rotated token needs no restart; never logged.
    #[serde(default)]
    pub token_file: Option<String>,
    /// Where tapping the push leads, e.g. warte's Outages view (https).
    #[serde(default)]
    pub click: Option<String>,
}

fn loopback_http(u: &str) -> bool {
    ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|p| {
            u.strip_prefix(p)
                .is_some_and(|r| r.is_empty() || r.starts_with([':', '/']))
        })
}

impl NotifyConfig {
    /// Refuse a config that would send alerts in clear text or to a
    /// malformed topic. An error names the key.
    pub fn check(&self) -> Result<(), String> {
        if !(self.ntfy_url.starts_with("https://") || loopback_http(&self.ntfy_url)) {
            return Err("[notify] ntfy_url: https, or http on loopback only".into());
        }
        let t = &self.topic;
        if t.is_empty()
            || t.len() > 64
            || !t
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("[notify] topic: 1 to 64 of A-Z, a-z, 0-9, - and _".into());
        }
        if let Some(c) = &self.click {
            if !c.starts_with("https://") {
                return Err("[notify] click: an https URL".into());
            }
        }
        if let Some(f) = &self.token_file {
            if f.trim().is_empty() {
                return Err("[notify] token_file: empty".into());
            }
        }
        Ok(())
    }
}

/// Replace every path-like word (starting with `/` or `~/`) with `[path]`,
/// keeping trailing punctuation.
pub fn redact(text: &str) -> String {
    text.split(' ')
        .map(|w| {
            let core = w.trim_end_matches(['.', ',', ':', ';', ')']);
            let lead = core.trim_start_matches(['(', '"', '\'']);
            if lead.starts_with('/') || lead.starts_with("~/") {
                let pre = &core[..core.len() - lead.len()];
                format!("{pre}[path]{}", &w[core.len()..])
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The ntfy JSON message for one recorded notice (`notice.v1`).
pub fn message(cfg: &NotifyConfig, n: &serde_json::Value) -> serde_json::Value {
    let tags: Vec<&str> = n["tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|t| t.as_str()).collect())
        .unwrap_or_default();
    let (priority, word, emoji) = if tags.contains(&"cleared") {
        (2, "cleared", "white_check_mark")
    } else if tags.contains(&"down") {
        (4, "down", "rotating_light")
    } else {
        (3, "degraded", "warning")
    };
    let text = redact(n["title"].as_str().unwrap_or_default());
    let mut m = serde_json::json!({
        "topic": cfg.topic,
        "title": format!("warte: {word}"),
        "message": text.chars().take(300).collect::<String>(),
        "priority": priority,
        "tags": [emoji],
    });
    if let Some(c) = &cfg.click {
        m["click"] = serde_json::json!(c);
    }
    m
}

/// Publish the notices to the topic; returns how many were accepted. A
/// failure is logged with the status only, never a body or the token.
pub async fn deliver(
    http: &reqwest::Client,
    cfg: &NotifyConfig,
    notices: &[serde_json::Value],
) -> usize {
    let token = match cfg.token_file.as_deref().map(crate::reviews::read_token) {
        None => None,
        Some(Ok(t)) => Some(t),
        Some(Err(_)) => {
            tracing::warn!("ntfy token file unreadable or not mode 600; no push sent");
            return 0;
        }
    };
    let mut ok = 0;
    for n in notices {
        let mut req = http
            .post(&cfg.ntfy_url)
            .timeout(std::time::Duration::from_secs(10))
            .json(&message(cfg, n));
        if let Some(t) = &token {
            req = req.bearer_auth(t);
        }
        match req.send().await {
            Ok(r) if r.status().is_success() => ok += 1,
            Ok(r) => tracing::warn!(status = r.status().as_u16(), "ntfy push refused"),
            Err(e) => tracing::warn!(
                timeout = e.is_timeout(),
                connect = e.is_connect(),
                "ntfy push failed"
            ),
        }
    }
    ok
}

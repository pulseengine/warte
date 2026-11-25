//! A house's warnings as alerts (REQ-ALERT-005). With a `[house]` section the
//! alerts loop reads the house controller's warning list every minute and
//! turns each `alarm` warning into a red finding keyed by the warning's id, so
//! it opens, clears and is pushed like any outage. A `check` warning, or a
//! severity warte does not know, is left to the house's own page.
//!
//! A `sensor_silent` warning waits until it has held for `silent_hold_min`
//! (6 hours by default): a sensor that misses a few readings is not news.
//!
//! When the list cannot be read, the last good list stands for
//! [`STALE_AFTER_S`]; after that its warnings clear. A read error never
//! reaches the alert text.

use crate::dashboard::fleet::Detected;
use serde::{Deserialize, Serialize};

/// How long the last good list stands when a read fails.
pub const STALE_AFTER_S: i64 = 600;
/// The default hold for `sensor_silent`, in minutes.
pub const SILENT_HOLD_MIN: u32 = 360;
const TEXT_MAX: usize = 160;

/// `[house]` in warte.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HouseConfig {
    /// The warning list, e.g. `http://127.0.0.1:8080/api/warnings`
    /// (https, or http on loopback only).
    pub url: String,
    /// Minutes a `sensor_silent` warning must hold before it alerts.
    #[serde(default = "default_hold")]
    pub silent_hold_min: u32,
}

fn default_hold() -> u32 {
    SILENT_HOLD_MIN
}

impl HouseConfig {
    /// Refuse a URL that would read the house in clear text over a network.
    pub fn check(&self) -> Result<(), String> {
        if !(self.url.starts_with("https://") || super::ntfy::loopback_http(&self.url)) {
            return Err("[house] url: https, or http on loopback only".into());
        }
        Ok(())
    }
}

/// One entry of the house's warning list. Unknown fields are ignored, so the
/// house can add some without breaking warte.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Warning {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub room: Option<String>,
    #[serde(default)]
    pub message: String,
    pub severity: String,
    #[serde(default)]
    pub since_ms: Option<i64>,
}

fn valid_id(id: &str) -> bool {
    id.len() == 16 && id.chars().all(|c| c.is_ascii_hexdigit())
}

/// The red findings for one warning list at `now_ms`.
pub fn detected(ws: &[Warning], now_ms: i64, hold_min: u32) -> Vec<Detected> {
    let hold_ms = i64::from(hold_min) * 60_000;
    ws.iter()
        .filter(|w| w.severity == "alarm" && valid_id(&w.id))
        .filter(|w| w.kind != "sensor_silent" || w.since_ms.is_some_and(|s| now_ms - s >= hold_ms))
        .map(|w| {
            let msg: String = w.message.chars().take(TEXT_MAX).collect();
            let text = match w.room.as_deref().filter(|r| !r.is_empty()) {
                Some(r) => format!("House, {r}: {msg}"),
                None => format!("House: {msg}"),
            };
            Detected {
                key: format!("house:{}", w.id.to_ascii_lowercase()),
                sev: 2,
                text,
            }
        })
        .collect()
}

/// The reader the alerts loop keeps between evaluations.
#[derive(Debug, Default)]
pub struct Reader {
    last: Vec<Warning>,
    at: i64,
}

impl Reader {
    /// The list to evaluate: a fresh one, or the last good one while it is
    /// younger than [`STALE_AFTER_S`], else none.
    pub fn take(&mut self, fresh: Option<Vec<Warning>>, now: i64) -> &[Warning] {
        match fresh {
            Some(l) => {
                self.last = l;
                self.at = now;
            }
            None if now - self.at > STALE_AFTER_S => self.last.clear(),
            None => {}
        }
        &self.last
    }
}

/// Read the warning list; `None` on any failure (logged without the body).
pub async fn fetch(http: &reqwest::Client, cfg: &HouseConfig) -> Option<Vec<Warning>> {
    let r = http
        .get(&cfg.url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;
    match r {
        Ok(r) if r.status().is_success() => match r.json::<Vec<Warning>>().await {
            Ok(l) => Some(l),
            Err(_) => {
                tracing::warn!("house warnings: not a warning list");
                None
            }
        },
        Ok(r) => {
            tracing::warn!(status = r.status().as_u16(), "house warnings: refused");
            None
        }
        Err(_) => {
            tracing::warn!("house warnings: unreachable");
            None
        }
    }
}

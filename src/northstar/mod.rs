//! The North Star's indicators, as published (REQ-NS-001).
//!
//! pulseengine.eu publishes the North Star (`data/north-star.toml`) and one
//! file per measured objective (`data/north-star/<id>.json`). warte polls them
//! over https, keeps every distinct observation append-only, and shows per
//! objective the value, the change against the previous observation, its age,
//! the guard and the published blind spot. warte renders and does not compute:
//! an objective without a file is "unmeasured", never 0.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;

fn default_base_url() -> String {
    "https://raw.githubusercontent.com/pulseengine/pulseengine.eu/main/data".into()
}
fn default_interval_s() -> u64 {
    6 * 3600
}
fn default_stale_after_days() -> i64 {
    45
}

/// `[north_star]` in warte.toml.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NorthStarConfig {
    /// Where `north-star.toml` and `north-star/<id>.json` are, over https.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_interval_s")]
    pub interval_s: u64,
    /// An observation older than this is shown as stale, not as current.
    #[serde(default = "default_stale_after_days")]
    pub stale_after_days: i64,
}

impl NorthStarConfig {
    pub fn check(&self) -> Result<(), String> {
        if !crate::news::link_ok(&self.base_url) {
            return Err("[north_star] base_url must be an https URL".into());
        }
        if self.interval_s < 600 {
            return Err("[north_star] interval_s must be at least 600".into());
        }
        if !(1..=3650).contains(&self.stale_after_days) {
            return Err("[north_star] stale_after_days must be 1 to 3650".into());
        }
        Ok(())
    }
}

/// An objective as the North Star file names it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Objective {
    pub id: String,
    pub title: String,
    pub guard: String,
}

/// The published file: only the fields warte shows; the rest is ignored.
pub fn parse_objectives(toml_text: &str) -> Result<Vec<Objective>, String> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        objectives: Vec<Raw>,
    }
    #[derive(Deserialize)]
    struct Raw {
        id: String,
        title: String,
        #[serde(default)]
        guard: String,
    }
    let f: File =
        toml::from_str(toml_text).map_err(|_| "north-star.toml does not parse".to_string())?;
    let out: Vec<Objective> = f
        .objectives
        .into_iter()
        .filter(|o| id_ok(&o.id))
        .map(|o| Objective {
            id: o.id,
            title: clean(&o.title, 120),
            guard: clean(&o.guard, 600),
        })
        .collect();
    if out.is_empty() {
        return Err("north-star.toml names no objective".into());
    }
    Ok(out)
}

/// An objective id as the North Star uses them: `O` and digits.
pub fn id_ok(id: &str) -> bool {
    id.len() <= 4
        && id.starts_with('O')
        && id.len() > 1
        && id[1..].chars().all(|c| c.is_ascii_digit())
}

/// Printable text, cut to `max` characters: published text is shown as data.
fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// One published measurement of one objective.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Observation {
    pub objective: String,
    /// When the publisher measured it (RFC 3339, as published).
    pub measured_at: String,
    pub value: Option<f64>,
    /// The denominator, when the indicator is a count of something.
    pub of: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blind_spot: Option<String>,
    /// The guard's own value, once publishers send one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_value: Option<String>,
    /// The publisher's schema version; 0 until one is sent.
    #[serde(default)]
    pub schema_version: u32,
}

/// Read a published `<id>.json` (schema 0: no version field yet). The file
/// must be for `objective` and carry a parseable `measured_at`; numbers are
/// taken as published; anything else in the file is ignored.
pub fn parse_observation(objective: &str, json: &str) -> Result<Observation, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|_| "not JSON".to_string())?;
    if v["objective"].as_str() != Some(objective) {
        return Err("the file names another objective".into());
    }
    let at = v["measured_at"].as_str().ok_or("no measured_at")?;
    measured_unix(at).ok_or("measured_at is not a time")?;
    let num = |k: &str| v[k].as_f64().filter(|x| x.is_finite());
    let text = |k: &str, max: usize| {
        v[k].as_str()
            .map(|s| clean(s, max))
            .filter(|s| !s.is_empty())
    };
    Ok(Observation {
        objective: objective.to_string(),
        measured_at: clean(at, 40),
        value: num("value"),
        of: num("of"),
        verifier: text("verifier", 200),
        blind_spot: text("blind_spot", 600),
        guard_value: v["guard_value"]
            .as_str()
            .map(|s| clean(s, 200))
            .or_else(|| num("guard_value").map(|x| x.to_string())),
        schema_version: v["schema_version"].as_u64().unwrap_or(0).min(1000) as u32,
    })
}

/// `measured_at` as unix seconds; the publisher writes `2026-11-18T11:32Z`
/// (no seconds) as well as full RFC 3339.
pub fn measured_unix(s: &str) -> Option<i64> {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(t.timestamp());
    }
    chrono::NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), "%Y-%m-%dT%H:%M")
        .ok()
        .map(|t| t.and_utc().timestamp())
}

/// What warte shows for one objective.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub guard: String,
    /// `measured`, `stale` (older than the limit) or `unmeasured`.
    pub state: String,
    pub latest: Option<Observation>,
    pub previous: Option<Observation>,
    pub age_days: Option<i64>,
}

/// Every objective of the North Star, in its order, with its latest and
/// previous observation (REQ-NS-001). No observation means `unmeasured`, and
/// so does a file that was gone at the last poll (`published`, when known):
/// the history is kept, but the publisher no longer measures it.
pub fn rows(
    objectives: &[Objective],
    history: &[Observation],
    published: Option<&std::collections::BTreeSet<String>>,
    now: i64,
    stale_after_days: i64,
) -> Vec<Row> {
    objectives
        .iter()
        .map(|o| {
            let gone = published.is_some_and(|p| !p.contains(&o.id));
            let mut mine: Vec<&Observation> = history
                .iter()
                .filter(|h| h.objective == o.id && !gone)
                .collect();
            mine.sort_by_key(|h| measured_unix(&h.measured_at).unwrap_or(0));
            let latest = mine.last().map(|h| (*h).clone());
            let previous = mine.len().checked_sub(2).map(|i| mine[i].clone());
            let age_days = latest
                .as_ref()
                .and_then(|l| measured_unix(&l.measured_at))
                .map(|t| (now - t).max(0) / 86_400);
            let state = match age_days {
                None => "unmeasured",
                Some(d) if d > stale_after_days => "stale",
                Some(_) => "measured",
            };
            Row {
                id: o.id.clone(),
                title: o.title.clone(),
                guard: o.guard.clone(),
                state: state.into(),
                latest,
                previous,
                age_days,
            }
        })
        .collect()
}

fn num(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        format!("{x:.2}")
    }
}

/// One line for a glance: each measured objective's value and age, then how
/// many are unmeasured or stale. Plain text; escape before rendering.
pub fn glance_line(rows: &[Row]) -> String {
    let mut parts: Vec<String> = rows
        .iter()
        .filter(|r| r.state == "measured")
        .filter_map(|r| {
            let l = r.latest.as_ref()?;
            let v = match (l.value, l.of) {
                (Some(v), Some(of)) => format!("{}/{}", num(v), num(of)),
                (Some(v), None) => num(v),
                _ => "?".into(),
            };
            Some(format!("{} {v} ({} d)", r.id, r.age_days.unwrap_or(0)))
        })
        .collect();
    let unmeasured = rows.iter().filter(|r| r.state == "unmeasured").count();
    let stale = rows.iter().filter(|r| r.state == "stale").count();
    if unmeasured > 0 {
        parts.push(format!("{unmeasured} unmeasured"));
    }
    if stale > 0 {
        parts.push(format!("{stale} stale"));
    }
    parts.join(" · ")
}

/// The kept observations, one JSON line each, append-only.
pub struct Store {
    path: std::path::PathBuf,
    all: Mutex<Vec<Observation>>,
}

impl Store {
    pub fn open(data_dir: &std::path::Path) -> Store {
        let path = data_dir.join("north-star.jsonl");
        let all = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        Store {
            path,
            all: Mutex::new(all),
        }
    }

    /// Keep `o` unless the same objective and `measured_at` is already kept.
    pub fn add(&self, o: Observation) -> std::io::Result<bool> {
        use std::io::Write;
        let mut all = self.all.lock().unwrap_or_else(|e| e.into_inner());
        if all
            .iter()
            .any(|x| x.objective == o.objective && x.measured_at == o.measured_at)
        {
            return Ok(false);
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(
            f,
            "{}",
            serde_json::to_string(&o).map_err(std::io::Error::other)?
        )?;
        all.push(o);
        Ok(true)
    }

    pub fn all(&self) -> Vec<Observation> {
        self.all.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// The poller and what it last saw.
pub struct NorthStar {
    pub config: NorthStarConfig,
    pub store: Store,
    objectives: Mutex<Vec<Objective>>,
    /// The objectives whose file existed at the last poll; none before it.
    published: Mutex<Option<std::collections::BTreeSet<String>>>,
    /// The last poll: when, and its result kind (`ok`, `http 404`, ...).
    pub last_poll: Mutex<Option<(i64, String)>>,
    http: reqwest::Client,
}

/// At most 1 MiB per published file.
const MAX_BODY: usize = 1 << 20;

impl NorthStar {
    pub fn new(config: NorthStarConfig, data_dir: &std::path::Path) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!(
                "warte/",
                env!("CARGO_PKG_VERSION"),
                " (north star)"
            ))
            .timeout(Duration::from_secs(30))
            // A redirect may not leave https.
            .redirect(reqwest::redirect::Policy::custom(|a| {
                if a.previous().len() >= 5 || a.url().scheme() != "https" {
                    a.stop()
                } else {
                    a.follow()
                }
            }))
            .build()
            .expect("reqwest client");
        NorthStar {
            config,
            store: Store::open(data_dir),
            objectives: Mutex::new(vec![]),
            published: Mutex::new(None),
            last_poll: Mutex::new(None),
            http,
        }
    }

    pub fn objectives(&self) -> Vec<Objective> {
        self.objectives
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn rows(&self, now: i64) -> Vec<Row> {
        let published = self
            .published
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        rows(
            &self.objectives(),
            &self.store.all(),
            published.as_ref(),
            now,
            self.config.stale_after_days,
        )
    }

    async fn fetch(&self, url: &str) -> Result<Option<String>, String> {
        let r = self.http.get(url).send().await.map_err(|e| {
            if e.is_timeout() {
                "timeout".to_string()
            } else {
                "fetch".to_string()
            }
        })?;
        if r.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !r.status().is_success() {
            return Err(format!("http {}", r.status().as_u16()));
        }
        if r.content_length().is_some_and(|n| n > MAX_BODY as u64) {
            return Err("too large".into());
        }
        let mut r = r;
        let mut b = Vec::new();
        while let Some(c) = r.chunk().await.map_err(|_| "fetch".to_string())? {
            if b.len() + c.len() > MAX_BODY {
                return Err("too large".into());
            }
            b.extend_from_slice(&c);
        }
        Ok(Some(String::from_utf8_lossy(&b).into_owned()))
    }

    /// Read the North Star and each objective's file once. A missing file is
    /// no observation (the objective stays unmeasured); a file that does not
    /// parse is skipped and named in the result.
    pub async fn poll(&self, now: i64) {
        let base = self.config.base_url.trim_end_matches('/');
        let result = match self.fetch(&format!("{base}/north-star.toml")).await {
            Err(e) => e,
            Ok(None) => "north-star.toml not found".into(),
            Ok(Some(t)) => match parse_objectives(&t) {
                Err(e) => e,
                Ok(objs) => {
                    let mut bad = vec![];
                    let mut published = std::collections::BTreeSet::new();
                    for o in &objs {
                        match self
                            .fetch(&format!("{base}/north-star/{}.json", o.id))
                            .await
                        {
                            Ok(None) => {}
                            Ok(Some(j)) => match parse_observation(&o.id, &j) {
                                Ok(obs) => {
                                    published.insert(o.id.clone());
                                    if self.store.add(obs).is_err() {
                                        bad.push(format!("{} not stored", o.id));
                                    }
                                }
                                Err(_) => bad.push(format!("{} unreadable", o.id)),
                            },
                            // unreachable is not gone: keep what was shown
                            Err(e) => {
                                published.insert(o.id.clone());
                                bad.push(format!("{} {e}", o.id))
                            }
                        }
                    }
                    *self.published.lock().unwrap_or_else(|e| e.into_inner()) = Some(published);
                    *self.objectives.lock().unwrap_or_else(|e| e.into_inner()) = objs;
                    if bad.is_empty() {
                        "ok".into()
                    } else {
                        bad.join(", ")
                    }
                }
            },
        };
        *self.last_poll.lock().unwrap_or_else(|e| e.into_inner()) = Some((now, result));
    }
}

/// Poll at start and then every `interval_s`.
pub fn spawn(n: std::sync::Arc<NorthStar>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(n.config.interval_s));
        loop {
            tick.tick().await;
            n.poll(chrono::Utc::now().timestamp()).await;
        }
    });
}

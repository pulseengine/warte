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

pub mod view;

fn default_base_url() -> String {
    "https://raw.githubusercontent.com/pulseengine/pulseengine.eu/main/data".into()
}
fn default_interval_s() -> u64 {
    6 * 3600
}
/// A month, as the publisher's monthly check counts it (REQ-NS-005).
fn default_stale_after_days() -> i64 {
    31
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
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct Objective {
    pub id: String,
    pub title: String,
    pub guard: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub indicator: String,
    /// The state the North Star file declares (`measured`, `unmeasured`), as
    /// published; empty when the file names none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub declared: String,
}

/// A product or tool repository and the objectives it pulls.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Puller {
    pub name: String,
    /// A product's own North Star; empty for a tool repository.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub north_star: String,
    pub pulls: Vec<String>,
    pub product: bool,
}

/// One commitment of a period, as published.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Commitment {
    pub period: String,
    /// The varve layers the period spans (`2026.11`).
    pub layers: Vec<String>,
    pub objective: String,
    /// `committed` or `stretch`.
    pub kind: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_of: Option<f64>,
}

/// Everything of north-star.toml the view shows (REQ-NS-005).
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct Board {
    pub north_star: String,
    pub updated: String,
    pub objectives: Vec<Objective>,
    pub pullers: Vec<Puller>,
    pub commitments: Vec<Commitment>,
}

/// The objectives of the published file (REQ-NS-001); see [`parse_board`].
pub fn parse_objectives(toml_text: &str) -> Result<Vec<Objective>, String> {
    parse_board(toml_text).map(|b| b.objectives)
}

/// The published north-star.toml, as data: every text cleaned and cut, lists
/// capped, objective ids checked; fields warte does not show are ignored.
pub fn parse_board(toml_text: &str) -> Result<Board, String> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        north_star: String,
        #[serde(default)]
        updated: String,
        #[serde(default)]
        objectives: Vec<RawObjective>,
        #[serde(default)]
        products: Vec<RawPuller>,
        #[serde(default)]
        tool_repos: Vec<RawPuller>,
        #[serde(default)]
        periods: Vec<RawPeriod>,
    }
    #[derive(Deserialize)]
    struct RawObjective {
        id: String,
        title: String,
        #[serde(default)]
        guard: String,
        #[serde(default)]
        summary: String,
        #[serde(default)]
        indicator: String,
        #[serde(default)]
        state: String,
    }
    #[derive(Deserialize)]
    struct RawPuller {
        name: String,
        #[serde(default)]
        north_star: String,
        #[serde(default)]
        pulls: Vec<String>,
    }
    #[derive(Deserialize)]
    struct RawPeriod {
        id: String,
        #[serde(default)]
        layers: Vec<String>,
        #[serde(default)]
        commitments: Vec<RawCommitment>,
    }
    #[derive(Deserialize)]
    struct RawCommitment {
        objective: String,
        #[serde(default)]
        kind: String,
        #[serde(default)]
        target: String,
        target_value: Option<f64>,
        target_of: Option<f64>,
    }
    let f: File =
        toml::from_str(toml_text).map_err(|_| "north-star.toml does not parse".to_string())?;
    let objectives: Vec<Objective> = f
        .objectives
        .into_iter()
        .filter(|o| id_ok(&o.id))
        .take(MAX_LIST)
        .map(|o| Objective {
            id: o.id,
            title: clean(&o.title, 120),
            guard: clean(&o.guard, 600),
            summary: clean(&o.summary, 600),
            indicator: clean(&o.indicator, 800),
            declared: clean(&o.state, 20),
        })
        .collect();
    if objectives.is_empty() {
        return Err("north-star.toml names no objective".into());
    }
    let ids = |v: Vec<String>| -> Vec<String> {
        v.into_iter().filter(|i| id_ok(i)).take(MAX_LIST).collect()
    };
    let pullers = f
        .products
        .into_iter()
        .map(|p| (p, true))
        .chain(f.tool_repos.into_iter().map(|p| (p, false)))
        .filter(|(p, _)| repo_name_ok(&p.name))
        .take(MAX_LIST)
        .map(|(p, product)| Puller {
            name: p.name,
            north_star: clean(&p.north_star, 600),
            pulls: ids(p.pulls),
            product,
        })
        .collect();
    let mut commitments = Vec::new();
    for p in f.periods.into_iter().take(MAX_LIST) {
        let period = clean(&p.id, 20);
        let layers: Vec<String> = p.layers.iter().take(12).map(|l| clean(l, 20)).collect();
        for c in p.commitments.into_iter().take(MAX_LIST) {
            if !id_ok(&c.objective) {
                continue;
            }
            let finite = |x: Option<f64>| x.filter(|v| v.is_finite());
            commitments.push(Commitment {
                period: period.clone(),
                layers: layers.clone(),
                objective: c.objective,
                kind: clean(&c.kind, 20),
                target: clean(&c.target, 600),
                target_value: finite(c.target_value),
                target_of: finite(c.target_of),
            });
        }
    }
    Ok(Board {
        north_star: clean(&f.north_star, 600),
        updated: clean(&f.updated, 40),
        objectives,
        pullers,
        commitments,
    })
}

/// At most this many entries of any published list are kept.
const MAX_LIST: usize = 50;

/// A repository name as GitHub allows it, so it can form a link.
pub fn repo_name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !s.starts_with('.')
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
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
    /// The period the value covers, when the indicator names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<String>,
    /// Per-repository facts (`repos[]`) and per-item evidence (`items[]`),
    /// in one list, as published (REQ-NS-005).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    /// What the value leaves out, and why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_counted: Vec<NotCounted>,
}

/// One row of an indicator file's evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Evidence {
    /// The repository (`wohl`) or the item (`loom#428`).
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub release: String,
    /// The published verdict (`pass`, `absent`, a verdict text), if any.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub verdict: String,
    /// Whether this row counts in the value, when the file says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<bool>,
    /// Negative controls and their outcome, as published.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<(String, String)>,
    /// An https link to the evidence.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NotCounted {
    pub what: String,
    pub reason: String,
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
        period: text("period", 60),
        evidence: evidence(&v),
        not_counted: v["not_counted"]
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX_LIST)
            .filter_map(|n| {
                let what = n["product"].as_str().or(n["repo"].as_str())?;
                Some(NotCounted {
                    what: clean(what, 100),
                    reason: n["reason"]
                        .as_str()
                        .map(|r| clean(r, 300))
                        .unwrap_or_default(),
                })
            })
            .collect(),
    })
}

/// `repos[]` and `items[]` of an indicator file as evidence rows. A release
/// links to its GitHub release page under the pulseengine organisation, as
/// the site does; an item links only to an https URL it carries.
fn evidence(v: &serde_json::Value) -> Vec<Evidence> {
    let s =
        |x: &serde_json::Value, max: usize| x.as_str().map(|t| clean(t, max)).unwrap_or_default();
    let mut out = Vec::new();
    for r in v["repos"].as_array().into_iter().flatten().take(MAX_LIST) {
        let repo = s(&r["repo"], 100);
        if !repo_name_ok(&repo) {
            continue;
        }
        let release = s(&r["release"], 80);
        let url = if repo_name_ok(&release) {
            format!("https://github.com/pulseengine/{repo}/releases/tag/{release}")
        } else {
            String::new()
        };
        let controls = r["controls"]
            .as_object()
            .into_iter()
            .flatten()
            .take(10)
            .map(|(k, c)| (clean(k, 80), s(c, 40)))
            .collect();
        out.push(Evidence {
            label: repo,
            release,
            verdict: s(&r["verify"], 80),
            counts: r["counts"].as_bool(),
            controls,
            url,
        });
    }
    for i in v["items"].as_array().into_iter().flatten().take(MAX_LIST) {
        let label = i["pr"]
            .as_str()
            .or(i["label"].as_str())
            .map(|t| clean(t, 120));
        let Some(label) = label.filter(|l| !l.is_empty()) else {
            continue;
        };
        let url = i["url"]
            .as_str()
            .filter(|u| crate::news::link_ok(u))
            .map(|u| clean(u, 300))
            .unwrap_or_default();
        let verdict = if i["in_value"].as_bool() == Some(false) {
            "reported separately, not in the value".to_string()
        } else {
            s(&i["verdict"], 120)
        };
        out.push(Evidence {
            label,
            verdict,
            counts: i["counts"].as_bool(),
            url,
            ..Default::default()
        });
    }
    out
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
            // the North Star declaring it unmeasured wins over any file
            let gone = published.is_some_and(|p| !p.contains(&o.id)) || o.declared == "unmeasured";
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

pub(crate) fn num(x: f64) -> String {
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
    board: Mutex<Option<Board>>,
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
            board: Mutex::new(None),
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

    /// The last North Star file read, if any poll has read one.
    pub fn board(&self) -> Option<Board> {
        self.board.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
            Ok(Some(t)) => match parse_board(&t) {
                Err(e) => e,
                Ok(board) => {
                    let objs = board.objectives.clone();
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
                    *self.board.lock().unwrap_or_else(|e| e.into_inner()) = Some(board);
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

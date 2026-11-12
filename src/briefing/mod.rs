//! The briefing (REQ-BRIEF-001..004): every `period_days` (default two), what the agents did, in
//! plain words for the family and with numbers for an IT reader.
//!
//! warte builds the FACT SHEET from records: merged pull requests, closed
//! issues, releases and pull requests left open, read from GitHub WITHOUT a
//! token (so only public repositories can ever appear), the CI ledger of
//! those repositories, and the notional costs of the last two weeks. It
//! writes a template briefing from the facts at once, so there is always
//! one. A scheduled model run elsewhere (warte never holds a model
//! credential) reads the facts and may replace the two plain layers; its
//! text is checked against the facts and refused when it names a repository,
//! a number or a version the facts do not hold. Layer 3, the IT section, is
//! always rendered from the facts. Every briefing is kept with its facts.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub mod github;

/// Days per briefing.
pub const PERIOD_DAYS: i64 = 2;
/// Words in the line and the three bullets together, at most.
pub const MAX_WORDS: usize = 130;
/// A pull request open longer than this waits for the owner.
pub const OPEN_WAIT_S: i64 = 86_400;

/// A change worth knowing about, dated by the owner in config: shown in the
/// IT section for two weeks after its date.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub date: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BriefingConfig {
    /// The GitHub organisation; only its public repositories are read.
    pub org: String,
    #[serde(default = "default_api_url")]
    pub api_url: String,
    /// Where the briefings are kept; default: the ground station's data dir.
    #[serde(default)]
    pub data_dir: Option<String>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    /// Days per period, 1 to 7; default [`PERIOD_DAYS`].
    #[serde(default = "default_period_days")]
    pub period_days: i64,
}

fn default_period_days() -> i64 {
    PERIOD_DAYS
}

impl BriefingConfig {
    /// Why the section is wrong, if it is.
    pub fn check(&self) -> Result<(), String> {
        if !(1..=7).contains(&self.period_days) {
            return Err(format!(
                "[briefing] period_days must be 1 to 7, not {}",
                self.period_days
            ));
        }
        Ok(())
    }
}

fn default_api_url() -> String {
    "https://api.github.com".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Item {
    pub number: u64,
    pub title: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RepoFacts {
    pub repo: String,
    pub merged: Vec<Item>,
    pub closed_issues: Vec<Item>,
    pub releases: Vec<String>,
    pub ci_runs: u32,
    pub ci_failed_jobs: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenPr {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub days_open: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Totals {
    pub repos: u32,
    pub merged: u32,
    pub closed_issues: u32,
    pub releases: u32,
    pub ci_runs: u32,
    pub ci_failed_jobs: u32,
    pub open_prs: u32,
}

/// Notional $ of the last 7 days against the 7 before, from the costs feed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TokenCheck {
    pub week_usd: f64,
    pub prev_week_usd: f64,
    pub label: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Facts {
    /// The GitHub organisation the facts are about.
    #[serde(default)]
    pub org: String,
    /// First and last day of the period (UTC), inclusive.
    pub from: String,
    pub until: String,
    pub built_at: i64,
    /// Public repositories of the org, active or not: a repository named in
    /// a text must be one with activity in `repos`.
    pub known_repos: Vec<String>,
    /// Repositories with any activity, most merged first.
    pub repos: Vec<RepoFacts>,
    /// The oldest open pull requests (at most 20 listed).
    pub open_prs: Vec<OpenPr>,
    /// How many were open for more than a day, listed or not.
    #[serde(default)]
    pub open_prs_total: u32,
    pub totals: Totals,
    pub tokens: Option<TokenCheck>,
    pub markers: Vec<Marker>,
    /// What could not be read; the facts are a lower bound then.
    pub gaps: Vec<String>,
}

/// The two plain layers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Text {
    pub line: String,
    pub bullets: Vec<String>,
}

/// What a model run sends.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    /// The period's first day, as in the facts.
    pub from: String,
    pub line: String,
    pub bullets: Vec<String>,
    /// The model id that wrote it.
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Briefing {
    pub facts: Facts,
    pub text: Text,
    /// `template` or `model`.
    pub source: String,
    pub model: Option<String>,
    /// Why the last model text for this period was refused.
    pub rejected: Option<String>,
    pub written_at: i64,
}

// ---------- periods ----------

pub fn day(t: i64) -> String {
    crate::hub::report::day_of(t)
}

/// The last closed period at `now` of the default length ([`PERIOD_DAYS`]).
pub fn last_period(now: i64) -> (String, String) {
    last_period_of(now, PERIOD_DAYS)
}

/// The last closed period at `now`: `days` UTC days, ending at the last day
/// boundary that is a multiple of `days` since the epoch, so every install
/// with the same length agrees. `(from, until)`, inclusive.
pub fn last_period_of(now: i64, days: i64) -> (String, String) {
    let days = days.max(1);
    let d = now.div_euclid(86_400);
    let end = d - d.rem_euclid(days); // exclusive
    (day((end - days) * 86_400), day((end - 1) * 86_400))
}

/// A period's length in words: "day", "two days", "4 days".
pub fn span_words(days: i64) -> String {
    match days {
        1 => "day".into(),
        2 => "two days".into(),
        n => format!("{n} days"),
    }
}

// ---------- facts from records ----------

/// CI facts per repository from ledger records, kept to `public` repos.
pub fn ci_facts(
    recs: &[crate::hub::model::JobRecord],
    public: &BTreeSet<String>,
) -> BTreeMap<String, (u32, u32)> {
    let mut runs: BTreeMap<String, BTreeSet<u64>> = BTreeMap::new();
    let mut failed: BTreeMap<String, u32> = BTreeMap::new();
    for r in recs.iter().filter(|r| public.contains(&r.repo)) {
        runs.entry(r.repo.clone()).or_default().insert(r.run);
        if r.r == "failure" {
            *failed.entry(r.repo.clone()).or_default() += 1;
        }
    }
    runs.into_iter()
        .map(|(k, v)| {
            let f = failed.get(&k).copied().unwrap_or(0);
            (k, (v.len() as u32, f))
        })
        .collect()
}

/// Notional $ of the 7 days ending `until` and the 7 before.
pub fn token_check(costs: &crate::costs::CostReport, until: &str) -> Option<TokenCheck> {
    let end = chrono::NaiveDate::parse_from_str(until, "%Y-%m-%d").ok()?;
    let mut week = 0.0;
    let mut prev = 0.0;
    for d in &costs.daily {
        let Ok(x) = chrono::NaiveDate::parse_from_str(&d.day, "%Y-%m-%d") else {
            continue;
        };
        let ago = (end - x).num_days();
        if (0..7).contains(&ago) {
            week += d.usd;
        } else if (7..14).contains(&ago) {
            prev += d.usd;
        }
    }
    Some(TokenCheck {
        week_usd: week,
        prev_week_usd: prev,
        label: crate::costs::LABEL.into(),
    })
}

impl Facts {
    /// Fill `totals` and order `repos`, after the parts are in.
    pub fn finish(&mut self) {
        self.repos.retain(|r| {
            !r.merged.is_empty()
                || !r.closed_issues.is_empty()
                || !r.releases.is_empty()
                || r.ci_runs > 0
        });
        self.repos.sort_by(|a, b| {
            b.merged
                .len()
                .cmp(&a.merged.len())
                .then(b.releases.len().cmp(&a.releases.len()))
                .then(a.repo.cmp(&b.repo))
        });
        let t = &mut self.totals;
        *t = Totals::default();
        for r in &self.repos {
            if !r.merged.is_empty() || !r.closed_issues.is_empty() || !r.releases.is_empty() {
                t.repos += 1;
            }
            t.merged += r.merged.len() as u32;
            t.closed_issues += r.closed_issues.len() as u32;
            t.releases += r.releases.len() as u32;
            t.ci_runs += r.ci_runs;
            t.ci_failed_jobs += r.ci_failed_jobs;
        }
        t.open_prs = self.open_prs_total.max(self.open_prs.len() as u32);
    }

    /// The period's length in days, from its first and last day.
    pub fn days(&self) -> i64 {
        let p = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok();
        match (p(&self.from), p(&self.until)) {
            (Some(a), Some(b)) => (b - a).num_days() + 1,
            _ => PERIOD_DAYS,
        }
    }

    /// Every number a text may use: counts, item numbers, days open.
    fn numbers(&self) -> BTreeSet<u64> {
        let t = &self.totals;
        let mut n: BTreeSet<u64> = [
            t.repos,
            t.merged,
            t.closed_issues,
            t.releases,
            t.ci_runs,
            t.ci_failed_jobs,
            t.open_prs,
        ]
        .iter()
        .map(|x| *x as u64)
        .collect();
        // "two days", "three things", and the like
        n.extend([1, 2, 3, self.days() as u64]);
        for r in &self.repos {
            n.extend([
                r.merged.len() as u64,
                r.closed_issues.len() as u64,
                r.releases.len() as u64,
                r.ci_runs as u64,
                r.ci_failed_jobs as u64,
            ]);
            n.extend(r.merged.iter().chain(&r.closed_issues).map(|i| i.number));
        }
        for p in &self.open_prs {
            n.insert(p.number);
            n.insert(p.days_open as u64);
        }
        n
    }

    fn active(&self) -> BTreeSet<String> {
        self.repos
            .iter()
            .map(|r| r.repo.to_lowercase())
            .chain(self.open_prs.iter().map(|p| p.repo.to_lowercase()))
            .collect()
    }

    fn versions(&self) -> BTreeSet<String> {
        self.repos
            .iter()
            .flat_map(|r| r.releases.iter())
            .map(|v| v.trim_start_matches('v').to_string())
            .collect()
    }
}

// ---------- the text check ----------

/// Why a text may not stand for these facts, or `Ok`.
pub fn check(text: &Text, f: &Facts) -> Result<(), String> {
    if text.bullets.len() != 3 {
        return Err("a briefing has one line and exactly three bullets".into());
    }
    let all: Vec<&str> = std::iter::once(text.line.as_str())
        .chain(text.bullets.iter().map(String::as_str))
        .collect();
    let words: usize = all.iter().map(|s| s.split_whitespace().count()).sum();
    if words > MAX_WORDS {
        return Err(format!("{words} words; at most {MAX_WORDS}"));
    }
    if text.line.trim().is_empty() || text.bullets.iter().any(|b| b.trim().is_empty()) {
        return Err("an empty line or bullet".into());
    }
    for s in &all {
        if s.len() > 400 {
            return Err("a line longer than 400 characters".into());
        }
        if s.chars()
            .any(|c| c.is_control() || "<>`[]{}|\\".contains(c))
            || s.contains("http")
            || s.contains("www.")
        {
            return Err("markup, a link or a control character".into());
        }
    }
    let joined = all.join(" ");
    let known: BTreeSet<String> = f.known_repos.iter().map(|r| r.to_lowercase()).collect();
    let active = f.active();
    let numbers = f.numbers();
    let versions = f.versions();
    for raw in joined.split(|c: char| c.is_whitespace() || ",;:!?()\"'".contains(c)) {
        let w = raw.trim_end_matches('.').trim_start_matches('#');
        if w.is_empty() {
            continue;
        }
        let lw = w.to_lowercase();
        if known.contains(&lw) && !active.contains(&lw) {
            return Err(format!("names {w}, which had no activity in the period"));
        }
        let v = w.trim_start_matches(['v', 'V']);
        if v.contains('.')
            && v.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        {
            if !versions.contains(v) {
                return Err(format!("version {w} is not a release of the period"));
            }
            continue;
        }
        if let Ok(n) = w.trim_end_matches('%').parse::<u64>() {
            if !numbers.contains(&n) {
                return Err(format!("the number {n} is not in the facts"));
            }
        }
    }
    Ok(())
}

// ---------- the template ----------

fn list(names: &[String]) -> String {
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        n => format!("{} and {}", names[..n - 1].join(", "), names[n - 1]),
    }
}

/// The plain layers written from the facts alone.
pub fn template(f: &Facts) -> Text {
    let t = &f.totals;
    let plural = |n: u32, one: &str, many: &str| {
        if n == 1 {
            one.to_string()
        } else {
            many.to_string()
        }
    };
    // counts and names only: the facts do not say who did the work or what
    // a failure led to, so neither does the template
    let line = if t.merged == 0 && t.releases == 0 {
        format!(
            "A quiet {}: nothing new was finished.",
            span_words(f.days())
        )
    } else {
        let mut l = format!(
            "In the last {} {} {} finished in {} {}",
            span_words(f.days()),
            t.merged,
            plural(t.merged, "change was", "changes were"),
            t.repos,
            plural(t.repos, "project", "projects")
        );
        if t.releases > 0 {
            l += &format!(
                ", and {} new {} published",
                t.releases,
                plural(t.releases, "version was", "versions were")
            );
        }
        l + "."
    };
    let top: Vec<String> = f
        .repos
        .iter()
        .filter(|r| !r.merged.is_empty())
        .take(3)
        .map(|r| format!("{} ({})", r.repo, r.merged.len()))
        .collect();
    let done = if top.is_empty() {
        "Done: nothing was merged.".to_string()
    } else {
        format!("Done: most work went into {}.", list(&top))
    };
    let mut worst: Vec<&RepoFacts> = f.repos.iter().filter(|r| r.ci_failed_jobs > 0).collect();
    worst.sort_by(|a, b| {
        b.ci_failed_jobs
            .cmp(&a.ci_failed_jobs)
            .then(a.repo.cmp(&b.repo))
    });
    let caught = if t.ci_failed_jobs > 0 {
        let names: Vec<String> = worst
            .iter()
            .take(3)
            .map(|r| format!("{} ({})", r.repo, r.ci_failed_jobs))
            .collect();
        format!(
            "Caught: the automatic checks flagged {} failed test {}, most in {}.",
            t.ci_failed_jobs,
            plural(t.ci_failed_jobs, "job", "jobs"),
            list(&names)
        )
    } else {
        "Caught: the automatic checks flagged no failed test job.".to_string()
    };
    let waits = match f.open_prs.iter().max_by_key(|p| p.days_open) {
        Some(p) => format!(
            "Waiting for you: {} {} open for more than a day, the oldest {} #{}.",
            t.open_prs,
            plural(t.open_prs, "change is", "changes are"),
            p.repo,
            p.number
        ),
        None => "Waiting for you: nothing.".to_string(),
    };
    Text {
        line,
        bullets: vec![done, caught, waits],
    }
}

// ---------- the store ----------

/// Every briefing, one file per period: `<data_dir>/briefings/<from>.json`.
pub struct Store {
    dir: PathBuf,
    lock: std::sync::Mutex<()>,
}

impl Store {
    pub fn open(data_dir: &str) -> Result<Store, String> {
        let dir = PathBuf::from(data_dir).join("briefings");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Store {
            dir,
            lock: Default::default(),
        })
    }

    fn path(&self, from: &str) -> Option<PathBuf> {
        // the period names a file: a real date only, never a path
        crate::hub::report::day_start(from)?;
        from.bytes()
            .all(|b| b.is_ascii_digit() || b == b'-')
            .then(|| self.dir.join(format!("{from}.json")))
    }

    /// A briefing as kept. A template's text is written again from its
    /// facts, so a better wording reaches the briefings already kept.
    pub fn get(&self, from: &str) -> Option<Briefing> {
        let b = std::fs::read(self.path(from)?).ok()?;
        let mut b: Briefing = serde_json::from_slice(&b).ok()?;
        if b.source == "template" {
            b.facts.finish();
            b.text = template(&b.facts);
        }
        Some(b)
    }

    pub fn put(&self, b: &Briefing) -> Result<(), String> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let p = self.path(&b.facts.from).ok_or("not a period")?;
        let tmp = p.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(b).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
    }

    /// Periods kept, newest first.
    pub fn periods(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.strip_suffix(".json").map(String::from)
            })
            .filter(|d| self.path(d).is_some())
            .collect();
        v.sort();
        v.reverse();
        v
    }

    pub fn latest(&self) -> Option<Briefing> {
        self.periods().first().and_then(|p| self.get(p))
    }

    /// A model's text for a period: stored when it fits the facts; otherwise
    /// the template stays and the refusal is recorded. `Err` says why.
    pub fn submit(&self, s: Submission, now: i64) -> Result<(), String> {
        let mut b = self.get(&s.from).ok_or("no briefing for that period")?;
        if !crate::costs::name_ok_pub(&s.model) {
            return Err("the model id is not allowed".into());
        }
        let text = Text {
            line: s.line.trim().to_string(),
            bullets: s.bullets.iter().map(|x| x.trim().to_string()).collect(),
        };
        match check(&text, &b.facts) {
            Ok(()) => {
                b.text = text;
                b.source = "model".into();
                b.model = Some(s.model);
                b.rejected = None;
                b.written_at = now;
                self.put(&b)
            }
            Err(why) => {
                b.rejected = Some(why.clone());
                self.put(&b)?;
                Err(why)
            }
        }
    }
}

/// A new briefing from facts, with the template's text.
pub fn from_facts(facts: Facts, now: i64) -> Briefing {
    Briefing {
        text: template(&facts),
        facts,
        source: "template".into(),
        model: None,
        rejected: None,
        written_at: now,
    }
}

/// Build the facts of the last closed period, if no briefing holds them yet,
/// and store the template briefing. Checked every hour.
pub fn spawn(
    store: std::sync::Arc<Store>,
    cfg: BriefingConfig,
    hub: Option<crate::hub::Hub>,
    costs: Option<std::sync::Arc<crate::costs::Costs>>,
) {
    tokio::spawn(async move {
        let client = github::Client::new(&cfg.api_url);
        loop {
            let now = crate::hub::now();
            let (from, until) = last_period_of(now, cfg.period_days);
            if store.get(&from).is_none() {
                match build(
                    &client,
                    &cfg,
                    hub.as_ref(),
                    costs.as_deref(),
                    &from,
                    &until,
                    now,
                )
                .await
                {
                    Ok(f) => {
                        if let Err(e) = store.put(&from_facts(f, now)) {
                            tracing::warn!(error = %e, "briefing not stored");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "briefing facts not read; retried in an hour")
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });
}

/// The facts of one period.
pub async fn build(
    c: &github::Client,
    cfg: &BriefingConfig,
    hub: Option<&crate::hub::Hub>,
    costs: Option<&crate::costs::Costs>,
    from: &str,
    until: &str,
    now: i64,
) -> Result<Facts, String> {
    let mut f = github::facts(c, &cfg.org, from, until, now).await?;
    let public: BTreeSet<String> = f.known_repos.iter().cloned().collect();
    if let Some(h) = hub {
        let h = h.clone();
        let (a, b) = (from.to_string(), until.to_string());
        let recs = tokio::task::spawn_blocking(move || h.ledger_days(&a, &b))
            .await
            .unwrap_or_default();
        for (repo, (runs, failed)) in ci_facts(&recs, &public) {
            let r = match f.repos.iter_mut().find(|r| r.repo == repo) {
                Some(r) => r,
                None => {
                    f.repos.push(RepoFacts {
                        repo: repo.clone(),
                        ..Default::default()
                    });
                    f.repos.last_mut().unwrap()
                }
            };
            r.ci_runs = runs;
            r.ci_failed_jobs = failed;
        }
    }
    f.tokens = costs
        .and_then(|c| c.latest())
        .and_then(|s| token_check(&s.report, until));
    let since = chrono::NaiveDate::parse_from_str(until, "%Y-%m-%d")
        .map(|d| {
            (d - chrono::Duration::days(14))
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_default();
    f.markers = cfg
        .markers
        .iter()
        .filter(|m| m.date.as_str() >= since.as_str() && m.date.as_str() <= until)
        .cloned()
        .collect();
    f.built_at = now;
    f.finish();
    Ok(f)
}

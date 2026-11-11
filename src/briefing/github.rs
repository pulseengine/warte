//! The briefing's GitHub reads (REQ-BRIEF-001): without a token, so only
//! public repositories are ever seen, and read-only. About a dozen requests
//! per period plus one release listing per repository pushed in it: well
//! inside the 60 an hour GitHub allows without a token.

use super::{Facts, Item, OpenPr, RepoFacts};
use crate::hub::parse::rfc3339_utc;
use serde_json::Value;
use std::time::Duration;

/// Pages followed per listing or search.
const MAX_PAGES: usize = 3;
/// A PR or issue title is cut to this many characters.
const TITLE_MAX: usize = 120;

pub struct Client {
    http: reqwest::Client,
    base: String,
}

impl Client {
    pub fn new(base: &str) -> Client {
        Client {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .user_agent(concat!("warte/", env!("CARGO_PKG_VERSION"), " briefing"))
                .build()
                .unwrap_or_default(),
            base: base.trim_end_matches('/').to_string(),
        }
    }

    async fn get(&self, path_and_query: &str) -> Result<Value, String> {
        let url = format!("{}{path_and_query}", self.base);
        let r = self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| format!("GitHub not reached: {}", e.without_url()))?;
        let status = r.status().as_u16();
        if status == 403 || status == 429 {
            return Err(format!("GitHub refused ({status}): rate limit"));
        }
        if !(200..300).contains(&status) {
            return Err(format!("GitHub answered {status}"));
        }
        r.json().await.map_err(|_| "GitHub sent no JSON".into())
    }

    /// Items of a search, up to [`MAX_PAGES`] pages; `true` when more remain.
    async fn search(&self, q: &str) -> Result<(Vec<Value>, bool), String> {
        let mut out = vec![];
        for page in 1..=MAX_PAGES {
            let v = self
                .get(&format!("/search/issues?q={q}&per_page=100&page={page}"))
                .await?;
            let items = v["items"].as_array().cloned().unwrap_or_default();
            let n = items.len();
            out.extend(items);
            let total = v["total_count"].as_u64().unwrap_or(0) as usize;
            if n < 100 || out.len() >= total {
                return Ok((out, false));
            }
        }
        Ok((out, true))
    }
}

/// `owner/name` of a search item, from its `repository_url`.
fn repo_of(item: &Value) -> Option<String> {
    item["repository_url"]
        .as_str()?
        .rsplit('/')
        .next()
        .map(String::from)
}

fn title(item: &Value) -> String {
    let t: String = item["title"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(TITLE_MAX)
        .collect();
    t
}

fn repo_mut<'a>(f: &'a mut Facts, repo: &str) -> &'a mut RepoFacts {
    if let Some(i) = f.repos.iter().position(|r| r.repo == repo) {
        return &mut f.repos[i];
    }
    f.repos.push(RepoFacts {
        repo: repo.to_string(),
        ..Default::default()
    });
    f.repos.last_mut().unwrap()
}

/// What GitHub says about `org`'s public repositories from `from` to
/// `until` (UTC days, inclusive). A failed read of the repository list is an
/// error; a failed search or release listing becomes a gap in the facts.
pub async fn facts(
    c: &Client,
    org: &str,
    from: &str,
    until: &str,
    now: i64,
) -> Result<Facts, String> {
    let mut f = Facts {
        org: org.to_string(),
        from: from.to_string(),
        until: until.to_string(),
        ..Default::default()
    };
    // public repositories only: without a token GitHub shows no other kind,
    // and `type=public` says so twice
    let mut repos: Vec<(String, i64)> = vec![];
    for page in 1..=MAX_PAGES {
        let v = c
            .get(&format!(
                "/orgs/{org}/repos?type=public&sort=pushed&per_page=100&page={page}"
            ))
            .await?;
        let list = v.as_array().cloned().unwrap_or_default();
        let n = list.len();
        for r in list {
            if r["private"].as_bool().unwrap_or(true) || r["archived"].as_bool().unwrap_or(false) {
                continue;
            }
            if let Some(name) = r["name"].as_str() {
                let pushed = r["pushed_at"].as_str().and_then(rfc3339_utc).unwrap_or(0);
                repos.push((name.to_string(), pushed));
            }
        }
        if n < 100 {
            break;
        }
    }
    f.known_repos = repos.iter().map(|r| r.0.clone()).collect();
    f.known_repos.sort();
    let known = f.known_repos.clone();
    let public = |r: &str| known.binary_search(&r.to_string()).is_ok();

    let range = format!("{from}..{until}");
    let mut merged = vec![];
    match c
        .search(&format!("org:{org}+is:pr+is:merged+merged:{range}"))
        .await
    {
        Ok((items, more)) => {
            if more {
                f.gaps
                    .push("more merged pull requests than were read".into());
            }
            merged = items;
        }
        Err(e) => f.gaps.push(format!("merged pull requests: {e}")),
    }
    for it in &merged {
        if let (Some(repo), Some(n)) = (repo_of(it), it["number"].as_u64()) {
            if public(&repo) {
                let t = title(it);
                repo_mut(&mut f, &repo).merged.push(Item {
                    number: n,
                    title: t,
                });
            }
        }
    }
    match c
        .search(&format!("org:{org}+is:issue+is:closed+closed:{range}"))
        .await
    {
        Ok((items, more)) => {
            if more {
                f.gaps.push("more closed issues than were read".into());
            }
            for it in &items {
                if let (Some(repo), Some(n)) = (repo_of(it), it["number"].as_u64()) {
                    if public(&repo) {
                        let t = title(it);
                        repo_mut(&mut f, &repo).closed_issues.push(Item {
                            number: n,
                            title: t,
                        });
                    }
                }
            }
        }
        Err(e) => f.gaps.push(format!("closed issues: {e}")),
    }
    let day_ago = crate::briefing::day(now - super::OPEN_WAIT_S);
    match c
        .search(&format!("org:{org}+is:pr+is:open+created:<{day_ago}"))
        .await
    {
        Ok((items, _)) => {
            for it in &items {
                let (Some(repo), Some(n)) = (repo_of(it), it["number"].as_u64()) else {
                    continue;
                };
                if !public(&repo) {
                    continue;
                }
                let created = it["created_at"]
                    .as_str()
                    .and_then(rfc3339_utc)
                    .unwrap_or(now);
                f.open_prs.push(OpenPr {
                    repo,
                    number: n,
                    title: title(it),
                    days_open: ((now - created).max(0) / 86_400) as u32,
                });
            }
            f.open_prs
                .sort_by(|a, b| b.days_open.cmp(&a.days_open).then(a.number.cmp(&b.number)));
            f.open_prs.truncate(20);
        }
        Err(e) => f.gaps.push(format!("open pull requests: {e}")),
    }
    // releases: only repositories pushed since the period began can have one
    let start = crate::hub::report::day_start(from).unwrap_or(0);
    let end = crate::hub::report::day_start(until).unwrap_or(now) + 86_400;
    for (repo, _) in repos.iter().filter(|r| r.1 >= start) {
        match c
            .get(&format!("/repos/{org}/{repo}/releases?per_page=20"))
            .await
        {
            Ok(v) => {
                for rel in v.as_array().cloned().unwrap_or_default() {
                    if rel["draft"].as_bool().unwrap_or(false) {
                        continue;
                    }
                    let at = rel["published_at"]
                        .as_str()
                        .and_then(rfc3339_utc)
                        .unwrap_or(0);
                    if (start..end).contains(&at) {
                        if let Some(tag) = rel["tag_name"].as_str() {
                            let tag: String =
                                tag.chars().filter(|c| !c.is_control()).take(40).collect();
                            repo_mut(&mut f, repo).releases.push(tag);
                        }
                    }
                }
            }
            Err(e) => f.gaps.push(format!("releases of {repo}: {e}")),
        }
    }
    Ok(f)
}

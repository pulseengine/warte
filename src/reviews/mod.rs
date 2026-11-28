//! The reviews view (REQ-PRREVIEW-001, warte#85): the pull requests one person
//! has to review, how big each is, whether it is green, how long it has
//! waited, and which need a re-review. Read with that reviewer's own
//! read-only token, one GraphQL search per bucket, on an interval.
//!
//! PR titles and authors are private work data: the view and its API answer
//! a signed-in user, or, without `[auth]`, only a warte bound to loopback.
//!
//! The queue (REQ-PRREVIEW-002, warte#236/#237) reads every configured
//! source (github.com, GitHub Enterprise Cloud or Server, each with its own
//! token and login) and adds, next to the review requests, the PRs and issues
//! assigned to the person and the mentions they have not answered since.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

/// One GitHub host the queue reads, with the person's token there.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    #[serde(default = "default_api")]
    pub api_url: String,
    /// A file holding the person's read-only token on this host (mode 0600).
    pub token_file: String,
    /// Teams whose review requests count on this host, as `org/team`.
    #[serde(default)]
    pub teams: Vec<String>,
    /// The name the view shows; default `login@host`.
    #[serde(default)]
    pub label: Option<String>,
}

/// `[reviews]` in warte.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewsConfig {
    /// A file holding the reviewer's read-only token (mode 0600). With
    /// `sources`, leave it out; without, it and `api_url`/`teams` are the
    /// one source.
    #[serde(default)]
    pub token_file: String,
    #[serde(default = "default_api")]
    pub api_url: String,
    /// Teams whose review requests count, as `org/team`.
    #[serde(default)]
    pub teams: Vec<String>,
    #[serde(default)]
    pub highlight_authors: Vec<String>,
    #[serde(default)]
    pub highlight_repos: Vec<String>,
    #[serde(default)]
    pub highlight_labels: Vec<String>,
    #[serde(default = "default_interval")]
    pub interval_s: u64,
    /// Every host the queue reads (`[[reviews.sources]]`).
    #[serde(default)]
    pub sources: Vec<Source>,
}

impl ReviewsConfig {
    /// The sources to read: the list, or the single top-level one.
    pub fn sources(&self) -> Vec<Source> {
        if !self.sources.is_empty() {
            return self.sources.clone();
        }
        if self.token_file.is_empty() {
            return vec![];
        }
        vec![Source {
            api_url: self.api_url.clone(),
            token_file: self.token_file.clone(),
            teams: self.teams.clone(),
            label: None,
        }]
    }

    /// This config narrowed to one source: its host and teams.
    pub fn for_source(&self, s: &Source) -> ReviewsConfig {
        ReviewsConfig {
            api_url: s.api_url.clone(),
            token_file: s.token_file.clone(),
            teams: s.teams.clone(),
            sources: vec![],
            ..self.clone()
        }
    }
}

/// The host part of an API URL, as a person reads it: `github.com` for
/// `https://api.github.com`, `<tenant>.ghe.com`, or a GHES host.
pub fn host_of(api_url: &str) -> String {
    let h = api_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let h = h.split('/').next().unwrap_or(h);
    h.strip_prefix("api.").unwrap_or(h).to_string()
}

fn default_api() -> String {
    "https://api.github.com".into()
}
fn default_interval() -> u64 {
    600
}

/// One pull request as the view needs it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Pr {
    /// The source it came from (its label).
    pub account: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    /// Why it is listed: `me`, `team:<org/team>`, `assigned`, `author`, `repo`.
    pub why: String,
    pub created_at: String,
    pub updated_at: String,
    /// When review was last requested, if known.
    pub requested_at: Option<String>,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    /// S, M, L or XL.
    pub size: String,
    /// `green`, `red`, `running` or `none`.
    pub build: String,
    /// The first failed check, when red.
    pub failed_check: Option<String>,
    /// APPROVED, CHANGES_REQUESTED, REVIEW_REQUIRED, or empty.
    pub decision: String,
    /// The reviewer's own latest review state, if any.
    pub my_review: Option<String>,
    /// Commits were pushed after the reviewer's latest review.
    pub re_review: bool,
    /// Approved, green and mergeable: nothing left for the reviewer.
    pub done: bool,
    pub highlighted: bool,
    pub labels: Vec<String>,
}

/// The size bucket of a change, by lines changed.
pub fn size(additions: u64, deletions: u64) -> &'static str {
    match additions + deletions {
        0..=50 => "S",
        51..=250 => "M",
        251..=1000 => "L",
        _ => "XL",
    }
}

/// One search result node into a [`Pr`], for the reviewer `me`.
pub fn from_node(n: &Value, why: &str, me: &str, cfg: &ReviewsConfig) -> Option<Pr> {
    let repo = n["repository"]["nameWithOwner"].as_str()?.to_string();
    let author = n["author"]["login"].as_str().unwrap_or("ghost").to_string();
    let (additions, deletions) = (
        n["additions"].as_u64().unwrap_or(0),
        n["deletions"].as_u64().unwrap_or(0),
    );
    let rollup = &n["commits"]["nodes"][0]["commit"]["statusCheckRollup"];
    let build = match rollup["state"].as_str() {
        Some("SUCCESS") => "green",
        Some("FAILURE" | "ERROR") => "red",
        Some("PENDING" | "EXPECTED") => "running",
        _ => "none",
    };
    let failed_check = rollup["contexts"]["nodes"].as_array().and_then(|cs| {
        cs.iter().find_map(|c| {
            let bad = matches!(
                c["conclusion"].as_str(),
                Some("FAILURE" | "TIMED_OUT" | "CANCELLED")
            ) || matches!(c["state"].as_str(), Some("FAILURE" | "ERROR"));
            bad.then(|| {
                c["name"]
                    .as_str()
                    .or(c["context"].as_str())
                    .unwrap_or("")
                    .to_string()
            })
        })
    });
    let head = n["headRefOid"].as_str().unwrap_or("");
    let mine = n["reviews"]["nodes"]
        .as_array()
        .map(|rs| {
            rs.iter()
                .filter(|r| {
                    r["author"]["login"]
                        .as_str()
                        .is_some_and(|l| l.eq_ignore_ascii_case(me))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let latest = mine.last();
    let my_review = latest.and_then(|r| r["state"].as_str()).map(String::from);
    let re_review = latest
        .and_then(|r| r["commit"]["oid"].as_str())
        .is_some_and(|oid| !head.is_empty() && oid != head);
    let decision = n["reviewDecision"].as_str().unwrap_or("").to_string();
    let done = decision == "APPROVED" && build == "green" && n["mergeable"] == "MERGEABLE";
    let labels: Vec<String> = n["labels"]["nodes"]
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(|x| x["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let highlighted = cfg
        .highlight_authors
        .iter()
        .any(|a| a.eq_ignore_ascii_case(&author))
        || cfg
            .highlight_repos
            .iter()
            .any(|r| r.eq_ignore_ascii_case(&repo))
        || labels.iter().any(|l| {
            cfg.highlight_labels
                .iter()
                .any(|h| h.eq_ignore_ascii_case(l))
        });
    let requested_at = n["timelineItems"]["nodes"]
        .as_array()
        .and_then(|t| t.last())
        .and_then(|e| e["createdAt"].as_str())
        .map(String::from);
    Some(Pr {
        account: String::new(),
        number: n["number"].as_u64()?,
        title: n["title"].as_str().unwrap_or("").to_string(),
        url: n["url"].as_str().unwrap_or("").to_string(),
        why: why.to_string(),
        created_at: n["createdAt"].as_str().unwrap_or("").to_string(),
        updated_at: n["updatedAt"].as_str().unwrap_or("").to_string(),
        requested_at,
        additions,
        deletions,
        changed_files: n["changedFiles"].as_u64().unwrap_or(0),
        size: size(additions, deletions).to_string(),
        build: build.to_string(),
        failed_check,
        decision,
        my_review,
        re_review,
        done,
        highlighted,
        labels,
        repo,
        author,
    })
}

/// Merge the buckets (a PR listed twice keeps its first, strongest reason)
/// and order them: re-review needed first, then the oldest waiting requests,
/// then the rest; done ones last.
pub fn order(mut prs: Vec<Pr>) -> Vec<Pr> {
    let mut seen = std::collections::BTreeSet::new();
    prs.retain(|p| seen.insert((p.repo.clone(), p.number)));
    prs.sort_by(|a, b| {
        let key = |p: &Pr| {
            (
                p.done,
                !p.re_review,
                p.requested_at
                    .clone()
                    .unwrap_or_else(|| p.created_at.clone()),
            )
        };
        key(a).cmp(&key(b))
    });
    prs
}

/// The GraphQL fields of one pull request.
const PR_FIELDS: &str = "... on PullRequest { number title url createdAt updatedAt additions deletions changedFiles headRefOid reviewDecision mergeable isDraft repository { nameWithOwner } author { login } labels(first: 10) { nodes { name } } commits(last: 1) { nodes { commit { statusCheckRollup { state contexts(first: 50) { nodes { ... on CheckRun { name conclusion } ... on StatusContext { context state } } } } } } } reviews(last: 30) { nodes { author { login } state commit { oid } } } timelineItems(itemTypes: [REVIEW_REQUESTED_EVENT], last: 1) { nodes { ... on ReviewRequestedEvent { createdAt } } } }";

/// The searches, one per bucket, with the reason each lists a PR.
pub fn searches(cfg: &ReviewsConfig) -> Vec<(String, String)> {
    let mut out = vec![(
        "me".to_string(),
        "is:pr is:open archived:false review-requested:@me".to_string(),
    )];
    for t in &cfg.teams {
        out.push((
            format!("team:{t}"),
            format!("is:pr is:open archived:false team-review-requested:{t}"),
        ));
    }
    out.push((
        "assigned".into(),
        "is:pr is:open archived:false assignee:@me".into(),
    ));
    for a in &cfg.highlight_authors {
        out.push((
            "author".into(),
            format!("is:pr is:open archived:false author:{a}"),
        ));
    }
    for r in &cfg.highlight_repos {
        out.push((
            "repo".into(),
            format!("is:pr is:open archived:false repo:{r}"),
        ));
    }
    out
}

/// The GraphQL fields of an issue or PR in the issue buckets. Bodies and
/// comments are read only to find a mention and a later reply; they are
/// never kept.
const ITEM_FIELDS: &str = "... on Issue { number title url createdAt updatedAt repository { nameWithOwner } author { login } body comments(last: 30) { nodes { author { login } createdAt body } } } ... on PullRequest { number title url createdAt updatedAt isDraft repository { nameWithOwner } author { login } body comments(last: 30) { nodes { author { login } createdAt body } } }";

/// The issue searches, with the reason each lists an item.
pub fn item_searches() -> Vec<(String, String)> {
    vec![
        (
            "assigned".into(),
            "is:issue is:open archived:false assignee:@me".into(),
        ),
        (
            "mentioned".into(),
            "is:open archived:false mentions:@me".into(),
        ),
    ]
}

/// An issue assigned to the person, or an issue or PR that mentions them.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Item {
    pub account: String,
    /// `issue` or `pr`.
    pub kind: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    /// `assigned` or `mentioned`.
    pub why: String,
    pub created_at: String,
    /// Assigned: when it was opened. Mentioned: the mention still unanswered.
    pub waiting_since: String,
}

/// Whether `text` mentions `@login` (case-insensitive, whole login).
pub fn mentions(text: &str, login: &str) -> bool {
    let (t, l) = (
        text.to_ascii_lowercase(),
        format!("@{}", login.to_ascii_lowercase()),
    );
    let mut from = 0;
    while let Some(i) = t[from..].find(&l) {
        let end = from + i + l.len();
        let next = t[end..].chars().next();
        if !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-') {
            return true;
        }
        from = end;
    }
    false
}

/// For a mention: when the person was last mentioned without answering
/// since, or None once they have answered. `posts` are (author, created_at,
/// body), oldest first, the item's own body first. A mention the text does
/// not show (a team mention, an edited body) counts from the first post,
/// and any post by the person answers it.
pub fn unanswered_since(me: &str, posts: &[(String, String, String)]) -> Option<String> {
    let by_me = |a: &str| a.eq_ignore_ascii_case(me);
    match posts
        .iter()
        .rposition(|p| mentions(&p.2, me) && !by_me(&p.0))
    {
        Some(k) => (!posts[k + 1..].iter().any(|p| by_me(&p.0))).then(|| posts[k].1.clone()),
        None => posts
            .first()
            .filter(|_| !posts.iter().any(|p| by_me(&p.0)))
            .map(|p| p.1.clone()),
    }
}

/// One node of an issue search into an [`Item`]; None for drafts, the
/// person's own items, and answered mentions.
pub fn item_from_node(n: &Value, why: &str, me: &str) -> Option<Item> {
    let author = n["author"]["login"].as_str().unwrap_or("ghost").to_string();
    if n["isDraft"] == true || author.eq_ignore_ascii_case(me) {
        return None;
    }
    let created_at = n["createdAt"].as_str().unwrap_or("").to_string();
    let waiting_since = if why == "mentioned" {
        let mut posts = vec![(
            author.clone(),
            created_at.clone(),
            n["body"].as_str().unwrap_or("").to_string(),
        )];
        for c in n["comments"]["nodes"].as_array().into_iter().flatten() {
            posts.push((
                c["author"]["login"].as_str().unwrap_or("ghost").to_string(),
                c["createdAt"].as_str().unwrap_or("").to_string(),
                c["body"].as_str().unwrap_or("").to_string(),
            ));
        }
        unanswered_since(me, &posts)?
    } else {
        created_at.clone()
    };
    Some(Item {
        account: String::new(),
        kind: if n.get("isDraft").is_some() {
            "pr"
        } else {
            "issue"
        }
        .into(),
        repo: n["repository"]["nameWithOwner"].as_str()?.to_string(),
        number: n["number"].as_u64()?,
        title: n["title"].as_str().unwrap_or("").to_string(),
        url: n["url"].as_str().unwrap_or("").to_string(),
        author,
        why: why.into(),
        created_at,
        waiting_since,
    })
}

/// Merge the issue buckets (assigned wins over mentioned) and order them by
/// how long they have waited, longest first.
pub fn order_items(mut items: Vec<Item>) -> Vec<Item> {
    let mut seen = std::collections::BTreeSet::new();
    items.retain(|i| seen.insert((i.account.clone(), i.repo.clone(), i.number)));
    items.sort_by(|a, b| (&a.why, &a.waiting_since).cmp(&(&b.why, &b.waiting_since)));
    items
}

/// One source's latest scan.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    /// The source's label (`login@host` unless configured).
    pub account: String,
    pub reviewer: String,
    pub scanned_at: i64,
    pub prs: Vec<Pr>,
    pub items: Vec<Item>,
    /// Why the last scan failed, if it did (never the token).
    pub error: Option<String>,
}

/// Every source's latest scan, in configured order.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Queue {
    pub accounts: Vec<Snapshot>,
}

pub type Shared = Arc<Mutex<Queue>>;

/// Read the token: a file only its owner may read.
pub fn read_token(path: &str) -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let m = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
        if m.permissions().mode() & 0o077 != 0 {
            return Err(format!("{path}: other users can read it; chmod 600"));
        }
    }
    let t = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let t = t.trim().to_string();
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return Err(format!("{path}: not a token"));
    }
    Ok(t)
}

async fn graphql(
    http: &reqwest::Client,
    api: &str,
    token: &str,
    query: &str,
    vars: Value,
) -> Result<Value, String> {
    let r = http
        .post(format!("{}/graphql", api.trim_end_matches('/')))
        .bearer_auth(token)
        .json(&json!({"query": query, "variables": vars}))
        .send()
        .await
        .map_err(|e| format!("GitHub unreachable: {}", e.without_url()))?;
    let status = r.status();
    let v: Value = r
        .json()
        .await
        .map_err(|_| format!("GitHub answered {status} without JSON"))?;
    if !status.is_success() || v.get("errors").is_some() {
        return Err(format!("GitHub refused the search ({status})"));
    }
    Ok(v)
}

/// One scan of one source: the person's login there, then each search. The
/// snapshot's account is `login@host`; the caller may relabel it.
pub async fn scan(
    http: &reqwest::Client,
    cfg: &ReviewsConfig,
    token: &str,
) -> Result<Snapshot, String> {
    let me = graphql(
        http,
        &cfg.api_url,
        token,
        "query { viewer { login } }",
        json!({}),
    )
    .await?["data"]["viewer"]["login"]
        .as_str()
        .ok_or("GitHub did not name the reviewer")?
        .to_string();
    let account = format!("{me}@{}", host_of(&cfg.api_url));
    let q = format!("query($q: String!) {{ search(query: $q, type: ISSUE, first: 50) {{ nodes {{ {PR_FIELDS} }} }} }}");
    let mut prs = Vec::new();
    for (why, query) in searches(cfg) {
        let v = graphql(http, &cfg.api_url, token, &q, json!({"q": query})).await?;
        for n in v["data"]["search"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if n["isDraft"] == true
                || n["author"]["login"]
                    .as_str()
                    .is_some_and(|a| a.eq_ignore_ascii_case(&me))
            {
                continue;
            }
            if let Some(mut p) = from_node(n, &why, &me, cfg) {
                p.account = account.clone();
                prs.push(p);
            }
        }
    }
    let qi = format!("query($q: String!) {{ search(query: $q, type: ISSUE, first: 50) {{ nodes {{ {ITEM_FIELDS} }} }} }}");
    let mut items = Vec::new();
    for (why, query) in item_searches() {
        let v = graphql(http, &cfg.api_url, token, &qi, json!({"q": query})).await?;
        for n in v["data"]["search"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(mut i) = item_from_node(n, &why, &me) {
                i.account = account.clone();
                items.push(i);
            }
        }
    }
    let prs = order(prs);
    // a PR already in the review buckets is not listed again as a mention
    items.retain(|i| !prs.iter().any(|p| p.repo == i.repo && p.number == i.number));
    Ok(Snapshot {
        account,
        reviewer: me,
        scanned_at: crate::hub::now(),
        prs,
        items: order_items(items),
        error: None,
    })
}

/// Scan every source every `interval_s`. A source that fails keeps its last
/// good lists, marked with the error; the others are not held up.
pub fn spawn(cfg: ReviewsConfig, shared: Shared) {
    tokio::spawn(async move {
        let http = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(concat!("warte/", env!("CARGO_PKG_VERSION")))
            .build()
        {
            Ok(h) => h,
            Err(_) => return,
        };
        let sources = cfg.sources();
        loop {
            for (k, src) in sources.iter().enumerate() {
                let one = cfg.for_source(src);
                let r = match read_token(&src.token_file) {
                    Ok(t) => scan(&http, &one, &t).await,
                    Err(e) => Err(e),
                };
                if let Ok(mut q) = shared.lock() {
                    while q.accounts.len() <= k {
                        q.accounts.push(Snapshot {
                            account: src.label.clone().unwrap_or_else(|| host_of(&src.api_url)),
                            ..Default::default()
                        });
                    }
                    match r {
                        Ok(mut snap) => {
                            if let Some(l) = &src.label {
                                snap.account = l.clone();
                                snap.prs.iter_mut().for_each(|p| p.account = l.clone());
                                snap.items.iter_mut().for_each(|i| i.account = l.clone());
                            }
                            q.accounts[k] = snap;
                        }
                        Err(e) => q.accounts[k].error = Some(e),
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(cfg.interval_s.max(60))).await;
        }
    });
}

fn pr_rows(prs: &[&Pr]) -> String {
    let esc = crate::dashboard::fleet::esc;
    prs.iter()
        .map(|p| {
            let class = [
                p.done.then_some("done"),
                p.re_review.then_some("rereview"),
                p.highlighted.then_some("highlight"),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
            format!(
                r#"<tr class="{class}"><td><a href="{}">{}#{}</a> {}<br><span class="dim">{} · {} · {}</span></td><td>{}{}</td><td>{} (+{} −{}, {} files)</td><td>{}{}</td><td>{}</td><td>{}</td></tr>"#,
                esc(&p.url),
                esc(&p.repo),
                p.number,
                esc(&p.title),
                esc(&p.author),
                esc(&p.why),
                esc(&p.account),
                if p.re_review { "<b>re-review</b> · " } else { "" },
                esc(p.my_review.as_deref().unwrap_or("not reviewed")),
                esc(&p.size),
                p.additions,
                p.deletions,
                p.changed_files,
                esc(&p.build),
                p.failed_check.as_deref().map(|c| format!(" ({})", esc(c))).unwrap_or_default(),
                esc(&p.decision),
                esc(p.requested_at.as_deref().unwrap_or(&p.created_at)),
            )
        })
        .collect()
}

fn pr_table(title: &str, prs: &[&Pr]) -> String {
    if prs.is_empty() {
        return String::new();
    }
    format!(
        r#"<section class="panel"><h3>{title} ({})</h3><div class="scroll"><table class="data-table"><thead><tr><th>Pull request</th><th>Your review</th><th>Size</th><th>Build</th><th>Decision</th><th>Waiting since</th></tr></thead><tbody>{}</tbody></table></div></section>"#,
        prs.len(),
        pr_rows(prs)
    )
}

fn item_table(title: &str, items: &[&Item]) -> String {
    let esc = crate::dashboard::fleet::esc;
    if items.is_empty() {
        return String::new();
    }
    let rows: String = items
        .iter()
        .map(|i| {
            format!(
                r#"<tr><td><a href="{}">{}#{}</a> {}<br><span class="dim">{} · {} · {}</span></td><td>{}</td></tr>"#,
                esc(&i.url),
                esc(&i.repo),
                i.number,
                esc(&i.title),
                esc(&i.kind),
                esc(&i.author),
                esc(&i.account),
                esc(&i.waiting_since),
            )
        })
        .collect();
    format!(
        r#"<section class="panel"><h3>{title} ({})</h3><div class="scroll"><table class="data-table"><thead><tr><th>Issue or pull request</th><th>Waiting since</th></tr></thead><tbody>{rows}</tbody></table></div></section>"#,
        items.len()
    )
}

/// The queue, every source or only `account`: review requests to the
/// person, then to their teams, assigned PRs, assigned issues and mentions
/// without an answer. Done PRs stay in their section, greyed and last.
pub fn view(q: &Queue, account: Option<&str>) -> String {
    let esc = crate::dashboard::fleet::esc;
    let shown: Vec<&Snapshot> = q
        .accounts
        .iter()
        .filter(|s| account.is_none_or(|a| s.account == a))
        .collect();
    let prs: Vec<&Pr> = shown.iter().flat_map(|s| &s.prs).collect();
    let items: Vec<&Item> = shown.iter().flat_map(|s| &s.items).collect();
    let pick = |f: &dyn Fn(&Pr) -> bool| prs.iter().copied().filter(|p| f(p)).collect::<Vec<_>>();
    let mine = pick(&|p| p.why == "me");
    let team = pick(&|p| p.why.starts_with("team:"));
    let assigned = pick(&|p| p.why == "assigned");
    let watched = pick(&|p| p.why == "author" || p.why == "repo");
    let assigned_issues: Vec<&Item> = items
        .iter()
        .copied()
        .filter(|i| i.why == "assigned")
        .collect();
    let mentioned: Vec<&Item> = items
        .iter()
        .copied()
        .filter(|i| i.why == "mentioned")
        .collect();
    let filter = if q.accounts.len() > 1 {
        let link = |label: &str, value: Option<&str>| {
            let on = value == account;
            let url = match value {
                Some(v) => format!("api/dashboard/reviews-partial?account={}", urlencode(v)),
                None => "api/dashboard/reviews-partial".into(),
            };
            format!(
                r##"<button type="button" class="nav-btn{}" hx-get="{}" hx-target="#content" hx-swap="innerHTML">{}</button>"##,
                if on { " active" } else { "" },
                esc(&url),
                esc(label)
            )
        };
        let mut f = link("All accounts", None);
        for s in &q.accounts {
            f.push_str(&link(&s.account, Some(&s.account)));
        }
        format!(r#"<nav class="nav account-filter" aria-label="Accounts">{f}</nav>"#)
    } else {
        String::new()
    };
    let errors: String = shown
        .iter()
        .filter_map(|s| {
            s.error.as_ref().map(|e| {
                format!(
                    r#"<p class="warn">{}: the last scan failed: {}. Its lists are from the scan before.</p>"#,
                    esc(&s.account),
                    esc(e)
                )
            })
        })
        .collect();
    let sections = [
        pr_table("Review requested from you", &mine),
        pr_table("Review requested from your teams", &team),
        pr_table("Pull requests assigned to you", &assigned),
        item_table("Issues assigned to you", &assigned_issues),
        item_table("Mentions you have not answered", &mentioned),
        pr_table("Watched authors and repositories", &watched),
    ]
    .concat();
    let note = if !sections.is_empty() {
        String::new()
    } else if shown.iter().all(|s| s.scanned_at == 0) {
        r#"<p class="empty">The first scan has not finished.</p>"#.into()
    } else {
        r#"<p class="empty">Nothing waits for you.</p>"#.into()
    };
    let who: Vec<String> = shown.iter().map(|s| esc(&s.account)).collect();
    format!(
        r#"<div class="overview reviews"><h2>My queue</h2>{filter}<p>For {}: re-reviews first, then the longest waiting; done ones last.</p>{errors}{note}{sections}</div>"#,
        if who.is_empty() {
            "nobody yet".to_string()
        } else {
            who.join(", ")
        },
    )
}

pub fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

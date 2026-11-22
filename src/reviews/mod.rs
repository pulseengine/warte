//! The reviews view (REQ-PRREVIEW-001, warte#85): the pull requests one person
//! has to review, how big each is, whether it is green, how long it has
//! waited, and which need a re-review. Read with that reviewer's own
//! read-only token, one GraphQL search per bucket, on an interval.
//!
//! PR titles and authors are private work data: the view and its API answer
//! a signed-in user, or, without `[auth]`, only a warte bound to loopback.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

/// `[reviews]` in warte.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewsConfig {
    /// A file holding the reviewer's read-only token (mode 0600).
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
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    /// Why it is listed: `me`, `team:<org/team>`, `author`, `repo`.
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

/// The latest scan, shared with the server.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub reviewer: String,
    pub scanned_at: i64,
    pub prs: Vec<Pr>,
    /// Why the last scan failed, if it did (never the token).
    pub error: Option<String>,
}

pub type Shared = Arc<Mutex<Snapshot>>;

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

/// One scan: the reviewer's login, then each search.
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
            if let Some(p) = from_node(n, &why, &me, cfg) {
                prs.push(p);
            }
        }
    }
    Ok(Snapshot {
        reviewer: me,
        scanned_at: crate::hub::now(),
        prs: order(prs),
        error: None,
    })
}

/// Scan every `interval_s`, keeping the last good list when a scan fails.
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
        loop {
            let r = match read_token(&cfg.token_file) {
                Ok(t) => scan(&http, &cfg, &t).await,
                Err(e) => Err(e),
            };
            if let Ok(mut s) = shared.lock() {
                match r {
                    Ok(snap) => *s = snap,
                    Err(e) => s.error = Some(e),
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(cfg.interval_s.max(60))).await;
        }
    });
}

/// The view: one row per PR, done ones greyed.
pub fn view(s: &Snapshot) -> String {
    let esc = crate::dashboard::fleet::esc;
    let rows: String = s
        .prs
        .iter()
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
                r#"<tr class="{class}"><td><a href="{}">{}#{}</a> {}<br><span class="dim">{} · {}</span></td><td>{}{}</td><td>{} (+{} −{}, {} files)</td><td>{}{}</td><td>{}</td><td>{}</td></tr>"#,
                esc(&p.url),
                esc(&p.repo),
                p.number,
                esc(&p.title),
                esc(&p.author),
                esc(&p.why),
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
        .collect();
    let note = match (&s.error, s.prs.is_empty()) {
        (Some(e), _) => format!(
            r#"<p class="warn">The last scan failed: {}. The list is from the scan before.</p>"#,
            esc(e)
        ),
        (None, true) if s.scanned_at == 0 => {
            r#"<p class="empty">The first scan has not finished.</p>"#.into()
        }
        (None, true) => r#"<p class="empty">Nothing waits for your review.</p>"#.into(),
        _ => String::new(),
    };
    format!(
        r#"<div class="overview reviews"><h2>Reviews</h2><p>For {}, re-reviews first, then the longest waiting; done ones last.</p>{note}<div class="scroll"><table class="data-table"><thead><tr><th>Pull request</th><th>Your review</th><th>Size</th><th>Build</th><th>Decision</th><th>Waiting since</th></tr></thead><tbody>{rows}</tbody></table></div></div>"#,
        esc(&s.reviewer),
    )
}

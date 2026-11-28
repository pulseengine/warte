//! Outside contributions (REQ-OUTSIDE-001, warte#238): open issues and pull
//! requests in the supervised orgs and accounts whose author is not one of
//! the insiders, newest activity first, with whether an insider has answered.
//!
//! "Inside" is GitHub's own author association on that repository: OWNER (the
//! account, or an org owner) or MEMBER (an org member, as the token can see
//! it), plus the people and teams configured as inside. Bots are left out.
//! On a user-owned repository only the owner is inside. Read with the same
//! per-source tokens as the queue, and private work data like it.

use crate::reviews::{host_of, read_token, Source};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

/// `[outside]` in warte.toml.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OutsideConfig {
    /// Orgs or user accounts to watch, per source label or host; the key is
    /// matched against a source's `label`, else its host (`github.com`).
    pub watch: std::collections::BTreeMap<String, Vec<String>>,
    /// Logins that count as inside wherever they post.
    #[serde(default)]
    pub inside_people: Vec<String>,
    /// Teams whose members count as inside, as `org/team` (read per scan on
    /// the host where that org lives).
    #[serde(default)]
    pub inside_teams: Vec<String>,
    #[serde(default = "default_interval")]
    pub interval_s: u64,
}

fn default_interval() -> u64 {
    900
}

/// One open issue or pull request from outside.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Contribution {
    pub account: String,
    /// `issue` or `pr`.
    pub kind: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    /// GitHub's association of the author with the repository.
    pub association: String,
    pub created_at: String,
    pub updated_at: String,
    /// An insider has commented since it was opened.
    pub answered: bool,
}

/// The associations that make an author inside.
fn inside_association(a: &str) -> bool {
    matches!(a, "OWNER" | "MEMBER")
}

fn is_bot(login: &str, typename: &str) -> bool {
    typename == "Bot" || login.ends_with("[bot]")
}

/// Whether a poster is inside: by association, or named, or in an inside team.
pub fn inside(login: &str, association: &str, people: &BTreeSet<String>) -> bool {
    inside_association(association) || people.contains(&login.to_ascii_lowercase())
}

/// One search node into a contribution; None when the author is inside or a bot.
pub fn from_node(n: &Value, account: &str, people: &BTreeSet<String>) -> Option<Contribution> {
    let author = n["author"]["login"].as_str().unwrap_or("ghost");
    let typename = n["author"]["__typename"].as_str().unwrap_or("");
    let association = n["authorAssociation"].as_str().unwrap_or("NONE");
    if is_bot(author, typename) || inside(author, association, people) {
        return None;
    }
    let answered = n["comments"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|c| {
            let l = c["author"]["login"].as_str().unwrap_or("");
            let t = c["author"]["__typename"].as_str().unwrap_or("");
            !is_bot(l, t) && inside(l, c["authorAssociation"].as_str().unwrap_or(""), people)
        });
    Some(Contribution {
        account: account.into(),
        kind: if n.get("isDraft").is_some() {
            "pr"
        } else {
            "issue"
        }
        .into(),
        repo: n["repository"]["nameWithOwner"].as_str()?.into(),
        number: n["number"].as_u64()?,
        title: n["title"].as_str().unwrap_or("").into(),
        url: n["url"].as_str().unwrap_or("").into(),
        author: author.into(),
        association: association.into(),
        created_at: n["createdAt"].as_str().unwrap_or("").into(),
        updated_at: n["updatedAt"].as_str().unwrap_or("").into(),
        answered,
    })
}

const FIELDS: &str = "... on Issue { number title url createdAt updatedAt authorAssociation repository { nameWithOwner } author { __typename login } comments(last: 30) { nodes { authorAssociation author { __typename login } } } } ... on PullRequest { number title url createdAt updatedAt isDraft authorAssociation repository { nameWithOwner } author { __typename login } comments(last: 30) { nodes { authorAssociation author { __typename login } } } }";

/// The search for one watched owner (org or user).
pub fn search(owner: &str) -> String {
    format!("user:{owner} is:open archived:false sort:updated-desc")
}

/// One source's latest scan.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub account: String,
    pub scanned_at: i64,
    pub items: Vec<Contribution>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Outside {
    pub accounts: Vec<Snapshot>,
}

pub type Shared = Arc<Mutex<Outside>>;

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

/// The label a source goes by: its configured label, else its host.
pub fn label_of(s: &Source) -> String {
    s.label.clone().unwrap_or_else(|| host_of(&s.api_url))
}

/// Scan one source's watched owners. Inside teams whose org is not on this
/// host are skipped (the team query fails there; its members stay outside).
pub async fn scan(
    http: &reqwest::Client,
    api_url: &str,
    account: &str,
    owners: &[String],
    cfg: &OutsideConfig,
    token: &str,
) -> Result<Snapshot, String> {
    let mut people: BTreeSet<String> = cfg
        .inside_people
        .iter()
        .map(|p| p.to_ascii_lowercase())
        .collect();
    for t in &cfg.inside_teams {
        let Some((org, slug)) = t.split_once('/') else {
            continue;
        };
        let q = "query($o: String!, $t: String!) { organization(login: $o) { team(slug: $t) { members(first: 100) { nodes { login } } } } }";
        if let Ok(v) = graphql(http, api_url, token, q, json!({"o": org, "t": slug})).await {
            for m in v["data"]["organization"]["team"]["members"]["nodes"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if let Some(l) = m["login"].as_str() {
                    people.insert(l.to_ascii_lowercase());
                }
            }
        }
    }
    let q = format!("query($q: String!) {{ search(query: $q, type: ISSUE, first: 100) {{ nodes {{ {FIELDS} }} }} }}");
    let mut items = Vec::new();
    for o in owners {
        let v = graphql(http, api_url, token, &q, json!({"q": search(o)})).await?;
        for n in v["data"]["search"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(c) = from_node(n, account, &people) {
                items.push(c);
            }
        }
    }
    let mut seen = BTreeSet::new();
    items.retain(|c| seen.insert((c.repo.clone(), c.number)));
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(Snapshot {
        account: account.into(),
        scanned_at: crate::hub::now(),
        items,
        error: None,
    })
}

/// The sources that have owners to watch, with those owners.
pub fn watched(sources: &[Source], cfg: &OutsideConfig) -> Vec<(Source, Vec<String>)> {
    sources
        .iter()
        .filter_map(|s| {
            let owners = cfg
                .watch
                .get(&label_of(s))
                .or_else(|| cfg.watch.get(&host_of(&s.api_url)))?;
            (!owners.is_empty()).then(|| (s.clone(), owners.clone()))
        })
        .collect()
}

/// Scan every watched source every `interval_s`; a failing source keeps its
/// last list, marked with the error.
pub fn spawn(sources: Vec<Source>, cfg: OutsideConfig, shared: Shared) {
    tokio::spawn(async move {
        let Ok(http) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(concat!("warte/", env!("CARGO_PKG_VERSION")))
            .build()
        else {
            return;
        };
        let list = watched(&sources, &cfg);
        loop {
            for (k, (src, owners)) in list.iter().enumerate() {
                let account = label_of(src);
                let r = match read_token(&src.token_file) {
                    Ok(t) => scan(&http, &src.api_url, &account, owners, &cfg, &t).await,
                    Err(e) => Err(e),
                };
                if let Ok(mut o) = shared.lock() {
                    while o.accounts.len() <= k {
                        o.accounts.push(Snapshot {
                            account: account.clone(),
                            ..Default::default()
                        });
                    }
                    match r {
                        Ok(s) => o.accounts[k] = s,
                        Err(e) => o.accounts[k].error = Some(e),
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(cfg.interval_s.max(300))).await;
        }
    });
}

/// The view: one table, unanswered first within the newest activity.
pub fn view(o: &Outside, account: Option<&str>) -> String {
    let esc = crate::dashboard::fleet::esc;
    let shown: Vec<&Snapshot> = o
        .accounts
        .iter()
        .filter(|s| account.is_none_or(|a| s.account == a))
        .collect();
    let mut items: Vec<&Contribution> = shown.iter().flat_map(|s| &s.items).collect();
    items.sort_by(|a, b| (a.answered, &b.updated_at).cmp(&(b.answered, &a.updated_at)));
    let filter = if o.accounts.len() > 1 {
        let link = |label: &str, value: Option<&str>| {
            let url = match value {
                Some(v) => format!(
                    "api/dashboard/outside-partial?account={}",
                    crate::reviews::urlencode(v)
                ),
                None => "api/dashboard/outside-partial".into(),
            };
            format!(
                r##"<button type="button" class="nav-btn{}" hx-get="{}" hx-target="#content" hx-swap="innerHTML">{}</button>"##,
                if value == account { " active" } else { "" },
                esc(&url),
                esc(label)
            )
        };
        let mut f = link("All accounts", None);
        for s in &o.accounts {
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
                    r#"<p class="warn">{}: the last scan failed: {}. Its list is from the scan before.</p>"#,
                    esc(&s.account),
                    esc(e)
                )
            })
        })
        .collect();
    let unanswered = items.iter().filter(|c| !c.answered).count();
    let rows: String = items
        .iter()
        .map(|c| {
            format!(
                r#"<tr class="{}"><td><a href="{}">{}#{}</a> {}<br><span class="dim">{} · {} · {}</span></td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                if c.answered { "done" } else { "" },
                esc(&c.url),
                esc(&c.repo),
                c.number,
                esc(&c.title),
                esc(&c.kind),
                esc(&c.association.to_ascii_lowercase().replace('_', " ")),
                esc(&c.account),
                esc(&c.author),
                if c.answered { "answered" } else { "<b>waiting</b>" },
                esc(&c.updated_at),
            )
        })
        .collect();
    let body = if !items.is_empty() {
        format!(
            r#"<div class="scroll"><table class="data-table"><thead><tr><th>Issue or pull request</th><th>Author</th><th>Inside answered</th><th>Last activity</th></tr></thead><tbody>{rows}</tbody></table></div>"#
        )
    } else if shown.iter().all(|s| s.scanned_at == 0) {
        r#"<p class="empty">The first scan has not finished.</p>"#.into()
    } else {
        r#"<p class="empty">Nothing open from outside.</p>"#.into()
    };
    format!(
        r#"<div class="overview outside"><h2>Outside</h2>{filter}<p>Open issues and pull requests whose author is not an owner, an org member or listed as inside; bots left out. {} of {} wait for an answer from inside.</p>{errors}{body}</div>"#,
        unanswered,
        items.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(author: &str, typ: &str, assoc: &str, comments: Value, pr: bool) -> Value {
        let mut n = json!({"number": 1, "title": "t", "url": "https://example.test/1",
            "createdAt": "2026-11-01T00:00:00Z", "updatedAt": "2026-11-02T00:00:00Z",
            "authorAssociation": assoc, "repository": {"nameWithOwner": "example-org/app"},
            "author": {"__typename": typ, "login": author}, "comments": {"nodes": comments}});
        if pr {
            n["isDraft"] = json!(false);
        }
        n
    }

    // rivet: verifies REQ-OUTSIDE-001
    #[test]
    fn owners_members_listed_people_and_bots_are_not_outside() {
        let people: BTreeSet<String> = ["friend".to_string()].into();
        let none = json!([]);
        assert!(from_node(
            &node("x", "User", "OWNER", none.clone(), false),
            "a",
            &people
        )
        .is_none());
        assert!(from_node(
            &node("x", "User", "MEMBER", none.clone(), false),
            "a",
            &people
        )
        .is_none());
        assert!(from_node(
            &node("Friend", "User", "NONE", none.clone(), false),
            "a",
            &people
        )
        .is_none());
        assert!(from_node(
            &node("renovate[bot]", "Bot", "NONE", none.clone(), true),
            "a",
            &people
        )
        .is_none());
        for assoc in [
            "COLLABORATOR",
            "CONTRIBUTOR",
            "FIRST_TIME_CONTRIBUTOR",
            "NONE",
        ] {
            let c = from_node(
                &node("stranger", "User", assoc, none.clone(), true),
                "a",
                &people,
            )
            .unwrap();
            assert_eq!((c.kind.as_str(), c.answered), ("pr", false), "{assoc}");
        }
    }

    // rivet: verifies REQ-OUTSIDE-001
    #[test]
    fn an_answer_counts_only_from_inside_and_never_from_a_bot() {
        let people = BTreeSet::new();
        let c = |a: &str, t: &str, assoc: &str| json!({"authorAssociation": assoc, "author": {"__typename": t, "login": a}});
        let n = node(
            "s",
            "User",
            "NONE",
            json!([c("other", "User", "NONE"), c("ci[bot]", "Bot", "MEMBER")]),
            false,
        );
        assert!(!from_node(&n, "a", &people).unwrap().answered);
        let n = node(
            "s",
            "User",
            "NONE",
            json!([c("m", "User", "MEMBER")]),
            false,
        );
        assert!(from_node(&n, "a", &people).unwrap().answered);
    }

    // rivet: verifies REQ-OUTSIDE-001
    #[test]
    fn sources_are_watched_by_label_or_host_and_the_view_puts_waiting_first() {
        let src = |api: &str, label: Option<&str>| Source {
            api_url: api.into(),
            token_file: "/t".into(),
            teams: vec![],
            label: label.map(String::from),
        };
        let cfg: OutsideConfig = toml::from_str(
            "[watch]\n\"github.com\" = [\"example-org\", \"example-user\"]\nwork = [\"example-corp\"]\n",
        )
        .unwrap();
        let w = watched(
            &[
                src("https://api.github.com", None),
                src("https://ghe.example.test/api/v3", Some("work")),
                src("https://other.example.test/api/v3", None),
            ],
            &cfg,
        );
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].1, ["example-org", "example-user"]);
        assert_eq!(w[1].1, ["example-corp"]);
        assert_eq!(
            search("example-org"),
            "user:example-org is:open archived:false sort:updated-desc"
        );

        let item = |n: u64, answered: bool, at: &str| Contribution {
            account: "github.com".into(),
            kind: "issue".into(),
            repo: "example-org/app".into(),
            number: n,
            title: "<i>x</i>".into(),
            url: "https://example.test".into(),
            author: "s".into(),
            association: "FIRST_TIME_CONTRIBUTOR".into(),
            created_at: at.into(),
            updated_at: at.into(),
            answered,
        };
        let o = Outside {
            accounts: vec![Snapshot {
                account: "github.com".into(),
                scanned_at: 1,
                items: vec![
                    item(1, true, "2026-11-05"),
                    item(2, false, "2026-11-01"),
                    item(3, false, "2026-11-03"),
                ],
                error: None,
            }],
        };
        let v = view(&o, None);
        let (a, b, c) = (
            v.find("app#3").unwrap(),
            v.find("app#2").unwrap(),
            v.find("app#1").unwrap(),
        );
        assert!(a < b && b < c, "waiting first, newest first: {v}");
        assert!(v.contains("2 of 3 wait") && v.contains("first time contributor"));
        assert!(!v.contains("<i>x</i>"));
    }
}

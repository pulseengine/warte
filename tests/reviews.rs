//! The reviews view (REQ-PRREVIEW-001, warte#85).

use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
use warte::reviews::{from_node, order, read_token, scan, searches, size, ReviewsConfig};

fn cfg(api: &str) -> ReviewsConfig {
    ReviewsConfig {
        token_file: String::new(),
        api_url: api.into(),
        teams: vec!["acme/core".into()],
        highlight_authors: vec!["boss".into()],
        highlight_repos: vec![],
        highlight_labels: vec!["urgent".into()],
        interval_s: 600,
        sources: vec![],
    }
}

fn node(
    number: u64,
    author: &str,
    head: &str,
    my_commit: Option<&str>,
    decision: &str,
    rollup: &str,
    mergeable: &str,
) -> Value {
    let reviews: Vec<Value> = my_commit
        .map(|c| {
            vec![json!({"author": {"login": "me"}, "state": "COMMENTED", "commit": {"oid": c}})]
        })
        .unwrap_or_default();
    json!({"number": number, "title": format!("PR {number}"), "url": format!("https://example.test/pr/{number}"),
        "createdAt": format!("2026-11-{:02}T00:00:00Z", 10 + number), "updatedAt": "2026-11-21T00:00:00Z",
        "additions": 40 * number, "deletions": 10, "changedFiles": 3, "headRefOid": head,
        "reviewDecision": decision, "mergeable": mergeable, "isDraft": false,
        "repository": {"nameWithOwner": "acme/app"}, "author": {"login": author},
        "labels": {"nodes": [{"name": if number == 3 { "urgent" } else { "chore" }}]},
        "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": rollup,
            "contexts": {"nodes": [{"name": "Build", "conclusion": "SUCCESS"}, {"name": "Clippy", "conclusion": "FAILURE"}]}}}}]},
        "reviews": {"nodes": reviews},
        "timelineItems": {"nodes": [{"createdAt": format!("2026-11-{:02}T00:00:00Z", 15 + number)}]}})
}

// rivet: verifies REQ-PRREVIEW-001
#[test]
fn a_node_becomes_a_pr_with_size_build_re_review_done_and_highlight() {
    let c = cfg("http://x");
    let p = from_node(
        &node(
            1,
            "dev",
            "abc",
            Some("old"),
            "REVIEW_REQUIRED",
            "FAILURE",
            "MERGEABLE",
        ),
        "me",
        "me",
        &c,
    )
    .unwrap();
    assert_eq!((p.size.as_str(), p.build.as_str()), ("S", "red"));
    assert_eq!(p.failed_check.as_deref(), Some("Clippy"));
    assert!(p.re_review, "my review is on an older commit");
    assert!(!p.done);
    let q = from_node(
        &node(
            2,
            "dev",
            "abc",
            Some("abc"),
            "APPROVED",
            "SUCCESS",
            "MERGEABLE",
        ),
        "me",
        "me",
        &c,
    )
    .unwrap();
    assert!(!q.re_review && q.done, "approved, green, mergeable");
    let r = from_node(
        &node(3, "boss", "abc", None, "", "PENDING", "UNKNOWN"),
        "author",
        "me",
        &c,
    )
    .unwrap();
    assert!(r.highlighted && r.my_review.is_none() && r.build == "running");
    assert_eq!(size(30, 20), "S");
    assert_eq!(size(200, 51), "L");
    assert_eq!(size(2000, 0), "XL");
}

// rivet: verifies REQ-PRREVIEW-001
#[test]
fn re_reviews_come_first_then_the_longest_waiting_and_done_last() {
    let c = cfg("http://x");
    let prs = vec![
        from_node(
            &node(
                2,
                "dev",
                "abc",
                Some("abc"),
                "APPROVED",
                "SUCCESS",
                "MERGEABLE",
            ),
            "me",
            "me",
            &c,
        )
        .unwrap(),
        from_node(
            &node(4, "dev", "h", None, "", "SUCCESS", "MERGEABLE"),
            "me",
            "me",
            &c,
        )
        .unwrap(),
        from_node(
            &node(3, "boss", "h", None, "", "SUCCESS", "MERGEABLE"),
            "me",
            "me",
            &c,
        )
        .unwrap(),
        from_node(
            &node(1, "dev", "abc", Some("old"), "", "FAILURE", "MERGEABLE"),
            "me",
            "me",
            &c,
        )
        .unwrap(),
        // the same PR again from another bucket keeps its first reason
        from_node(
            &node(1, "dev", "abc", Some("old"), "", "FAILURE", "MERGEABLE"),
            "team:acme/core",
            "me",
            &c,
        )
        .unwrap(),
    ];
    let o = order(prs);
    let nums: Vec<u64> = o.iter().map(|p| p.number).collect();
    assert_eq!(nums, [1, 3, 4, 2]);
    assert_eq!(o[0].why, "me");
    let s = searches(&c);
    assert_eq!(s.len(), 4);
    assert!(s[1].1.contains("team-review-requested:acme/core"));
    assert!(s[2].1.contains("assignee:@me") && s[3].1.contains("author:boss"));
}

// rivet: verifies REQ-PRREVIEW-001
#[tokio::test]
async fn a_scan_asks_who_i_am_then_each_search_and_skips_drafts_and_my_own() {
    let app = Router::new().route(
        "/graphql",
        post(|Json(b): Json<Value>| async move {
            let q = b["query"].as_str().unwrap_or("");
            if q.contains("viewer") {
                return Json(json!({"data": {"viewer": {"login": "me"}}}));
            }
            let mut draft = node(5, "dev", "h", None, "", "SUCCESS", "MERGEABLE");
            draft["isDraft"] = json!(true);
            let mine = node(6, "me", "h", None, "", "SUCCESS", "MERGEABLE");
            let ok = node(1, "dev", "abc", Some("old"), "", "FAILURE", "MERGEABLE");
            Json(json!({"data": {"search": {"nodes": [draft, mine, ok]}}}))
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let http = reqwest::Client::new();
    let s = scan(&http, &cfg(&base), "t0ken").await.unwrap();
    assert_eq!(s.reviewer, "me");
    let nums: Vec<u64> = s.prs.iter().map(|p| p.number).collect();
    assert_eq!(
        nums,
        [1],
        "drafts and the reviewer's own PRs are left out, duplicates merged"
    );

    // a refusal never echoes the token
    let bad = Router::new().route(
        "/graphql",
        post(|| async { Json(json!({"errors": [{"message": "Bad credentials"}]})) }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, bad).await.unwrap() });
    let e = scan(&http, &cfg(&base), "s3cret-token").await.unwrap_err();
    assert!(!e.contains("s3cret"), "{e}");
}

// rivet: verifies REQ-PRREVIEW-001
#[test]
fn the_token_file_must_be_private_and_hold_one_token() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("t");
    std::fs::write(&p, "ghp_example\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_token(p.to_str().unwrap())
            .unwrap_err()
            .contains("chmod 600"));
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert_eq!(read_token(p.to_str().unwrap()).unwrap(), "ghp_example");
    std::fs::write(&p, "two tokens").unwrap();
    assert!(read_token(p.to_str().unwrap()).is_err());
}

async fn serve_with_bind(bind: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        bind: bind.into(),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move {
        let _keep = dir;
        axum::serve(l, app).await.unwrap()
    });
    base
}

// rivet: verifies REQ-PRREVIEW-001
#[tokio::test]
async fn without_a_login_the_reviews_answer_only_on_loopback() {
    let lan = serve_with_bind("0.0.0.0:8080").await;
    assert_eq!(
        reqwest::get(format!("{lan}/api/reviews"))
            .await
            .unwrap()
            .status()
            .as_u16(),
        401
    );
    let page = reqwest::get(format!("{lan}/api/dashboard/reviews-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("private work data"), "{page}");
    let local = serve_with_bind("127.0.0.1:8080").await;
    assert_eq!(
        reqwest::get(format!("{local}/api/reviews"))
            .await
            .unwrap()
            .status()
            .as_u16(),
        404,
        "loopback, no [reviews]"
    );
}

use warte::reviews::{
    host_of, item_from_node, mentions, unanswered_since, view, Item, Queue, Snapshot, Source,
};

fn msg(a: &str, t: &str, body: &str) -> (String, String, String) {
    (a.into(), t.into(), body.into())
}

// rivet: verifies REQ-PRREVIEW-002
#[test]
fn a_mention_waits_until_the_person_answers_after_it() {
    assert!(mentions("ping @Me, please", "me"));
    assert!(mentions("@me", "me"));
    assert!(
        !mentions("@meow and @me-too", "me"),
        "a longer login is someone else"
    );
    assert!(!mentions("email me@example.test", "me"));
    // mentioned in a comment, no answer since: waits from that comment
    let p = [
        msg("a", "t1", "issue text"),
        msg("b", "t2", "@me look"),
        msg("c", "t3", "+1"),
    ];
    assert_eq!(unanswered_since("me", &p).as_deref(), Some("t2"));
    // answered after the mention: off the list
    let p = [msg("a", "t1", "@me ?"), msg("me", "t2", "on it")];
    assert_eq!(unanswered_since("me", &p), None);
    // answered, then mentioned again: waits from the new mention
    let p = [
        msg("a", "t1", "@me ?"),
        msg("me", "t2", "ok"),
        msg("a", "t3", "@ME again"),
    ];
    assert_eq!(unanswered_since("me", &p).as_deref(), Some("t3"));
    // the text does not show the mention (a team mention): any post of mine answers
    let p = [msg("a", "t1", "@acme/core ?"), msg("b", "t2", "x")];
    assert_eq!(unanswered_since("me", &p).as_deref(), Some("t1"));
    let p = [msg("a", "t1", "@acme/core ?"), msg("me", "t2", "x")];
    assert_eq!(unanswered_since("me", &p), None);
    // my own mention of myself does not count
    let p = [msg("me", "t1", "note to @me")];
    assert_eq!(unanswered_since("me", &p), None);
}

fn issue(number: u64, author: &str, body: &str, comments: Value, pr: bool) -> Value {
    let mut n = json!({"number": number, "title": format!("<b>Issue {number}</b>"),
        "url": format!("https://example.test/i/{number}"), "createdAt": "2026-11-01T00:00:00Z",
        "updatedAt": "2026-11-02T00:00:00Z", "repository": {"nameWithOwner": "example-org/app"},
        "author": {"login": author}, "body": body, "comments": {"nodes": comments}});
    if pr {
        n["isDraft"] = json!(false);
    }
    n
}

// rivet: verifies REQ-PRREVIEW-002
#[test]
fn issue_nodes_become_assigned_or_mentioned_items() {
    let a = item_from_node(&issue(7, "dev", "text", json!([]), false), "assigned", "me").unwrap();
    assert_eq!((a.kind.as_str(), a.why.as_str()), ("issue", "assigned"));
    assert_eq!(a.waiting_since, "2026-11-01T00:00:00Z");
    let m = item_from_node(
        &issue(8, "dev", "hi", json!([{"author": {"login": "x"}, "createdAt": "2026-11-03T00:00:00Z", "body": "@me ?"}]), true),
        "mentioned",
        "me",
    )
    .unwrap();
    assert_eq!(
        (m.kind.as_str(), m.waiting_since.as_str()),
        ("pr", "2026-11-03T00:00:00Z")
    );
    let answered = issue(
        9,
        "dev",
        "@me ?",
        json!([{"author": {"login": "ME"}, "createdAt": "t", "body": "done"}]),
        false,
    );
    assert!(item_from_node(&answered, "mentioned", "me").is_none());
    assert!(
        item_from_node(&issue(10, "me", "x", json!([]), false), "assigned", "me").is_none(),
        "my own"
    );
}

// rivet: verifies REQ-PRREVIEW-002
#[test]
fn sources_are_the_list_or_the_one_top_level_token_and_hosts_read_plainly() {
    let mut c = cfg("https://api.github.com");
    assert!(c.sources().is_empty(), "no token file, no source");
    c.token_file = "/t".into();
    let one = c.sources();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].teams, ["acme/core"]);
    let t: warte::reviews::ReviewsConfig = toml::from_str(
        r#"
        [[sources]]
        token_file = "/a"
        [[sources]]
        api_url = "https://ghe.example.test/api/v3"
        token_file = "/b"
        teams = ["example-org/core"]
        label = "work"
        "#,
    )
    .unwrap();
    let s = t.sources();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].api_url, "https://api.github.com");
    assert_eq!(s[1].label.as_deref(), Some("work"));
    let narrowed = t.for_source(&s[1]);
    assert_eq!(
        (narrowed.api_url.as_str(), narrowed.teams.len()),
        ("https://ghe.example.test/api/v3", 1)
    );
    assert_eq!(host_of("https://api.github.com"), "github.com");
    assert_eq!(
        host_of("https://api.example-tenant.ghe.com"),
        "example-tenant.ghe.com"
    );
    assert_eq!(
        host_of("https://ghe.example.test/api/v3"),
        "ghe.example.test"
    );
}

// rivet: verifies REQ-PRREVIEW-002
#[tokio::test]
async fn a_scan_fills_assigned_prs_assigned_issues_and_open_mentions() {
    let app = Router::new().route(
        "/graphql",
        post(|Json(b): Json<Value>| async move {
            let q = b["query"].as_str().unwrap_or("");
            let s = b["variables"]["q"].as_str().unwrap_or("");
            if q.contains("viewer") {
                return Json(json!({"data": {"viewer": {"login": "me"}}}));
            }
            let nodes = if q.contains("reviewDecision") {
                if s.contains("assignee:@me") {
                    vec![node(2, "dev", "h", None, "", "SUCCESS", "MERGEABLE")]
                } else if s.contains("review-requested:@me") {
                    vec![node(1, "dev", "h", None, "", "SUCCESS", "MERGEABLE")]
                } else {
                    vec![]
                }
            } else if s.contains("assignee:@me") {
                vec![issue(7, "dev", "x", json!([]), false)]
            } else {
                // a mention on the PR already under review is not listed twice
                let mut dup = issue(1, "dev", "@me", json!([]), true);
                dup["repository"]["nameWithOwner"] = json!("acme/app");
                vec![issue(8, "dev", "@me ?", json!([]), false), dup]
            };
            Json(json!({"data": {"search": {"nodes": nodes}}}))
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let base = format!("http://{addr}");
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let s = scan(&reqwest::Client::new(), &cfg(&base), "t0ken")
        .await
        .unwrap();
    assert_eq!(s.account, format!("me@{addr}"));
    let whys: Vec<(u64, &str)> = s.prs.iter().map(|p| (p.number, p.why.as_str())).collect();
    assert!(
        whys.contains(&(1, "me")) && whys.contains(&(2, "assigned")),
        "{whys:?}"
    );
    let items: Vec<(u64, &str)> = s.items.iter().map(|i| (i.number, i.why.as_str())).collect();
    assert_eq!(items, [(7, "assigned"), (8, "mentioned")]);
    assert!(s.prs.iter().all(|p| p.account == s.account));
}

fn snap(account: &str, items: Vec<Item>) -> Snapshot {
    Snapshot {
        account: account.into(),
        reviewer: "me".into(),
        scanned_at: 1,
        prs: vec![],
        items,
        error: None,
    }
}

fn item(account: &str, n: u64, why: &str) -> Item {
    Item {
        account: account.into(),
        kind: "issue".into(),
        repo: "example-org/app".into(),
        number: n,
        title: "<script>x</script>".into(),
        url: "https://example.test".into(),
        author: "dev".into(),
        why: why.into(),
        created_at: "t".into(),
        waiting_since: "t".into(),
    }
}

// rivet: verifies REQ-PRREVIEW-002
#[test]
fn the_view_has_a_section_per_bucket_and_filters_by_account() {
    let q = Queue {
        accounts: vec![
            snap("me@github.com", vec![item("me@github.com", 1, "assigned")]),
            Snapshot {
                error: Some("GitHub refused the search (401)".into()),
                ..snap("work", vec![item("work", 2, "mentioned")])
            },
        ],
    };
    let all = view(&q, None);
    assert!(
        all.contains("Issues assigned to you (1)")
            && all.contains("Mentions you have not answered (1)"),
        "{all}"
    );
    assert!(
        all.contains("All accounts") && all.contains("account=me%40github.com"),
        "{all}"
    );
    assert!(all.contains("work: the last scan failed"));
    assert!(!all.contains("<script>x"), "titles are escaped");
    let one = view(&q, Some("work"));
    assert!(
        !one.contains("Issues assigned to you")
            && one.contains("Mentions you have not answered (1)"),
        "{one}"
    );
    let single = Queue {
        accounts: vec![snap("me@github.com", vec![])],
    };
    let v = view(&single, None);
    assert!(!v.contains("All accounts"), "no filter for one account");
    assert!(v.contains("Nothing waits for you."));
    let _ = Source {
        api_url: String::new(),
        token_file: String::new(),
        teams: vec![],
        label: None,
    };
}

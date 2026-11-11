//! Issue triage queue (REQ-TRIAGE-001/002), against a mock GitHub event feed.
//! Synthetic org, repositories and people only.

use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Router,
};
use serde_json::{json, Value};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use warte::triage::{from_event, Triage, TriageConfig};

fn cfg(base: &str) -> TriageConfig {
    TriageConfig {
        org: "acme".into(),
        api_url: base.into(),
        interval_s: 600,
        per_hour: 2,
        agent_markers: vec!["Generated with [Claude Code]".into()],
        skip_logins: vec!["acme-ci".into()],
        data_dir: None,
    }
}

fn ev(id: &str, repo: &str, n: u64, action: &str, actor: &str, body: &str) -> Value {
    json!({"id": id, "type": "IssuesEvent", "actor": {"login": actor},
           "repo": {"name": format!("acme/{repo}")},
           "payload": {"action": action, "issue": {"number": n, "title": format!("Issue {n}: it <b>breaks</b>"),
                       "body": body, "user": {"login": actor, "type": "User"}}}})
}

// rivet: verifies REQ-TRIAGE-001
#[test]
fn only_issues_people_open_are_queued() {
    let c = cfg("http://x");
    let it = from_event(
        &ev("1", "spar", 5, "opened", "alice", "It crashes"),
        &c,
        100,
    )
    .expect("a person's issue");
    assert_eq!(
        (it.repo.as_str(), it.number, it.author.as_str()),
        ("spar", 5, "alice")
    );
    assert_eq!(it.url, "https://github.com/acme/spar/issues/5");
    assert!(from_event(&ev("2", "spar", 6, "reopened", "alice", ""), &c, 0).is_some());
    for (what, e) in [
        ("closed", ev("3", "spar", 7, "closed", "alice", "")),
        ("a bot", ev("4", "spar", 8, "opened", "dependabot[bot]", "")),
        (
            "a skipped login",
            ev("5", "spar", 9, "opened", "acme-ci", ""),
        ),
        (
            "an agent's issue",
            ev(
                "6",
                "spar",
                10,
                "opened",
                "owner",
                "text\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)",
            ),
        ),
    ] {
        assert!(from_event(&e, &c, 0).is_none(), "{what} was queued");
    }
    let mut pr = ev("7", "spar", 11, "opened", "alice", "");
    pr["payload"]["issue"]["pull_request"] = json!({"url": "x"});
    assert!(from_event(&pr, &c, 0).is_none(), "a pull request");
    let mut other = ev("8", "spar", 12, "opened", "alice", "");
    other["repo"]["name"] = "elsewhere/spar".into();
    assert!(from_event(&other, &c, 0).is_none(), "another org");
    let mut org = ev("9", "spar", 13, "opened", "alice", "");
    org["payload"]["issue"]["user"]["type"] = "Organization".into();
    assert!(from_event(&org, &c, 0).is_none(), "not a User");
    let mut push = ev("10", "spar", 14, "opened", "alice", "");
    push["type"] = "PushEvent".into();
    assert!(from_event(&push, &c, 0).is_none());
}

#[derive(Clone, Default)]
struct Mock {
    seen: Arc<Mutex<Vec<String>>>,
    events: Arc<Mutex<Vec<Value>>>,
}

async fn feed(State(m): State<Mock>, req: Request) -> Response {
    assert!(
        req.headers().get("authorization").is_none(),
        "triage sends no token"
    );
    let inm = req
        .headers()
        .get("if-none-match")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let ev = m.events.lock().unwrap().clone();
    let tag = format!("\"{}\"", ev.len());
    if req.uri().path() != "/orgs/acme/events" {
        return (StatusCode::NOT_FOUND, "no").into_response();
    }
    if inm.as_deref() == Some(tag.as_str()) {
        m.seen.lock().unwrap().push("304".into());
        return StatusCode::NOT_MODIFIED.into_response();
    }
    m.seen.lock().unwrap().push("200".into());
    let mut r = axum::Json(Value::Array(ev)).into_response();
    r.headers_mut().insert("etag", tag.parse().unwrap());
    r
}

async fn mock() -> (Mock, String) {
    let m = Mock::default();
    let app = Router::new().fallback(feed).with_state(m.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (m, base)
}

// rivet: verifies REQ-TRIAGE-001
#[tokio::test]
async fn the_feed_is_polled_conditionally_and_the_hour_caps_what_is_handed_out() {
    let (m, base) = mock().await;
    // newest first, as GitHub lists them
    *m.events.lock().unwrap() = vec![
        ev("13", "rivet", 3, "opened", "carol", "c"),
        ev("12", "spar", 2, "opened", "bob", "b"),
        ev("11", "spar", 1, "opened", "alice", "a"),
        ev("10", "spar", 9, "opened", "dependabot[bot]", ""),
    ];
    let dir = tempfile::tempdir().unwrap();
    let t = Triage::open(dir.path().to_str().unwrap(), cfg(&base)).unwrap();
    let http = reqwest::Client::new();
    assert_eq!(t.poll(&http, 1000).await.unwrap(), 3);
    // the same feed again: 304, nothing new
    assert_eq!(t.poll(&http, 1600).await.unwrap(), 0);
    assert_eq!(m.seen.lock().unwrap().clone(), vec!["200", "304"]);

    // two an hour: the oldest two are ready, one waits
    let o = t.offer(2000);
    assert_eq!(
        o.ready.iter().map(|i| i.number).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!((o.waiting, o.budget_left), (1, 2));
    t.done("spar", 1, "replied", 2000).unwrap();
    t.done("spar", 2, "asked", 2100).unwrap();
    let o = t.offer(2200);
    assert_eq!(
        (o.ready.len(), o.waiting, o.budget_left),
        (0, 1, 0),
        "the hour is used up"
    );
    let o = t.offer(2100 + 3601);
    assert_eq!(
        o.ready[0].repo, "rivet",
        "an hour later the next one is ready"
    );
    assert!(t.done("spar", 99, "replied", 0).is_err(), "not queued");
    assert!(
        t.done("rivet", 3, "deleted-the-repo", 0).is_err(),
        "unknown outcome"
    );

    // a reopened issue is queued again; the state survives a restart
    m.events
        .lock()
        .unwrap()
        .insert(0, ev("14", "spar", 1, "reopened", "alice", ""));
    assert_eq!(t.poll(&http, 9000).await.unwrap(), 1);
    drop(t);
    let t = Triage::open(dir.path().to_str().unwrap(), cfg(&base)).unwrap();
    assert_eq!(
        t.queued().into_iter().collect::<Vec<_>>(),
        vec!["rivet#3".to_string(), "spar#1".to_string()]
    );
    // the old ETag goes out after the restart too
    assert_eq!(t.poll(&http, 9100).await.unwrap(), 0);
    assert_eq!(m.seen.lock().unwrap().last().unwrap(), "304");
}

// rivet: verifies REQ-TRIAGE-002
#[tokio::test]
async fn the_run_reads_the_queue_openly_and_marks_done_with_its_right() {
    let (m, gh) = mock().await;
    *m.events.lock().unwrap() = vec![ev("1", "spar", 1, "opened", "alice", "a")];
    let dir = tempfile::tempdir().unwrap();
    let hub_dir = dir.path().join("hub");
    std::fs::create_dir_all(&hub_dir).unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: Some(warte::config::HubConfig {
            udp: "127.0.0.1:0".into(),
            allow: vec![Ipv4Addr::LOCALHOST.into()],
            data_dir: hub_dir.to_str().unwrap().into(),
            org: "acme".into(),
            probe_self: false,
            costs_file: None,
            skill_usage_file: None,
        }),
        triage: Some(cfg(&gh)),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let token = state
        .tokens
        .issue_token("triage-job", vec!["triage:write".into()])
        .unwrap();
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    let mut v = Value::Null;
    for _ in 0..50 {
        v = reqwest::get(format!("{base}/api/triage"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if v["ready"].as_array().is_some_and(|a| !a.is_empty()) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(v["ready"][0]["number"], 1, "{v}");
    let c = reqwest::Client::new();
    let post = |t: Option<&str>, b: Value| {
        let mut r = c
            .post(format!("{base}/api/triage/done"))
            .header("Content-Type", "application/json")
            .body(b.to_string());
        if let Some(t) = t {
            r = r.bearer_auth(t.to_string());
        }
        r.send()
    };
    let body = json!({"repo": "spar", "number": 1, "outcome": "replied"});
    assert_eq!(
        post(None, body.clone()).await.unwrap().status().as_u16(),
        401
    );
    let extra = json!({"repo": "spar", "number": 1, "outcome": "replied", "comment": "x"});
    assert_eq!(
        post(Some(&token), extra).await.unwrap().status().as_u16(),
        400
    );
    assert_eq!(
        post(Some(&token), body.clone())
            .await
            .unwrap()
            .status()
            .as_u16(),
        201
    );
    assert_eq!(
        post(Some(&token), body).await.unwrap().status().as_u16(),
        422,
        "done twice"
    );
    let v: Value = reqwest::get(format!("{base}/api/triage"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["recent"][0]["outcome"], "replied");
}

//! Outside contributions (REQ-OUTSIDE-001, warte#238): a scan against a
//! stand-in GraphQL, and the same access rule as the queue.

use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
use warte::outside::{scan, OutsideConfig};

fn node(n: u64, author: &str, assoc: &str, updated: &str, comments: Value) -> Value {
    json!({"number": n, "title": format!("item {n}"), "url": format!("https://example.test/{n}"),
        "createdAt": "2026-11-01T00:00:00Z", "updatedAt": updated, "authorAssociation": assoc,
        "repository": {"nameWithOwner": "example-org/app"},
        "author": {"__typename": "User", "login": author}, "comments": {"nodes": comments}})
}

// rivet: verifies REQ-OUTSIDE-001
#[tokio::test]
async fn a_scan_keeps_outside_authors_reads_inside_teams_and_survives_a_foreign_team() {
    let app = Router::new().route(
        "/graphql",
        post(|Json(b): Json<Value>| async move {
            let q = b["query"].as_str().unwrap_or("");
            let v = &b["variables"];
            if q.contains("organization") {
                // the inside team lives here; the other org is not on this host
                if v["o"] == "example-org" {
                    return Json(json!({"data": {"organization": {"team": {"members": {"nodes": [{"login": "Teammate"}]}}}}}));
                }
                return Json(json!({"errors": [{"message": "Could not resolve to an Organization"}]}));
            }
            assert_eq!(v["q"], "user:example-org is:open archived:false sort:updated-desc");
            let member = json!({"authorAssociation": "MEMBER", "author": {"__typename": "User", "login": "m"}});
            Json(json!({"data": {"search": {"nodes": [
                node(1, "stranger", "FIRST_TIME_CONTRIBUTOR", "2026-11-02T00:00:00Z", json!([])),
                node(2, "member", "MEMBER", "2026-11-03T00:00:00Z", json!([])),
                node(3, "teammate", "CONTRIBUTOR", "2026-11-04T00:00:00Z", json!([])),
                node(4, "other", "NONE", "2026-11-05T00:00:00Z", json!([member])),
                node(1, "stranger", "FIRST_TIME_CONTRIBUTOR", "2026-11-02T00:00:00Z", json!([])),
            ]}}}))
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let cfg = OutsideConfig {
        inside_teams: vec!["example-org/core".into(), "elsewhere/core".into()],
        ..Default::default()
    };
    let s = scan(
        &reqwest::Client::new(),
        &base,
        "github.com",
        &["example-org".into()],
        &cfg,
        "t0ken",
    )
    .await
    .unwrap();
    let got: Vec<(u64, bool)> = s.items.iter().map(|c| (c.number, c.answered)).collect();
    assert_eq!(
        got,
        [(4, true), (1, false)],
        "newest first, member and team member left out, duplicate merged"
    );
}

async fn serve(bind: &str) -> String {
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

// rivet: verifies REQ-OUTSIDE-001
#[tokio::test]
async fn without_a_login_outside_contributions_answer_only_on_loopback() {
    let lan = serve("0.0.0.0:8080").await;
    let r = reqwest::get(format!("{lan}/api/outside")).await.unwrap();
    assert_eq!(r.status().as_u16(), 401);
    let page = reqwest::get(format!("{lan}/api/dashboard/outside-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("private work data"), "{page}");
    let local = serve("127.0.0.1:8080").await;
    let r = reqwest::get(format!("{local}/api/outside")).await.unwrap();
    assert_eq!(r.status().as_u16(), 404, "loopback, no [outside]");
}

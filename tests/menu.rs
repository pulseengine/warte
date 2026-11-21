//! The grouped menu (REQ-DASH-019): five groups instead of 19 tabs, empty
//! views hidden, red and amber counts on the groups. The page itself is
//! checked in tests/playwright; these pin the structure and the nav-state.

use std::collections::BTreeSet;

const PAGE: &str = include_str!("../templates/dashboard.html");

/// (group, view label, partial route) for every view button, in page order.
fn views() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut group = String::new();
    for line in PAGE.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix(r#"<nav class="nav nav-views" data-group=""#) {
            group = rest.split('"').next().unwrap().to_string();
        }
        if l.contains(r#"class="nav-btn"#) {
            let route = l
                .split(r#"hx-get=""#)
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            let label = l
                .rsplit("</button>")
                .nth(1)
                .unwrap()
                .rsplit('>')
                .next()
                .unwrap();
            assert!(!group.is_empty(), "a view outside any group: {l}");
            out.push((group.clone(), label.to_string(), route.to_string()));
        }
    }
    out
}

// rivet: verifies REQ-DASH-019
#[test]
fn five_groups_hold_every_view_once_and_overview_is_the_landing() {
    let groups: Vec<&str> = PAGE
        .lines()
        .filter_map(|l| l.split(r#"class="nav-group"#).nth(1))
        .filter_map(|l| l.split(r#"data-group=""#).nth(1))
        .map(|l| l.split('"').next().unwrap())
        .collect();
    assert_eq!(groups, ["now", "ci", "machines", "agents", "compliance"]);
    let v = views();
    assert_eq!(v.len(), 20, "{v:?}");
    let routes: BTreeSet<&str> = v.iter().map(|x| x.2.as_str()).collect();
    assert_eq!(routes.len(), v.len(), "a view is in the menu twice");
    let labels: BTreeSet<&str> = v.iter().map(|x| x.1.as_str()).collect();
    assert_eq!(
        labels.len(),
        v.len(),
        "two views share a label (and a URL hash)"
    );
    // North Star opens the Now group; the page itself still lands on Overview
    let now: Vec<&str> = v
        .iter()
        .filter(|x| x.0 == "now")
        .map(|x| x.1.as_str())
        .collect();
    assert_eq!(now, ["North Star", "Overview", "Outages", "Briefing"]);
    let landing: Vec<&str> = PAGE
        .lines()
        .filter(|l| l.contains("<button") && l.contains("data-default"))
        .collect();
    assert_eq!(landing.len(), 1);
    assert!(landing[0].contains(">Overview</button>"));
    // every group is one of the five, and each has a view
    for g in &groups {
        assert!(v.iter().any(|x| x.0 == *g), "group {g} has no view");
    }
}

async fn serve() -> (tempfile::TempDir, String, warte::db::Database) {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db.clone()).unwrap();
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (dir, base, db)
}

async fn nav(base: &str) -> serde_json::Value {
    reqwest::get(format!("{base}/api/dashboard/nav-state"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

// rivet: verifies REQ-DASH-019
#[tokio::test]
async fn an_empty_view_is_hidden_and_shown_again_once_it_has_data() {
    let (_d, base, db) = serve().await;
    let s = nav(&base).await;
    assert_eq!(
        s["hidden"],
        serde_json::json!(["friction", "artifacts"]),
        "{s}"
    );
    for g in ["now", "ci", "machines", "agents", "compliance"] {
        assert!(
            s["badges"][g]["red"].is_u64() && s["badges"][g]["amber"].is_u64(),
            "{s}"
        );
    }
    // planted: one friction report; Friction comes back into the row
    sqlx::query("INSERT INTO agents (id, name, repo, identity_type, public_key, instance_id) VALUES ('a', 'a', 'r', 'test', x'00', 'i')")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO agent_data (id, agent_id, data_type, content, instance_id) VALUES ('f', 'a', 'friction', 'x', 'i')")
        .execute(db.pool())
        .await
        .unwrap();
    let s = nav(&base).await;
    assert_eq!(s["hidden"], serde_json::json!(["artifacts"]), "{s}");
}

// rivet: verifies REQ-DASH-019
#[tokio::test]
async fn every_view_in_the_menu_still_answers_on_its_route() {
    let (_d, base, _db) = serve().await;
    let mut checked = 0;
    for (_, label, route) in views() {
        let r = reqwest::get(format!("{base}/{route}")).await.unwrap();
        assert!(
            r.status().is_success(),
            "{label} ({route}) answered {}",
            r.status()
        );
        checked += 1;
    }
    assert_eq!(checked, 20);
}

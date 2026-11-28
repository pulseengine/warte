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
    assert_eq!(v.len(), 25, "{v:?}");
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
    assert_eq!(s["hidden"], serde_json::json!(["friction"]), "{s}");
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
    assert_eq!(s["hidden"], serde_json::json!([]), "{s}");
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
    assert_eq!(checked, 25);
}

fn finding(severity: &'static str, text: &str) -> warte::probe::Finding {
    warte::probe::Finding {
        host: "a-private-host".into(),
        severity,
        text: text.into(),
    }
}

fn red(repo: &str, kind: &str) -> warte::briefing::RedRun {
    warte::briefing::RedRun {
        repo: repo.into(),
        workflow: "CI".into(),
        kind: kind.into(),
        run: 1,
        at: 0,
    }
}

// rivet: verifies REQ-DASH-020
#[test]
fn the_strip_lists_only_always_open_items_and_names_no_host() {
    use warte::dashboard::attention;
    // nothing waiting: no item, so no strip
    assert!(attention(&[], &[]).is_empty());
    // planted: an amber disk, a red non-disk finding and a red scheduled run
    // are not items
    let quiet = attention(
        &[
            finding("amber", "disk / is 90% full"),
            finding("red", "no report for 3 h"),
        ],
        &[red("spar", "schedule")],
    );
    assert!(quiet.is_empty(), "{quiet:?}");
    // planted: two full disks and a red main put the strip up
    let a = attention(
        &[
            finding("red", "disk / is 97% full"),
            finding("red", "disk /data is 99% full"),
        ],
        &[red("rivet", "main")],
    );
    assert_eq!(a.len(), 2, "{a:?}");
    assert_eq!(a[0]["kind"], "critical-disk");
    assert_eq!(a[0]["text"], "2 disk(s) 95% full or more");
    assert_eq!(a[0]["view"], "machines");
    assert_eq!(a[1]["kind"], "red-main");
    assert_eq!(a[1]["text"], "rivet is red on main: CI");
    assert_eq!(a[1]["view"], "ci-history");
    // every item opens a view in the menu; no host is named on the public strip
    let views: BTreeSet<String> = views()
        .into_iter()
        .map(|(_, l, _)| l.to_lowercase().replace(' ', "-"))
        .collect();
    for item in &a {
        assert!(views.contains(item["view"].as_str().unwrap()), "{item}");
        assert!(!item.to_string().contains("a-private-host"), "{item}");
    }
}

fn failed_main(run: u64, repo: &str) -> warte::hub::model::JobRecord {
    warte::hub::model::JobRecord {
        d: "2026-11-21".into(),
        id: run * 10,
        run,
        att: 1,
        repo: repo.into(),
        wf: "CI".into(),
        job: "test".into(),
        pool: "rust-cpu".into(),
        runner: "r1".into(),
        ev: "push".into(),
        c: 1_795_000_000,
        s: Some(1_795_000_010),
        e: Some(1_795_000_100 + run as i64),
        r: "failure".into(),
        sig: None,
        br: Some("main".into()),
    }
}

// rivet: verifies REQ-DASH-020
#[test]
fn a_private_repository_never_reaches_the_public_strip_by_name_or_count() {
    use std::collections::BTreeMap;
    use warte::dashboard::attention;
    let public: BTreeSet<String> = ["rivet".to_string()].into();
    let defaults: BTreeMap<String, String> = [
        ("rivet".to_string(), "main".to_string()),
        ("private-one".to_string(), "main".to_string()),
    ]
    .into();
    // planted: only a private repository is red on main
    let (red, unplaced) =
        warte::briefing::red_runs(&[failed_main(1, "private-one")], &public, &defaults);
    assert!(red.is_empty(), "{red:?}");
    assert_eq!(unplaced, 0, "a private repository is not even counted");
    assert!(attention(&[], &red).is_empty());
    // beside a public red main, the strip holds the public one alone
    let (red, _) = warte::briefing::red_runs(
        &[failed_main(1, "private-one"), failed_main(2, "rivet")],
        &public,
        &defaults,
    );
    let a = attention(&[], &red);
    assert_eq!(a.len(), 1, "{a:?}");
    assert_eq!(a[0]["text"], "rivet is red on main: CI");
    assert!(!serde_json::to_string(&a).unwrap().contains("private-one"));
}

// rivet: verifies REQ-DASH-020
#[tokio::test]
async fn with_nothing_waiting_nav_state_has_no_item_and_the_page_has_the_strip_hidden() {
    let (_d, base, _db) = serve().await;
    let s = nav(&base).await;
    assert_eq!(s["attention"], serde_json::json!([]), "{s}");
    let strip: Vec<&str> = PAGE
        .lines()
        .filter(|l| l.contains(r#"id="attention""#))
        .collect();
    assert_eq!(strip.len(), 1);
    assert!(
        strip[0].contains(" hidden>"),
        "the strip starts hidden: {}",
        strip[0]
    );
    // items are written as text, never as HTML
    assert!(PAGE.contains("link.textContent = a.text;"));
}

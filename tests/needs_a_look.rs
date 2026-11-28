//! REQ-DASH-021 (warte#235): each badged view opens with a "Needs a look"
//! block whose lines are exactly what its top-bar badge counts.

use warte::probe::{enroll, send};

async fn serve() -> (tempfile::TempDir, String, warte::db::Database) {
    let dir = tempfile::tempdir().unwrap();
    let hub: warte::config::HubConfig = serde_json::from_value(serde_json::json!({
        "udp": "127.0.0.1:0", "allow": ["127.0.0.1"],
        "data_dir": dir.path().join("hub").to_str().unwrap(), "probe_self": false
    }))
    .unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: Some(hub),
        temper: Some(toml::from_str("host = \"example-host\"").unwrap()),
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

fn iso(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

fn check(id: &str, ok: bool, severity: &str) -> serde_json::Value {
    serde_json::json!({"id": id, "ok": ok, "severity": severity,
                       "code": if ok { "ok" } else { "missing" }})
}

fn report() -> serde_json::Value {
    let t = warte::hub::now();
    serde_json::json!({
        "schema": "temper-report.v1",
        "generated_at": iso(t - 60),
        "temper": {"version": "2.4.0", "commit": "a".repeat(40), "config_sha256": "b".repeat(64)},
        "org": "example-org",
        "source": {"trigger": "webhook", "last_sweep_at": iso(t - 600),
                   "api": {"calls_last_hour": 12, "remaining": 4980, "limit": 5000, "reset_at": iso(t + 600)}},
        "summary": {"repos": 2, "compliant": 0, "noncompliant": 2, "unknown": 0},
        "repos": [
            {"name": "alpha", "checked_at": iso(t - 100), "default_branch": "main",
             "checks": [check("protection.present", true, "critical"),
                        check("protection.signed_commits", false, "medium")]},
            {"name": "beta", "checked_at": iso(t - 100), "default_branch": "main",
             "checks": [check("protection.present", false, "critical"),
                        check("protection.signed_commits", false, "high")]}
        ],
        "org_checks": [],
        "reviews": {"window_days": 7, "posted": 0, "suppressed_no_findings": 0, "findings": 0,
                    "dropped_unquoted": 0, "rivet_oracle_runs": 0, "errors": 0},
        "tasks": {"pending": 0, "in_progress": 0, "failed": 0, "oldest_pending_age_s": 0, "stale_in_progress": 0}
    })
}

async fn get(url: String) -> String {
    reqwest::get(url).await.unwrap().text().await.unwrap()
}

/// The lines of the page's "Needs a look" block; None when it has none.
fn block_lines(page: &str) -> Option<usize> {
    let start = page.find(r#"id="needs-a-look""#)?;
    let end = start + page[start..].find("</section>").unwrap();
    Some(page[start..end].matches("<li ").count())
}

// rivet: verifies REQ-DASH-021
#[tokio::test]
async fn the_compliance_badge_counts_the_lines_of_its_block_worst_first() {
    let (dir, base, db) = serve().await;
    // before any report: one amber line, "no report yet", and the badge says 1
    let nav: serde_json::Value =
        serde_json::from_str(&get(format!("{base}/api/dashboard/nav-state")).await).unwrap();
    assert_eq!(
        nav["badges"]["compliance"],
        serde_json::json!({"red": 0, "amber": 1}),
        "{nav}"
    );
    let page = get(format!("{base}/api/dashboard/compliance-partial")).await;
    assert_eq!(block_lines(&page), Some(1), "{page}");

    let p = dir.path().join("p");
    let token = enroll::mint(&db, "example-host", warte::hub::now())
        .await
        .unwrap();
    enroll::enroll(&base, &token, "example-host", &p, None)
        .await
        .unwrap();
    let file = dir.path().join("report.json");
    std::fs::write(&file, serde_json::to_vec(&report()).unwrap()).unwrap();
    send::send_feed(
        &reqwest::Client::new(),
        &base,
        &p,
        "temper",
        &file,
        warte::hub::now(),
    )
    .await
    .unwrap();

    let nav: serde_json::Value =
        serde_json::from_str(&get(format!("{base}/api/dashboard/nav-state")).await).unwrap();
    assert_eq!(
        nav["badges"]["compliance"],
        serde_json::json!({"red": 3, "amber": 0}),
        "{nav}"
    );
    assert_eq!(nav["views"]["compliance"], nav["badges"]["compliance"]);
    // the badge's number is the number of lines, and nothing is listed twice
    let page = get(format!("{base}/api/dashboard/compliance-partial")).await;
    assert_eq!(block_lines(&page), Some(3), "{page}");
    // it sits at the top, right under the heading
    let h2 = page.find("</h2>").unwrap();
    assert_eq!(page[h2 + 5..].find("<section"), Some(0), "{page}");
    // critical first, then high, then medium
    let (c, h, m) = (
        page.find("beta: protection.present").unwrap(),
        page.find("beta: protection.signed_commits").unwrap(),
        page.find("alpha: protection.signed_commits").unwrap(),
    );
    assert!(c < h && h < m, "{page}");

    // views with nothing to look at carry no block and a zero badge
    assert_eq!(
        nav["views"]["machines"],
        serde_json::json!({"red": 0, "amber": 0})
    );
    let machines = get(format!("{base}/api/dashboard/machines-partial")).await;
    assert_eq!(block_lines(&machines), None);
    let outages = get(format!("{base}/api/dashboard/outages-partial")).await;
    let n = nav["views"]["outages"]["red"].as_u64().unwrap()
        + nav["views"]["outages"]["amber"].as_u64().unwrap();
    assert_eq!(block_lines(&outages).unwrap_or(0) as u64, n, "{outages}");
}

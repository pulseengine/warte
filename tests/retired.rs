//! The artifact store and multi-instance sync are gone (REQ-RETIRE-001,
//! warte#37, #38): never finished, so nothing may still serve or advertise
//! them.

async fn serve() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (dir, base)
}

// rivet: verifies REQ-RETIRE-001
#[tokio::test]
async fn the_removed_routes_answer_like_any_unknown_route() {
    let (_d, base) = serve().await;
    let c = reqwest::Client::new();
    let unknown = c
        .get(format!("{base}/api/no-such-route"))
        .send()
        .await
        .unwrap()
        .status();
    for p in [
        "/api/artifacts",
        "/api/artifacts/x/download",
        "/api/sync/state",
        "/api/dashboard/artifacts-partial",
    ] {
        let s = c.get(format!("{base}{p}")).send().await.unwrap().status();
        assert_eq!(s, unknown, "{p}");
    }
    let s = c
        .post(format!("{base}/api/sync/update"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .status();
    assert!(!s.is_success(), "{s}");
    // the overview no longer counts artifacts
    let o: serde_json::Value = c
        .get(format!("{base}/api/dashboard/overview"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        o.get("agents").is_some() && o.get("artifacts").is_none(),
        "{o}"
    );
}

// rivet: verifies REQ-RETIRE-001
#[test]
fn nothing_still_advertises_them() {
    let menu = include_str!("../templates/dashboard.html");
    assert!(!menu.contains("artifacts-partial"));
    let topics: Vec<&str> = warte::docs::list_topics().iter().map(|(n, _)| *n).collect();
    assert!(topics.contains(&"api-reference"), "{topics:?}");
    for gone in ["artifact-distribution", "multi-instance-sync"] {
        assert!(!topics.contains(&gone), "{gone}");
    }
    let ops: Vec<String> = warte::contract::current_contract()
        .operations
        .into_iter()
        .map(|o| o.name)
        .collect();
    assert!(ops.contains(&"capture_data".to_string()), "{ops:?}");
    for gone in ["download_artifact", "sync_update"] {
        assert!(!ops.iter().any(|o| o == gone), "{gone}");
    }
    assert!(!warte::auth::MUTATING_ROUTES
        .iter()
        .any(|r| r.pattern.starts_with("/api/sync")));
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_warte"))
        .args(["artifact", "list"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "an unknown subcommand");
}

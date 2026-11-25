//! Oracle for REQ-HEALTH-UPTIME-001: /api/health reports the server's uptime.

async fn serve() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let app = warte::server::build_router(warte::server::build_state(config, db).unwrap());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (dir, base)
}

async fn uptime(base: &str) -> u64 {
    let v: serde_json::Value = reqwest::get(format!("{base}/api/health"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["uptime_secs"].as_u64().expect("uptime_secs is a number")
}

// rivet: verifies REQ-HEALTH-UPTIME-001
#[tokio::test]
async fn uptime_counts_from_the_start() {
    let (_dir, base) = serve().await;
    let first = uptime(&base).await;
    assert!(first < 60, "a fresh server reports {first}s");
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    let later = uptime(&base).await;
    assert!(later >= first + 2, "{first} then {later}");
}

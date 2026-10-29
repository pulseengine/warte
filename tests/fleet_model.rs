//! Oracles for REQ-FLEET-001: the fleet model from spar, against reality.

use std::net::Ipv4Addr;

async fn serve(model: Option<std::path::PathBuf>) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let hub = dir.path().join("hub");
    std::fs::create_dir_all(&hub).unwrap();
    // one probe report: the laptop is reporting, and a machine the model does not know
    let m = warte::probe::Machines::open(hub.to_str().unwrap());
    for host in ["laptop", "stranger"] {
        let mut r: warte::probe::Report = serde_json::from_value(serde_json::json!({
            "probe_version": "t", "host": host, "taken_at": 1, "uptime_s": 1,
            "os": {"name": "L", "kernel": "6", "arch": "a"},
            "cpu": {"ncpu": 1, "load": [0.0, 0.0, 0.0]},
            "memory": {"total_mb": 1, "avail_mb": 1, "swap_total_mb": 0, "swap_used_mb": 0},
            "disks": [], "tools": [], "sessions": []
        }))
        .unwrap();
        r.host = host.into();
        m.record(r, "probe", warte::hub::now());
    }
    drop(m);
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        fleet_model: model,
        hub: Some(warte::config::HubConfig {
            udp: "127.0.0.1:0".into(),
            allow: vec![Ipv4Addr::LOCALHOST.into()],
            data_dir: hub.to_str().unwrap().into(),
            org: "pulseengine".into(),
            probe_self: false,
            costs_file: None,
        }),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let app = warte::server::build_router(warte::server::build_state(config, db).unwrap());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (dir, base)
}

fn fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/example_fleet.instance.json")
}

// rivet: partially-verifies REQ-FLEET-001
#[tokio::test]
async fn the_model_and_its_differences_are_served_and_shown() {
    let (_d, base) = serve(Some(fixture())).await;
    let v: serde_json::Value = reqwest::get(format!("{base}/api/fleet"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["model"]["root"], "Example_Fleet::ExampleFleet.impl");
    let diffs = v["differences"].as_array().unwrap();
    let says = |h: &str| {
        diffs
            .iter()
            .filter(|d| d["host"] == h)
            .map(|d| d["text"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert!(says("tower")[0].contains("not reporting"), "{v}");
    assert!(
        says("stranger")[0].contains("not in the fleet model"),
        "{v}"
    );
    assert!(
        says("laptop").is_empty(),
        "the laptop reports and is modelled"
    );

    let page = reqwest::get(format!("{base}/api/dashboard/machines-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("Fleet model vs reality"), "{page}");
    assert!(
        page.contains(r#"class="machine modelled""#) && page.contains("pixhawk-6xrt"),
        "the modelled tower and its board show as dashed: {page}"
    );
}

// rivet: partially-verifies REQ-FLEET-001
#[tokio::test]
async fn a_missing_or_unreadable_model_is_said_not_fatal() {
    let (_d, base) = serve(None).await;
    assert_eq!(
        reqwest::get(format!("{base}/api/fleet"))
            .await
            .unwrap()
            .status(),
        404
    );
    let page = reqwest::get(format!("{base}/api/dashboard/machines-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!page.contains("Fleet model"), "no model, no panel");

    let (_d2, base2) = serve(Some("/nonexistent/fleet.json".into())).await;
    let r = reqwest::get(format!("{base2}/api/fleet")).await.unwrap();
    assert_eq!(r.status(), 503);
    assert!(r.text().await.unwrap().contains("fleet model unreadable"));
    let page = reqwest::get(format!("{base2}/api/dashboard/machines-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("/nonexistent/fleet.json"), "{page}");
    // the rest of warte still works
    assert_eq!(
        reqwest::get(format!("{base2}/api/health"))
            .await
            .unwrap()
            .status(),
        200
    );
}

//! Oracles for temper's report via the probe (REQ-TEMPER-002): the
//! configured host's probe sends temper-report.v1 as a signed CCSDS
//! sequence; warte stores and shows it. Another host, a replay, a revoked
//! host, altered packets and a report outside the schema are refused.

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
        temper: Some(toml::from_str("host = \"netcup\"").unwrap()),
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

async fn enrolled(base: &str, db: &warte::db::Database, host: &str, dir: &std::path::Path) {
    let token = enroll::mint(db, host, warte::hub::now()).await.unwrap();
    enroll::enroll(base, &token, host, dir, None).await.unwrap();
}

fn iso(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

fn report(swept_ago: i64) -> serde_json::Value {
    let t = warte::hub::now();
    serde_json::json!({
        "schema": "temper-report.v1",
        "generated_at": iso(t - 60),
        "temper": {"version": "2.4.0", "commit": "a".repeat(40), "config_sha256": "b".repeat(64)},
        "org": "pulseengine",
        "source": {"trigger": "webhook", "last_sweep_at": iso(t - swept_ago),
                   "api": {"calls_last_hour": 12, "remaining": 4980, "limit": 5000, "reset_at": iso(t + 600)}},
        "summary": {"repos": 2, "compliant": 1, "noncompliant": 1, "unknown": 0},
        "repos": [
            {"name": "warte", "checked_at": iso(t - 100), "default_branch": "main",
             "checks": [{"id": "protection.present", "ok": true, "severity": "critical", "code": "ok"}]},
            {"name": "spar", "checked_at": iso(t - 100), "default_branch": "main",
             "checks": [{"id": "protection.signed_commits", "ok": false, "severity": "high", "code": "missing"}]}
        ],
        "org_checks": [],
        "reviews": {"window_days": 7, "posted": 3, "suppressed_no_findings": 5, "findings": 4,
                    "dropped_unquoted": 1, "rivet_oracle_runs": 2, "errors": 0},
        "tasks": {"pending": 0, "in_progress": 0, "failed": 1, "oldest_pending_age_s": 0, "stale_in_progress": 0}
    })
}

async fn compliance(base: &str) -> serde_json::Value {
    reqwest::get(format!("{base}/api/compliance"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// A signed feed request for `packets`, as the probe in `dir` would send it.
fn signed(dir: &std::path::Path, packets: &[u8], at: i64) -> send::SignedFeed {
    use base64::Engine;
    let key = std::fs::read_to_string(dir.join("probe-key.pem")).unwrap();
    send::SignedFeed {
        sig: hex::encode(
            send::sign_bytes(&key, &send::feed_message("temper", at, packets)).unwrap(),
        ),
        cert: std::fs::read_to_string(dir.join("probe-cert.pem")).unwrap(),
        name: "temper".into(),
        ccsds: base64::engine::general_purpose::STANDARD.encode(packets),
        at,
    }
}

async fn post(base: &str, f: &send::SignedFeed) -> u16 {
    reqwest::Client::new()
        .post(format!("{base}/api/probe/feed/temper"))
        .json(f)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

fn packets(v: &serde_json::Value) -> Vec<u8> {
    warte::hub::telemetry::segment(
        warte::hub::telemetry::APID_FEED_TEMPER,
        &serde_json::to_vec(v).unwrap(),
    )
    .unwrap()
}

// rivet: verifies REQ-TEMPER-002
#[tokio::test]
async fn the_netcup_probe_forwards_temper_report_and_warte_shows_it() {
    let (dir, base, db) = serve().await;
    let p = dir.path().join("p");
    enrolled(&base, &db, "netcup", &p).await;
    assert_eq!(compliance(&base).await["report"], serde_json::Value::Null);
    // a big report crosses many CCSDS packets
    let mut r = report(3600);
    let one = r["repos"][0].clone();
    for i in 0..300 {
        let mut x = one.clone();
        x["name"] = format!("repo-{i}").into();
        r["repos"].as_array_mut().unwrap().push(x);
    }
    let file = dir.path().join("report.json");
    std::fs::write(&file, serde_json::to_vec(&r).unwrap()).unwrap();
    assert!(std::fs::metadata(&file).unwrap().len() > 20 * 1293);
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
    let c = compliance(&base).await;
    assert_eq!(c["host"], "netcup");
    assert_eq!(c["stale"], false);
    assert_eq!(c["failures"], 1);
    assert_eq!(c["report"]["repos"].as_array().unwrap().len(), 302);
    let page = reqwest::get(format!("{base}/api/dashboard/compliance-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        page.contains("protection.signed_commits") && page.contains("spar"),
        "{page}"
    );
    assert!(
        page.contains("1 failed"),
        "temper's failed task is shown: {page}"
    );
    assert!(
        page.contains("last sweep 1.0 h ago") || page.contains("last sweep 60 min ago"),
        "{page}"
    );
}

// rivet: verifies REQ-TEMPER-002
#[tokio::test]
async fn another_host_a_replay_altered_packets_and_a_bad_report_are_refused() {
    let (dir, base, db) = serve().await;
    let (n, g) = (dir.path().join("n"), dir.path().join("g"));
    enrolled(&base, &db, "netcup", &n).await;
    enrolled(&base, &db, "gale", &g).await;
    let now = warte::hub::now();
    let pk = packets(&report(60));
    // another enrolled probe may not speak for temper
    assert_eq!(post(&base, &signed(&g, &pk, now)).await, 403);
    // the configured one may, once; the same request again is a replay
    let ok = signed(&n, &pk, now);
    assert_eq!(post(&base, &ok).await, 204);
    assert_eq!(post(&base, &ok).await, 403);
    // altered packets: the signature no longer verifies
    let mut f = signed(&n, &pk, now + 1);
    let mut bad = pk.clone();
    bad[20] ^= 1;
    use base64::Engine;
    f.ccsds = base64::engine::general_purpose::STANDARD.encode(&bad);
    assert_eq!(post(&base, &f).await, 403);
    // signed, but not a complete sequence on the feed's APID
    let wrong = warte::hub::telemetry::segment(warte::hub::telemetry::APID_HOST, b"{}").unwrap();
    assert_eq!(post(&base, &signed(&n, &wrong, now + 2)).await, 403);
    // too old
    assert_eq!(
        post(&base, &signed(&n, &pk, now - send::MAX_SKEW_S - 5)).await,
        403
    );
    // a report outside the schema: rejected, nothing stored, input not quoted
    let mut evil = report(60);
    evil["repos"][0]["name"] = "<script>x</script>".into();
    let r = reqwest::Client::new()
        .post(format!("{base}/api/probe/feed/temper"))
        .json(&signed(&n, &packets(&evil), now + 3))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert!(!r.text().await.unwrap().contains("script"));
    let c = compliance(&base).await;
    assert!(!c.to_string().contains("script"));
    assert_eq!(
        c["report"]["repos"][0]["name"], "warte",
        "the earlier report stays"
    );
    // revoked: refused
    enroll::revoke(&db, "netcup", warte::hub::now())
        .await
        .unwrap();
    assert_eq!(post(&base, &signed(&n, &pk, now + 4)).await, 403);
}

// rivet: verifies REQ-TEMPER-002
#[tokio::test]
async fn an_old_sweep_shows_as_stale_and_unknown_feeds_are_refused_by_the_cli() {
    let (dir, base, db) = serve().await;
    let p = dir.path().join("p");
    enrolled(&base, &db, "netcup", &p).await;
    assert_eq!(
        post(
            &base,
            &signed(&p, &packets(&report(27 * 3600)), warte::hub::now())
        )
        .await,
        204
    );
    assert_eq!(compliance(&base).await["stale"], true);
    let page = reqwest::get(format!("{base}/api/dashboard/compliance-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("STALE"), "{page}");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_warte"))
        .args([
            "probe",
            "send",
            "--ground",
            "http://x",
            "--feed",
            "secrets=/etc/shadow",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown feed"));
}

// rivet: verifies REQ-DOCS-010
// rivet: verifies REQ-TEMPER-002
#[tokio::test]
async fn the_middleware_exempts_exactly_the_documented_probe_paths() {
    let (_d, base, _db) = serve().await;
    let c = reqwest::Client::new();
    for p in warte::auth::PROBE_OPEN_PATHS {
        let s = c
            .post(format!("{base}{p}"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16();
        assert_ne!(
            s, 401,
            "{p} is listed as open but the middleware asks for a token"
        );
    }
    for p in ["/api/probe/feed/other", "/api/probe/tokens", "/api/costs"] {
        let s = c
            .post(format!("{base}{p}"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(s.status().as_u16(), 401, "{p} must need a token");
    }
}

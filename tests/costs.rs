//! Costs: ingest, storage and view of the notional-cost report (REQ-COSTS-001).
//! Synthetic reports only: no real repositories, hosts or paths.

use std::net::Ipv4Addr;

fn report() -> serde_json::Value {
    let tokens = serde_json::json!({"input": 10, "cache_write_5m": 1, "cache_write_1h": 2,
                                    "cache_read": 1000, "output": 50, "requests": 3});
    let window = serde_json::json!({
        "usd": 12.5, "usd_main": 10.0, "usd_subagents": 2.0, "usd_advisor": 0.5, "sessions": 1,
        "by_repo": [{"repo": "alpha", "usd": 12.5}],
        "by_model": [{"model": "model-big", "usd": 12.0, "tokens": tokens},
                     {"model": "model-small", "usd": 0.5, "tokens": tokens}],
        "by_category": [{"category": "review", "usd": 2.0, "subagents": 1}],
        "cheaper_tier": {"calls_down": 1, "calls_same": 1, "calls_up": 0, "calls_unknown": 0,
                         "call_share": 0.5, "usd_subagents_down": 0.5, "usd_subagents": 2.0, "usd_share": 0.25},
        "unpriced_requests": 0
    });
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let month = chrono::Utc::now().format("%Y-%m").to_string();
    serde_json::json!({
        "schema": 1, "kind": "claude-code-costs", "notional": true,
        "label": "notional list price, not billed",
        "generated_at": "2026-10-29T04:30:00Z",
        "prices": {"version": "2026-10-29", "source": "https://example.invalid/pricing.md",
                   "per_mtok": {"model-big": [4, 5, 8, 0.2, 20]}, "long_prompt": 100000,
                   "per_mtok_long": {}, "per_mtok_fast": {}},
        "classifier": {"version": 2, "categories": ["review", "implement", "other"]},
        "counts": {"sessions": 1, "subagents": 1},
        "unpriced_models": {},
        "windows": {"all": window.clone(), "last_30d": window.clone(), month: window},
        "daily": [{"day": today, "usd": 12.5, "usd_main": 10.0, "usd_subagents": 2.0,
                   "usd_advisor": 0.5, "usd_subagents_down": 0.5}],
        "sessions": [{"id": "s0123456789ab", "date": today, "machine": "host-a", "repo": "alpha",
                      "model": "model-big", "tokens": tokens, "usd": 12.5, "usd_main": 10.0,
                      "usd_subagents": 2.0, "usd_advisor": 0.5, "n_subagents": 1}],
        "subagents": [{"id": "a0123456789ab", "session": "s0123456789ab", "date": today,
                       "agent_type": "general-purpose", "model": "model-small", "parent_model": "model-big",
                       "route": "down", "category": "review", "linked": true, "tokens": tokens, "usd": 2.0}]
    })
}

struct Server {
    dir: tempfile::TempDir,
    base: String,
    /// A bearer token issued by this server's token manager (REQ-API-002).
    token: String,
}

async fn serve() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let hub_dir = dir.path().join("hub");
    std::fs::create_dir_all(&hub_dir).unwrap();
    let costs_file = dir.path().join("pushed").join("costs.json");
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: Some(warte::config::HubConfig {
            udp: "127.0.0.1:0".into(),
            allow: vec![Ipv4Addr::LOCALHOST.into()],
            data_dir: hub_dir.to_str().unwrap().into(),
            org: "pulseengine".into(),
            probe_self: false,
            costs_file: Some(costs_file.to_str().unwrap().into()),
        }),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let token = state.tokens.issue_token("costs-job", vec![]).unwrap();
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    Server { dir, base, token }
}

impl Server {
    fn pushed(&self) -> std::path::PathBuf {
        self.dir.path().join("pushed").join("costs.json")
    }

    /// What the workstation job does: copy the whole report into costs_file.
    fn push_file(&self, body: &serde_json::Value, mtime_s: i64) {
        let p = self.pushed();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body.to_string()).unwrap();
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime_s as u64);
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }
}

async fn post(s: &Server, body: &serde_json::Value) -> (u16, String) {
    let r = reqwest::Client::new()
        .post(format!("{}/api/costs", s.base))
        .bearer_auth(&s.token)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    (r.status().as_u16(), r.text().await.unwrap())
}

async fn get_json(base: &str, p: &str) -> serde_json::Value {
    reqwest::get(format!("{base}{p}"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn page(base: &str) -> String {
    reqwest::get(format!("{base}/api/dashboard/costs-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

// rivet: verifies REQ-COSTS-001
#[tokio::test]
async fn a_report_is_stored_served_and_shown_as_notional() {
    let srv = serve().await;
    let base = srv.base.clone();
    // before the first push: 200 with report null, and the view says so
    let v = get_json(&base, "/api/costs").await;
    assert!(v["report"].is_null(), "{v}");
    assert!(page(&base).await.contains("No cost report received yet"));

    let (code, body) = post(&srv, &report()).await;
    assert_eq!(code, 201, "{body}");
    let v = get_json(&base, "/api/costs").await;
    assert_eq!(v["report"]["sessions"][0]["repo"], "alpha");
    assert_eq!(v["label"], "notional list price, not billed");
    assert_eq!(v["stale"], false);

    let html = page(&base).await;
    assert!(html.contains("notional list price, not billed"), "{html}");
    assert!(html.contains("none of this is billed"));
    for want in [
        "$12.5",
        "alpha",
        "model-big",
        "review",
        "50%",
        "host-a",
        "Top sessions",
    ] {
        assert!(html.contains(want), "{want} missing: {html}");
    }

    // a push replaces the report whole
    let mut r2 = report();
    r2["sessions"][0]["repo"] = "beta".into();
    assert_eq!(post(&srv, &r2).await.0, 201);
    assert_eq!(
        get_json(&base, "/api/costs").await["report"]["sessions"][0]["repo"],
        "beta"
    );

    // and survives a restart of the ground station
    let again = warte::costs::Costs::open(srv.dir.path().join("hub").to_str().unwrap(), None);
    assert_eq!(again.latest().unwrap().report.sessions[0].repo, "beta");
}

// rivet: verifies REQ-COSTS-001
#[tokio::test]
async fn free_text_and_unknown_fields_are_rejected_and_not_stored() {
    let srv = serve().await;
    let base = srv.base.clone();
    let mut cases: Vec<(&str, serde_json::Value)> = Vec::new();
    // an Agent description must never reach the public read API (warte#46)
    let mut r = report();
    r["subagents"][0]["description"] = "review the parser for the customer".into();
    cases.push(("description field", r));
    let mut r = report();
    r["sessions"][0]["prompt"] = "secret".into();
    cases.push(("prompt field", r));
    let mut r = report();
    r["subagents"][0]["category"] = "fix the login bug in acme".into();
    cases.push(("text in a name", r));
    let mut r = report();
    r["sessions"][0]["repo"] = "/home/someone/git/alpha".into();
    cases.push(("a path as a repo", r));
    let mut r = report();
    r["sessions"][0]["usd"] = (-1.0).into();
    cases.push(("negative amount", r));
    let mut r = report();
    r["label"] = "billed".into();
    cases.push(("not labelled notional", r));
    let mut r = report();
    r["schema"] = 2.into();
    cases.push(("another schema", r));
    let mut r = report();
    r["windows"]["<script>"] = r["windows"]["all"].clone();
    cases.push(("a window name", r));
    for (what, body) in cases {
        let (code, msg) = post(&srv, &body).await;
        assert_eq!(code, 400, "{what} accepted: {msg}");
    }
    assert!(
        get_json(&base, "/api/costs").await["report"].is_null(),
        "a rejected report was stored"
    );
    let (code, _) = post(&srv, &serde_json::json!({"not": "a report"})).await;
    assert_eq!(code, 400);
}

// rivet: verifies REQ-COSTS-001
#[tokio::test]
async fn a_report_larger_than_the_default_body_limit_is_accepted() {
    let srv = serve().await;
    let mut r = report();
    let one = r["subagents"][0].clone();
    let many: Vec<serde_json::Value> = (0..12000)
        .map(|i| {
            let mut a = one.clone();
            a["id"] = format!("a{i:012}").into();
            a
        })
        .collect();
    r["subagents"] = many.into();
    let body = r.to_string();
    assert!(body.len() > 2 * 1024 * 1024, "{} bytes", body.len());
    let (code, msg) = post(&srv, &r).await;
    assert_eq!(code, 201, "{msg}");
}

// rivet: verifies REQ-COSTS-001
#[test]
fn the_view_escapes_names_and_flags_a_stale_report() {
    let dir = tempfile::tempdir().unwrap();
    let c = warte::costs::Costs::open(dir.path().to_str().unwrap(), None);
    // stored directly, past validation: the view must still escape
    let mut r: warte::costs::CostReport = serde_json::from_value(report()).unwrap();
    r.sessions[0].repo = "<img src=x onerror=alert(1)>".into();
    r.windows.get_mut("last_30d").unwrap().by_repo[0].repo = "<b>x</b>".into();
    let received = warte::hub::now() - 3 * 24 * 3600;
    c.record(r, received).unwrap();
    let html = warte::dashboard::costs::view(Some(&c), warte::hub::now());
    assert!(
        !html.contains("<img src=x") && !html.contains("<b>x</b>"),
        "{html}"
    );
    assert!(html.contains("&lt;img src=x"));
    assert!(html.contains("pill stale"), "a 3-day-old report is stale");
    assert!(warte::dashboard::costs::view(None, 0).contains("No cost report received yet"));
}

// rivet: verifies REQ-COSTS-001
#[tokio::test]
async fn a_post_without_a_token_is_refused() {
    let srv = serve().await;
    let r = reqwest::Client::new()
        .post(format!("{}/api/costs", srv.base))
        .header("Content-Type", "application/json")
        .body(report().to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        401,
        "a write needs a bearer token (REQ-API-002)"
    );
    assert!(get_json(&srv.base, "/api/costs").await["report"].is_null());
}

// rivet: verifies REQ-COSTS-001
#[tokio::test]
async fn the_pushed_file_is_read_when_it_changes_and_checked_like_a_post() {
    let srv = serve().await;
    let base = srv.base.clone();
    let now = warte::hub::now();
    srv.push_file(&report(), now - 60);
    let v = get_json(&base, "/api/costs").await;
    assert_eq!(v["report"]["sessions"][0]["repo"], "alpha", "{v}");
    assert_eq!(
        v["received_at"],
        now - 60,
        "received_at is the file's mtime"
    );

    // a newer file replaces it
    let mut r2 = report();
    r2["sessions"][0]["repo"] = "beta".into();
    srv.push_file(&r2, now - 30);
    assert_eq!(
        get_json(&base, "/api/costs").await["report"]["sessions"][0]["repo"],
        "beta"
    );

    // a file with a task description is rejected: nothing shown, the view says why
    let mut bad = report();
    bad["subagents"][0]["description"] = "SECRET-TASK review the parser".into();
    srv.push_file(&bad, now - 10);
    let v = get_json(&base, "/api/costs").await;
    assert!(v["report"].is_null(), "{v}");
    let why = v["rejected"].as_str().unwrap();
    assert!(
        why.contains("not a costs report")
            && !why.contains("SECRET")
            && !why.contains("description"),
        "{why}"
    );
    let html = page(&base).await;
    assert!(
        html.contains("rejected") && !html.contains("SECRET-TASK"),
        "{html}"
    );
    // free text in a name: rejected, and the reason does not quote it
    let mut bad = report();
    bad["sessions"][0]["repo"] = "SECRET-TASK fix the login".into();
    srv.push_file(&bad, now - 5);
    let v = get_json(&base, "/api/costs").await;
    assert!(
        v["report"].is_null() && !v.to_string().contains("SECRET"),
        "{v}"
    );
    assert!(!page(&base).await.contains("SECRET"));

    // a valid file again: shown again
    srv.push_file(&report(), now);
    let v = get_json(&base, "/api/costs").await;
    assert_eq!(v["report"]["sessions"][0]["repo"], "alpha");
    assert!(v.get("rejected").is_none() || v["rejected"].is_null());
}

/// A `costs_file` that exists but cannot be read is a rejection naming the
/// error kind, never the "nothing received yet" empty state (#80).
// rivet: verifies REQ-COSTS-001
#[cfg(unix)]
#[test]
fn an_unreadable_costs_file_is_rejected_not_empty() {
    use std::os::unix::fs::PermissionsExt;
    let set_mode = |p: &std::path::Path, m: u32| {
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap()
    };
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();

    // The file itself is mode 0000: it can be stat'ed but not read.
    let file = src.path().join("costs.json");
    std::fs::write(&file, report().to_string()).unwrap();
    set_mode(&file, 0o000);
    if std::fs::read(&file).is_ok() {
        set_mode(&file, 0o600);
        return; // running as root: permission bits are not enforced
    }
    let c = warte::costs::Costs::open(data.path().to_str().unwrap(), file.to_str());
    let why = c.rejected().expect("an unreadable file must be reported");
    assert!(why.contains("permission denied"), "{why}");
    assert!(
        !why.contains(src.path().to_str().unwrap()),
        "names no path: {why}"
    );
    assert!(c.latest().is_none());
    set_mode(&file, 0o600);

    // Its directory is mode 0000: even metadata fails, as when a unit
    // sandbox hides the path.
    let hidden = src.path().join("hidden");
    std::fs::create_dir(&hidden).unwrap();
    let inner = hidden.join("costs.json");
    std::fs::write(&inner, report().to_string()).unwrap();
    set_mode(&hidden, 0o000);
    let c = warte::costs::Costs::open(data.path().to_str().unwrap(), inner.to_str());
    let why = c.rejected();
    set_mode(&hidden, 0o700);
    let why = why.expect("a denied metadata read must be reported");
    assert!(why.contains("permission denied"), "{why}");
    assert!(
        !why.contains(src.path().to_str().unwrap()),
        "names no path: {why}"
    );

    // A file that does not exist yet stays the quiet empty state.
    let missing = src.path().join("not-yet.json");
    let c = warte::costs::Costs::open(data.path().to_str().unwrap(), missing.to_str());
    assert_eq!(c.rejected(), None);
    assert!(c.latest().is_none());
}

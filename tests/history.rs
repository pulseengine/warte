//! Oracles for machine history (REQ-MHIST-001..003).

use warte::history::{project, History};

const H: i64 = 3600;
const D: i64 = 86400;

fn open() -> (tempfile::TempDir, History) {
    let t = tempfile::tempdir().unwrap();
    let h = History::open(t.path().to_str().unwrap()).unwrap();
    (t, h)
}

fn s(name: &str, v: f64) -> Vec<(String, f64)> {
    vec![(name.to_string(), v)]
}

// rivet: verifies REQ-MHIST-001
#[test]
fn points_read_back_with_gaps_as_null() {
    let (_t, h) = open();
    h.record("box", 0, &s("load.1", 1.0));
    h.record("box", 180, &s("load.1", 2.0));
    h.sync();
    let r = h.series("box", &["load.1".into()], 0, 240);
    assert_eq!(r.len(), 1);
    assert_eq!((r[0].t0, r[0].step), (0, 60));
    assert_eq!(r[0].values, vec![Some(1.0), None, None, Some(2.0), None]);
    let j = serde_json::to_string(&r[0]).unwrap();
    assert!(j.contains("[1.0,null,null,2.0,null]"), "{j}");
}

// rivet: verifies REQ-MHIST-001
#[test]
fn same_minute_replaces() {
    let (_t, h) = open();
    h.record("box", 10, &s("load.1", 1.0));
    h.record("box", 50, &s("load.1", 2.0));
    h.sync();
    let r = h.series("box", &["load.1".into()], 0, 59);
    assert_eq!(r[0].values, vec![Some(2.0)]);
    // another source's series in the same minute is kept, not wiped
    h.record("box", 30, &s("runners.busy", 3.0));
    h.sync();
    let r = h.series("box", &["load.1".into(), "runners.busy".into()], 0, 59);
    assert_eq!((r[0].values[0], r[1].values[0]), (Some(2.0), Some(3.0)));
}

// rivet: verifies REQ-MHIST-001
#[test]
fn long_ranges_use_hourly_rollups() {
    let (_t, h) = open();
    for m in 0..120 {
        h.record("box", m * 60, &s("mem.used_pct", m as f64));
    }
    h.maintain(3 * H);
    h.sync();
    let r = h.series("box", &["mem.used_pct".into()], 0, 8 * D);
    assert_eq!((r[0].t0, r[0].step), (0, 3600));
    assert_eq!(r[0].values[0], Some(29.5));
    assert_eq!(r[0].min.as_ref().unwrap()[0], Some(0.0));
    assert_eq!(r[0].max.as_ref().unwrap()[0], Some(59.0));
    assert_eq!(r[0].values[1], Some(89.5));
    assert_eq!(r[0].values[2], None);
    assert_eq!(r[0].unit, "%");
}

// rivet: verifies REQ-MHIST-001
#[test]
fn pruning_drops_old_minutes_keeps_hours() {
    let (_t, h) = open();
    for m in 0..60 {
        h.record("box", m * 60, &s("load.1", 1.0));
    }
    h.maintain(8 * D);
    h.sync();
    let minutes = h.series("box", &["load.1".into()], 0, H);
    assert!(minutes[0].values.iter().all(|v| v.is_none()));
    h.sync();
    let hours = h.series("box", &["load.1".into()], 0, 8 * D);
    assert_eq!(hours[0].values[0], Some(1.0));
    // and hours go after the retention
    h.maintain(91 * D);
    h.sync();
    let hours = h.series("box", &["load.1".into()], 0, 8 * D);
    assert!(hours[0].values.iter().all(|v| v.is_none()));
}

// rivet: verifies REQ-MHIST-001
#[test]
fn a_late_minute_is_rolled_into_its_hour_again() {
    let (_t, h) = open();
    h.record("box", 0, &s("load.1", 1.0));
    // hour 0 is rolled at 01:00:30; a report taken at 00:01 (within the
    // 5-minute clock allowance) arrives after that
    h.maintain(H + 30);
    h.record("box", 60, &s("load.1", 3.0));
    h.maintain(H + 90);
    h.sync();
    let r = h.series("box", &["load.1".into()], 0, 8 * D);
    assert_eq!(r[0].values[0], Some(2.0));
}

// rivet: verifies REQ-MHIST-001
#[test]
fn flushed_data_survives_reopen() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().to_str().unwrap();
    {
        let h = History::open(dir).unwrap();
        h.record("box", 60, &s("load.1", 4.0));
        h.flush();
    }
    let h = History::open(dir).unwrap();
    assert_eq!(
        h.series("box", &["load.1".into()], 60, 60)[0].values,
        vec![Some(4.0)]
    );
}

// rivet: verifies REQ-MHIST-001
#[test]
fn hosts_and_metrics_list_what_was_recorded() {
    let (_t, h) = open();
    h.record(
        "a",
        0,
        &[
            ("load.1".into(), 1.0),
            ("disk.used_pct{mount=/}".into(), 50.0),
        ],
    );
    h.record("b", 0, &s("sessions", 2.0));
    h.sync();
    assert_eq!(h.hosts(), vec!["a".to_string(), "b".to_string()]);
    assert_eq!(
        h.metrics("a"),
        vec!["disk.used_pct{mount=/}".to_string(), "load.1".to_string()]
    );
    h.sync();
    assert!(h.metrics("nobody").is_empty());
    assert_eq!(h.errors(), 0);
}

fn report(host: &str) -> warte::probe::Report {
    serde_json::from_value(serde_json::json!({
        "probe_version": "t", "host": host, "taken_at": 1_800_000_000,
        "os": {"name": "Linux", "kernel": "6", "arch": "x86_64"},
        "uptime_s": 100, "cpu": {"ncpu": 4, "load": [1.0, 2.0, 3.0]},
        "memory": {"total_mb": 1000, "avail_mb": 250, "swap_total_mb": 100, "swap_used_mb": 10},
        "disks": [{"mount": "/", "fs": "ext4", "size_gb": 10.0, "used_gb": 5.0, "used_pct": 50.0, "removable": false}],
        "tools": [{"name": "rivet", "version": "1", "path": "/Users/secret/.cargo/bin/rivet"}],
        "sessions": [{"agent": "claude", "pid": 4242, "started_at": 1, "dir": "/Users/secret/project", "activity": "SECRET-ERROR"}],
        "inventory": {"plugins": [], "scheduled": [], "errors": ["SECRET-ERROR /Users/secret"]}
    }))
    .unwrap()
}

// rivet: verifies REQ-MHIST-001
#[test]
fn projection_is_numbers_only() {
    let p = project::from_report(&report("box"));
    let all = format!("{p:?}");
    assert!(!all.contains("secret") && !all.contains("SECRET"), "{all}");
    let get = |n: &str| p.iter().find(|(k, _)| k == n).map(|(_, v)| *v);
    assert_eq!(get("load.1"), Some(1.0));
    assert_eq!(get("load.per_core"), Some(0.5));
    assert_eq!(get("mem.used_pct"), Some(75.0));
    assert_eq!(get("swap.used_mb"), Some(10.0));
    assert_eq!(get("disk.used_pct{mount=/}"), Some(50.0));
    assert_eq!(get("sessions"), Some(1.0));
    // no IO reported: no io series at all, never a flat zero
    assert!(!p
        .iter()
        .any(|(k, _)| k.starts_with("io.") || k.starts_with("psi.io")));
}

// rivet: verifies REQ-MHIST-001
#[test]
fn bad_clock_uses_receive_time() {
    let now = 1_800_000_030;
    assert_eq!(project::minute(now - 20, now), (now - 20) - (now - 20) % 60);
    assert_eq!(project::minute(now + 3600, now), now - now % 60);
    assert_eq!(project::minute(now - 3600, now), now - now % 60);
}

// rivet: verifies REQ-MHIST-001
#[test]
fn reports_through_machines_land_in_history() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().to_str().unwrap();
    let m = warte::probe::Machines::open(dir);
    let h = History::open(dir).unwrap();
    m.attach_history(h.clone());
    let r = report("box");
    let at = r.taken_at;
    m.record(r, "probe", at + 5);
    h.sync();
    let got = h.series("box", &["load.1".into()], at - 60, at + 60);
    assert!(got[0].values.contains(&Some(1.0)), "{:?}", got[0].values);
}

/// A warte with machines and history in a temp dir; returns (dir, base url, store).
async fn serve_with_history() -> (tempfile::TempDir, String, History) {
    let dir = tempfile::tempdir().unwrap();
    let hub_dir = dir.path().join("hub");
    std::fs::create_dir_all(&hub_dir).unwrap();
    let cfg = warte::config::HubConfig {
        udp: "127.0.0.1:0".into(),
        allow: vec![std::net::Ipv4Addr::LOCALHOST.into()],
        data_dir: hub_dir.to_str().unwrap().into(),
        org: "pulseengine".into(),
        probe_self: false,
        costs_file: None,
        skill_usage_file: None,
    };
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: Some(cfg),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let h = state.machines.as_ref().unwrap().history().unwrap().clone();
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (dir, base, h)
}

async fn get(url: String) -> (u16, String) {
    let r = reqwest::get(url).await.unwrap();
    (r.status().as_u16(), r.text().await.unwrap())
}

// rivet: verifies REQ-MHIST-002
#[tokio::test]
async fn api_series_shape_and_discovery() {
    let (_d, base, h) = serve_with_history().await;
    // recent times: the store's start-up upkeep prunes minutes older than 7 days
    let now = chrono::Utc::now().timestamp();
    let t = now - now % 60 - 600;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    h.record("box", t, &s("mem.used_pct", 40.0));
    h.record("box", t + 180, &s("mem.used_pct", 41.0));
    h.sync();
    let (c, b) = get(format!("{base}/api/history/machines")).await;
    assert_eq!(c, 200);
    assert!(b.contains(r#""box""#), "{b}");
    let (c, b) = get(format!("{base}/api/history/machines/box/metrics")).await;
    assert_eq!(c, 200);
    assert!(b.contains(r#"{"metric":"mem.used_pct","unit":"%"}"#), "{b}");
    let (c, b) = get(format!(
        "{base}/api/history/machines/box/series?metric=mem.used_pct&from={t}&to={}",
        t + 240
    ))
    .await;
    assert_eq!(c, 200, "{b}");
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    let s0 = &v["series"][0];
    assert_eq!(
        (s0["t0"].as_i64(), s0["step"].as_i64()),
        (Some(t), Some(60))
    );
    assert_eq!(
        s0["values"],
        serde_json::json!([40.0, null, null, 41.0, null])
    );
    assert_eq!(s0["unit"], "%");
}

// rivet: verifies REQ-MHIST-002
#[tokio::test]
async fn api_rejects_bad_requests() {
    let (_d, base, h) = serve_with_history().await;
    h.record("box", 600, &s("load.1", 1.0));
    h.sync();
    let u = |q: &str| format!("{base}/api/history/machines/box/series?{q}");
    assert_eq!(get(u("metric=load.1&from=900&to=600")).await.0, 400);
    assert_eq!(
        get(u(&format!("metric=load.1&from=0&to={}", 401 * 86400)))
            .await
            .0,
        400
    );
    assert_eq!(get(u("from=0&to=60")).await.0, 400);
    // times that would overflow are refused before any arithmetic
    assert_eq!(get(u("metric=load.1&to=-9223372036854775808")).await.0, 400);
    assert_eq!(
        get(u("metric=load.1&from=-9223372036854775808&to=0"))
            .await
            .0,
        400
    );
    assert_eq!(get(u("metric=load.1&to=9223372036854775807")).await.0, 400);
    assert_eq!(
        get(format!(
            "{base}/api/history/machines/nobody/series?metric=load.1"
        ))
        .await
        .0,
        404
    );
    assert_eq!(
        get(format!("{base}/api/history/machines/..%2F..%2Fetc/metrics"))
            .await
            .0,
        404
    );
}

// rivet: verifies REQ-MHIST-003
#[test]
fn a_gap_breaks_the_line() {
    use warte::dashboard::charts::{bucket, path};
    let v = [Some(1.0), Some(2.0), None, None, Some(1.0), Some(1.0)];
    let d = path(&bucket(&v, 240), 2.0, 100.0, 10.0);
    assert_eq!(d.matches('M').count(), 2, "{d}");
    // a bucket with no data at all stays a gap after bucketing
    let b = bucket(&[Some(1.0), None, None, None, Some(3.0), None], 3);
    assert_eq!(b, vec![(0, Some(1.0)), (2, None), (4, Some(3.0))]);
}

fn machine_view(with_history: bool) -> (tempfile::TempDir, String) {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().to_str().unwrap();
    let m = warte::probe::Machines::open(dir);
    if with_history {
        m.attach_history(History::open(dir).unwrap());
    }
    let now = chrono::Utc::now().timestamp();
    let mut r = report("box");
    r.taken_at = now - 120;
    m.record(r.clone(), "probe", now - 120);
    r.taken_at = now;
    m.record(r, "probe", now);
    if let Some(h) = m.history() {
        h.sync();
    }
    let html = warte::dashboard::machines::view_range(Some(&m), None, Some("7d"));
    (t, html)
}

// rivet: verifies REQ-MHIST-003
#[test]
fn machine_cards_show_trends_and_a_range_switch() {
    let (_t, html) = machine_view(true);
    assert!(html.contains(r#"class="sparks""#), "{html}");
    assert!(html.contains("load/core") && html.contains("memory") && html.contains("disk /"));
    assert!(html.contains(r#"class="rbtn on" hx-get="api/dashboard/machines-partial?range=7d""#));
    assert!(html.contains("machine-history-partial?host=box&amp;range=7d"));
    assert!(!html.contains("secret") && !html.contains("SECRET"));
}

// rivet: verifies REQ-MHIST-003
#[test]
fn without_a_store_there_is_no_history_panel() {
    let (_t, html) = machine_view(false);
    assert!(!html.contains("sparks") && !html.contains("rbtn") && !html.contains("mhist"));
}

// rivet: verifies REQ-MHIST-003
#[test]
fn the_history_panel_groups_charts() {
    let (_t, h) = open();
    let now = chrono::Utc::now().timestamp();
    let r = report("box");
    h.record("box", now - 60, &project::from_report(&r));
    h.sync();
    let html = warte::dashboard::machines::history_panel(&h, "box", Some("1h"));
    for g in [
        "<h4>CPU</h4>",
        "<h4>Memory</h4>",
        "<h4>Disks</h4>",
        "<h4>Agents</h4>",
    ] {
        assert!(html.contains(g), "{g} missing: {html}");
    }
    assert!(!html.contains("<h4>IO</h4>"), "no IO series, no IO group");
    assert!(html.contains("<title>"), "hover titles");
    assert!(warte::dashboard::machines::history_panel(&h, "nobody", None).contains("no history"));
}

// rivet: verifies REQ-MHIST-001
#[test]
fn extreme_ranges_do_not_panic() {
    let (_t, h) = open();
    h.record("box", 0, &s("load.1", 1.0));
    h.sync();
    let r = h.series("box", &["load.1".into()], i64::MIN / 2, i64::MAX / 2);
    assert!(r.is_empty() || r[0].values.len() <= 1_000_000);
    assert!(h
        .series("box", &["load.1".into()], 10, 0)
        .iter()
        .all(|x| x.values.is_empty()));
}

// rivet: verifies REQ-MHIST-001
#[test]
fn minutes_written_into_rolled_hours_are_rolled_up_later() {
    // the ground station's import arrives after the store has rolled up to now
    let (_t, h) = open();
    h.maintain(10 * H);
    for m in 0..60 {
        h.record_waiting("ci", 2 * H + m * 60, &s("runners.busy", 2.0));
    }
    h.maintain(10 * H + 60);
    let r = h.series("ci", &["runners.busy".into()], 0, 8 * D);
    assert_eq!(r[0].values[2], Some(2.0), "{:?}", &r[0].values[..4]);
}

// rivet: verifies REQ-MHIST-001
#[test]
fn a_new_series_id_survives_reopen_and_is_not_reused() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().to_str().unwrap();
    {
        let h = History::open(dir).unwrap();
        h.record("a", 60, &[("x".into(), 1.0), ("y".into(), 2.0)]);
        h.flush();
    }
    let h = History::open(dir).unwrap();
    h.record("a", 120, &s("z", 3.0));
    h.sync();
    let r = h.series("a", &["x".into(), "y".into(), "z".into()], 60, 120);
    assert_eq!(r[0].values, vec![Some(1.0), None]);
    assert_eq!(r[1].values, vec![Some(2.0), None]);
    assert_eq!(r[2].values, vec![None, Some(3.0)]);
}

// rivet: verifies REQ-MHIST-003
#[test]
fn a_twenty_minute_outage_shows_on_a_day_long_sparkline() {
    use warte::dashboard::charts::{bucket, path};
    let mut v = vec![Some(1.0); 1440];
    for x in &mut v[600..620] {
        *x = None;
    }
    // one late report alone is no gap
    v[100] = None;
    let d = path(&bucket(&v, 60), 2.0, 120.0, 24.0);
    assert_eq!(d.matches('M').count(), 2, "{d}");
}

// rivet: verifies REQ-MHIST-003
#[test]
fn similar_host_names_get_different_chart_anchors() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().to_str().unwrap();
    let m = warte::probe::Machines::open(dir);
    m.attach_history(History::open(dir).unwrap());
    let now = chrono::Utc::now().timestamp();
    for host in ["web.1", "web-1"] {
        let mut r = report(host);
        r.taken_at = now;
        m.record(r, "probe", now);
    }
    m.history().unwrap().sync();
    let html = warte::dashboard::machines::view_range(Some(&m), None, None);
    let ids: Vec<&str> = html
        .match_indices(r#"id="hist-"#)
        .map(|(i, _)| &html[i..i + 24])
        .collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_ne!(ids[0], ids[1]);
}

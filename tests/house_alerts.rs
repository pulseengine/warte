//! A house controller's warnings as alerts (REQ-ALERT-005).

use warte::alerts::house::{detected, HouseConfig, Reader, Warning, STALE_AFTER_S};
use warte::alerts::{Event, Tracker};

const H: i64 = 3_600_000;

fn w(id: &str, kind: &str, sev: &str, since_ms: Option<i64>) -> Warning {
    serde_json::from_value(serde_json::json!({
        "id": id, "kind": kind, "room": "Bad", "series": [7, 3],
        "message": "Feuchte 81% seit 3 h", "severity": sev, "since_ms": since_ms,
        "later_field": 1
    }))
    .unwrap()
}

// rivet: verifies REQ-ALERT-005
#[test]
fn alarms_become_red_findings_keyed_by_id_and_checks_stay_quiet() {
    let now = 100 * H;
    let ws = [
        w("00112233aabbccdd", "mould_risk", "alarm", Some(now - H)),
        w("0011223344556677", "too_cold", "check", Some(now - H)),
        w("8899aabbccddeeff", "frost", "urgent", None),
        w("not-an-id", "frost", "alarm", None),
        w("ffeeddccbbaa9988", "source_down", "alarm", None),
    ];
    let d = detected(&ws, now, 360);
    let keys: Vec<&str> = d.iter().map(|d| d.key.as_str()).collect();
    assert_eq!(keys, ["house:00112233aabbccdd", "house:ffeeddccbbaa9988"]);
    assert!(d.iter().all(|d| d.sev == 2));
    assert_eq!(d[0].text, "House, Bad: Feuchte 81% seit 3 h");
}

// rivet: verifies REQ-ALERT-005
#[test]
fn a_silent_sensor_waits_for_its_hold() {
    let now = 100 * H;
    let at = |age: i64| {
        [w(
            "0123456789abcdef",
            "sensor_silent",
            "alarm",
            Some(now - age),
        )]
    };
    assert!(detected(&at(5 * H), now, 360).is_empty());
    assert_eq!(detected(&at(6 * H), now, 360).len(), 1);
    assert_eq!(detected(&at(2 * H), now, 60).len(), 1);
    // no start time: never old enough
    let none = [w("0123456789abcdef", "sensor_silent", "alarm", None)];
    assert!(detected(&none, now, 360).is_empty());
}

// rivet: verifies REQ-ALERT-005
#[test]
fn an_alarm_opens_once_and_clears_when_its_id_is_gone() {
    let now = 100 * H;
    let ws = [w("00112233aabbccdd", "frost", "alarm", None)];
    let mut t = Tracker::default();
    let mut events = vec![];
    for _ in 0..4 {
        events.extend(t.step(&detected(&ws, now, 360)));
    }
    assert!(matches!(events.as_slice(), [Event::Opened(d)] if d.key == "house:00112233aabbccdd"));
    let mut cleared = vec![];
    for _ in 0..3 {
        cleared.extend(t.step(&detected(&[], now, 360)));
    }
    assert!(
        matches!(cleared.as_slice(), [Event::Cleared { key, .. }] if key == "house:00112233aabbccdd")
    );
}

// rivet: verifies REQ-ALERT-005
#[test]
fn a_failed_read_keeps_the_last_list_for_ten_minutes_only() {
    let mut r = Reader::default();
    let l = vec![w("00112233aabbccdd", "frost", "alarm", None)];
    assert_eq!(r.take(Some(l), 1_000).len(), 1);
    assert_eq!(r.take(None, 1_000 + STALE_AFTER_S).len(), 1);
    assert!(r.take(None, 1_001 + STALE_AFTER_S).is_empty());
}

// rivet: verifies REQ-ALERT-005
#[test]
fn the_url_must_not_cross_a_network_in_clear_text() {
    let c = |u: &str| {
        HouseConfig {
            url: u.into(),
            silent_hold_min: 360,
        }
        .check()
    };
    assert!(c("http://127.0.0.1:8080/api/warnings").is_ok());
    assert!(c("https://house.example/api/warnings").is_ok());
    assert!(c("http://house.example/api/warnings").is_err());
    assert!(c("http://127.0.0.1.example/x").is_err());
    let cfg: warte::config::Config =
        toml::from_str("[house]\nurl = \"http://127.0.0.1:1/w\"\n").unwrap();
    assert_eq!(cfg.house.unwrap().silent_hold_min, 360);
    assert!(
        toml::from_str::<warte::config::Config>("[house]\nurl = \"x\"\ntoken = \"y\"\n").is_err()
    );
}

// rivet: verifies REQ-ALERT-005
#[tokio::test]
async fn the_list_is_read_over_http_and_a_bad_answer_reads_as_none() {
    use axum::{http::StatusCode, routing::get, Router};
    let app = Router::new()
        .route(
            "/ok",
            get(|| async {
                r#"[{"id":"00112233aabbccdd","kind":"frost","room":"Keller","series":[1],"message":"2 °C","severity":"alarm","since_ms":null}]"#
            }),
        )
        .route("/bad", get(|| async { "not json" }))
        .route("/down", get(|| async { StatusCode::INTERNAL_SERVER_ERROR }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let http = reqwest::Client::new();
    let cfg = |p: &str| HouseConfig {
        url: format!("http://{addr}{p}"),
        silent_hold_min: 360,
    };
    let ok = warte::alerts::house::fetch(&http, &cfg("/ok"))
        .await
        .unwrap();
    assert_eq!(ok.len(), 1);
    assert_eq!(ok[0].room.as_deref(), Some("Keller"));
    assert!(warte::alerts::house::fetch(&http, &cfg("/bad"))
        .await
        .is_none());
    assert!(warte::alerts::house::fetch(&http, &cfg("/down"))
        .await
        .is_none());
}

// rivet: verifies REQ-ALERT-005
#[tokio::test]
async fn a_bad_house_section_stops_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        house: Some(HouseConfig {
            url: "http://house.example/api/warnings".into(),
            silent_hold_min: 360,
        }),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let e = warte::server::build_state(config, db)
        .err()
        .expect("refused");
    assert!(e.to_string().contains("[house] url"), "{e}");
}

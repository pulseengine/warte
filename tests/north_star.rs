//! The North Star's published indicators (REQ-NS-001), against a local mock
//! of the published files. The shapes follow pulseengine.eu's
//! data/north-star.toml and data/north-star/<id>.json as of 2026-11-20.

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Router};
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use warte::northstar::{
    glance_line, measured_unix, parse_objectives, parse_observation, rows, NorthStar,
    NorthStarConfig, Observation, Store,
};

const TOML: &str = r#"
north_star = "x"
[[objectives]]
id = "O1"
title = "Runs on target"
guard = "A skipped hardware run counts as a failure."
state = "unmeasured"
[[objectives]]
id = "O2"
title = "One-command evidence bundle"
guard = "The verify command must fail when any piece of evidence is removed."
state = "measured"
[[objectives]]
id = "O3"
title = "Every verified claim is backed"
guard = "Ledger entries may only shrink."
[[objectives]]
id = "O4"
title = "Not only one person"
guard = "Reverted or reopened work."
[[objectives]]
id = "O5"
title = "We run on our own chain"
guard = "A gate behind a *_GAP switch counts as not running."
"#;

fn obs(o: &str, at: &str, value: f64, of: f64) -> Observation {
    Observation {
        objective: o.into(),
        measured_at: at.into(),
        value: Some(value),
        of: Some(of),
        verifier: None,
        blind_spot: None,
        guard_value: None,
        schema_version: 0,
    }
}

fn day(s: &str) -> i64 {
    measured_unix(s).unwrap()
}

// rivet: verifies REQ-NS-001
#[test]
fn five_objectives_two_files_give_three_unmeasured_never_zero() {
    let objs = parse_objectives(TOML).unwrap();
    assert_eq!(objs.len(), 5);
    let history = vec![
        obs("O2", "2026-11-17T19:27Z", 1.0, 2.0),
        obs("O4", "2026-11-18T11:32Z", 1.0, 839.0),
    ];
    let now = day("2026-11-20T08:00Z");
    let r = rows(&objs, &history, None, now, 45);
    let state = |id: &str| r.iter().find(|x| x.id == id).unwrap().state.clone();
    assert_eq!(state("O1"), "unmeasured");
    assert_eq!(state("O3"), "unmeasured");
    assert_eq!(state("O5"), "unmeasured");
    assert_eq!(state("O2"), "measured");
    for x in r.iter().filter(|x| x.state == "unmeasured") {
        assert!(
            x.latest.is_none(),
            "an unmeasured objective carries no value, never 0"
        );
    }
    assert_eq!(
        glance_line(&r),
        "O2 1/2 (2 d) · O4 1/839 (1 d) · 3 unmeasured"
    );
    // the guard travels with the row
    assert!(r[1].guard.contains("must fail when any piece"));
}

// rivet: verifies REQ-NS-001
#[test]
fn the_latest_and_previous_go_by_time_and_old_data_is_stale() {
    let objs = parse_objectives(TOML).unwrap();
    // planted: kept out of order
    let history = vec![
        obs("O4", "2026-11-18T11:32Z", 1.0, 839.0),
        obs("O4", "2026-10-15T09:00Z", 0.0, 812.0),
        obs("O4", "2026-11-01T09:00Z", 1.0, 820.0),
    ];
    let r = rows(&objs, &history, None, day("2026-11-20T08:00Z"), 45);
    let o4 = r.iter().find(|x| x.id == "O4").unwrap();
    assert_eq!(o4.latest.as_ref().unwrap().of, Some(839.0));
    assert_eq!(o4.previous.as_ref().unwrap().of, Some(820.0));
    // planted: 46 days later the same data is stale, not current
    let late = rows(&objs, &history, None, day("2027-01-03T12:00Z"), 45);
    assert_eq!(late.iter().find(|x| x.id == "O4").unwrap().state, "stale");
    assert!(glance_line(&late).contains("1 stale"));
}

// rivet: verifies REQ-NS-001
#[test]
fn a_published_file_is_read_as_data_and_bad_ones_are_refused() {
    let real = r#"{"objective": "O4", "measured_at": "2026-11-18T11:32Z", "verifier": "GitHub search",
        "period": "2026-10-01..2026-10-31", "value": 1, "of": 839, "items": [],
        "blind_spot": "time from ready to first pick-up is not measured yet"}"#;
    let o = parse_observation("O4", real).unwrap();
    assert_eq!(
        (o.value, o.of, o.schema_version),
        (Some(1.0), Some(839.0), 0)
    );
    assert!(o.blind_spot.unwrap().contains("pick-up"));
    // planted: another objective's file, no time, a time that is no time, not JSON
    assert!(parse_observation("O2", real).is_err());
    assert!(parse_observation("O4", r#"{"objective":"O4","value":1}"#).is_err());
    assert!(parse_observation("O4", r#"{"objective":"O4","measured_at":"soon"}"#).is_err());
    assert!(parse_observation("O4", "<html>").is_err());
    // control characters never reach the page
    let esc =
        r#"{"objective":"O4","measured_at":"2026-11-18T11:32Z","blind_spot":"a\u001b[31mred"}"#;
    assert!(!parse_observation("O4", esc)
        .unwrap()
        .blind_spot
        .unwrap()
        .contains('\u{1b}'));
    // a toml without objectives, or with an id that is not O<n>, names nothing
    assert!(parse_objectives("north_star = \"x\"").is_err());
    assert!(parse_objectives("[[objectives]]\nid = \"../x\"\ntitle = \"t\"").is_err());
}

// rivet: verifies REQ-NS-001
#[test]
fn the_store_keeps_each_observation_once_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(dir.path());
    assert!(s.add(obs("O2", "2026-11-17T19:27Z", 1.0, 2.0)).unwrap());
    assert!(
        !s.add(obs("O2", "2026-11-17T19:27Z", 1.0, 2.0)).unwrap(),
        "same time, kept once"
    );
    assert!(s.add(obs("O2", "2026-11-19T19:27Z", 2.0, 3.0)).unwrap());
    let again = Store::open(dir.path());
    assert_eq!(again.all().len(), 2);
}

type Files = Arc<Mutex<BTreeMap<String, String>>>;

async fn serve(files: Files) -> String {
    async fn file(
        State(f): State<Files>,
        axum::extract::Path(p): axum::extract::Path<String>,
    ) -> axum::response::Response {
        match f.lock().unwrap().get(&p) {
            Some(b) => b.clone().into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }
    let app = Router::new()
        .route("/data/{*p}", get(file))
        .with_state(files);
    let l = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let base = format!("http://{}/data", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    base
}

// rivet: verifies REQ-NS-001
#[tokio::test]
async fn a_poll_reads_what_is_published_and_a_removed_file_turns_unmeasured() {
    let files: Files = Arc::new(Mutex::new(BTreeMap::new()));
    {
        let mut f = files.lock().unwrap();
        f.insert("north-star.toml".into(), TOML.into());
        f.insert(
            "north-star/O2.json".into(),
            r#"{"objective":"O2","measured_at":"2026-11-17T19:27Z","value":1,"of":2}"#.into(),
        );
        f.insert(
            "north-star/O4.json".into(),
            r#"{"objective":"O4","measured_at":"2026-11-18T11:32Z","value":1,"of":839}"#.into(),
        );
        // planted: a file that does not parse is named, not shown
        f.insert("north-star/O5.json".into(), "not json".into());
    }
    let base = serve(files.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = NorthStarConfig {
        base_url: base,
        interval_s: 3600,
        stale_after_days: 45,
    };
    let n = NorthStar::new(cfg, dir.path());
    let now = day("2026-11-20T08:00Z");
    n.poll(now).await;
    let r = n.rows(now);
    assert_eq!(r.len(), 5);
    let st: Vec<(String, String)> = r.iter().map(|x| (x.id.clone(), x.state.clone())).collect();
    assert_eq!(
        st,
        [
            ("O1", "unmeasured"),
            ("O2", "measured"),
            ("O3", "unmeasured"),
            ("O4", "measured"),
            ("O5", "unmeasured")
        ]
        .map(|(a, b)| (a.to_string(), b.to_string()))
    );
    let last = n.last_poll.lock().unwrap().clone().unwrap();
    assert_eq!(last.1, "O5 unreadable", "{last:?}");

    // planted: the publisher removes O2; one poll later it is unmeasured,
    // and its history is kept
    files.lock().unwrap().remove("north-star/O2.json");
    n.poll(now).await;
    let r = n.rows(now);
    assert_eq!(r.iter().find(|x| x.id == "O2").unwrap().state, "unmeasured");
    assert_eq!(
        n.store.all().iter().filter(|o| o.objective == "O2").count(),
        1
    );
}

// rivet: verifies REQ-NS-001
#[test]
fn the_config_refuses_plain_http_and_silly_numbers() {
    let ok = NorthStarConfig {
        base_url: "https://raw.githubusercontent.com/pulseengine/pulseengine.eu/main/data".into(),
        interval_s: 21600,
        stale_after_days: 45,
    };
    assert!(ok.check().is_ok());
    for bad in [
        NorthStarConfig {
            base_url: "http://example.org/data".into(),
            ..ok.clone()
        },
        NorthStarConfig {
            interval_s: 5,
            ..ok.clone()
        },
        NorthStarConfig {
            stale_after_days: 0,
            ..ok.clone()
        },
    ] {
        assert!(bad.check().is_err(), "{bad:?}");
    }
    let parsed: NorthStarConfig = toml::from_str("").unwrap();
    assert_eq!(parsed, ok, "the defaults read the public files every 6 h");
}

/// Against the real published files; run by hand with `--ignored` (network).
#[tokio::test]
#[ignore]
async fn live_published_files_parse() {
    let dir = tempfile::tempdir().unwrap();
    let n = NorthStar::new(toml::from_str("").unwrap(), dir.path());
    let now = chrono::Utc::now().timestamp();
    n.poll(now).await;
    let last = n.last_poll.lock().unwrap().clone().unwrap();
    assert_eq!(last.1, "ok", "{last:?}");
    let r = n.rows(now);
    assert!(r.len() >= 5, "{r:?}");
    eprintln!("LIVE: {}", glance_line(&r));
}

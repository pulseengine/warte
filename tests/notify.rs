//! Alerts reach an ntfy topic (REQ-ALERT-003).

use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
use std::sync::{Arc, Mutex};
use warte::alerts::{self, ntfy, Event};
use warte::dashboard::fleet::Detected;

type Seen = Arc<Mutex<Vec<(Option<String>, serde_json::Value)>>>;

async fn mock() -> (String, Seen) {
    async fn take(
        State(s): State<Seen>,
        h: HeaderMap,
        Json(v): Json<serde_json::Value>,
    ) -> &'static str {
        let auth = h
            .get("authorization")
            .map(|a| a.to_str().unwrap().to_string());
        s.lock().unwrap().push((auth, v));
        "{}"
    }
    let seen: Seen = Default::default();
    let app = Router::new()
        .route("/", post(take))
        .with_state(seen.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (base, seen)
}

fn cfg(url: &str, token_file: Option<String>) -> ntfy::NotifyConfig {
    ntfy::NotifyConfig {
        ntfy_url: url.into(),
        topic: "warte-alerts".into(),
        token_file,
        click: Some("https://example.org/warte/outages".into()),
    }
}

fn disk(sev: u8) -> Detected {
    Detected {
        key: "disk:/srv/ci".into(),
        sev,
        text: "Disk /srv/ci is 97% full.".into(),
    }
}

// rivet: verifies REQ-ALERT-003
#[test]
fn the_config_is_checked() {
    assert!(cfg("https://ntfy.example/", None).check().is_ok());
    assert!(cfg("http://127.0.0.1:2586/", None).check().is_ok());
    for bad in [
        "http://ntfy.example/",
        "http://127.0.0.1.example/",
        "ftp://x/",
    ] {
        assert!(cfg(bad, None).check().is_err(), "{bad}");
    }
    let mut c = cfg("https://ntfy.example/", None);
    for bad in ["", "a b", "a/b", &"x".repeat(65)] {
        c.topic = bad.into();
        assert!(c.check().is_err(), "{bad:?}");
    }
    let mut c = cfg("https://ntfy.example/", None);
    c.click = Some("http://example.org/".into());
    assert!(c.check().is_err());
    let t: Result<warte::config::Config, _> =
        toml::from_str("[notify]\nntfy_url = \"https://n/\"\ntopic = \"t\"\nsecret = \"x\"\n");
    assert!(t.is_err(), "unknown keys are refused");
}

// rivet: verifies REQ-ALERT-003
#[tokio::test]
async fn a_bad_notify_section_stops_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        notify: Some(cfg("http://ntfy.example/", None)),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let e = warte::server::build_state(config, db)
        .err()
        .expect("refused");
    assert!(e.to_string().contains("ntfy_url"), "{e}");
}

// rivet: verifies REQ-ALERT-003
#[test]
fn paths_never_reach_the_push_and_priority_follows_severity() {
    assert_eq!(
        ntfy::redact("Disk /srv/ci is 100% full."),
        "Disk [path] is 100% full."
    );
    assert_eq!(ntfy::redact("see (~/x/y), ok"), "see ([path]), ok");
    assert_eq!(ntfy::redact("host-a: 3/4 up"), "host-a: 3/4 up");
    let c = cfg("https://ntfy.example/", None);
    let now = chrono::Utc::now();
    let cases = [
        (Event::Opened(disk(2)), 4, "warte: down"),
        (Event::Opened(disk(1)), 3, "warte: degraded"),
        (Event::Escalated(disk(2)), 4, "warte: down"),
        (
            Event::Cleared {
                key: "disk:/srv/ci".into(),
                text: "Disk /srv/ci is 97% full.".into(),
            },
            2,
            "warte: cleared",
        ),
    ];
    for (e, prio, title) in cases {
        let m = ntfy::message(&c, &alerts::notice(&e, now, None));
        assert_eq!(m["priority"], prio, "{m}");
        assert_eq!(m["title"], title, "{m}");
        assert_eq!(m["topic"], "warte-alerts");
        assert_eq!(m["click"], "https://example.org/warte/outages");
        let s = m.to_string();
        assert!(s.contains("[path]") && s.contains("97% full"), "{s}");
        assert!(!s.contains("/var/lib") && !s.contains("disk:"), "{s}");
    }
}

// rivet: verifies REQ-ALERT-003
#[tokio::test]
async fn each_notice_is_pushed_with_the_token_from_its_file() {
    use std::os::unix::fs::PermissionsExt;
    let (url, seen) = mock().await;
    let dir = tempfile::tempdir().unwrap();
    let tf = dir.path().join("ntfy-token");
    std::fs::write(&tf, "tk_example\n").unwrap();
    std::fs::set_permissions(&tf, std::fs::Permissions::from_mode(0o600)).unwrap();
    let c = cfg(&url, Some(tf.display().to_string()));
    let now = chrono::Utc::now();
    let ns = alerts::notices(&[Event::Opened(disk(2)), Event::Opened(disk(1))], now, None);
    assert_eq!(ntfy::deliver(&reqwest::Client::new(), &c, &ns).await, 2);
    {
        let got = seen.lock().unwrap();
        assert_eq!(got.len(), 2);
        assert!(got
            .iter()
            .all(|(a, _)| a.as_deref() == Some("Bearer tk_example")));
        assert_eq!(got[0].1["priority"], 4, "worst first");
    }

    // a token file other users can read sends nothing
    std::fs::set_permissions(&tf, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(ntfy::deliver(&reqwest::Client::new(), &c, &ns).await, 0);
    assert_eq!(seen.lock().unwrap().len(), 2);

    // without a token file, no Authorization header
    let c = cfg(&url, None);
    assert_eq!(
        ntfy::deliver(&reqwest::Client::new(), &c, &ns[..1]).await,
        1
    );
    assert_eq!(seen.lock().unwrap()[2].0, None);
}

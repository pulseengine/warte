//! Oracles for the dependency canaries (REQ-PREDICT-003, DD-PREDICT-002).

use std::net::SocketAddr;
use std::time::Duration;
use warte::canary::{
    answer, judge, next, panel, query, round, CanariesConfig, Outcome, Report, Resolver, Via,
    REPORT_AFTER,
};

const NOW: i64 = 1_800_000_000;

/// A reply to query `id` with response code `rcode` and `answers` records.
fn reply(id: u16, rcode: u8, answers: u16) -> Vec<u8> {
    let mut r = id.to_be_bytes().to_vec();
    r.extend_from_slice(&[0x81, 0x80 | rcode]);
    r.extend_from_slice(&[0, 1]);
    r.extend_from_slice(&answers.to_be_bytes());
    r.extend_from_slice(&[0, 0, 0, 0]);
    r
}

// rivet: verifies REQ-PREDICT-003
// rivet: verifies DD-PREDICT-002
#[test]
fn a_query_asks_for_the_a_record_with_recursion() {
    let q = query(0x1234, "ghcr.io");
    assert_eq!(&q[0..2], &[0x12, 0x34], "id");
    assert_eq!(&q[2..4], &[0x01, 0x00], "a query, recursion desired");
    assert_eq!(&q[4..12], &[0, 1, 0, 0, 0, 0, 0, 0], "one question");
    assert_eq!(&q[12..], b"\x04ghcr\x02io\x00\x00\x01\x00\x01");
    // a trailing dot is the same name
    assert_eq!(query(1, "ghcr.io."), query(1, "ghcr.io"));
}

// rivet: verifies REQ-PREDICT-003
#[test]
fn replies_are_read_by_their_code() {
    assert_eq!(answer(7, &reply(7, 0, 2)), Some(Outcome::Ok));
    assert_eq!(answer(7, &reply(7, 0, 0)), Some(Outcome::NoAddress));
    assert_eq!(answer(7, &reply(7, 2, 0)), Some(Outcome::ServerFailure));
    assert_eq!(answer(7, &reply(7, 3, 0)), Some(Outcome::NoSuchName));
    assert_eq!(answer(7, &reply(7, 5, 0)), Some(Outcome::Refused));
    assert_eq!(answer(7, &reply(7, 4, 0)), Some(Outcome::Failed));
    // not a reply to this query: another id, a query, too short
    assert_eq!(answer(8, &reply(7, 0, 1)), None);
    assert_eq!(answer(7, &query(7, "ghcr.io")), None);
    assert_eq!(answer(7, &reply(7, 0, 1)[..11]), None);
}

/// A resolver on localhost that answers every query with `rcode`, or never.
async fn mock(rcode: Option<u8>) -> SocketAddr {
    let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = sock.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        while let Ok((n, from)) = sock.recv_from(&mut buf).await {
            if n < 12 {
                continue;
            }
            if let Some(rc) = rcode {
                let id = u16::from_be_bytes([buf[0], buf[1]]);
                let _ = sock
                    .send_to(&reply(id, rc, if rc == 0 { 1 } else { 0 }), from)
                    .await;
            }
        }
    });
    addr
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// rivet: verifies REQ-PREDICT-003
#[tokio::test]
async fn a_round_asks_each_resolver_and_tells_failures_apart() {
    let resolvers = vec![
        Resolver {
            label: "good".into(),
            via: Via::Udp(mock(Some(0)).await),
        },
        Resolver {
            label: "broken".into(),
            via: Via::Udp(mock(Some(2)).await),
        },
        Resolver {
            label: "silent".into(),
            via: Via::Udp(mock(None).await),
        },
    ];
    let n = names(&["archive.example.org", "crates.example.org"]);
    let results = round(&n, &resolvers, Duration::from_millis(400)).await;
    assert_eq!(results.len(), 6);
    for (name, res, outcome, _) in &results {
        let want = match res.as_str() {
            "good" => Outcome::Ok,
            "broken" => Outcome::ServerFailure,
            _ => Outcome::TimedOut,
        };
        assert_eq!(*outcome, want, "{name} via {res}");
    }
}

type Row = (&'static str, &'static str, Outcome);

fn round_of(rows: &[Row]) -> Vec<(String, String, Outcome, u64)> {
    rows.iter()
        .map(|(n, r, o)| (n.to_string(), r.to_string(), *o, 5))
        .collect()
}

/// The report after `rounds` identical rounds, the last at NOW.
fn after(rounds: u32, n: &[&str], rs: &[&str], rows: &[Row]) -> Report {
    let mut r: Option<Report> = None;
    for i in 0..rounds {
        let at = NOW - (rounds - 1 - i) as i64 * 300;
        r = Some(next(
            r.as_ref(),
            at,
            300,
            &names(n),
            &names(rs),
            round_of(rows),
        ));
    }
    r.unwrap()
}

// rivet: verifies REQ-PREDICT-003
#[test]
fn a_failure_counts_after_two_rounds_and_clears_on_success() {
    let n = ["a.example.org"];
    let rs = ["google"];
    let bad = [("a.example.org", "google", Outcome::TimedOut)];
    let one = after(1, &n, &rs, &bad);
    assert_eq!(one.checks[0].failing_rounds, 1);
    assert_eq!(one.checks[0].failing_since, Some(NOW));
    assert!(
        judge(&one, NOW).is_empty(),
        "one lost packet is not an incident"
    );

    let three = after(3, &n, &rs, &bad);
    assert_eq!(three.checks[0].failing_rounds, 3);
    assert_eq!(
        three.checks[0].failing_since,
        Some(NOW - 600),
        "since the first failure"
    );
    const { assert!(REPORT_AFTER <= 3) };
    assert!(!judge(&three, NOW).is_empty());

    let ok = next(
        Some(&three),
        NOW + 300,
        300,
        &names(&n),
        &names(&rs),
        round_of(&[("a.example.org", "google", Outcome::Ok)]),
    );
    assert_eq!(ok.checks[0].failing_rounds, 0);
    assert_eq!(ok.checks[0].failing_since, None);
    assert!(judge(&ok, NOW + 300).is_empty());
}

// rivet: verifies REQ-PREDICT-003
#[test]
fn the_cause_is_named_from_which_resolvers_still_answer() {
    let n = ["archive.example.org", "crates.example.org"];
    let rs = ["system", "google", "quad9"];
    use Outcome::*;

    // the 2026-11-11 shape: one name fails through one resolver only
    let r = after(
        2,
        &n,
        &rs,
        &[
            ("archive.example.org", "system", Ok),
            ("archive.example.org", "google", ServerFailure),
            ("archive.example.org", "quad9", Ok),
            ("crates.example.org", "system", Ok),
            ("crates.example.org", "google", Ok),
            ("crates.example.org", "quad9", Ok),
        ],
    );
    let f = judge(&r, NOW);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].key, "canary:archive.example.org");
    assert_eq!(f[0].sev, 1);
    assert!(
        f[0].text.contains("fails through google (server failure)")
            && f[0].text.contains("resolves through system, quad9")
            && f[0].text.contains("a resolver problem"),
        "{}",
        f[0].text
    );

    // a name fails through every resolver: the dependency's DNS
    let r = after(
        2,
        &n,
        &rs,
        &[
            ("archive.example.org", "system", Failed),
            ("archive.example.org", "google", NoSuchName),
            ("archive.example.org", "quad9", NoSuchName),
            ("crates.example.org", "system", Ok),
            ("crates.example.org", "google", Ok),
            ("crates.example.org", "quad9", Ok),
        ],
    );
    let f = judge(&r, NOW);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].sev, 2);
    assert!(
        f[0].text.contains("through any of system, google, quad9 (")
            && f[0].text.contains("the dependency's DNS"),
        "{}",
        f[0].text
    );

    // one resolver fails for every name: that resolver, once, not per name
    let r = after(
        2,
        &n,
        &rs,
        &[
            ("archive.example.org", "system", Ok),
            ("archive.example.org", "google", TimedOut),
            ("archive.example.org", "quad9", Ok),
            ("crates.example.org", "system", Ok),
            ("crates.example.org", "google", TimedOut),
            ("crates.example.org", "quad9", Ok),
        ],
    );
    let f = judge(&r, NOW);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].key, "canary-resolver:google");
    assert!(
        f[0].text.contains("that resolver is failing"),
        "{}",
        f[0].text
    );

    // nothing resolves anywhere: the network, once
    let all: Vec<Row> = n
        .iter()
        .flat_map(|name| rs.iter().map(move |res| (*name, *res, TimedOut)))
        .collect();
    let f = judge(&after(2, &n, &rs, &all), NOW);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!((f[0].key.as_str(), f[0].sev), ("canary-all", 2));

    // keys hold no numbers or times: alerts deduplicate on them
    for d in judge(&after(5, &n, &rs, &all), NOW) {
        assert!(!d.key.chars().any(|c| c.is_ascii_digit()), "{}", d.key);
    }

    // the round itself stopped
    let r = after(2, &n, &rs, &all);
    let f = judge(&r, NOW + 3 * 300 + 1);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].key, "canary-stale");
}

fn cfg(text: &str) -> CanariesConfig {
    #[derive(serde::Deserialize)]
    struct File {
        canaries: CanariesConfig,
    }
    toml::from_str::<File>(text).unwrap().canaries
}

// rivet: verifies REQ-PREDICT-003
#[test]
fn the_section_is_checked_before_warte_starts() {
    let good = cfg(r#"
[canaries]
names = ["archive.example.org"]
[[canaries.resolver]]
label = "google"
address = "192.0.2.53"
[[canaries.resolver]]
label = "other"
address = "192.0.2.54:5353"
"#);
    let rs = good.resolvers().unwrap();
    assert_eq!(rs.len(), 3);
    assert_eq!(rs[0].via, Via::System);
    assert_eq!(rs[1].via, Via::Udp("192.0.2.53:53".parse().unwrap()));
    assert_eq!(rs[2].via, Via::Udp("192.0.2.54:5353".parse().unwrap()));
    assert_eq!(good.every_s, 300);

    for (bad, why) in [
        ("names = []", "empty"),
        ("names = [\"a b.example.org\"]", "not a host name"),
        ("names = [\"a.example.org\"]\nevery_s = 30", "at least 60"),
        ("names = [\"a.example.org\"]\nsystem = false", "no resolver"),
        (
            "names = [\"a.example.org\"]\n[[canaries.resolver]]\nlabel = \"system\"\naddress = \"192.0.2.1\"",
            "label other than",
        ),
        (
            "names = [\"a.example.org\"]\n[[canaries.resolver]]\nlabel = \"g\"\naddress = \"192.0.2.1\"\n[[canaries.resolver]]\nlabel = \"g\"\naddress = \"192.0.2.2\"",
            "used twice",
        ),
        (
            "names = [\"a.example.org\"]\n[[canaries.resolver]]\nlabel = \"g\"\naddress = \"dns.example.org\"",
            "not an IP address",
        ),
    ] {
        let e = cfg(&format!("[canaries]\n{bad}")).resolvers().unwrap_err();
        assert!(e.contains(why), "{bad}: {e}");
    }
}

// rivet: verifies REQ-PREDICT-003
// rivet: verifies DD-PREDICT-002
#[tokio::test]
async fn what_is_shown_names_resolvers_by_label_never_by_address() {
    let broken = mock(Some(2)).await;
    let resolvers = vec![Resolver {
        label: "router".into(),
        via: Via::Udp(broken),
    }];
    let n = names(&["archive.example.org"]);
    let labels = names(&["router"]);
    let mut r = None;
    for i in 0..2 {
        let results = round(&n, &resolvers, Duration::from_millis(400)).await;
        r = Some(next(r.as_ref(), NOW + i * 300, 300, &n, &labels, results));
    }
    let r = r.unwrap();
    let mut shown = serde_json::to_string(&r).unwrap() + &panel(Some(&r));
    for d in judge(&r, NOW + 300) {
        shown += &d.key;
        shown += &d.text;
    }
    assert!(
        shown.contains("router") && shown.contains("server failure"),
        "{shown}"
    );
    assert!(!shown.contains("127.0.0.1"), "{shown}");
    assert!(!shown.contains(&broken.port().to_string()), "{shown}");
}

// rivet: verifies REQ-PREDICT-003
#[test]
fn the_documented_section_is_a_working_one() {
    let doc = include_str!("../docs/topics/deployment.md");
    let at = doc
        .find("```toml\n[canaries]")
        .expect("a [canaries] example");
    let block = &doc[at + 8..];
    let block = &block[..block.find("```").unwrap()];
    let c = cfg(block);
    let rs = c.resolvers().unwrap();
    assert!(rs.len() >= 3, "the example shows several resolvers");
    // placeholders only: documentation addresses and example names
    for r in &c.resolver {
        assert!(
            r.address.starts_with("192.0.2.")
                || r.address.starts_with("8.8.")
                || r.address.starts_with("1.1.1.")
                || r.address.starts_with("9.9.9."),
            "{}",
            r.address
        );
    }
}

/// warte served with `canaries`, on a local port.
async fn serve(canaries: Option<CanariesConfig>) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        canaries,
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

// rivet: verifies REQ-PREDICT-003
#[tokio::test]
async fn the_api_and_overview_answer_without_a_ground_station() {
    let (_d, base) = serve(None).await;
    let r = reqwest::get(format!("{base}/api/canaries")).await.unwrap();
    assert_eq!(r.status(), 503, "without [canaries]");

    let good = mock(Some(0)).await;
    let c = cfg(&format!(
        "[canaries]\nnames = [\"archive.example.org\"]\nsystem = false\n[[canaries.resolver]]\nlabel = \"local\"\naddress = \"{good}\""
    ));
    let (_d, base) = serve(Some(c)).await;
    // the first round runs at once
    let mut body = String::new();
    for _ in 0..50 {
        let r = reqwest::get(format!("{base}/api/canaries")).await.unwrap();
        if r.status() == 200 {
            body = r.text().await.unwrap();
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let v: serde_json::Value = serde_json::from_str(&body).expect(&body);
    assert_eq!(v["report"]["checks"][0]["resolver"], "local");
    assert_eq!(v["report"]["checks"][0]["outcome"], "ok");
    assert_eq!(v["findings"], serde_json::json!([]));
    assert!(!body.contains(&good.port().to_string()), "{body}");
    // and the overview shows the panel
    let ov = reqwest::get(format!("{base}/api/dashboard/overview-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(ov.contains("<h3>Dependencies</h3>"), "{ov}");
}

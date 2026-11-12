//! Oracles for expiry dates (REQ-PREDICT-004, DD-PREDICT-003).

use rcgen::{BasicConstraints, Certificate, CertificateParams, DnType, IsCa, KeyPair};
use std::sync::Arc;
use std::time::Duration;
use warte::expiry::{
    certificates, check, detected, fetch, panel, pem_certificates, report, ExpiryConfig, Item,
    Kind, Own, PemFile, Problem, TlsTarget,
};

const DAY: i64 = 86_400;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn at(days: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::from_unix_timestamp(now() + days * DAY).unwrap()
}

/// A certificate ending in `days`, signed by `issuer` or by itself.
fn cert(
    cn: &str,
    days: i64,
    ca: bool,
    issuer: Option<(&Certificate, &KeyPair)>,
) -> (Certificate, KeyPair) {
    let key = KeyPair::generate().unwrap();
    let mut p = CertificateParams::new(vec![cn.to_string()]).unwrap();
    p.distinguished_name.push(DnType::CommonName, cn);
    p.not_before = at(-1);
    p.not_after = at(days);
    if ca {
        p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    }
    let c = match issuer {
        Some((ic, ik)) => p.signed_by(&key, ic, ik).unwrap(),
        None => p.self_signed(&key).unwrap(),
    };
    (c, key)
}

fn ends(i: &Item) -> i64 {
    (i.expires_at.unwrap() - now() + DAY / 2) / DAY
}

// rivet: verifies REQ-PREDICT-004
#[test]
fn certificates_are_named_by_their_place_in_the_chain() {
    let (root, rk) = cert("root.example.internal", 400, true, None);
    let (inter, ik) = cert("inter.example.internal", 200, true, Some((&root, &rk)));
    let (leaf, _) = cert("leaf.example.internal", 20, false, Some((&inter, &ik)));
    let items = certificates(
        Kind::Tls,
        "proxy",
        &[
            leaf.der().to_vec(),
            inter.der().to_vec(),
            root.der().to_vec(),
        ],
    );
    let parts: Vec<&str> = items.iter().map(|i| i.part.as_deref().unwrap()).collect();
    assert_eq!(parts, ["leaf", "intermediate 1", "root"]);
    let days: Vec<i64> = items.iter().map(ends).collect();
    assert_eq!(days, [20, 200, 400]);

    // garbage is unreadable, nothing is "no certificate"
    let bad = certificates(Kind::Pem, "f", &[b"nope".to_vec()]);
    assert_eq!(bad[0].problem, Some(Problem::Unreadable));
    assert_eq!(
        certificates(Kind::Pem, "f", &[])[0].problem,
        Some(Problem::NoCertificate)
    );

    // PEM text, with a key in it that is skipped
    let text = format!("{}{}", root.pem(), rk.serialize_pem()) + &inter.pem();
    assert_eq!(pem_certificates(text.as_bytes()).unwrap().len(), 2);
}

/// A TLS server on localhost sending `chain`.
async fn tls_server(chain: Vec<Vec<u8>>, key: &KeyPair) -> std::net::SocketAddr {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            chain.into_iter().map(Into::into).collect(),
            rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((s, _)) = l.accept().await {
            let a = acceptor.clone();
            tokio::spawn(async move {
                let _ = a.accept(s).await;
            });
        }
    });
    addr
}

// rivet: verifies REQ-PREDICT-004
// rivet: verifies DD-PREDICT-003
#[tokio::test]
async fn an_endpoint_s_chain_is_read_without_trusting_it() {
    // an internal CA nobody trusts, as a proxy's own would be
    let (root, rk) = cert("Local Root", 3650, true, None);
    let (inter, ik) = cert("Local Intermediate", 5, true, Some((&root, &rk)));
    let (leaf, lk) = cert("warte.example.internal", 60, false, Some((&inter, &ik)));
    let addr = tls_server(vec![leaf.der().to_vec(), inter.der().to_vec()], &lk).await;
    let t = TlsTarget {
        label: "dashboard".into(),
        address: addr.to_string(),
        server_name: Some("warte.example.internal".into()),
    };
    let ders = fetch(&t, Duration::from_secs(5)).await.unwrap();
    assert_eq!(ders.len(), 2);

    let cfg = ExpiryConfig {
        tls: vec![
            t,
            TlsTarget {
                label: "gone".into(),
                address: "127.0.0.1:1".into(),
                server_name: None,
            },
        ],
        ..Default::default()
    };
    let items = check(&cfg, &Own::default(), Duration::from_secs(5)).await;
    let r = report(&cfg, now(), &items, &[]);
    let inter_item = r
        .items
        .iter()
        .find(|i| i.part.as_deref() == Some("intermediate 1"))
        .unwrap();
    assert_eq!(ends(inter_item), 5);
    let gone = r.items.iter().find(|i| i.what == "gone").unwrap();
    assert_eq!(gone.problem, Some(Problem::Unreachable));

    // the intermediate ending in 5 days is red; the leaf at 60 is quiet
    let f = detected(&r, now());
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].sev, 2);
    assert_eq!(f[0].key, "expiry:tls:dashboard:intermediate 1");
    assert!(
        f[0].text
            .starts_with("dashboard (intermediate 1 certificate) expires"),
        "{}",
        f[0].text
    );

    // labels are shown, addresses never
    let shown = serde_json::to_string(&r).unwrap() + &panel(&r, now()) + &f[0].text;
    assert!(!shown.contains("127.0.0.1"), "{shown}");
    assert!(!shown.contains(&addr.port().to_string()), "{shown}");
    assert!(!shown.contains("example.internal"), "{shown}");
}

// rivet: verifies REQ-PREDICT-004
#[tokio::test]
async fn own_certificates_and_files_are_read_and_paths_never_shown() {
    let dir = tempfile::tempdir().unwrap();
    let (ca, _) = cert("warte CA", 25, true, None);
    let ca_path = dir.path().join("ca.pem");
    std::fs::write(&ca_path, ca.pem()).unwrap();
    let (root, _) = cert("Proxy Root", 3, true, None);
    let root_path = dir.path().join("secret-place-root.crt");
    std::fs::write(&root_path, root.pem()).unwrap();
    let cfg = ExpiryConfig {
        pem: vec![
            PemFile {
                label: "proxy root".into(),
                path: root_path,
            },
            PemFile {
                label: "missing".into(),
                path: dir.path().join("nope.pem"),
            },
        ],
        ..Default::default()
    };
    let own = Own {
        ca: Some(ca_path),
        tls: None,
    };
    let r = report(
        &cfg,
        now(),
        &check(&cfg, &own, Duration::from_secs(1)).await,
        &[],
    );
    let f = detected(&r, now());
    let keys: Vec<(&str, u8)> = f.iter().map(|d| (d.key.as_str(), d.sev)).collect();
    assert_eq!(
        keys,
        [
            ("expiry:pem:proxy root:root", 2),
            ("expiry:own:warte CA:root", 1)
        ]
    );
    let missing = r.items.iter().find(|i| i.what == "missing").unwrap();
    assert_eq!(missing.problem, Some(Problem::Unreadable));
    let shown = serde_json::to_string(&r).unwrap() + &panel(&r, now());
    assert!(
        !shown.contains("secret-place") && !shown.contains(".pem"),
        "{shown}"
    );
}

// rivet: verifies REQ-PREDICT-004
#[test]
fn dates_and_tokens_count_like_certificates() {
    let day = |d: i64| {
        chrono::DateTime::from_timestamp(now() + d * DAY, 0)
            .unwrap()
            .format("%Y-%m-%d")
            .to_string()
    };
    let cfg: ExpiryConfig = toml::from_str(&format!(
        "[[date]]\nwhat = \"App key\"\non = \"{}\"\n[[date]]\nwhat = \"signing key\"\non = \"{}\"\n[[date]]\nwhat = \"far\"\non = \"{}\"",
        day(-2),
        day(20),
        day(200)
    ))
    .unwrap();
    cfg.check().unwrap();
    let r = report(&cfg, 0, &[], &[("briefing-job".into(), now() + 3 * DAY)]);
    let f = detected(&r, now());
    let got: Vec<(&str, u8)> = f.iter().map(|d| (d.key.as_str(), d.sev)).collect();
    assert_eq!(
        got,
        [
            ("expiry:date:App key:", 2),
            ("expiry:token:briefing-job:", 2),
            ("expiry:date:signing key:", 1),
        ]
    );
    assert!(f[0].text.contains("expired on"), "{}", f[0].text);
    assert!(
        f[1].text.starts_with("the token for briefing-job expires"),
        "{}",
        f[1].text
    );
    // keys hold no dates: alerts deduplicate on them
    assert!(f.iter().all(|d| !d.key.chars().any(|c| c.is_ascii_digit())));
    // the panel lists the next 90 days and counts the rest
    let p = panel(&r, now());
    assert!(p.contains("signing key") && !p.contains(">far<"), "{p}");
    assert!(p.contains("1 more run out later than 90 days"), "{p}");
}

// rivet: verifies REQ-PREDICT-004
#[test]
fn a_used_token_is_remembered_until_a_week_after_it_ends() {
    let tm = warte::auth::TokenManager::from_seed(&[7u8; 32], 24);
    let t = tm
        .issue_token("costs-job", vec!["costs:write".into()])
        .unwrap();
    assert!(tm.seen(now()).is_empty(), "not used yet");
    tm.verify_token(&t).unwrap();
    let seen = tm.seen(now());
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "costs-job");
    assert!((seen[0].1 - (now() + DAY)).abs() < 60);
    // still listed (as expired) for a week after it ends, then forgotten
    assert_eq!(tm.seen(now() + 7 * DAY).len(), 1);
    assert!(tm.seen(now() + 9 * DAY).is_empty());
}

// rivet: verifies REQ-PREDICT-004
#[test]
fn the_section_is_checked_before_warte_starts() {
    for (bad, why) in [
        ("every_s = 60", "at least 600"),
        ("red_days = 40", "red_days <= amber_days"),
        (
            "[[tls]]\nlabel = \"x\"\naddress = \"example.org\"",
            "host:port",
        ),
        (
            "[[tls]]\nlabel = \"\"\naddress = \"example.org:443\"",
            "needs a label",
        ),
        ("[[date]]\nwhat = \"k\"\non = \"next year\"", "YYYY-MM-DD"),
        (
            "[[date]]\nwhat = \"k\"\non = \"2027-01-01\"\n[[pem]]\nlabel = \"k\"\npath = \"/x\"",
            "used twice",
        ),
    ] {
        let c: ExpiryConfig = toml::from_str(bad).unwrap();
        let e = c.check().unwrap_err();
        assert!(e.contains(why), "{bad}: {e}");
    }
    ExpiryConfig::default().check().unwrap();
}

// rivet: verifies REQ-PREDICT-004
#[test]
fn the_documented_section_is_a_working_one() {
    let doc = include_str!("../docs/topics/deployment.md");
    let at = doc.find("```toml\n[expiry]").expect("an [expiry] example");
    let block = &doc[at + 8..];
    let block = &block[..block.find("```").unwrap()];
    #[derive(serde::Deserialize)]
    struct File {
        expiry: ExpiryConfig,
    }
    let c = toml::from_str::<File>(block).unwrap().expiry;
    c.check().unwrap();
    assert!(!c.tls.is_empty() && !c.pem.is_empty() && !c.date.is_empty());
    for t in &c.tls {
        assert!(t.address.contains("example.internal"), "{}", t.address);
    }
}

// rivet: verifies REQ-PREDICT-004
#[tokio::test]
async fn the_api_and_overview_show_it_without_any_section() {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let token = state
        .tokens
        .issue_token("briefing-job", vec!["briefing:write".into()])
        .unwrap();
    state.tokens.verify_token(&token).unwrap();
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    // the first check runs at once and finds warte's own CA
    let mut v = serde_json::Value::Null;
    for _ in 0..50 {
        v = reqwest::get(format!("{base}/api/expiry"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if v["report"]["checked_at"].as_i64().unwrap_or(0) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let items = v["report"]["items"].as_array().unwrap();
    assert!(
        items
            .iter()
            .any(|i| i["kind"] == "own" && i["what"] == "warte CA"),
        "{v}"
    );
    assert!(
        items
            .iter()
            .any(|i| i["kind"] == "token" && i["what"] == "briefing-job"),
        "{v}"
    );
    // a token for a day is red
    assert!(v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["key"] == "expiry:token:briefing-job:" && f["sev"] == 2));
    assert!(!v.to_string().contains(dir.path().to_str().unwrap()));
    let ov = reqwest::get(format!("{base}/api/dashboard/overview-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        ov.contains("<h3>Expiring</h3>") && ov.contains("the token for briefing-job"),
        "{ov}"
    );
}

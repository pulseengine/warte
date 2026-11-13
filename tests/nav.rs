//! nav v1: the home link and app switcher (REQ-NAV-001), against the
//! contract in docs/topics/nav.md.

use warte::nav::{check, html, NavConfig, NavFile};

const DOC: &str = include_str!("../docs/topics/nav.md");

/// The example file in the contract doc.
fn example() -> serde_json::Value {
    let start = DOC.find("```json\n").expect("example") + 8;
    let end = start + DOC[start..].find("```").unwrap();
    serde_json::from_str(&DOC[start..end]).expect("the doc's example is JSON")
}

fn parse(v: &serde_json::Value) -> Result<NavFile, String> {
    warte::nav::parse(&v.to_string())
}

fn hrefs(page: &str) -> Vec<String> {
    let nav = &page[page.find("<nav class=\"apps\"").unwrap()..];
    let nav = &nav[..nav.find("</nav>").unwrap()];
    nav.split("href=\"")
        .skip(1)
        .map(|s| s[..s.find('"').unwrap()].to_string())
        .collect()
}

// rivet: verifies REQ-NAV-001
#[test]
fn the_contract_example_renders_every_link_and_no_other() {
    let f = parse(&example()).unwrap();
    check(&f, "warte").unwrap();
    let h = html(&f, "warte");
    let mut want = vec![f.home.url.clone()];
    want.extend(f.apps.iter().map(|a| a.url.clone()));
    assert_eq!(hrefs(&h), want, "{h}");
    assert_eq!(h.matches("<a ").count(), 1 + f.apps.len(), "{h}");
    assert_eq!(h.matches("aria-current=\"page\"").count(), 1, "{h}");
    assert!(h.contains("aria-current=\"page\">warte</a>"), "{h}");
    assert!(h.contains(">← tower</a>"), "{h}");
    assert!(h.contains("title=\"control tower\""), "{h}");
}

// rivet: verifies REQ-NAV-001
#[test]
fn names_and_descriptions_are_escaped() {
    let mut v = example();
    v["apps"][1]["name"] = "<b>x</b>".into();
    v["apps"][1]["desc"] = "a \"quote\" & <tag>".into();
    let f = parse(&v).unwrap();
    check(&f, "warte").unwrap();
    let h = html(&f, "warte");
    assert!(h.contains("&lt;b&gt;x&lt;/b&gt;"), "{h}");
    assert!(h.contains("a &quot;quote&quot; &amp; &lt;tag&gt;"), "{h}");
    assert!(!h.contains("<b>"), "{h}");
}

// rivet: verifies REQ-NAV-001
#[test]
fn every_rule_refuses_a_file_and_names_the_field() {
    let long = "x".repeat(41);
    let many: Vec<serde_json::Value> = (0..21)
        .map(|i| serde_json::json!({"id": format!("a{i}"), "name": "a", "url": "/"}))
        .collect();
    let cases: Vec<(&str, Edit, &str)> = vec![
        (
            "version 2",
            Box::new(|v| v["version"] = 2.into()),
            "version",
        ),
        (
            "unknown top key",
            Box::new(|v| v["extra"] = 1.into()),
            "extra",
        ),
        (
            "unknown home key",
            Box::new(|v| v["home"]["x"] = 1.into()),
            "x",
        ),
        (
            "unknown app key",
            Box::new(|v| v["apps"][0]["icon"] = "i".into()),
            "icon",
        ),
        (
            "missing url",
            Box::new(|v| {
                v["apps"][0].as_object_mut().unwrap().remove("url");
            }),
            "url",
        ),
        (
            "backslash url",
            Box::new(|v| v["apps"][1]["url"] = "/\\evil.example".into()),
            "apps[1].url",
        ),
        (
            "backslash home url",
            Box::new(|v| v["home"]["url"] = "/a\\b".into()),
            "home.url",
        ),
        (
            "home as an array",
            Box::new(|v| v["home"] = serde_json::json!(["t", "/"])),
            "home",
        ),
        (
            "app as an array",
            Box::new(|v| v["apps"][1] = serde_json::json!(["photos", "Photos", "/photos/"])),
            "apps[1]",
        ),
        (
            "version as a string",
            Box::new(|v| v["version"] = "1".into()),
            "version",
        ),
        (
            "version as a float",
            Box::new(|v| v["version"] = serde_json::json!(1.0)),
            "version",
        ),
        (
            "name of the wrong type",
            Box::new(|v| v["apps"][1]["name"] = 7.into()),
            "apps[1].name",
        ),
        (
            "uppercase id",
            Box::new(|v| v["apps"][0]["id"] = "Warte".into()),
            "apps[0].id",
        ),
        (
            "empty id",
            Box::new(|v| v["apps"][0]["id"] = "".into()),
            "apps[0].id",
        ),
        (
            "long id",
            Box::new(|v| v["apps"][0]["id"] = "a".repeat(33).into()),
            "apps[0].id",
        ),
        (
            "duplicate id",
            Box::new(|v| v["apps"][1]["id"] = "warte".into()),
            "apps[1].id",
        ),
        (
            "blank name",
            Box::new(|v| v["apps"][1]["name"] = "  ".into()),
            "apps[1].name",
        ),
        (
            "long name",
            Box::new(move |v| v["apps"][1]["name"] = long.clone().into()),
            "apps[1].name",
        ),
        (
            "long home name",
            Box::new(|v| v["home"]["name"] = "y".repeat(41).into()),
            "home.name",
        ),
        (
            "protocol-relative",
            Box::new(|v| v["apps"][1]["url"] = "//evil.example/".into()),
            "apps[1].url",
        ),
        (
            "http",
            Box::new(|v| v["apps"][1]["url"] = "http://a.example/".into()),
            "apps[1].url",
        ),
        (
            "javascript",
            Box::new(|v| v["apps"][1]["url"] = "javascript:alert(1)".into()),
            "apps[1].url",
        ),
        (
            "relative",
            Box::new(|v| v["apps"][1]["url"] = "photos/".into()),
            "apps[1].url",
        ),
        (
            "space",
            Box::new(|v| v["apps"][1]["url"] = "/a b".into()),
            "apps[1].url",
        ),
        (
            "quote",
            Box::new(|v| v["apps"][1]["url"] = "/a\"b".into()),
            "apps[1].url",
        ),
        (
            "angle",
            Box::new(|v| v["apps"][1]["url"] = "/a<b".into()),
            "apps[1].url",
        ),
        (
            "home url",
            Box::new(|v| v["home"]["url"] = "//x".into()),
            "home.url",
        ),
        (
            "long desc",
            Box::new(|v| v["apps"][0]["desc"] = "d".repeat(121).into()),
            "apps[0].desc",
        ),
        (
            "no apps",
            Box::new(|v| v["apps"] = serde_json::json!([])),
            "apps",
        ),
        (
            "21 apps",
            Box::new(move |v| v["apps"] = many.clone().into()),
            "apps",
        ),
        (
            "self absent",
            Box::new(|v| v["apps"][0]["id"] = "other".into()),
            "self",
        ),
    ];
    for (what, edit, field) in cases {
        let mut v = example();
        edit(&mut v);
        let err = match parse(&v) {
            Err(e) => e,
            Ok(f) => check(&f, "warte").expect_err(what),
        };
        assert!(err.contains(field), "{what}: {err:?} does not name {field}");
    }
    // duplicate keys are refused, null desc is no desc
    assert!(warte::nav::parse(
        r#"{"version":1,"version":1,"home":{"name":"h","url":"/"},"apps":[]}"#
    )
    .is_err());
    let mut v = example();
    v["apps"][1]["desc"] = serde_json::Value::Null;
    check(&parse(&v).unwrap(), "warte").unwrap();
    // the limits themselves pass: 40-character names, 32-character ids, 120-character desc
    let mut v = example();
    v["apps"][1]["name"] = "n".repeat(40).into();
    v["apps"][1]["id"] = "a".repeat(32).into();
    v["apps"][1]["desc"] = "d".repeat(120).into();
    check(&parse(&v).unwrap(), "warte").unwrap();
}

/// One way to break the example file.
type Edit = Box<dyn Fn(&mut serde_json::Value)>;

fn write_example(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("nav.json");
    std::fs::write(&p, example().to_string()).unwrap();
    p
}

async fn page(nav: Option<NavConfig>) -> Result<String, String> {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        nav,
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).map_err(|e| format!("{e:#}"))?;
    let r = warte::server::build_router(state)
        .oneshot(
            axum::http::Request::get("/")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let b = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    Ok(String::from_utf8(b.to_vec()).unwrap())
}

// rivet: verifies REQ-NAV-001
#[tokio::test]
async fn the_dashboard_renders_the_file_and_nothing_without_it() {
    // without [nav]: the page is the template with only the placeholder gone
    let plain = page(None).await.unwrap();
    assert!(!plain.contains("{{NAV}}") && !plain.contains("class=\"apps\""));
    let idle = serde_json::to_string(&warte::config::IdleConfig::default())
        .unwrap()
        .replace('\'', "&#39;");
    let template = include_str!("../templates/dashboard.html");
    assert_eq!(template.matches("{{NAV}}").count(), 1);
    assert_eq!(
        plain,
        template
            .replace("{{NAV}}", "")
            .replace("{{VERSION}}", env!("CARGO_PKG_VERSION"))
            .replace("{{IDLE}}", &idle),
        "without [nav] the page is the template as before"
    );

    let dir = tempfile::tempdir().unwrap();
    let file = write_example(dir.path());
    let with = page(Some(NavConfig {
        file: file.clone(),
        self_id: "warte".into(),
    }))
    .await
    .unwrap();
    assert_eq!(
        hrefs(&with),
        [
            "/",
            "/warte/",
            "/photos/",
            "https://login.example.internal/"
        ]
    );
    assert_eq!(
        with.replacen(&html(&parse(&example()).unwrap(), "warte"), "", 1),
        plain
    );

    // a broken file stops startup and names the rule and the file
    std::fs::write(
        &file,
        r#"{"version": 2, "home": {"name": "h", "url": "/"}, "apps": []}"#,
    )
    .unwrap();
    let err = page(Some(NavConfig {
        file: file.clone(),
        self_id: "warte".into(),
    }))
    .await
    .expect_err("refused");
    assert!(err.contains("[nav]") && err.contains("version"), "{err}");
    // and so does a missing one
    let err = page(Some(NavConfig {
        file: dir.path().join("absent.json"),
        self_id: "warte".into(),
    }))
    .await
    .expect_err("refused");
    assert!(err.contains("absent.json"), "{err}");
}

// rivet: verifies REQ-NAV-001
#[test]
fn warte_toml_needs_both_file_and_self() {
    let ok: Result<warte::config::Config, _> =
        toml::from_str("[nav]\nfile = \"/etc/n.json\"\nself = \"warte\"\n");
    assert!(ok.is_ok(), "{ok:?}");
    let no_self: Result<warte::config::Config, _> =
        toml::from_str("[nav]\nfile = \"/etc/n.json\"\n");
    assert!(no_self.unwrap_err().to_string().contains("self"));
    let extra: Result<warte::config::Config, _> =
        toml::from_str("[nav]\nfile = \"/n\"\nself = \"w\"\nicon = 1\n");
    assert!(extra.is_err());
}

fn run(args: &[&str]) -> (i32, String, String) {
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_warte"))
        .args(args)
        .env_remove("WARTE_CONFIG")
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into(),
        String::from_utf8_lossy(&o.stderr).into(),
    )
}

// rivet: verifies REQ-NAV-002
#[test]
fn the_check_command_gives_the_startup_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let file = write_example(dir.path());
    let f = file.to_str().unwrap();
    let (code, out, err) = run(&["nav", "check", f, "--self", "warte"]);
    assert_eq!((code, out.trim()), (0, "ok: nav v1, 3 apps"), "{err}");
    let (code, _, _) = run(&["nav", "check", f]);
    assert_eq!(code, 0);
    // an id not in the file
    let (code, _, err) = run(&["nav", "check", f, "--self", "other"]);
    assert_eq!(code, 1);
    assert!(err.contains("[nav]") && err.contains("self"), "{err}");
    // the startup message, word for word
    let mut v = example();
    v["apps"][2]["url"] = "//x/".into();
    std::fs::write(&file, v.to_string()).unwrap();
    let (code, _, err) = run(&["nav", "check", f, "--self", "warte"]);
    assert_eq!(code, 1);
    let startup = warte::nav::load(&NavConfig {
        file: file.clone(),
        self_id: "warte".into(),
    })
    .unwrap_err();
    assert!(err.contains(&startup), "{err:?} lacks {startup:?}");
    assert!(startup.contains("apps[2].url"), "{startup}");
}

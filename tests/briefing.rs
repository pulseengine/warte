//! The briefing (REQ-BRIEF-001..003), against a mock GitHub. Synthetic org
//! and repositories only.

use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Router,
};
use serde_json::json;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use warte::briefing::{self, github, BriefingConfig, Facts, Text};
use warte::hub::now;

fn iso(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

#[derive(Clone, Default)]
struct Mock {
    seen: Arc<Mutex<Vec<String>>>,
    /// Searches answer 403 (a rate limit).
    searches_fail: Arc<Mutex<bool>>,
}

/// Noon of the period's first day.
fn in_period() -> i64 {
    let (from, _) = briefing::last_period(now());
    warte::hub::report::day_start(&from).unwrap() + 12 * 3600
}

async fn github_mock(State(m): State<Mock>, req: Request) -> Response {
    let uri = req.uri().to_string();
    m.seen.lock().unwrap().push(uri.clone());
    assert!(
        req.headers().get("authorization").is_none(),
        "the briefing sends no token"
    );
    let path = req.uri().path().to_string();
    let q = req.uri().query().unwrap_or_default().to_string();
    let t = in_period();
    let repo_url = |r: &str| format!("https://api.github.invalid/repos/acme/{r}");
    let body = match path.as_str() {
        "/orgs/acme/repos" => json!([
            {"name": "alpha", "private": false, "archived": false, "pushed_at": iso(t)},
            {"name": "beta", "private": false, "archived": false, "pushed_at": iso(t - 40 * 86400)},
            {"name": "secret", "private": true, "archived": false, "pushed_at": iso(t)},
            {"name": "attic", "private": false, "archived": true, "pushed_at": iso(t)}
        ]),
        "/search/issues" if *m.searches_fail.lock().unwrap() => {
            return (StatusCode::FORBIDDEN, "rate limited").into_response()
        }
        "/search/issues" if q.contains("is:merged") => json!({"total_count": 3, "items": [
            {"number": 10, "title": "Add the <b>thing</b>", "repository_url": repo_url("alpha")},
            {"number": 11, "title": "Fix the other thing", "repository_url": repo_url("alpha")},
            {"number": 5, "title": "secret work", "repository_url": repo_url("secret")}
        ]}),
        "/search/issues" if q.contains("is:issue") => json!({"total_count": 1, "items": [
            {"number": 7, "title": "It broke", "repository_url": repo_url("alpha")}
        ]}),
        "/search/issues" if q.contains("is:open") => json!({"total_count": 1, "items": [
            {"number": 3, "title": "Waiting PR", "repository_url": repo_url("beta"),
             "created_at": iso(now() - 3 * 86400 - 60)}
        ]}),
        "/repos/acme/alpha/releases" => json!([
            {"tag_name": "v1.2.0", "draft": false, "published_at": iso(t)},
            {"tag_name": "v1.1.0", "draft": false, "published_at": iso(t - 30 * 86400)},
            {"tag_name": "v9.9.9", "draft": true, "published_at": iso(t)}
        ]),
        _ => return (StatusCode::NOT_FOUND, format!("mock has no {uri}")).into_response(),
    };
    axum::Json(body).into_response()
}

async fn mock() -> (Mock, String) {
    let m = Mock::default();
    let app = Router::new().fallback(github_mock).with_state(m.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (m, base)
}

fn cfg(base: &str) -> BriefingConfig {
    BriefingConfig {
        org: "acme".into(),
        api_url: base.into(),
        data_dir: None,
        markers: vec![warte::briefing::Marker {
            date: briefing::last_period(now()).1,
            text: "advisor moved to a cheaper model".into(),
        }],
        period_days: 2,
    }
}

fn rec(id: u64, run: u64, repo: &str, d: &str, r: &str) -> warte::hub::model::JobRecord {
    let t = in_period();
    warte::hub::model::JobRecord {
        d: d.into(),
        id,
        run,
        att: 1,
        repo: repo.into(),
        wf: "CI".into(),
        job: "test".into(),
        pool: "rust-cpu".into(),
        runner: "r1".into(),
        ev: "push".into(),
        c: t,
        s: Some(t + 10),
        e: Some(t + 100),
        r: r.into(),
        sig: None,
        br: Some("main".into()),
    }
}

async fn facts(base: &str) -> Facts {
    let (from, until) = briefing::last_period(now());
    let dir = tempfile::tempdir().unwrap();
    let hub = warte::hub::Hub::open(dir.path().to_str().unwrap(), "acme").unwrap();
    hub.ingest_ledger(
        vec![
            rec(1, 100, "alpha", &from, "success"),
            rec(2, 100, "alpha", &from, "failure"),
            rec(3, 101, "alpha", &until, "failure"),
            // a private repository's CI never reaches the briefing
            rec(4, 102, "secret", &from, "failure"),
        ],
        vec![],
    );
    let c = github::Client::new(base);
    briefing::build(&c, &cfg(base), Some(&hub), None, &from, &until, now())
        .await
        .unwrap()
}

// rivet: verifies REQ-BRIEF-001
#[tokio::test]
async fn the_facts_hold_public_repositories_only_and_come_from_records() {
    let (m, base) = mock().await;
    let f = facts(&base).await;
    assert_eq!(
        f.known_repos,
        vec!["alpha", "beta"],
        "private and archived repositories are left out"
    );
    let alpha = f.repos.iter().find(|r| r.repo == "alpha").expect("alpha");
    assert_eq!(
        alpha.merged.iter().map(|i| i.number).collect::<Vec<_>>(),
        vec![10, 11]
    );
    assert_eq!(alpha.closed_issues[0].number, 7);
    assert_eq!(
        alpha.releases,
        vec!["v1.2.0"],
        "only releases in the period, no drafts"
    );
    assert_eq!((alpha.ci_runs, alpha.ci_failed_jobs), (2, 2));
    assert!(f.repos.iter().all(|r| r.repo != "secret"));
    assert_eq!(f.open_prs[0].repo, "beta");
    assert_eq!(f.open_prs[0].days_open, 3);
    assert_eq!(
        (f.totals.merged, f.totals.releases, f.totals.repos),
        (2, 1, 1)
    );
    assert_eq!(f.markers.len(), 1);
    assert!(f.gaps.is_empty(), "{:?}", f.gaps);
    // only releases of repositories pushed in the period are asked for
    let seen = m.seen.lock().unwrap().clone();
    assert!(seen
        .iter()
        .any(|u| u.starts_with("/repos/acme/alpha/releases")));
    assert!(
        !seen.iter().any(|u| u.contains("/repos/acme/beta/releases")),
        "{seen:?}"
    );

    // a refused search is a gap, not an error and not an empty-looking period
    *m.searches_fail.lock().unwrap() = true;
    let f = facts(&base).await;
    assert_eq!(f.gaps.len(), 3, "{:?}", f.gaps);
    assert!(f.gaps[0].contains("rate limit"));
}

fn text(line: &str, b: [&str; 3]) -> Text {
    Text {
        line: line.into(),
        bullets: b.iter().map(|s| s.to_string()).collect(),
    }
}

// rivet: verifies REQ-BRIEF-002
#[tokio::test]
async fn a_text_may_name_only_what_the_facts_hold() {
    let (_m, base) = mock().await;
    let f = facts(&base).await;
    // the template always passes its own check
    briefing::check(&briefing::template(&f), &f).unwrap();
    let good = text(
        "The helpers finished 2 changes and published a new version of alpha.",
        [
            "Done: alpha got 2 fixes (#10, #11) and release v1.2.0.",
            "Caught: the checks failed 2 times and held the work back until it passed.",
            "Waiting for you: one change in beta, open for 3 days.",
        ],
    );
    briefing::check(&good, &f).unwrap();
    // the period's own dates are facts: a model may name the year and day
    let (y, m, d) = (&f.until[..4], &f.until[5..7], &f.until[8..10]);
    let dated = text(
        &format!(
            "In {y}, up to {}.{}, alpha moved on.",
            d.trim_start_matches('0'),
            m
        ),
        [
            &format!("Done on {}: alpha got 2 fixes (#10, #11).", f.until),
            "Caught: the checks failed 2 times.",
            "Waiting for you: one change in beta.",
        ],
    );
    briefing::check(&dated, &f).unwrap();
    // but a year outside the period is still refused
    let next_year = y.parse::<u64>().unwrap() + 1;
    let other = text(
        &format!("By {next_year} alpha moved on."),
        [
            "Done: alpha.",
            "Caught: nothing.",
            "Waiting for you: nothing.",
        ],
    );
    assert!(briefing::check(&other, &f)
        .unwrap_err()
        .contains(&next_year.to_string()));
    for (what, bad, want) in [
        (
            "an invented number",
            // above 31 and no year: never one of the period's dates
            text("They finished 47 changes.", ["a", "b", "c"]),
            "47",
        ),
        ("an invented PR", text("Fixed #99.", ["a", "b", "c"]), "99"),
        (
            "an invented version",
            text("Released v2.0.0.", ["a", "b", "c"]),
            "v2.0.0",
        ),
        (
            "a link",
            text("See https://example.invalid", ["a", "b", "c"]),
            "link",
        ),
        (
            "markup",
            text("<script>x</script>", ["a", "b", "c"]),
            "markup",
        ),
        (
            "two bullets",
            Text {
                line: "x".into(),
                bullets: vec!["a".into(), "b".into()],
            },
            "three",
        ),
        (
            "too long",
            text(&"word ".repeat(140), ["a", "b", "c"]),
            "words",
        ),
    ] {
        let err = briefing::check(&bad, &f).expect_err(what);
        assert!(err.contains(want), "{what}: {err}");
    }
    // the open PR in beta makes beta active for the waiting bullet only
    let mut f2 = f.clone();
    f2.open_prs.clear();
    let err = briefing::check(&good, &f2).unwrap_err();
    assert!(err.contains("names beta"), "{err}");
}

#[test]
fn periods_are_two_closed_days_that_tile() {
    for t in [
        1_794_355_200_i64,
        1_794_355_200 + 86_400,
        1_794_355_200 + 86_399,
    ] {
        let (from, until) = briefing::last_period(t);
        let a = warte::hub::report::day_start(&from).unwrap();
        let b = warte::hub::report::day_start(&until).unwrap();
        assert_eq!(b - a, 86_400, "two days");
        assert!(b + 86_400 <= t, "the period is over");
        let (next, _) = briefing::last_period(t + 2 * 86_400);
        assert_eq!(
            warte::hub::report::day_start(&next).unwrap(),
            b + 86_400,
            "periods tile"
        );
    }
}

// rivet: verifies REQ-BRIEF-004
#[test]
fn a_one_day_period_is_yesterday_and_the_template_says_day() {
    for t in [1_794_355_200_i64, 1_794_355_200 + 1, 1_794_355_200 + 86_399] {
        let (from, until) = briefing::last_period_of(t, 1);
        assert_eq!(from, until, "one day");
        let a = warte::hub::report::day_start(&from).unwrap();
        assert_eq!(a, t - t.rem_euclid(86_400) - 86_400, "yesterday");
        let (next, _) = briefing::last_period_of(t + 86_400, 1);
        assert_eq!(
            warte::hub::report::day_start(&next).unwrap(),
            a + 86_400,
            "days tile"
        );
    }
    // a seven-day period is seven days, and the default is still two
    let (from, until) = briefing::last_period_of(1_794_355_200, 7);
    let span = warte::hub::report::day_start(&until).unwrap()
        - warte::hub::report::day_start(&from).unwrap();
    assert_eq!(span, 6 * 86_400);
    assert_eq!(
        briefing::last_period(1_794_355_200),
        briefing::last_period_of(1_794_355_200, 2)
    );

    let quiet = |from: &str, until: &str| {
        briefing::template(&Facts {
            from: from.into(),
            until: until.into(),
            ..Default::default()
        })
        .line
    };
    assert_eq!(
        quiet("2026-11-11", "2026-11-11"),
        "A quiet day: nothing new was finished."
    );
    assert_eq!(
        quiet("2026-11-10", "2026-11-11"),
        "A quiet two days: nothing new was finished."
    );
    // a model text may say "one day" of a one-day period
    let f = Facts {
        from: "2026-11-11".into(),
        until: "2026-11-11".into(),
        ..Default::default()
    };
    assert_eq!(f.days(), 1);

    let parsed: BriefingConfig = toml::from_str("org = \"acme\"").unwrap();
    assert_eq!(parsed.period_days, 2, "absent means two days");
    let one: BriefingConfig = toml::from_str("org = \"acme\"\nperiod_days = 1").unwrap();
    assert!(one.check().is_ok());
    for bad in [0, 8, -1] {
        let c = BriefingConfig {
            period_days: bad,
            ..one.clone()
        };
        assert!(c.check().unwrap_err().contains("period_days"), "{bad}");
    }
}

struct Server {
    _dir: tempfile::TempDir,
    base: String,
    token: String,
}

async fn serve(gh: &str) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let hub_dir = dir.path().join("hub");
    std::fs::create_dir_all(&hub_dir).unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: Some(warte::config::HubConfig {
            udp: "127.0.0.1:0".into(),
            allow: vec![Ipv4Addr::LOCALHOST.into()],
            data_dir: hub_dir.to_str().unwrap().into(),
            org: "acme".into(),
            probe_self: false,
            costs_file: None,
            skill_usage_file: None,
        }),
        briefing: Some(cfg(gh)),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let token = state
        .tokens
        .issue_token("briefing-job", vec!["briefing:write".into()])
        .unwrap();
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    Server {
        _dir: dir,
        base,
        token,
    }
}

async fn get(base: &str, p: &str) -> serde_json::Value {
    reqwest::get(format!("{base}{p}"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn post(s: &Server, token: Option<&str>, body: &serde_json::Value) -> (u16, String) {
    let mut r = reqwest::Client::new()
        .post(format!("{}/api/briefing", s.base))
        .header("Content-Type", "application/json")
        .body(body.to_string());
    if let Some(t) = token {
        r = r.bearer_auth(t);
    }
    let r = r.send().await.unwrap();
    (r.status().as_u16(), r.text().await.unwrap())
}

// rivet: verifies REQ-BRIEF-002
// rivet: verifies REQ-BRIEF-003
#[tokio::test]
async fn a_model_text_replaces_the_template_only_when_it_fits_the_facts() {
    let (_m, gh) = mock().await;
    let s = serve(&gh).await;
    // the template briefing is written at start
    let mut v = serde_json::Value::Null;
    for _ in 0..50 {
        v = get(&s.base, "/api/briefing").await;
        if !v["briefing"].is_null() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(v["briefing"]["source"], "template", "{v}");
    let facts = get(&s.base, "/api/briefing/facts").await;
    let from = facts["from"].as_str().unwrap().to_string();
    assert_eq!(facts["rules"]["bullets"], 3);

    let good = json!({"from": from, "model": "claude-haiku-5-5",
        "line": "The helpers finished 2 changes in alpha and published one new version.",
        "bullets": ["Done: two fixes in alpha and release v1.2.0.",
                    "Caught: the checks failed 2 times and held the work back.",
                    "Waiting for you: one change in beta, open for 3 days."]});
    assert_eq!(post(&s, None, &good).await.0, 401, "writing needs a token");
    let mut bad = good.clone();
    bad["line"] = "The helpers finished 40 changes in secret.".into();
    let (code, why) = post(&s, Some(&s.token), &bad).await;
    assert_eq!(code, 422, "{why}");
    let v = get(&s.base, "/api/briefing").await;
    assert_eq!(v["briefing"]["source"], "template", "the template stays");
    assert!(v["briefing"]["rejected"].as_str().unwrap().contains("40"));

    let (code, why) = post(&s, Some(&s.token), &good).await;
    assert_eq!(code, 201, "{why}");
    let v = get(&s.base, "/api/briefing").await;
    assert_eq!(v["briefing"]["source"], "model");
    assert_eq!(v["briefing"]["model"], "claude-haiku-5-5");
    assert!(v["briefing"]["rejected"].is_null());
    assert_eq!(v["periods"][0], from.as_str());

    let html = reqwest::get(format!("{}/api/dashboard/briefing-partial", s.base))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    for want in [
        "The helpers finished 2 changes",
        "For the IT reader",
        "https://github.com/acme/alpha/pull/10",
        "v1.2.0",
        "advisor moved to a cheaper model",
        "Open for more than a day",
    ] {
        assert!(html.contains(want), "{want} missing: {html}");
    }
    // GitHub's titles are escaped, and the private repository never shows
    assert!(
        !html.contains("<b>thing</b>") && !html.contains("secret"),
        "{html}"
    );

    // a period that is not a date is not a file name
    let r = reqwest::get(format!("{}/api/briefing?from=../../etc", s.base))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 404);
}

// rivet: verifies REQ-BRIEF-002
#[test]
fn the_template_claims_counts_only_and_counts_past_the_list() {
    let mut f = Facts {
        known_repos: vec!["alpha".into(), "beta".into()],
        repos: vec![
            warte::briefing::RepoFacts {
                repo: "alpha".into(),
                merged: vec![warte::briefing::Item {
                    number: 1,
                    title: "x".into(),
                }],
                ci_runs: 9,
                ci_failed_jobs: 4,
                ..Default::default()
            },
            warte::briefing::RepoFacts {
                repo: "beta".into(),
                ci_runs: 3,
                ci_failed_jobs: 7,
                ..Default::default()
            },
        ],
        open_prs: (0..20)
            .map(|i| warte::briefing::OpenPr {
                repo: "alpha".into(),
                number: 100 + i,
                title: "t".into(),
                days_open: 2,
            })
            .collect(),
        open_prs_total: 26,
        ..Default::default()
    };
    f.finish();
    let t = briefing::template(&f);
    let all = format!("{} {}", t.line, t.bullets.join(" "));
    // what the facts cannot say is not said
    for never in ["agents", "until they passed", "held"] {
        assert!(!all.contains(never), "{never}: {all}");
    }
    assert!(
        t.line
            .starts_with("In the last two days 1 change was finished in 1 project"),
        "{}",
        t.line
    );
    assert_eq!(
        t.bullets[1],
        "Caught: the automatic checks flagged 11 failed test jobs, most in beta (7) and alpha (4)."
    );
    assert!(
        t.bullets[2].starts_with("Waiting for you: 26 changes are open"),
        "the list holds 20, the count is 26: {}",
        t.bullets[2]
    );
    briefing::check(&t, &f).unwrap();
}

/// Run the operator kit's briefing job against `s` with a fake model that
/// prints `answers` in turn, one per call. Returns the exit status and stderr.
async fn run_job(s: &Server, answers: &[serde_json::Value]) -> (bool, String) {
    let dir = tempfile::tempdir().unwrap();
    for (i, a) in answers.iter().enumerate() {
        std::fs::write(dir.path().join(format!("answer{}", i + 1)), a.to_string()).unwrap();
    }
    std::fs::write(dir.path().join("token"), &s.token).unwrap();
    // the fake model counts its calls and reads the prompt like a model would
    let model = format!(
        "d={d}; n=$(( $(cat $d/n 2>/dev/null || echo 0) + 1 )); echo $n > $d/n; \
         cat > $d/prompt$n; cat $d/answer$n",
        d = dir.path().display()
    );
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/deploy/operator/jobs/briefing.sh"
    );
    let base = s.base.clone();
    let tok = dir.path().join("token");
    let out = tokio::task::spawn_blocking(move || {
        std::process::Command::new("sh")
            .arg(script)
            .env("WARTE_URL", base)
            .env("WARTE_TOKEN_FILE", tok)
            .env_remove("CREDENTIALS_DIRECTORY")
            .env("MODEL_CMD", model)
            .env("MODEL_ID", "claude-haiku-5-5")
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let prompts: Vec<String> = (1..=answers.len())
        .filter_map(|n| std::fs::read_to_string(dir.path().join(format!("prompt{n}"))).ok())
        .collect();
    (
        out.status.success(),
        format!(
            "{}\n--- prompts ---\n{}",
            String::from_utf8_lossy(&out.stderr),
            prompts.join("\n---\n")
        ),
    )
}

// rivet: verifies REQ-BRIEF-005
#[tokio::test(flavor = "multi_thread")]
async fn the_briefing_job_retries_a_refused_answer_once_with_the_reason() {
    let (_m, gh) = mock().await;
    let s = serve(&gh).await;
    for _ in 0..50 {
        if !get(&s.base, "/api/briefing").await["briefing"].is_null() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let two = json!({"line": "A quiet day.", "bullets": ["Done: little.", "Nothing waits."]});
    let wrong = json!({"line": "The helpers finished 40 changes.",
        "bullets": ["Done: 40 changes.", "Nothing went wrong.", "Nothing waits."]});
    let good = json!({
        "line": "The helpers finished 2 changes in alpha and published one new version.",
        "bullets": ["Done: two fixes in alpha and release v1.2.0.",
                    "Caught: the checks failed 2 times and held the work back.",
                    "Waiting for you: one change in beta, open for 3 days."]});

    // two bad answers: the job gives up, the plain version stays
    let (ok, log) = run_job(&s, &[two.clone(), wrong.clone()]).await;
    assert!(!ok, "{log}");
    assert!(log.contains("exactly three bullets"), "{log}");
    let v = get(&s.base, "/api/briefing").await;
    assert_eq!(v["briefing"]["source"], "template", "{v}");

    // the second try is told why the first was refused, and passes
    let (ok, log) = run_job(&s, &[wrong, good]).await;
    assert!(ok, "{log}");
    let second = log.rsplit("\n---\n").next().unwrap();
    assert!(
        second.contains("was refused by the check") && second.contains("40"),
        "{log}"
    );
    let v = get(&s.base, "/api/briefing").await;
    assert_eq!(v["briefing"]["source"], "model", "{v}");

    // a period a model already wrote is left alone, without a model call
    let (ok, log) = run_job(&s, &[]).await;
    assert!(ok, "{log}");
}

// rivet: verifies REQ-BRIEF-006
#[test]
fn a_briefing_after_missed_days_catches_up_and_replaces_the_plain_ones() {
    let dir = tempfile::tempdir().unwrap();
    let store = briefing::Store::open(dir.path().to_str().unwrap()).unwrap();
    let day = |d: &str| Facts {
        from: d.into(),
        until: d.into(),
        ..Default::default()
    };
    // 9 Nov written by a model; 10 and 11 Nov stayed plain
    let mut sun = briefing::from_facts(day("2026-11-09"), 0);
    sun.source = "model".into();
    store.put(&sun).unwrap();
    store
        .put(&briefing::from_facts(day("2026-11-10"), 0))
        .unwrap();
    store
        .put(&briefing::from_facts(day("2026-11-11"), 0))
        .unwrap();

    assert!(store.covers("2026-11-11"));
    assert!(!store.covers("2026-11-12"));
    // 12 Nov's briefing starts the day after the last model-written one
    let (start, replaces) = store.catch_up_from("2026-11-12", "2026-11-12");
    assert_eq!(start, "2026-11-10");
    assert_eq!(replaces, vec!["2026-11-11", "2026-11-10"]);
    let f = Facts {
        from: start.clone(),
        until: "2026-11-12".into(),
        ..Default::default()
    };
    store.keep(f, replaces, 1).unwrap();
    // one entry for 10..12 Nov; the plain 11 Nov is gone, the model's 9 Nov kept
    assert_eq!(store.periods(), vec!["2026-11-10", "2026-11-09"]);
    let b = store.latest().unwrap();
    assert_eq!(
        (b.facts.from.as_str(), b.facts.until.as_str()),
        ("2026-11-10", "2026-11-12")
    );
    assert_eq!(b.replaces.len(), 2);
    assert!(store.covers("2026-11-11") && store.covers("2026-11-12"));
    assert_eq!(store.get("2026-11-09").unwrap().source, "model");

    // a job that read the facts before the period grew is told to read again
    let sub = |until: Option<&str>| {
        let mut v = json!({"from": "2026-11-10", "model": "m", "line": "A quiet stretch: nothing new was finished.",
            "bullets": ["Nothing was finished.", "Nothing went wrong.", "Nothing waits for you."]});
        if let Some(u) = until {
            v["until"] = json!(u);
        }
        serde_json::from_value::<briefing::Submission>(v).unwrap()
    };
    let err = store.submit(sub(Some("2026-11-10")), 2).unwrap_err();
    assert!(err.contains("read the facts again"), "{err}");
    assert_eq!(store.latest().unwrap().source, "template");
    // with the right until, or none (an older job), the text is checked as before
    store.submit(sub(Some("2026-11-12")), 2).unwrap();
    assert_eq!(store.latest().unwrap().source, "model");
}

// rivet: verifies REQ-BRIEF-006
#[test]
fn a_catch_up_stops_at_seven_days_and_at_a_gap() {
    let dir = tempfile::tempdir().unwrap();
    let store = briefing::Store::open(dir.path().to_str().unwrap()).unwrap();
    // ten plain days in a row: 1..10 Nov
    for d in 1..=10 {
        let d = format!("2026-11-{d:02}");
        store
            .put(&briefing::from_facts(
                Facts {
                    from: d.clone(),
                    until: d,
                    ..Default::default()
                },
                0,
            ))
            .unwrap();
    }
    let (start, replaces) = store.catch_up_from("2026-11-11", "2026-11-11");
    assert_eq!(start, "2026-11-05", "11 Nov and the six days before: seven");
    assert_eq!(replaces.len(), 6);
    // a missing day breaks the chain: only the briefings after it join
    let dir = tempfile::tempdir().unwrap();
    let store = briefing::Store::open(dir.path().to_str().unwrap()).unwrap();
    for d in ["2026-11-07", "2026-11-09"] {
        store
            .put(&briefing::from_facts(
                Facts {
                    from: d.into(),
                    until: d.into(),
                    ..Default::default()
                },
                0,
            ))
            .unwrap();
    }
    assert_eq!(
        store.catch_up_from("2026-11-10", "2026-11-10"),
        ("2026-11-09".to_string(), vec!["2026-11-09".to_string()])
    );
    // a two-day period catches up two-day periods
    assert_eq!(
        store.catch_up_from("2026-11-12", "2026-11-13").0,
        "2026-11-12"
    );
}

fn job(
    run: u64,
    repo: &str,
    wf: &str,
    ev: &str,
    br: Option<&str>,
    r: &str,
) -> warte::hub::model::JobRecord {
    let mut j = rec(run * 10, run, repo, "2026-11-18", r);
    j.wf = wf.into();
    j.ev = ev.into();
    j.br = br.map(String::from);
    j.e = Some(1_795_000_000 + run as i64);
    j
}

// rivet: verifies REQ-BRIEF-007
#[test]
fn red_now_is_the_latest_run_on_the_default_branch_or_of_a_schedule() {
    let public: std::collections::BTreeSet<String> = ["alpha", "beta", "gamma", "delta"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let defaults: std::collections::BTreeMap<String, String> = [
        ("alpha", "main"),
        ("beta", "main"),
        ("gamma", "trunk"),
        ("delta", "main"),
    ]
    .iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    let recs = vec![
        // alpha: main failed, then green: not red
        job(1, "alpha", "CI", "push", Some("main"), "failure"),
        job(2, "alpha", "CI", "push", Some("main"), "success"),
        // beta: main green, then failed: red; one job of the run passed
        job(3, "beta", "CI", "push", Some("main"), "success"),
        job(4, "beta", "CI", "push", Some("main"), "failure"),
        job(4, "beta", "CI", "push", Some("main"), "success"),
        // planted: a failure on another branch, and on a default branch not called main
        job(5, "alpha", "CI", "push", Some("feature"), "failure"),
        job(6, "gamma", "CI", "push", Some("trunk"), "timed_out"),
        // scheduled: delta's nightly failed and stays red; alpha's failed, then green
        job(7, "delta", "Nightly", "schedule", None, "failure"),
        job(8, "alpha", "Nightly", "schedule", None, "failure"),
        job(9, "alpha", "Nightly", "schedule", None, "success"),
        // a pull request failure is not a red main
        job(10, "delta", "CI", "pull_request", Some("main"), "failure"),
        // planted: no branch (an older agent), and a repository that is not public
        job(11, "delta", "CI", "push", None, "failure"),
        job(12, "private-one", "CI", "push", Some("main"), "failure"),
    ];
    let (red, unplaced) = briefing::red_runs(&recs, &public, &defaults);
    let got: Vec<(String, String, String, u64)> = red
        .iter()
        .map(|r| (r.kind.clone(), r.repo.clone(), r.workflow.clone(), r.run))
        .collect();
    assert_eq!(
        got,
        vec![
            ("main".into(), "beta".into(), "CI".into(), 4),
            ("main".into(), "gamma".into(), "CI".into(), 6),
            ("schedule".into(), "delta".into(), "Nightly".into(), 7),
        ]
    );
    assert_eq!(
        unplaced, 1,
        "a push job without a branch is counted, never red"
    );
}

// rivet: verifies REQ-BRIEF-007
#[test]
fn the_view_strikes_through_done_items_and_lists_red_now_first() {
    let dir = tempfile::tempdir().unwrap();
    let store = briefing::Store::open(dir.path().to_str().unwrap()).unwrap();
    let (from, until) = briefing::last_period(now());
    let mut f = Facts {
        org: "example-org".into(),
        from,
        until,
        known_repos: vec!["alpha".into(), "beta".into()],
        ..Default::default()
    };
    f.repos.push(briefing::RepoFacts {
        repo: "alpha".into(),
        merged: vec![briefing::Item {
            number: 12,
            title: "Fix <b>bold</b> parser".into(),
        }],
        closed_issues: vec![briefing::Item {
            number: 9,
            title: "Crash on start".into(),
        }],
        ..Default::default()
    });
    f.open_prs.push(briefing::OpenPr {
        repo: "beta".into(),
        number: 3,
        title: "Still open".into(),
        days_open: 4,
    });
    f.red.push(briefing::RedRun {
        repo: "beta".into(),
        workflow: "Nightly".into(),
        kind: "schedule".into(),
        run: 77,
        at: 1_795_000_000,
    });
    store.keep(f, vec![], 1).unwrap();
    let html = warte::dashboard::briefing::view(Some(&store), None);
    assert!(html.contains("<b>Red now</b>"), "{html}");
    assert!(html.contains("actions/runs/77"), "{html}");
    assert!(
        html.find("Red now").unwrap() < html.find("brief-line").unwrap(),
        "red first"
    );
    assert!(html.contains(r#"<s>alpha <a href="https://github.com/example-org/alpha/pull/12">#12</a> Fix &lt;b&gt;bold&lt;/b&gt; parser</s>"#), "{html}");
    assert!(
        html.contains("/issues/9\">#9</a> Crash on start</s>"),
        "{html}"
    );
    // an open pull request is not struck through
    let open = html.find("Still open").unwrap();
    assert!(
        !html[..open].ends_with("<s>") && !html[open..open + 30].contains("</s>"),
        "{html}"
    );
    // nothing red: no block
    let dir2 = tempfile::tempdir().unwrap();
    let quiet = briefing::Store::open(dir2.path().to_str().unwrap()).unwrap();
    let (from, until) = briefing::last_period(now());
    quiet
        .keep(
            Facts {
                org: "example-org".into(),
                from,
                until,
                ..Default::default()
            },
            vec![],
            1,
        )
        .unwrap();
    assert!(!warte::dashboard::briefing::view(Some(&quiet), None).contains("Red now"));
}

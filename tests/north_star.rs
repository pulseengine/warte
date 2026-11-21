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
        ..Default::default()
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
        stale_after_days: 31,
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

// ---- the North Star view (REQ-NS-005) ----

/// The published file in its 2026-11-20 shape, cut down.
const BOARD: &str = r#"
north_star = "A safety-critical product team can take a component to a microcontroller."
updated = "2026-11-20"
[[products]]
name = "relay"
north_star = "Flight-software components, machine-checked."
pulls = ["O2", "O3"]
[[products]]
name = "wohl"
north_star = "A home supervision system you can leave alone."
pulls = ["O2"]
[[tool_repos]]
name = "gale"
pulls = ["O3"]
[[objectives]]
id = "O1"
title = "Runs on target"
summary = "A component runs on a real microcontroller."
indicator = "Per target: the ladder as pass or fail per rung."
guard = "A skipped hardware run counts as a failure."
state = "unmeasured"
[[objectives]]
id = "O2"
title = "One-command evidence bundle"
summary = "A consumer release carries one verifiable bundle."
indicator = "Consumer repositories whose latest release carries the full bundle."
guard = "The verify command must fail when any piece of evidence is removed."
state = "measured"
[[objectives]]
id = "O4"
title = "Not only one person <script>"
indicator = "Merged pull requests per month by people other than the maintainer."
guard = "Reverted or reopened work."
state = "measured"
[[periods]]
id = "2026-Q4"
layers = ["2026.11", "2026.12"]
[[periods.commitments]]
objective = "O2"
kind = "committed"
target = "One wohl release carries the full bundle."
[[periods]]
id = "2027-Q1"
layers = ["2027.01", "2027.02", "2027.03"]
[[periods.commitments]]
objective = "O2"
kind = "stretch"
target = "relay adds the MC/DC part."
"#;

const O2: &str = r#"{"schema_version":1,"objective":"O2","measured_at":"2026-11-17T19:27Z",
"verifier":"sigil wsc v0.12.1","value":1,"of":2,
"repos":[{"repo":"relay","release":"falcon-v1.142.0","bundle":null,"verify":"absent","controls":null,"counts":false},
{"repo":"wohl","release":"v0.7.0","verify":"pass","controls":{"deleted evidence file":"red","altered signed component":"red"},"counts":true}],
"blind_spot":"inner bundle consistency only; the cosign/SLSA outer chain is not checked"}"#;

const O4: &str = r#"{"schema_version":1,"objective":"O4","measured_at":"2026-11-18T11:32Z",
"verifier":"GitHub search","period":"2026-10-01..2026-10-31","value":1,"of":839,
"items":[{"pr":"loom#428","url":"https://github.com/pulseengine/loom/pull/428"},{"pr":"x#1","url":"javascript:alert(1)"}],
"not_counted":[{"product":"jess","reason":"no release yet"}],
"blind_spot":"time from ready to first pick-up is not measured yet"}"#;

async fn view_of(files: &Files, n: &NorthStar, now: i64) -> String {
    let _ = files;
    n.poll(now).await;
    let last = n.last_poll.lock().unwrap().clone();
    warte::northstar::view::page(
        n.board().as_ref(),
        &n.rows(now),
        &n.store.all(),
        last.as_ref(),
        now,
        n.config.stale_after_days,
    )
}

/// The card of one objective, from its opening tag to the next card.
fn card<'a>(page: &'a str, id: &str) -> &'a str {
    let start = page
        .find(&format!(r#"id="ns-{id}""#))
        .unwrap_or_else(|| panic!("no card for {id}"));
    let rest = &page[start..];
    &rest[..rest.find("</article>").unwrap()]
}

fn ns(base: String) -> (tempfile::TempDir, NorthStar) {
    let dir = tempfile::tempdir().unwrap();
    let n = NorthStar::new(
        NorthStarConfig {
            base_url: base,
            interval_s: 3600,
            stale_after_days: 31,
        },
        dir.path(),
    );
    (dir, n)
}

// rivet: verifies REQ-NS-005
#[tokio::test]
async fn the_view_shows_the_published_files_and_moves_when_they_change() {
    let files: Files = Arc::new(Mutex::new(BTreeMap::new()));
    {
        let mut f = files.lock().unwrap();
        f.insert("north-star.toml".into(), BOARD.into());
        f.insert("north-star/O2.json".into(), O2.into());
        f.insert("north-star/O4.json".into(), O4.into());
    }
    let (_d, n) = ns(serve(files.clone()).await);
    let now = day("2026-11-20T08:00Z");
    let page = view_of(&files, &n, now).await;

    assert!(page.contains("A safety-critical product team"));
    assert!(page.contains("Updated 2026-11-20"));
    // O2: the value with its denominator, the evidence rows and the controls
    let o2 = card(&page, "O2");
    assert!(o2.contains("<strong>1 of 2</strong>"), "{o2}");
    assert!(o2.contains(r#"aria-label="1 of 2""#));
    assert!(o2.contains("measured 2026-11-17T19:27Z with sigil wsc v0.12.1"));
    assert!(o2.contains("first observation"));
    assert!(o2.contains(r#"<a href="https://github.com/pulseengine/wohl/releases/tag/v0.7.0" rel="noopener">wohl</a> v0.7.0: pass"#), "{o2}");
    assert!(o2.contains("deleted evidence file red"));
    assert!(o2.contains("relay</a> falcon-v1.142.0: absent"));
    assert!(o2.contains("does not count"));
    // guard and blind spot always shown, the members listed
    assert!(o2.contains("The verify command must fail"));
    assert!(o2.contains("inner bundle consistency only"));
    assert!(o2.contains(">relay</a>") && o2.contains(">wohl</a>"));
    // the current period's commitment, marked, and the next one
    assert!(o2.contains("2026-Q4 (now)") && o2.contains("One wohl release carries"));
    assert!(o2.contains("2027-Q1</span>") && o2.contains("relay adds the MC/DC part"));
    // the trend is warte's own, per layer, never interpolated
    assert!(o2.contains("observed by warte, not the official per-layer series"));
    assert!(o2.contains(r#"<span class="ns-layer">2026.11</span> 1 of 2"#));
    assert!(o2.contains(r#"<span class="ns-layer">2026.12</span> —"#));
    // O1 is declared unmeasured and has no file: the word, never a number
    let o1 = card(&page, "O1");
    assert!(o1.contains(">unmeasured</p>"), "{o1}");
    assert!(
        !o1.contains(" of "),
        "an unmeasured objective shows no value: {o1}"
    );
    assert!(o1.contains("A skipped hardware run counts as a failure."));
    assert!(o1.contains("no product or tool repository names it"));
    // O4: items link only over https, published text is escaped
    let o4 = card(&page, "O4");
    assert!(o4.contains(
        r#"<a href="https://github.com/pulseengine/loom/pull/428" rel="noopener">loom#428</a>"#
    ));
    assert!(
        o4.contains("<li>x#1</li>"),
        "a javascript: URL is not a link: {o4}"
    );
    assert!(!page.contains("javascript:"));
    assert!(o4.contains("Not only one person &lt;script&gt;"));
    assert!(!page.contains("<script>"));
    assert!(o4.contains("Not counted: jess (no release yet)."));
    assert!(o4.contains("period 2026-10-01..2026-10-31"));
    // no percentage outside the bar's own width
    let text: String = page
        .split("style=\"")
        .enumerate()
        .map(|(i, s)| {
            if i == 0 {
                s
            } else {
                s.split_once('"').map_or("", |x| x.1)
            }
        })
        .collect();
    assert!(!text.contains('%'), "a percentage without its denominator");

    // planted: the publisher measures again; the page follows
    files.lock().unwrap().insert(
        "north-star/O2.json".into(),
        O2.replace("2026-11-17T19:27Z", "2026-12-02T10:00Z")
            .replace(r#""value":1"#, r#""value":2"#),
    );
    let later = day("2026-12-03T08:00Z");
    let page = view_of(&files, &n, later).await;
    let o2 = card(&page, "O2");
    assert!(o2.contains("<strong>2 of 2</strong>"), "{o2}");
    assert!(o2.contains("was 1 of 2 on 2026-11-17T19:27Z"));
    assert!(o2.contains(r#"<span class="ns-layer">2026.11</span> 1 of 2"#));
    assert!(o2.contains(r#"<span class="ns-layer">2026.12</span> 2 of 2"#));
    // planted: O4's file goes; one poll later it reads unmeasured
    files.lock().unwrap().remove("north-star/O4.json");
    let page = view_of(&files, &n, later).await;
    assert!(card(&page, "O4").contains(">unmeasured</p>"));
}

// rivet: verifies REQ-NS-005
#[tokio::test]
async fn stale_off_target_and_an_empty_denominator_come_only_from_published_fields() {
    let with_target = BOARD.replace(
        "target = \"One wohl release carries the full bundle.\"",
        "target = \"One wohl release carries the full bundle.\"\ntarget_value = 2\ntarget_of = 2",
    );
    for (board, o2, now, want_state, want) in [
        // no target published: measured, never off-target
        (
            BOARD.to_string(),
            O2.to_string(),
            "2026-11-20T08:00Z",
            "measured",
            "<strong>1 of 2</strong>",
        ),
        // a target above the value: off-target, the target shown
        (
            with_target.clone(),
            O2.to_string(),
            "2026-11-20T08:00Z",
            "off-target",
            "target 2 of 2",
        ),
        // a target outside the current period does not apply
        (
            with_target.clone(),
            O2.to_string(),
            "2027-01-05T08:00Z",
            "stale",
            "<strong>1 of 2</strong>",
        ),
        // older than a month: stale, the age in red, the value still shown
        (
            BOARD.to_string(),
            O2.to_string(),
            "2026-12-20T08:00Z",
            "stale",
            "ns-age--stale",
        ),
        // nothing in the denominator
        (
            BOARD.to_string(),
            O2.replace(r#""value":1,"of":2"#, r#""value":0,"of":0"#),
            "2026-11-20T08:00Z",
            "measured",
            "no repo in the denominator",
        ),
        // declared unmeasured although a file exists: the declaration wins
        (
            BOARD.replace(
                "state = \"measured\"\n[[objectives]]\nid = \"O4\"",
                "state = \"unmeasured\"\n[[objectives]]\nid = \"O4\"",
            ),
            O2.to_string(),
            "2026-11-20T08:00Z",
            "unmeasured",
            ">unmeasured</p>",
        ),
    ] {
        let files: Files = Arc::new(Mutex::new(BTreeMap::new()));
        {
            let mut f = files.lock().unwrap();
            f.insert("north-star.toml".into(), board);
            f.insert("north-star/O2.json".into(), o2);
        }
        let (_d, n) = ns(serve(files.clone()).await);
        let page = view_of(&files, &n, day(now)).await;
        let c = card(&page, "O2");
        assert!(
            c.contains(&format!(r#"ns-state--{want_state}">{want_state}<"#)),
            "{now}: want {want_state}: {c}"
        );
        assert!(c.contains(want), "{now}: want {want}: {c}");
        if want_state == "unmeasured" {
            assert!(!c.contains(" of 2"), "{c}");
        }
    }
}

// rivet: verifies REQ-NS-005
#[test]
fn before_the_first_read_the_view_says_so() {
    let page = warte::northstar::view::page(None, &[], &[], None, 0, 31);
    assert!(
        page.contains("has not been read yet (not read yet)"),
        "{page}"
    );
}

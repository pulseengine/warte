//! Agent triage of alerts (REQ-ALERT-006): the gate decision, the measures,
//! the redacted run context, and gate mode across ticks.

use serde_json::{json, Value};
use warte::alerts::{notice, Event};
use warte::alerttriage::{
    decide, measures, runs_for, Gate, Mode, Push, Row, Store, Verdict, HOLD_S, RUNNER_STALE_S,
};
use warte::dashboard::fleet::Detected;

const T: i64 = 1_800_000_000;

// rivet: verifies REQ-ALERT-006
#[test]
fn red_and_escalations_page_whatever_the_verdict_and_silence_fails_open() {
    use Push::*;
    let poll = Some(T - 60);
    let noise = Some(Verdict::Noise);
    // shadow: everything pages
    assert_eq!(decide(Mode::Shadow, 1, false, noise, T, T, poll), Now);
    // gate: red and escalations page even when the agent says noise
    assert_eq!(decide(Mode::Gate, 2, false, noise, T, T, poll), Now);
    assert_eq!(decide(Mode::Gate, 1, true, noise, T, T, poll), Now);
    // gate, amber: the verdict decides
    assert_eq!(
        decide(Mode::Gate, 1, false, Some(Verdict::Real), T, T, poll),
        Now
    );
    assert_eq!(decide(Mode::Gate, 1, false, noise, T, T, poll), Drop);
    // no verdict: held until 30 minutes, then it pages
    assert_eq!(
        decide(
            Mode::Gate,
            1,
            false,
            None,
            T,
            T + HOLD_S - 1,
            Some(T + HOLD_S - 60)
        ),
        Hold
    );
    assert_eq!(
        decide(
            Mode::Gate,
            1,
            false,
            None,
            T,
            T + HOLD_S,
            Some(T + HOLD_S - 60)
        ),
        Now
    );
    // a runner that never polled, or is silent for 30 minutes, holds nothing
    assert_eq!(decide(Mode::Gate, 1, false, None, T, T, None), Now);
    assert_eq!(
        decide(Mode::Gate, 1, false, None, T, T, Some(T - RUNNER_STALE_S)),
        Now
    );
    assert_eq!(
        decide(
            Mode::Gate,
            1,
            false,
            None,
            T,
            T,
            Some(T - RUNNER_STALE_S + 1)
        ),
        Hold
    );
}

fn r(key: &str, agent: Option<Verdict>, owner: Option<Verdict>) -> Row {
    Row {
        id: format!("id-{key}"),
        key: key.into(),
        sev: 1,
        text: key.into(),
        opened_at: T,
        verdict: agent,
        reason: None,
        owner,
    }
}

// rivet: verifies REQ-ALERT-006
#[test]
fn measures_count_missed_real_with_denominators_and_name_the_wrong_keys() {
    use Verdict::*;
    let rows = vec![
        r("disk", Some(Noise), Some(Real)),
        r("disk", Some(Noise), Some(Real)),
        r("queue", Some(Real), Some(Noise)),
        r("github", Some(Noise), Some(Noise)),
        r("github", Some(Noise), Some(Noise)),
        r("house", Some(Real), Some(Real)),
        r("late", Some(Real), None),
        r("new", None, None),
    ];
    let m = measures(&rows);
    assert_eq!(m.labelled, 6);
    assert_eq!(m.agree, 3);
    assert_eq!(m.missed_real, 2);
    assert_eq!((m.owner_real, m.owner_noise), (3, 3));
    assert_eq!(m.noise_caught, 2);
    assert_eq!((m.unlabelled, m.no_verdict), (1, 1));
    assert_eq!(
        m.wrong_keys,
        [("disk".to_string(), 2), ("queue".to_string(), 1)]
    );
    assert!(!m.judgeable());
    // judged only at 30 labelled with 5 owner-real
    let mut many: Vec<Row> = (0..25).map(|_| r("x", Some(Noise), Some(Noise))).collect();
    many.extend((0..5).map(|_| r("y", Some(Real), Some(Real))));
    assert!(measures(&many).judgeable());
    many.pop();
    assert!(!measures(&many).judgeable());
}

// rivet: verifies REQ-ALERT-006
#[test]
fn the_run_context_names_no_runner_and_no_host() {
    let rec: warte::hub::model::JobRecord = serde_json::from_value(json!({
        "d": "2027-01-15", "id": 1, "run": 2, "att": 1, "repo": "relay", "wf": "ci",
        "job": "build", "pool": "linux-x64", "runner": "ci-host-07-runner-3", "ev": "push",
        "c": T, "s": T + 5, "e": T + 60, "r": "failure", "sig": "disk-full:ci-host-07"
    }))
    .unwrap();
    let other: warte::hub::model::JobRecord = serde_json::from_value(json!({
        "d": "2027-01-15", "id": 3, "run": 4, "att": 1, "repo": "spar", "wf": "ci",
        "job": "build", "pool": "macos", "runner": "", "ev": "push",
        "c": T, "s": null, "e": null, "r": "success"
    }))
    .unwrap();
    let runs = runs_for(&[rec, other], "ci:relay", "relay CI failing");
    assert_eq!(runs.len(), 1);
    let s = runs[0].to_string();
    assert!(!s.contains("ci-host-07"), "{s}");
    assert_eq!(runs[0]["signature"], "disk-full");
    assert_eq!(runs[0]["repo"], "relay");
}

async fn store() -> Store {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    Store::new(pool)
}

fn opened(key: &str, sev: u8) -> Value {
    notice(
        &Event::Opened(Detected {
            key: key.into(),
            sev,
            text: format!("{key} is off"),
        }),
        chrono::DateTime::from_timestamp(T, 0).unwrap(),
        None,
    )
}

fn cleared(key: &str) -> Value {
    notice(
        &Event::Cleared {
            key: key.into(),
            text: format!("{key} is off"),
        },
        chrono::DateTime::from_timestamp(T, 0).unwrap(),
        None,
    )
}

async fn record(s: &Store, n: &Value) -> String {
    let (key, sev, _) = warte::alerttriage::opened(n).unwrap();
    let id = n["id"].as_str().unwrap().to_string();
    s.record(&id, &key, sev, n["title"].as_str().unwrap(), T, &json!({}))
        .await
        .unwrap();
    id
}

// rivet: verifies REQ-ALERT-006
#[tokio::test]
async fn gate_mode_holds_amber_until_a_verdict_or_thirty_minutes() {
    let s = store().await;
    let mut g = Gate::default();

    // the runner never polled: an amber alert pages at once
    let a = opened("disk:a", 1);
    record(&s, &a).await;
    assert_eq!(g.step(&s, std::slice::from_ref(&a), T).await.len(), 1);

    // the runner polls: amber is held, red pages at once
    s.polled(T).await;
    let b = opened("disk:b", 1);
    let b_id = record(&s, &b).await;
    let red = opened("github:actions", 2);
    let red_id = record(&s, &red).await;
    s.verdict(&red_id, Verdict::Noise, "status page flaps", "m", T)
        .await
        .unwrap();
    let out = g.step(&s, &[b.clone(), red.clone()], T).await;
    assert_eq!(out, vec![red.clone()], "red pages whatever the verdict");

    // noise verdict: never pages, and neither does its clear
    s.verdict(&b_id, Verdict::Noise, "brief spike", "m", T + 60)
        .await
        .unwrap();
    s.polled(T + 60).await;
    assert!(g.step(&s, &[], T + 60).await.is_empty());
    assert!(g.step(&s, &[cleared("disk:b")], T + 120).await.is_empty());

    // real verdict: pages on the next tick
    let c = opened("disk:c", 1);
    let c_id = record(&s, &c).await;
    assert!(g
        .step(&s, std::slice::from_ref(&c), T + 120)
        .await
        .is_empty());
    s.verdict(&c_id, Verdict::Real, "filling fast", "m", T + 180)
        .await
        .unwrap();
    assert_eq!(g.step(&s, &[], T + 180).await, vec![c]);

    // no verdict: pages at 30 minutes while the runner keeps polling
    let d = opened("disk:d", 1);
    record(&s, &d).await;
    s.polled(T + 200).await;
    assert!(g
        .step(&s, std::slice::from_ref(&d), T + 200)
        .await
        .is_empty());
    s.polled(T + 200 + HOLD_S - 60).await;
    assert!(g.step(&s, &[], T + 200 + HOLD_S - 1).await.is_empty());
    assert_eq!(g.step(&s, &[], T + 200 + HOLD_S).await, vec![d]);

    // the runner falls silent: a held alert pages long before 30 minutes
    let e = opened("disk:e", 1);
    record(&s, &e).await;
    let at = T + 10 * HOLD_S;
    s.polled(at - RUNNER_STALE_S + 100).await;
    assert!(g.step(&s, std::slice::from_ref(&e), at).await.is_empty());
    assert!(g.step(&s, &[], at + 99).await.is_empty());
    assert_eq!(g.step(&s, &[], at + 100).await, vec![e]);
}

// rivet: verifies REQ-ALERT-006
#[tokio::test]
async fn a_verdict_is_taken_once_and_unknown_alerts_are_refused() {
    let s = store().await;
    let a = opened("queue:x", 1);
    let id = record(&s, &a).await;
    assert_eq!(s.pending(T).await.len(), 1);
    assert!(s.verdict(&id, Verdict::Real, "r", "m", T).await.is_ok());
    assert_eq!(
        s.verdict(&id, Verdict::Noise, "r", "m", T).await,
        Err(warte::alerttriage::VerdictError::Already)
    );
    assert_eq!(
        s.verdict("nope", Verdict::Noise, "r", "m", T).await,
        Err(warte::alerttriage::VerdictError::NotFound)
    );
    assert!(s.pending(T).await.is_empty());
    // the owner may change their mind
    assert!(s.owner(&id, Verdict::Noise, "u1", T).await);
    assert!(s.owner(&id, Verdict::Real, "u1", T).await);
    assert!(!s.owner("nope", Verdict::Real, "u1", T).await);
    assert_eq!(s.rows(T - 1).await[0].owner, Some(Verdict::Real));
}

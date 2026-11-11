//! Oracles for Coming up (REQ-PREDICT-001/002, DD-PREDICT-001).

use warte::forecast::{
    detected, fit, forecast, judge, panel, Forecast, Item, Outcome, HORIZON_DAYS, LIMIT_PCT,
};
use warte::history::History;

const H: i64 = 3600;
const D: i64 = 86400;
const NOW: i64 = 100 * D;
const DISK: &str = "disk.used_pct{mount=/var}";

/// Hourly points ending at NOW: `n` of them, `start` then `per_day` more each day.
fn rising(n: i64, start: f64, per_day: f64) -> Vec<(i64, f64)> {
    (0..n)
        .map(|i| {
            let t = NOW - (n - 1 - i) * H;
            (t, start + per_day * (i * H) as f64 / D as f64)
        })
        .collect()
}

// rivet: verifies DD-PREDICT-001
#[test]
fn a_straight_line_fits_exactly() {
    let f = fit(&rising(48, 50.0, 2.0)).unwrap();
    assert!((f.slope * D as f64 - 2.0).abs() < 1e-9, "{f:?}");
    assert!((f.r2 - 1.0).abs() < 1e-9);
    assert_eq!((f.n, f.span_s), (48, 47 * H));
    assert!(fit(&[(0, 1.0)]).is_none());
    // a flat line has no fit quality to speak of, and no rise
    let flat = fit(&[(0, 5.0), (H, 5.0), (2 * H, 5.0)]).unwrap();
    assert_eq!((flat.slope, flat.r2), (0.0, 0.0));
}

// rivet: verifies REQ-PREDICT-001
#[test]
fn a_steadily_filling_disk_is_forecast_with_its_evidence() {
    let pts = rising(48, 80.0, 1.0);
    let latest = pts.last().unwrap().1;
    let Outcome::Due(i) = judge("box", DISK, &pts, NOW) else {
        panic!("expected a forecast")
    };
    assert_eq!((i.host.as_str(), i.what.as_str()), ("box", "disk /var"));
    assert_eq!((i.span_h, i.points), (47, 48));
    assert!((i.per_day - 1.0).abs() < 1e-9 && i.r2 > 0.99);
    assert_eq!(i.now, latest);
    // at the current rate from the latest value, not from the line's start
    let days = (LIMIT_PCT - latest) / 1.0;
    assert!((i.days - days).abs() < 1e-9, "{}", i.days);
    assert_eq!(i.due_at, NOW + (days * D as f64) as i64);
    assert!(i.days <= HORIZON_DAYS);
    // memory is watched too
    assert!(matches!(
        judge("box", "mem.used_pct", &rising(48, 90.0, 2.0), NOW),
        Outcome::Due(Item { ref what, .. }) if what == "memory"
    ));
}

// rivet: verifies REQ-PREDICT-001
#[test]
fn too_little_data_is_said_not_guessed() {
    // too few points
    assert_eq!(
        judge("box", DISK, &rising(10, 90.0, 5.0), NOW),
        Outcome::NotEnough
    );
    // enough points, too short a span (20 points over 19 minutes)
    let short: Vec<(i64, f64)> = (0..20)
        .map(|i| (NOW - 60 * (19 - i), 90.0 + i as f64))
        .collect();
    assert_eq!(judge("box", DISK, &short, NOW), Outcome::NotEnough);
    assert_eq!(judge("box", DISK, &[], NOW), Outcome::NotEnough);
}

// rivet: verifies REQ-PREDICT-001
#[test]
fn flat_falling_noisy_far_or_already_full_is_not_a_forecast() {
    // flat
    assert_eq!(
        judge("box", DISK, &rising(48, 80.0, 0.0), NOW),
        Outcome::Steady
    );
    // falling
    assert_eq!(
        judge("box", DISK, &rising(48, 80.0, -3.0), NOW),
        Outcome::Steady
    );
    // rising, but too slowly to reach the limit within the horizon
    assert_eq!(
        judge("box", DISK, &rising(48, 50.0, 0.5), NOW),
        Outcome::Steady
    );
    // already over the limit: that is today's finding, not a forecast
    assert_eq!(
        judge("box", DISK, &rising(48, 95.0, 1.0), NOW),
        Outcome::Steady
    );
    // a swing around a rise that would reach the limit in days fits badly
    // and is held back
    let noisy: Vec<(i64, f64)> = rising(72, 85.0, 2.0)
        .into_iter()
        .enumerate()
        .map(|(i, (t, y))| (t, y + if i % 2 == 0 { 8.0 } else { -8.0 }))
        .collect();
    assert!(fit(&noisy).unwrap().r2 < 0.7);
    assert_eq!(judge("box", DISK, &noisy, NOW), Outcome::Steady);
    // only disks and memory are watched
    assert_eq!(
        judge("box", "load.1", &rising(48, 1.0, 5.0), NOW),
        Outcome::Steady
    );
}

fn item(host: &str, days: f64) -> Item {
    Item {
        host: host.into(),
        metric: DISK.into(),
        what: "disk /var".into(),
        now: 90.0,
        limit: 95.0,
        per_day: 5.0 / days,
        due_at: NOW + (days * D as f64) as i64,
        days,
        span_h: 48,
        points: 48,
        r2: 0.98,
    }
}

// rivet: verifies REQ-PREDICT-002
#[test]
fn a_forecast_due_within_three_days_is_a_finding_and_within_one_red() {
    let f = Forecast {
        items: vec![item("a", 0.5), item("b", 2.0), item("c", 10.0)],
        ..Default::default()
    };
    let d = detected(&f);
    assert_eq!(d.len(), 2, "{d:?}");
    assert_eq!((d[0].sev, d[1].sev), (2, 1));
    assert_eq!(d[0].key, format!("forecast:a:{DISK}"));
    assert!(
        d[0].text
            .contains("a: disk /var reaches 95% in about 12 hours"),
        "{}",
        d[0].text
    );
    assert!(d[1].text.contains("about 2 days"), "{}", d[1].text);
    // the key does not move while the forecast's numbers do
    let later = detected(&Forecast {
        items: vec![item("a", 0.4)],
        ..Default::default()
    });
    assert_eq!(later[0].key, d[0].key);
}

// rivet: verifies REQ-PREDICT-001
#[test]
fn the_panel_shows_evidence_escapes_names_and_says_when_data_is_short() {
    let f = Forecast {
        generated_at: NOW,
        horizon_days: HORIZON_DAYS,
        limit_pct: LIMIT_PCT,
        watched: 4,
        not_enough_data: 1,
        items: vec![item("<b>x</b>", 0.5)],
    };
    let html = panel(Some(&f));
    assert!(html.contains("Coming up"), "{html}");
    assert!(
        html.contains("&lt;b&gt;x&lt;/b&gt;") && !html.contains("<b>x</b>"),
        "{html}"
    );
    assert!(html.contains("48 h, 48 points, R² 0.98"), "{html}");
    assert!(html.contains("1 with not enough data yet"), "{html}");
    assert!(html.contains("badrow"), "{html}");
    let mut empty = f.clone();
    empty.items.clear();
    let none = panel(Some(&empty));
    assert!(
        none.contains("Nothing is forecast to reach its limit"),
        "{none}"
    );
    assert_eq!(panel(None), "");
}

// rivet: verifies REQ-PREDICT-001
#[test]
fn forecasts_come_from_the_history_store() {
    let t = tempfile::tempdir().unwrap();
    let h = History::open(t.path().to_str().unwrap()).unwrap();
    // three days, one sample an hour: /var fills 2%/day, memory is flat
    for i in 0..72 {
        let at = NOW - (72 - i) * H;
        let x = 80.0 + 2.0 * (i * H) as f64 / D as f64;
        h.record(
            "box",
            at,
            &[
                (DISK.into(), x),
                ("mem.used_pct".into(), 40.0),
                ("load.1".into(), 1.0),
            ],
        );
    }
    // a machine that started reporting an hour ago
    h.record("new", NOW - H, &[(DISK.into(), 50.0)]);
    h.maintain(NOW);
    h.sync();
    let f = forecast(&h, &["box".into(), "new".into(), "gone".into()], NOW);
    assert_eq!(f.watched, 3, "{f:?}");
    assert_eq!(f.not_enough_data, 1, "{f:?}");
    assert_eq!(f.items.len(), 1, "{f:?}");
    let i = &f.items[0];
    assert_eq!((i.host.as_str(), i.what.as_str()), ("box", "disk /var"));
    assert!((i.per_day - 2.0).abs() < 1e-6, "{i:?}");
    assert!(i.days > 3.0 && i.days < 5.0, "{i:?}");
}

//! Oracles for v1.4.0 part 2: warte's own CI telemetry is gone, and the views
//! that used it render the ground station's data instead (REQ-STORE-002).

use std::net::{IpAddr, Ipv4Addr};
use warte::dashboard::fleet;
use warte::hub::model::{PoolQueue, Queue, QueuedEntry, Runner, Snapshot, WaitStats};
use warte::hub::telemetry::{encode_snapshot, SeqCounters};
use warte::hub::{now, Hub};

const CI_HOST: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 201));

fn hub_with(s: &Snapshot) -> (tempfile::TempDir, Hub) {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let mut seq = SeqCounters::default();
    for f in encode_snapshot(s, &mut seq) {
        hub.ingest(&f, CI_HOST, &[CI_HOST]);
    }
    (dir, hub)
}

fn fleet_snapshot() -> Snapshot {
    let mut pools = std::collections::BTreeMap::new();
    pools.insert(
        "rust-cpu".to_string(),
        PoolQueue {
            jobs_queued: 7,
            oldest_s: 900,
            median_s: 300,
        },
    );
    let mut waits = std::collections::BTreeMap::new();
    waits.insert(
        "rust-cpu".to_string(),
        WaitStats {
            n: 40,
            p50: 60,
            p90: 600,
            max: 1200,
        },
    );
    Snapshot {
        agent_version: "0.1.0".into(),
        host: "ci-test".into(),
        taken_at: now(),
        runners: vec![
            Runner {
                id: 1,
                local_busy: true,
                gh_status: Some("online".into()),
                ..Default::default()
            },
            Runner {
                id: 2,
                gh_status: Some("online".into()),
                ..Default::default()
            },
        ],
        queue: Some(Queue {
            scanned_at: now(),
            jobs_queued: 7,
            runs_queued: 1,
            pools,
            waits,
            queued_complete: true,
            queued: vec![QueuedEntry {
                // author-controlled: must come out escaped
                repo: "spar".into(),
                run_id: 9,
                workflow: "<img src=x onerror=alert(1)>".into(),
                event: "pull_request".into(),
                branch: "feat/x".into(),
                pr: Some(511),
                pool: "rust-cpu".into(),
                jobs: 7,
                since: now() - 600,
            }],
            ..Default::default()
        }),
        ..Default::default()
    }
}

// rivet: verifies REQ-STORE-002
#[tokio::test]
async fn ci_telemetry_tables_are_gone_after_migration() {
    let dir = tempfile::tempdir().unwrap();
    let db = warte::db::Database::connect(&dir.path().join("state.db"))
        .await
        .unwrap();
    let tables: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table'")
            .fetch_all(db.pool())
            .await
            .unwrap();
    let tables: Vec<&str> = tables.iter().map(|(n,)| n.as_str()).collect();
    assert!(
        tables.contains(&"agents"),
        "migrations did not run: {tables:?}"
    );
    for gone in ["ci_jobs", "wait_history", "ci_status"] {
        assert!(
            !tables.contains(&gone),
            "{gone} still exists; CI telemetry belongs to the ground station"
        );
    }
}

// rivet: verifies REQ-DASH-005
// rivet: verifies REQ-STORE-002
#[test]
fn queue_view_shows_the_hubs_queue_escaped() {
    let (_d, hub) = hub_with(&fleet_snapshot());
    let html = fleet::queue(Some(&hub));
    assert!(html.contains("rust-cpu"), "pool missing: {html}");
    assert!(html.contains("#511"), "queued PR missing");
    assert!(html.contains("1/2"), "busy/total runners missing");
    assert!(
        !html.contains("<img"),
        "author-controlled workflow name was not escaped"
    );
    assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    assert!(
        html.contains(">fresh<"),
        "a just-received snapshot is fresh"
    );
}

// rivet: verifies REQ-STORE-002
#[test]
fn critical_path_uses_measured_waits_from_the_hub() {
    let (_d, hub) = hub_with(&fleet_snapshot());
    let html = fleet::bottlenecks(Some(&hub));
    assert!(
        html.contains("spar") && html.contains("7 jobs queued"),
        "{html}"
    );
    // p50 60s, p90 10m, max 20m for rust-cpu
    assert!(
        html.contains("<td>rust-cpu</td><td>40</td><td>60s</td><td>10m</td><td>20m</td>"),
        "{html}"
    );
}

// rivet: verifies REQ-ASK-004
// rivet: verifies REQ-STORE-002
#[test]
fn ask_gives_the_hubs_eta_answer() {
    let (_d, hub) = hub_with(&fleet_snapshot());
    let html = fleet::ask(Some(&hub), "spar", Some(511), None);
    assert!(html.contains("ETA for spar PR #511"), "{html}");
    assert!(
        html.contains(">queued<"),
        "status from /api/eta missing: {html}"
    );
    assert!(html.contains("Queued in <code>rust-cpu</code>"), "{html}");
    let not_queued = fleet::ask(Some(&hub), "spar", Some(1), None);
    assert!(not_queued.contains(">not_queued<"), "{not_queued}");
}

// rivet: verifies REQ-STORE-002
#[test]
fn stale_or_missing_data_is_said_not_hidden() {
    let mut old = fleet_snapshot();
    old.taken_at = now() - 3600;
    // received_at is set on ingest; to get a stale view, age the stored state
    let (dir, hub) = hub_with(&old);
    drop(hub);
    let state = dir.path().join("hub-state.json");
    // state is persisted every 5th frame; write it ourselves with an old receive time
    let snap = serde_json::to_value(&old).unwrap();
    std::fs::write(
        &state,
        serde_json::json!({"latest": snap, "received_at": now() - 3600, "history": []}).to_string(),
    )
    .unwrap();
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    assert!(fleet::queue(Some(&hub)).contains(">stale<"));
    assert!(fleet::health_cards(Some(&hub)).contains("(stale)"));

    let dir2 = tempfile::tempdir().unwrap();
    let empty = Hub::open(dir2.path().to_str().unwrap(), "pulseengine").unwrap();
    assert!(fleet::queue(Some(&empty)).contains("no telemetry has arrived"));
    assert!(fleet::queue(None).contains("not configured"));
}

/// Serve the full warte router (as `warte serve` builds it) on an ephemeral port.
async fn serve_app(hub: Option<warte::config::HubConfig>) -> (tempfile::TempDir, String) {
    let (d, base, _db) = serve_app_db(hub).await;
    (d, base)
}

async fn serve_app_db(
    hub: Option<warte::config::HubConfig>,
) -> (tempfile::TempDir, String, warte::db::Database) {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub,
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db.clone()).unwrap();
    let app = warte::server::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (dir, format!("http://{addr}"), db)
}

async fn status(url: String) -> u16 {
    reqwest::get(url).await.unwrap().status().as_u16()
}

// rivet: verifies REQ-ASK-004
// rivet: verifies REQ-STORE-002
#[tokio::test]
async fn the_router_serves_api_and_dashboard_where_documented() {
    let (_d, base) = serve_app(None).await;
    // the API is under /api, once (it was under /api/api before v1.4.0)
    assert_eq!(status(format!("{base}/api/health")).await, 200);
    assert_eq!(status(format!("{base}/api/api/health")).await, 404);
    // the unauthenticated realtime broadcast is not mounted (REQ-REALTIME-001
    // returns with auth in v1.5.0)
    for p in ["/api/events", "/api/ws"] {
        assert_eq!(status(format!("{base}{p}")).await, 404, "{p}");
    }
    // the dashboard page is reachable and shows the real version
    let page = reqwest::get(format!("{base}/")).await.unwrap();
    assert_eq!(page.status(), 200);
    let page = page.text().await.unwrap();
    // every same-origin URL in the page is relative, so the dashboard also works
    // behind a path prefix (Caddy `handle_path /warte/*`)
    let refs: Vec<String> = [
        "href=\"",
        "hx-get=\"",
        "hx-post=\"",
        "sse-connect=\"",
        "src=\"",
    ]
    .iter()
    .flat_map(|attr| {
        page.split(attr)
            .skip(1)
            .map(|r| r.split('"').next().unwrap().to_string())
    })
    .collect();
    assert!(refs.len() >= 10, "found only {refs:?}");
    for r in &refs {
        assert!(
            !r.starts_with('/') || r.starts_with("//"),
            "absolute same-origin URL {r:?} escapes a path prefix"
        );
    }
    // and each relative target resolves
    for r in refs
        .iter()
        .filter(|r| r.starts_with("api/dashboard/") || r.ends_with(".css"))
    {
        assert_eq!(status(format!("{base}/{r}")).await, 200, "{r}");
    }
    // unknown paths get a readable 404, not an empty body a browser saves as a file
    let nf = reqwest::get(format!("{base}/no-such-page")).await.unwrap();
    assert_eq!(nf.status(), 404);
    assert!(
        nf.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/plain"),
        "404 without a text content type"
    );
    assert_eq!(nf.text().await.unwrap(), "not found");
    assert!(
        page.contains(&format!("warte v{}", env!("CARGO_PKG_VERSION"))),
        "footer version"
    );
    // the rewired views answer, and say why they are empty
    let q = reqwest::get(format!("{base}/api/dashboard/queue-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(q.contains("not configured"), "{q}");
    for p in ["critical-path-partial", "health-partial"] {
        assert_eq!(
            status(format!("{base}/api/dashboard/{p}")).await,
            200,
            "{p}"
        );
    }
    // removed endpoints stay removed, and the old auth exemption for them is gone:
    // an unauthenticated POST is refused before routing
    let c = reqwest::Client::new();
    for p in ["/api/webhooks/github", "/api/webhooks/gitlab"] {
        let r = c.post(format!("{base}{p}")).send().await.unwrap();
        assert_eq!(r.status(), 401, "{p}");
    }
    assert_eq!(
        status(format!("{base}/api/dashboard/ask?repo=x&pr=1")).await,
        404
    );
    // no hub configured: no /ci
    assert_eq!(status(format!("{base}/ci/healthz")).await, 404);
    // Ask is a read-only GET an anonymous dashboard viewer can use (a POST
    // needs auth and came back 401 with an empty body)
    let r = reqwest::get(format!(
        "{base}/api/dashboard/ask-partial?repo=pulseengine%2Frivet&branch=main"
    ))
    .await
    .unwrap();
    assert_eq!(r.status(), 200);
    let t = r.text().await.unwrap();
    assert!(t.contains("ETA for rivet @ main"), "{t}");
    // no view claims freshness it did not measure
    for p in [
        "queue-partial",
        "health-partial",
        "agents-partial",
        "overview-partial",
    ] {
        let t = reqwest::get(format!("{base}/api/dashboard/{p}"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(
            !t.contains(">fresh<"),
            "{p} claims fresh without a snapshot: {t}"
        );
    }
}

// rivet: verifies REQ-SMITHY-001
#[tokio::test]
async fn serve_with_a_hub_mounts_the_ground_station_at_ci() {
    let hub_dir = tempfile::tempdir().unwrap();
    let cfg = warte::config::HubConfig {
        udp: "127.0.0.1:0".into(),
        allow: vec![Ipv4Addr::LOCALHOST.into()],
        data_dir: hub_dir.path().to_str().unwrap().into(),
        org: "pulseengine".into(),
    };
    let (_d, base) = serve_app(Some(cfg)).await;
    assert_eq!(status(format!("{base}/ci/healthz")).await, 200);
    // the hub's web view, where a browser lands (found by the live parity run)
    for p in ["/ci", "/ci/", "/ci/index.html"] {
        assert_eq!(status(format!("{base}{p}")).await, 200, "{p}");
    }
    assert_eq!(status(format!("{base}/ci/api/openapi.json")).await, 200);
    // before telemetry: the hub says so, and so does the view
    assert_eq!(
        status(format!("{base}/ci/api/eta?repo=spar&pr=1")).await,
        503
    );
    let q = reqwest::get(format!("{base}/api/dashboard/queue-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(q.contains("no telemetry has arrived"), "{q}");
}

const HOSTILE: &str = "<script>alert(1)</script>";

fn assert_escaped(html: &str, what: &str) {
    assert!(
        !html.contains("<script>alert(1)</script>"),
        "{what}: author-controlled text reached the page unescaped"
    );
}

// rivet: verifies REQ-DASH-005
// rivet: verifies REQ-ASK-004
// rivet: verifies REQ-DASH-014
#[test]
fn overview_is_the_tower_view_of_the_hub_snapshot() {
    let mut s = fleet_snapshot();
    s.runners[1].gh_busy = Some(true); // GitHub lags: the host says idle
    if let Some(q) = s.queue.as_mut() {
        let mut late = q.queued[0].clone();
        late.run_id = 10;
        late.workflow = "Late".into();
        late.since = now() - 3600; // past the pool's p90 (600 s)
        q.queued.push(late);
    }
    let (_d, hub) = hub_with(&s);
    let html = fleet::overview(Some(&hub));
    // runway gauge: one cell per runner, lit for the busy one only
    assert_eq!(html.matches(r#"<i class="on""#).count(), 1, "{html}");
    assert_eq!(html.matches("title=\"runner").count(), 2);
    // flight strips: the host's view decides, GitHub's disagreement is shown
    assert!(html.contains("R01") && html.contains("AIRBORNE"));
    assert!(html.contains("R02") && html.contains("HOLDING"));
    assert!(html.contains("GH busy"), "{html}");
    // departures: longest waiting first, overdue past p90
    let late = html.find("Late").expect("late run on the board");
    let fresh = html.find("&lt;img").expect("first run on the board");
    assert!(late < fresh, "longest waiting first");
    assert!(html.contains("overdue"));
    assert!(html.contains("Runners online"));
    // no hub, or a hub with no telemetry: say why
    assert!(fleet::overview(None).contains("not configured"));
    let e = tempfile::tempdir().unwrap();
    let empty = Hub::open(e.path().to_str().unwrap(), "pulseengine").unwrap();
    assert!(fleet::overview(Some(&empty)).contains("no telemetry"));
}

// rivet: verifies REQ-STORE-002
#[tokio::test]
async fn every_view_escapes_author_controlled_text() {
    // hub snapshot whose repo/branch/pool are hostile and the entry has no PR
    let mut s = fleet_snapshot();
    if let Some(q) = s.queue.as_mut() {
        q.queued[0].repo = HOSTILE.into();
        q.queued[0].branch = HOSTILE.into();
        q.queued[0].pool = HOSTILE.into();
        q.queued[0].pr = None;
        q.pools.insert(HOSTILE.into(), Default::default());
        q.waits.insert(HOSTILE.into(), Default::default());
    }
    // runner fields come from job logs and repo-defined labels: hostile too
    s.host = HOSTILE.into();
    s.errors = vec![HOSTILE.into()];
    s.runners[0].job = Some(HOSTILE.into());
    s.runners[0].labels = vec![HOSTILE.into()];
    s.runners[0].ctx = Some(warte::hub::model::JobContext {
        repo: HOSTILE.into(),
        workflow: HOSTILE.into(),
        branch: HOSTILE.into(),
        ..Default::default()
    });
    s.runners[1].last_result = Some(HOSTILE.into());
    let hub_dir = tempfile::tempdir().unwrap();
    {
        let (_d, hub) = hub_with(&s);
        // persist it into hub_dir for the served instance
        drop(hub);
    }
    std::fs::write(
        hub_dir.path().join("hub-state.json"),
        serde_json::json!({"latest": s, "received_at": now(), "history": []}).to_string(),
    )
    .unwrap();
    let cfg = warte::config::HubConfig {
        udp: "127.0.0.1:0".into(),
        allow: vec![Ipv4Addr::LOCALHOST.into()],
        data_dir: hub_dir.path().to_str().unwrap().into(),
        org: "pulseengine".into(),
    };
    let (_d, base, db) = serve_app_db(Some(cfg)).await;

    // agent name/repo and agent_data rows are written by agents
    sqlx::query("INSERT INTO agents (id, name, repo, identity_type, public_key, instance_id) VALUES (?, ?, ?, 'test', x'00', 'i')")
        .bind(HOSTILE)
        .bind(HOSTILE)
        .bind(HOSTILE)
        .execute(db.pool())
        .await
        .unwrap();
    for dt in ["hardware-catalog", "hardware-claim", "friction", "schedule"] {
        sqlx::query("INSERT INTO agent_data (id, agent_id, data_type, content, instance_id) VALUES (?, ?, ?, ?, 'i')")
            .bind(format!("d-{dt}"))
            .bind(HOSTILE)
            .bind(dt)
            .bind(HOSTILE)
            .execute(db.pool())
            .await
            .unwrap();
    }

    let mut checked = 0;
    for p in [
        "overview-partial",
        "agents-partial",
        "artifacts-partial",
        "queue-partial",
        "critical-path-partial",
        "resources-partial",
        "friction-partial",
        "schedule-partial",
        "health-partial",
    ] {
        let r = reqwest::get(format!("{base}/api/dashboard/{p}"))
            .await
            .unwrap();
        if r.status() == 404 {
            continue;
        }
        let t = r.text().await.unwrap();
        assert_escaped(&t, p);
        checked += 1;
    }
    assert!(checked >= 8, "only {checked} views answered");
    // rivet: verifies REQ-DASH-014 (escaping)
    // the overview did receive the hostile runner fields, escaped
    let o = reqwest::get(format!("{base}/api/dashboard/overview-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(o.matches("&lt;script&gt;").count() >= 6, "{o}");
    // the queue view did receive the hostile names, escaped
    let q = reqwest::get(format!("{base}/api/dashboard/queue-partial"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(q.contains("&lt;script&gt;"), "{q}");
    // Ask echoes its input back
    let a = reqwest::get(format!(
        "{base}/api/dashboard/ask-partial?repo=%3Cscript%3Ealert(1)%3C%2Fscript%3E&branch=%3Cscript%3Ealert(1)%3C%2Fscript%3E"
    ))
    .await
    .unwrap()
    .text()
    .await
    .unwrap();
    assert_escaped(&a, "ask-partial");
}

// rivet: verifies REQ-DASH-005
// rivet: verifies REQ-STORE-002
#[tokio::test]
async fn with_a_hub_stale_data_is_shown_as_stale() {
    let hub_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        hub_dir.path().join("hub-state.json"),
        serde_json::json!({"latest": fleet_snapshot(), "received_at": now() - 3600, "history": []})
            .to_string(),
    )
    .unwrap();
    let cfg = warte::config::HubConfig {
        udp: "127.0.0.1:0".into(),
        allow: vec![Ipv4Addr::LOCALHOST.into()],
        data_dir: hub_dir.path().to_str().unwrap().into(),
        org: "pulseengine".into(),
    };
    let (_d, base) = serve_app(Some(cfg)).await;
    for p in ["queue-partial", "critical-path-partial", "health-partial"] {
        let t = reqwest::get(format!("{base}/api/dashboard/{p}"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(
            !t.contains(">fresh<"),
            "{p} claims fresh on hour-old data: {t}"
        );
        assert!(
            t.contains("stale"),
            "{p} does not say the data is stale: {t}"
        );
    }
}

const PLAYWRIGHT_FIXTURE: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/playwright/fixture/hub");

/// Regenerate the Playwright fixture's hub state from the real model types:
/// `cargo test --test fleet_views -- --ignored regenerate_playwright_fixture`
#[test]
#[ignore]
fn regenerate_playwright_fixture() {
    let state = serde_json::json!({
        "latest": fleet_snapshot(),
        // fixed receive time: the dashboard must show this data as stale
        "received_at": 1_793_000_000,
        "history": [],
    });
    std::fs::write(
        format!("{PLAYWRIGHT_FIXTURE}/hub-state.json"),
        serde_json::to_string_pretty(&state).unwrap() + "\n",
    )
    .unwrap();
}

// The committed fixture and the hub's model must not drift apart: if the
// model changes, this fails until the fixture is regenerated.
#[test]
fn playwright_fixture_loads_into_the_hub() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        format!("{PLAYWRIGHT_FIXTURE}/hub-state.json"),
        dir.path().join("hub-state.json"),
    )
    .expect("tests/playwright/fixture/hub/hub-state.json is missing");
    let hub = Hub::open(dir.path().to_str().unwrap(), "pulseengine").unwrap();
    let (s, _) = hub.snapshot().expect("fixture has no snapshot");
    assert!(
        s.queue.as_ref().is_some_and(|q| !q.queued.is_empty()),
        "fixture must have queued runs for the browser tests"
    );
}

// rivet: verifies REQ-ASK-004
#[test]
fn ask_lists_what_is_in_flight_so_you_click_instead_of_typing() {
    let mut s = fleet_snapshot();
    // a branch name that needs URL encoding, and a running job on runner 1
    if let Some(q) = s.queue.as_mut() {
        let mut e = q.queued[0].clone();
        e.run_id = 77;
        e.pr = None;
        e.branch = "feat/a b&c".into();
        q.queued.push(e);
    }
    s.runners[0].ctx = Some(warte::hub::model::JobContext {
        repo: "pulseengine/rivet".into(),
        workflow: "CI".into(),
        pr: Some(9),
        run_id: 5,
        ..Default::default()
    });
    let (_d, hub) = hub_with(&s);
    let (picker, repos) = fleet::ask_picker(Some(&hub));
    // one clickable row per PR or branch, asking the same view
    assert!(
        picker.contains(r#"hx-get="api/dashboard/ask-partial?repo=spar&amp;pr=511""#),
        "{picker}"
    );
    assert!(
        picker.contains("repo=spar&amp;branch=feat%2Fa%20b%26c"),
        "branch is URL-encoded: {picker}"
    );
    assert!(
        picker.contains("repo=rivet&amp;pr=9"),
        "running work is listed too: {picker}"
    );
    assert!(picker.contains("1 running"));
    // workflow names are author-controlled
    assert!(!picker.contains("<img"), "{picker}");
    // the repo input suggests the repos in flight
    assert!(
        repos.contains(r#"<option value="spar">"#) && repos.contains(r#"<option value="rivet">"#)
    );
    assert_eq!(fleet::ask_picker(None), (String::new(), String::new()));
}

// rivet: verifies REQ-STORE-002
#[test]
fn health_and_outages_come_from_the_snapshot_not_from_decoration() {
    let mut s = fleet_snapshot();
    s.machine.ncpu = 32;
    s.machine.load = [40.0, 30.0, 20.0];
    s.machine.psi.cpu.avg60 = 61.6;
    s.machine.disks = vec![warte::hub::model::Disk {
        mount: "/var/lib/runners".into(),
        size_gb: 1000.0,
        used_gb: 960.0,
        used_pct: 96.0,
    }];
    s.runners[1].gh_status = Some("offline".into());
    s.errors = vec!["<script>x</script> failed".into()];
    let (_d, hub) = hub_with(&s);
    let h = fleet::health_view(Some(&hub));
    assert!(h.contains("40.0 / 30.0 / 20.0 on 32"), "{h}");
    assert!(h.contains("96% of 1000 GB"), "{h}");
    assert!(h.contains("&lt;script&gt;x&lt;/script&gt; failed") && !h.contains("<script>x"));
    let o = fleet::incidents(Some(&hub));
    assert!(o.contains("Offline runners: runner2."), "{o}");
    assert!(o.contains("Disk /var/lib/runners is 96% full."), "{o}");
    assert!(o.contains("CPU pressure 61.6%"), "{o}");
    // worst first: the red items come before the amber ones
    assert!(o.find("Offline runners").unwrap() < o.find("CPU pressure").unwrap());
    assert!(!o.contains("operational"), "no hardcoded status: {o}");
    assert!(!o.contains("<script>x"));
    // a calm fleet says so
    let (_d2, calm) = hub_with(&fleet_snapshot());
    assert!(fleet::incidents(Some(&calm)).contains("all clear"));
    assert!(fleet::incidents(None).contains("not configured"));
}

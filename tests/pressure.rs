//! Disk pressure (REQ-PRESSURE-001, REQ-PRESSURE-002, DD-PRESSURE-001).

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use warte::probe::pressure::{
    candidates, held, is_session_id, measure, size_bytes, slug, worst, Kind, Live, Thresholds,
    Verdict,
};
use warte::probe::{Cpu, Disk, Memory, Os, Report};

const SID: &str = "1fda52ce-6246-43d4-abfc-83ffee08f158";
const SID2: &str = "0b7f3e2a-1111-4222-8333-944455556666";

// rivet: verifies REQ-PRESSURE-001
#[test]
fn verdicts_follow_the_thresholds_and_the_worst_volume_wins() {
    let t = Thresholds::default();
    assert_eq!(t.verdict(9.9), Verdict::Critical);
    assert_eq!(t.verdict(14.0), Verdict::Low);
    // negative control: plenty free
    assert_eq!(t.verdict(25.0), Verdict::Ok);
    assert_eq!(t.verdict(20.0), Verdict::Ok);
    // changeable
    let t = Thresholds {
        low_gb: 30.0,
        critical_gb: 5.0,
    };
    assert_eq!(t.verdict(25.0), Verdict::Low);
    assert_eq!(t.verdict(6.0), Verdict::Low);
    assert_eq!(
        worst([Verdict::Ok, Verdict::Critical, Verdict::Low]),
        Verdict::Critical
    );
    assert_eq!(worst([]), Verdict::Unknown);
    // a missing key keeps its default
    let t: Thresholds = toml::from_str("low_gb = 50").unwrap();
    assert_eq!(t.low_gb, 50.0);
    assert_eq!(t.critical_gb, 10.0);
}

fn mkdirs(p: &Path, bytes: usize) {
    std::fs::create_dir_all(p).unwrap();
    std::fs::write(p.join("f"), vec![7u8; bytes]).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?}");
}

// rivet: verifies REQ-PRESSURE-002
#[test]
fn scratchpads_map_to_their_session_and_checkouts_give_target_and_worktrees() {
    let tmp = tempfile::tempdir().unwrap();
    let tmp = std::fs::canonicalize(tmp.path()).unwrap();
    // a checkout with target/ and a worktree
    let repo = tmp.join("src/repo-a");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "x"]);
    mkdirs(&repo.join("target/debug"), 4096);
    let wt = tmp.join("src/repo-a-wt");
    git(
        &repo,
        &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "w"],
    );
    let live = vec![
        Live {
            agent: "claude".into(),
            pid: 41,
            cwd: repo.clone(),
        },
        Live {
            agent: "claude".into(),
            pid: 42,
            cwd: wt.clone(),
        },
    ];
    // Claude Code's per-session directories
    let root = tmp.join("claude-501");
    mkdirs(&root.join(slug(&repo)).join(SID).join("scratchpad"), 8192);
    // a session that ended in a directory nobody works in
    mkdirs(&root.join("-gone-dir").join(SID2).join("scratchpad"), 10);
    // not sessions
    mkdirs(&root.join("bash-edit-diff").join(SID2), 10);
    mkdirs(&root.join(slug(&repo)).join("notes"), 10);

    let c = candidates(&live, std::slice::from_ref(&root));
    let pads: Vec<_> = c.iter().filter(|x| x.kind == Kind::Scratchpad).collect();
    assert_eq!(pads.len(), 2, "{c:#?}");
    let mine = pads
        .iter()
        .find(|x| x.session_id.as_deref() == Some(SID))
        .unwrap();
    assert_eq!(mine.agents.len(), 1);
    assert_eq!(mine.agents[0].pid, 41, "the scratchpad maps to its session");
    assert_eq!(mine.repo.as_deref(), Some("repo-a"));
    let gone = pads
        .iter()
        .find(|x| x.session_id.as_deref() == Some(SID2))
        .unwrap();
    assert!(gone.agents.is_empty());

    let targets: Vec<_> = c.iter().filter(|x| x.kind == Kind::Target).collect();
    assert_eq!(targets.len(), 1, "{c:#?}");
    assert_eq!(targets[0].path, repo.join("target"));
    assert_eq!(
        targets[0].agents.iter().map(|a| a.pid).collect::<Vec<_>>(),
        vec![41],
        "the worktree's session is not counted for the main checkout"
    );
    let wts: Vec<_> = c.iter().filter(|x| x.kind == Kind::Worktree).collect();
    assert_eq!(wts.len(), 1, "{c:#?}");
    assert_eq!(wts[0].path, wt);
    assert_eq!(wts[0].agents[0].pid, 42);

    // sized, on the volume of the temp dir; elsewhere skipped
    let mounts = vec!["/".to_string(), tmp.to_string_lossy().into_owned()];
    let here = warte::probe::pressure::mount_of(&tmp, &mounts).unwrap();
    let s = measure(
        c.clone(),
        &mounts,
        std::slice::from_ref(&here),
        Duration::from_secs(30),
        7,
    );
    assert_eq!(s.measured_at, 7);
    assert!(s.holders.iter().all(|h| h.mount == here && h.complete));
    let pad = s
        .holders
        .iter()
        .find(|h| h.session_id.as_deref() == Some(SID))
        .unwrap();
    assert!(pad.live);
    assert_eq!(pad.pid, Some(41));
    let gone = s
        .holders
        .iter()
        .find(|h| h.session_id.as_deref() == Some(SID2))
        .unwrap();
    assert!(!gone.live);
    assert_eq!(gone.pid, None);
    let none = measure(
        c,
        &mounts,
        &["/elsewhere".into()],
        Duration::from_secs(30),
        7,
    );
    assert!(none.holders.is_empty(), "only holders on a wanted volume");

    // what leaves the machine: totals per kind per volume, no path or id
    let h = held(&s);
    let j = serde_json::to_string(&h).unwrap();
    for secret in [SID, "repo-a", "claude-501", "scratchpad\"", "\"41"] {
        assert!(!j.contains(secret), "{secret} in {j}");
    }
    assert_eq!(h.volumes.len(), 1);
    assert_eq!(h.volumes[0].mount, here);
}

// rivet: verifies REQ-PRESSURE-002
#[test]
fn sizing_follows_no_symlink_and_says_when_it_ran_out_of_time() {
    let tmp = tempfile::tempdir().unwrap();
    let big = tmp.path().join("big");
    mkdirs(&big, 1 << 20);
    let d = tmp.path().join("d");
    mkdirs(&d, 10);
    std::os::unix::fs::symlink(&big, d.join("link")).unwrap();
    let (b, complete) = size_bytes(&d, std::time::Instant::now() + Duration::from_secs(10));
    assert!(complete);
    assert!(b < 1 << 20, "the symlinked 1 MiB is not counted: {b}");
    let (bb, _) = size_bytes(&big, std::time::Instant::now() + Duration::from_secs(10));
    assert!(bb >= 1 << 20, "{bb}");
    assert!(is_session_id(SID));
    assert!(!is_session_id("bash-edit-diff"));
    assert!(!is_session_id("1fda52ce-6246-43d4-abfc-83ffee08f15g"));
    assert_eq!(
        slug(&PathBuf::from("/Volumes/Home/git/a.b")),
        "-Volumes-Home-git-a-b"
    );
}

fn report(
    host: &str,
    free: &[(&str, f64, f64)],
    held: Option<warte::probe::pressure::Held>,
) -> Report {
    Report {
        probe_version: "t".into(),
        host: host.into(),
        taken_at: warte::hub::now(),
        os: Os {
            name: "L".into(),
            kernel: "6".into(),
            arch: "a".into(),
        },
        uptime_s: 1,
        cpu: Cpu {
            ncpu: 1,
            load: [0.0; 3],
        },
        memory: Memory {
            total_mb: 1,
            avail_mb: 1,
            swap_total_mb: 0,
            swap_used_mb: 0,
        },
        disks: free
            .iter()
            .map(|(m, size, free)| Disk {
                mount: m.to_string(),
                fs: "apfs".into(),
                size_gb: *size,
                used_gb: size - free,
                used_pct: 0.0,
                removable: false,
            })
            .collect(),
        tools: vec![],
        sessions: vec![],
        io: None,
        inventory: None,
        health: None,
        hardware: None,
        pressure: held,
    }
}

async fn serve(reports: Vec<(Report, i64)>, t: Option<Thresholds>) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let hub_dir = dir.path().join("hub");
    std::fs::create_dir_all(&hub_dir).unwrap();
    let m = warte::probe::Machines::open(hub_dir.to_str().unwrap());
    for (r, at) in reports {
        m.record(r, "probe", at);
    }
    drop(m);
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        hub: Some(warte::config::HubConfig {
            udp: "127.0.0.1:0".into(),
            allow: vec![Ipv4Addr::LOCALHOST.into()],
            data_dir: hub_dir.to_str().unwrap().into(),
            org: "pulseengine".into(),
            probe_self: false,
            costs_file: None,
            skill_usage_file: None,
        }),
        pressure: t.unwrap_or_default(),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let app = warte::server::build_router(warte::server::build_state(config, db).unwrap());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (dir, base)
}

async fn get(url: String) -> (u16, serde_json::Value) {
    let r = reqwest::get(url).await.unwrap();
    (r.status().as_u16(), r.json().await.unwrap())
}

// rivet: verifies REQ-PRESSURE-001
#[tokio::test]
async fn api_pressure_answers_per_volume_with_totals_and_no_names() {
    let now = warte::hub::now();
    let held = warte::probe::pressure::Held {
        measured_at: now - 60,
        volumes: vec![warte::probe::pressure::HeldOn {
            mount: "/data".into(),
            scratchpad_gb: 3.5,
            target_gb: 0.0,
            worktree_gb: 1.0,
        }],
    };
    let (_d, base) = serve(
        vec![
            (
                report(
                    "tight",
                    &[("/data", 245.0, 8.0), ("/ext", 2000.0, 700.0)],
                    Some(held),
                ),
                now,
            ),
            (report("roomy", &[("/data", 245.0, 25.0)], None), now),
            (report("silent", &[("/data", 245.0, 2.0)], None), now - 3600),
        ],
        None,
    )
    .await;

    let (code, v) = get(format!("{base}/api/pressure?host=tight")).await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["verdict"], "critical", "{v}");
    assert_eq!(v["recheck_s"], 60);
    assert!(v["reason"].as_str().unwrap().contains("/data"), "{v}");
    let vols = v["volumes"].as_array().unwrap();
    assert_eq!(vols.len(), 2);
    assert_eq!(vols[0]["mount"], "/data");
    assert_eq!(vols[0]["free_gb"], 8.0);
    assert_eq!(vols[0]["verdict"], "critical");
    assert_eq!(vols[0]["held_gb"]["scratchpad"], 3.5);
    assert_eq!(vols[0]["held_gb"]["worktree"], 1.0);
    assert_eq!(vols[1]["verdict"], "ok");
    assert!(vols[1]["held_gb"].is_null());

    // negative control: 25 GB free is ok
    let (_, v) = get(format!("{base}/api/pressure?host=roomy")).await;
    assert_eq!(v["verdict"], "ok", "{v}");
    // no fresh report: unknown, not ok
    let (_, v) = get(format!("{base}/api/pressure?host=silent")).await;
    assert_eq!(v["verdict"], "unknown", "{v}");
    assert_eq!(v["volumes"][0]["verdict"], "unknown");

    // without host: 400 and the hosts to choose from
    let (code, v) = get(format!("{base}/api/pressure")).await;
    assert_eq!(code, 400);
    assert_eq!(v["error"], "host is required");
    assert!(v["hosts"].as_array().unwrap().iter().any(|h| h == "tight"));
    let (code, _) = get(format!("{base}/api/pressure?host=nope")).await;
    assert_eq!(code, 404);

    // /api/machines publishes the same totals and nothing more
    let (_, m) = get(format!("{base}/api/machines")).await;
    let t = m["machines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["host"] == "tight")
        .unwrap();
    assert_eq!(t["report"]["pressure"]["volumes"][0]["scratchpad_gb"], 3.5);
}

// rivet: verifies REQ-PRESSURE-001
#[tokio::test]
async fn the_thresholds_come_from_the_config() {
    let now = warte::hub::now();
    let (_d, base) = serve(
        vec![(report("roomy", &[("/data", 245.0, 25.0)], None), now)],
        Some(Thresholds {
            low_gb: 30.0,
            critical_gb: 10.0,
        }),
    )
    .await;
    let (_, v) = get(format!("{base}/api/pressure?host=roomy")).await;
    assert_eq!(v["verdict"], "low", "{v}");
    assert!(v["reason"].as_str().unwrap().contains("30"), "{v}");
}

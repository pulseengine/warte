//! Scheduled jobs across hosts (REQ-JOBS-001).
//!
//! Hosts push one small state file per job into a directory on the ground
//! station (`<dir>/<host>/<job>.json`, written by `pe-job`); a manifest lists
//! the files that must exist and how long each may go without a success
//! (`<path> <window_s>` per line, `#` comments). warte reads both on request:
//!
//! - **ok**: a success within its window
//! - **failing**: the last run failed, but a success is still within the window
//! - **overdue**: no success within the window (a host down or a timer stopped
//!   shows up here, not as silence)
//! - **missing**: listed in the manifest, no state file at all
//!
//! Files that are present but not listed are shown with their own
//! `expect_within_s`. Everything read is author-controlled text and is
//! escaped where it is drawn.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Expected {
    pub path: String,
    pub window_s: i64,
}

/// Parse the manifest. Lines that are not `<relative path> <seconds>` are
/// skipped; so are absolute paths and paths with `..`.
pub fn parse_expected(text: &str) -> Vec<Expected> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let path = it.next()?.to_string();
            let window_s = it.next()?.parse().ok()?;
            safe(&path).then_some(Expected { path, window_s })
        })
        .collect()
}

fn safe(rel: &str) -> bool {
    !rel.starts_with('/')
        && rel.ends_with(".json")
        && rel
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
}

/// The state file's fields, as `docs/topics/job-state.md` documents them.
pub const STATE_FILE_FIELDS: &[&str] = &[
    "name",
    "host",
    "finished",
    "exit",
    "duration_s",
    "expect_within_s",
    "last_success",
];

#[derive(Debug, Deserialize)]
struct StateFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    finished: Option<i64>,
    #[serde(default)]
    exit: Option<i32>,
    #[serde(default)]
    duration_s: Option<i64>,
    #[serde(default)]
    expect_within_s: Option<i64>,
    #[serde(default)]
    last_success: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Job {
    pub host: String,
    pub name: String,
    /// ok, failing, overdue or missing
    pub state: String,
    pub last_run: Option<i64>,
    pub last_success: Option<i64>,
    pub exit: Option<i32>,
    pub duration_s: Option<i64>,
    pub window_s: i64,
    /// Seconds until overdue (negative once overdue); None when never succeeded.
    pub due_in_s: Option<i64>,
}

impl Job {
    pub fn bad(&self) -> bool {
        self.state == "overdue" || self.state == "missing"
    }
}

fn names_of(rel: &str) -> (String, String) {
    let stem = rel.trim_end_matches(".json");
    match stem.rsplit_once('/') {
        Some((host, name)) => (host.to_string(), name.to_string()),
        None => (String::new(), stem.to_string()),
    }
}

fn judge(rel: &str, window: Option<i64>, f: Option<StateFile>, now: i64) -> Job {
    let (dir_host, file_name) = names_of(rel);
    let Some(f) = f else {
        return Job {
            host: dir_host,
            name: file_name,
            state: "missing".into(),
            last_run: None,
            last_success: None,
            exit: None,
            duration_s: None,
            window_s: window.unwrap_or(0),
            due_in_s: None,
        };
    };
    let window_s = window.or(f.expect_within_s).unwrap_or(0);
    let due_in_s = f.last_success.map(|t| t + window_s - now);
    let state = match due_in_s {
        Some(d) if d >= 0 && f.exit.unwrap_or(0) != 0 => "failing",
        Some(d) if d >= 0 => "ok",
        _ => "overdue",
    };
    Job {
        host: if f.host.is_empty() { dir_host } else { f.host },
        name: if f.name.is_empty() { file_name } else { f.name },
        state: state.into(),
        last_run: f.finished,
        last_success: f.last_success,
        exit: f.exit,
        duration_s: f.duration_s,
        window_s,
        due_in_s,
    }
}

fn read_state(dir: &Path, rel: &str) -> Option<StateFile> {
    let b = std::fs::read(dir.join(rel)).ok()?;
    if b.len() > 64 * 1024 {
        return None;
    }
    serde_json::from_slice(&b).ok()
}

/// Every job: the manifest's, then unlisted state files. Worst first.
pub fn read(dir: &Path, manifest: Option<&str>, now: i64) -> Vec<Job> {
    let expected = manifest.map(parse_expected).unwrap_or_default();
    let mut seen = std::collections::BTreeSet::new();
    let mut jobs: Vec<Job> = expected
        .iter()
        .map(|e| {
            seen.insert(e.path.clone());
            judge(&e.path, Some(e.window_s), read_state(dir, &e.path), now)
        })
        .collect();
    // unlisted files, one level of host directories deep
    let mut extra = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() {
                if let Ok(sub) = std::fs::read_dir(e.path()) {
                    for s in sub.flatten() {
                        extra.push(format!("{n}/{}", s.file_name().to_string_lossy()));
                    }
                }
            } else {
                extra.push(n);
            }
        }
    }
    extra.sort();
    for rel in extra {
        if safe(&rel) && !seen.contains(&rel) {
            if let Some(f) = read_state(dir, &rel) {
                jobs.push(judge(&rel, None, Some(f), now));
            }
        }
    }
    let rank = |j: &Job| match j.state.as_str() {
        "missing" => 0,
        "overdue" => 1,
        "failing" => 2,
        _ => 3,
    };
    jobs.sort_by(|a, b| (rank(a), &a.host, &a.name).cmp(&(rank(b), &b.host, &b.name)));
    jobs
}

#[cfg(test)]
mod tests {
    // rivet: verifies REQ-TEMPLATE-002
    #[test]
    fn the_documented_state_fields_are_the_parsed_ones() {
        let mut m = serde_json::Map::new();
        for f in super::STATE_FILE_FIELDS {
            let v = if *f == "name" || *f == "host" {
                serde_json::json!("x")
            } else {
                serde_json::json!(7)
            };
            m.insert(f.to_string(), v);
        }
        let f: super::StateFile = serde_json::from_value(m.into()).unwrap();
        assert_eq!((f.name.as_str(), f.host.as_str()), ("x", "x"));
        assert_eq!(
            (
                f.finished,
                f.exit,
                f.duration_s,
                f.expect_within_s,
                f.last_success
            ),
            (Some(7), Some(7), Some(7), Some(7), Some(7)),
            "every documented field is read"
        );
    }

    use super::*;

    fn write(dir: &Path, rel: &str, v: serde_json::Value) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, v.to_string()).unwrap();
    }

    // rivet: verifies REQ-JOBS-001
    #[test]
    fn the_manifest_is_read_strictly() {
        let e = parse_expected(
            "# comment\nnetcup-vps/certbot.json 108000\nprefix-sync.json 900 # trailing\n\
             ../etc/passwd.json 5\n/abs.json 5\nbad-line\nx.json notanumber\n",
        );
        assert_eq!(
            e,
            vec![
                Expected {
                    path: "netcup-vps/certbot.json".into(),
                    window_s: 108000
                },
                Expected {
                    path: "prefix-sync.json".into(),
                    window_s: 900
                },
            ]
        );
    }

    // rivet: verifies REQ-JOBS-001
    #[test]
    fn ok_failing_overdue_missing_and_unlisted() {
        let d = tempfile::tempdir().unwrap();
        let now = 1_000_000;
        let job = |name: &str, host: &str, exit: i32, last_success: i64| {
            serde_json::json!({"name": name, "host": host, "finished": now - 10, "exit": exit,
                "duration_s": 1, "expect_within_s": 3600, "last_success": last_success})
        };
        write(
            d.path(),
            "netcup-vps/certbot.json",
            job("certbot", "netcup-vps", 0, now - 100),
        );
        write(
            d.path(),
            "mini/costs.json",
            job("costs", "mini", 2, now - 100),
        );
        write(
            d.path(),
            "mini/archive.json",
            job("archive", "mini", 0, now - 7200),
        );
        write(
            d.path(),
            "macbook/extra.json",
            job("extra", "macbook", 0, now - 10),
        );
        let manifest = "netcup-vps/certbot.json 108000\nmini/costs.json 3600\n\
                        mini/archive.json 3600\nmacbook/gone.json 600\n";
        let jobs = read(d.path(), Some(manifest), now);
        let st = |n: &str| jobs.iter().find(|j| j.name == n).unwrap().state.clone();
        assert_eq!(st("certbot"), "ok");
        assert_eq!(st("costs"), "failing");
        assert_eq!(st("archive"), "overdue");
        assert_eq!(st("gone"), "missing");
        assert_eq!(
            st("extra"),
            "ok",
            "an unlisted file is shown with its own window"
        );
        // worst first
        assert_eq!(jobs[0].state, "missing");
        assert_eq!(jobs[1].state, "overdue");
        let certbot = jobs.iter().find(|j| j.name == "certbot").unwrap();
        assert_eq!(certbot.due_in_s, Some(108000 - 100));
        assert!(jobs.iter().filter(|j| j.bad()).count() == 2);
    }
}

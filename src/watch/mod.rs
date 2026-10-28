//! `warte watch`: block until a PR's or branch's CI changes, then exit
//! (REQ-WATCH-001/002, DD-WATCH-001).
//!
//! What is watched is the set of the PR's workflow runs and whether each is
//! queued or running, as the ground station's latest snapshot shows it. The
//! cursor is that state: `<status>.<digest>`. The server keeps no event log,
//! so a caller that resumes with `--since` cannot miss a change however long
//! it was away; changes inside one snapshot interval (a minute) are not seen,
//! which is the resolution of the telemetry itself.

use crate::hub::model::Snapshot;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// The server holds a request at most this long; the CLI chains requests.
pub const MAX_HOLD_S: u64 = 900;
pub const DEFAULT_HOLD_S: u64 = 300;
/// Held requests at once. A watcher is cheap, but not free: the cap keeps a
/// flood of open requests from tying up the server.
pub const MAX_WATCHERS: usize = 64;
/// Exit code of `warte watch` when the timeout passes with no change.
pub const EXIT_TIMEOUT: i32 = 3;

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Queued,
    Running,
}

/// The CI state of one PR or branch.
#[derive(Debug, PartialEq)]
pub struct Picture {
    /// `running`, `queued`, `running+queued`, `not_queued` or `unknown`,
    /// with the same meaning as the ground station's `/api/eta`.
    pub status: &'static str,
    pub runs: BTreeMap<u64, RunState>,
    pub queued_complete: bool,
}

/// The PR's (or branch's) runs, matched exactly as `/api/eta` matches them.
/// A run with one job running and others queued counts as running.
pub fn picture(
    s: &Snapshot,
    org: &str,
    repo: &str,
    pr: Option<u32>,
    branch: Option<&str>,
) -> Picture {
    let full = format!("{org}/{repo}");
    let hit = |p: Option<u32>, b: &str| match (pr, branch) {
        (Some(n), _) => p == Some(n),
        (None, Some(br)) => b == br,
        (None, None) => false,
    };
    let mut runs = BTreeMap::new();
    let q = s.queue.as_ref();
    if let Some(q) = q {
        for e in q
            .queued
            .iter()
            .filter(|e| e.repo == repo && hit(e.pr, &e.branch))
        {
            runs.insert(e.run_id, RunState::Queued);
        }
        for j in q
            .hosted
            .iter()
            .filter(|j| j.status == "in_progress" && j.repo == full && hit(j.pr, &j.branch))
        {
            runs.insert(j.run_id, RunState::Running);
        }
    }
    for c in s
        .runners
        .iter()
        .filter(|r| r.local_busy)
        .filter_map(|r| r.ctx.as_ref())
        .filter(|c| c.repo == full && hit(c.pr, &c.branch))
    {
        runs.insert(c.run_id, RunState::Running);
    }
    let complete = q.map(|q| q.queued_complete).unwrap_or(false);
    let running = runs.values().any(|s| *s == RunState::Running);
    let queued = runs.values().any(|s| *s == RunState::Queued);
    let status = match (running, queued) {
        (true, true) => "running+queued",
        (true, false) => "running",
        (false, true) => "queued",
        (false, false) if complete => "not_queued",
        (false, false) => "unknown",
    };
    Picture {
        status,
        runs,
        queued_complete: complete,
    }
}

impl Picture {
    pub fn cursor(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.status);
        for (id, st) in &self.runs {
            h.update(format!(";{id}:{st:?}"));
        }
        // `+` would read as a space in a hand-written query string
        format!(
            "{}.{}",
            self.status.replace('+', "-"),
            &hex::encode(h.finalize())[..16]
        )
    }
}

/// A cursor from a caller: `<status>.<16 hex>`, URL-safe (`running-queued`). Anything else is rejected,
/// not compared, so a typo cannot read as "changed" forever.
pub fn valid_cursor(c: &str) -> bool {
    let Some((status, digest)) = c.split_once('.') else {
        return false;
    };
    [
        "running",
        "queued",
        "running-queued",
        "not_queued",
        "unknown",
    ]
    .contains(&status)
        && digest.len() == 16
        && digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[derive(Serialize, Debug)]
pub struct Run {
    pub run_id: u64,
    pub state: RunState,
}

#[derive(Serialize, Debug)]
pub struct AsOf {
    pub taken_at: i64,
    pub age_s: i64,
}

#[derive(Serialize, Debug)]
pub struct Answer {
    /// False: the hold timed out with no change.
    pub changed: bool,
    /// The status the caller's cursor described.
    pub from: String,
    pub status: &'static str,
    /// Pass back as `since` to continue from here.
    pub cursor: String,
    pub runs: Vec<Run>,
    /// False: the queue scan was incomplete, so a run missing from `runs`
    /// may still be queued. A run that disappeared has finished or is unseen.
    pub queued_complete: bool,
    pub as_of: AsOf,
}

pub fn answer(p: &Picture, since: &str, received_at: i64, taken_at: i64, now: i64) -> Answer {
    let cursor = p.cursor();
    Answer {
        changed: cursor != since,
        from: since
            .split('.')
            .next()
            .unwrap_or_default()
            .replace('-', "+"),
        status: p.status,
        cursor,
        runs: p
            .runs
            .iter()
            .map(|(id, st)| Run {
                run_id: *id,
                state: *st,
            })
            .collect(),
        queued_complete: p.queued_complete,
        as_of: AsOf {
            taken_at,
            age_s: (now - received_at).max(0),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_are_validated_not_trusted() {
        let p = Picture {
            status: "queued",
            runs: [(7, RunState::Queued)].into(),
            queued_complete: true,
        };
        assert!(valid_cursor(&p.cursor()));
        let both = Picture {
            status: "running+queued",
            runs: [(7, RunState::Running), (8, RunState::Queued)].into(),
            queued_complete: true,
        };
        let c = both.cursor();
        assert!(valid_cursor(&c) && !c.contains('+'), "URL-safe: {c}");
        assert_eq!(answer(&both, &c, 0, 0, 0).from, "running+queued");
        for bad in [
            "",
            "queued",
            "queued.",
            "done.0123456789abcdef",
            "queued.0123456789ABCDEF",
            "queued.0123456789abcde",
            "queued.0123456789abcdefg",
        ] {
            assert!(!valid_cursor(bad), "{bad}");
        }
    }

    #[test]
    fn the_cursor_changes_with_the_runs_not_only_the_status() {
        let a = Picture {
            status: "running",
            runs: [(1, RunState::Running)].into(),
            queued_complete: true,
        };
        let b = Picture {
            status: "running",
            runs: [(2, RunState::Running)].into(),
            queued_complete: true,
        };
        assert_ne!(a.cursor(), b.cursor(), "run 1 finished, run 2 started");
        assert_eq!(a.cursor(), a.cursor());
    }
}

/// The CLI side: chain long-polls until a change or the overall timeout.
pub mod client {
    use std::time::{Duration, Instant};

    /// Prints the final answer as JSON and returns the exit code: 0 on a
    /// change, [`super::EXIT_TIMEOUT`] when `timeout_s` passes without one.
    pub async fn run(
        base: &str,
        repo: &str,
        pr: Option<u32>,
        branch: Option<&str>,
        mut since: Option<String>,
        timeout_s: u64,
    ) -> anyhow::Result<i32> {
        let http = reqwest::Client::new();
        let deadline = Instant::now() + Duration::from_secs(timeout_s);
        loop {
            let left = deadline.saturating_duration_since(Instant::now()).as_secs();
            let hold = left.min(super::MAX_HOLD_S);
            let mut query: Vec<(&str, String)> =
                vec![("repo", repo.into()), ("timeout", hold.to_string())];
            if let Some(n) = pr {
                query.push(("pr", n.to_string()));
            }
            if let Some(b) = branch {
                query.push(("branch", b.into()));
            }
            if let Some(c) = &since {
                query.push(("since", c.clone()));
            }
            let resp = http
                .get(format!("{base}/api/watch"))
                .query(&query)
                .timeout(Duration::from_secs(hold + 30))
                .send()
                .await?;
            let status = resp.status();
            let body = resp.text().await?;
            if !status.is_success() {
                anyhow::bail!("{base}/api/watch: {status}: {}", body.trim());
            }
            let v: serde_json::Value = serde_json::from_str(&body)?;
            let changed = v["changed"].as_bool().unwrap_or(false);
            if changed || hold == 0 || Instant::now() >= deadline {
                println!("{}", serde_json::to_string(&v)?);
                return Ok(if changed { 0 } else { super::EXIT_TIMEOUT });
            }
            // carry the baseline forward: a change between two holds is still seen
            since = v["cursor"].as_str().map(String::from);
        }
    }
}

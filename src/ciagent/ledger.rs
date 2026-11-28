//! The conveyor belt's ledger (agent side): every finished job of the org, read
//! back from GitHub per UTC day and kept on the CI host until the hub has it.
//!
//! GitHub is the store of record. A day is re-read on the next two days (runs
//! that finish after midnight), then closed; older days are backfilled within a
//! call budget. Jobs that start and finish between two queue scans are not
//! missed, because nothing here depends on watching state transitions.
use crate::ciagent::github::Gh;
use crate::ciagent::incidents;
use crate::ciagent::model::{DayManifest, JobRecord};
use crate::ciagent::parse;
use crate::ciagent::report::{day_of, day_start};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// Stop below this many remaining API calls: the dispatcher shares the token.
pub const RATE_FLOOR: i64 = 2000;
/// Days re-read before a day is closed (today, yesterday, the day before).
pub const OPEN_DAYS: i64 = 3;
/// Closed days re-sent per pass. UDP loses ~0.1-0.3% of packets and
/// the transport has no acknowledgement, so each closed day is streamed once
/// more; the hub dedups by job id. A few per pass keeps the outbox small while
/// the backlog of already-closed days catches up.
pub const RESEND_PER_PASS: usize = 2;
/// Failed GETs tolerated per pass before it stops (a failed run is retried next pass).
pub const MAX_ERRORS: u32 = 10;

/// One ledger line from a run and one of its jobs (`filter=all` job object).
pub fn record_of(day: &str, repo: &str, run: &Value, j: &Value) -> JobRecord {
    let labels: Vec<String> = j["labels"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| l.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let t = |k: &str| j[k].as_str().and_then(parse::rfc3339_utc);
    let started = t("started_at");
    let runner = j["runner_name"].as_str().unwrap_or("").to_string();
    JobRecord {
        d: day.to_string(),
        id: j["id"].as_u64().unwrap_or(0),
        run: j["run_id"]
            .as_u64()
            .or_else(|| run["id"].as_u64())
            .unwrap_or(0),
        att: j["run_attempt"].as_u64().unwrap_or(1) as u32,
        repo: repo.to_string(),
        wf: run["name"].as_str().unwrap_or("").to_string(),
        job: j["name"].as_str().unwrap_or("").to_string(),
        pool: parse::pool_of(&labels),
        // a job that never got a runner has no meaningful start (GitHub fills
        // started_at for skipped jobs anyway): keep it out of wait/work
        s: if runner.is_empty() { None } else { started },
        e: if runner.is_empty() {
            None
        } else {
            t("completed_at")
        },
        runner,
        ev: run["event"].as_str().unwrap_or("").to_string(),
        c: t("created_at").unwrap_or(0),
        r: j["conclusion"].as_str().unwrap_or("").to_string(),
        sig: None,
    }
}

/// Files under the agent's StateDirectory:
///   <day>.jsonl     the day's records
///   <day>.runs      `run_id:attempt` already fetched (one per line); a re-run is a new attempt
///   <day>.manifest  DayManifest JSON (count + xor of ids)
///   outbox.jsonl    records not yet sent to the hub
pub struct Store {
    dir: PathBuf,
    /// The ledger thread appends to the outbox while the send loop drains it:
    /// without this lock a drain could rewrite the file over a fresh append.
    outbox: std::sync::Mutex<()>,
}

impl Store {
    pub fn open(dir: &str) -> Result<Store, String> {
        let dir = PathBuf::from(dir).join("ledger");
        fs::create_dir_all(&dir).map_err(|e| format!("ledger dir {}: {e}", dir.display()))?;
        Ok(Store {
            dir,
            outbox: std::sync::Mutex::new(()),
        })
    }
    fn p(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
    pub fn fetched(&self, day: &str) -> HashSet<String> {
        fs::read_to_string(self.p(&format!("{day}.runs")))
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }
    /// Job ids already recorded for a day (re-fetching a re-run returns old attempts too).
    pub fn ids(&self, day: &str) -> HashSet<u64> {
        fs::read_to_string(self.p(&format!("{day}.jsonl")))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<JobRecord>(l).ok())
            .map(|r| r.id)
            .collect()
    }
    pub fn manifest(&self, day: &str) -> DayManifest {
        fs::read_to_string(self.p(&format!("{day}.manifest")))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(DayManifest {
                day: day.to_string(),
                ..Default::default()
            })
    }
    fn put_manifest(&self, m: &DayManifest) -> Result<(), String> {
        let tmp = self.p(&format!("{}.manifest.tmp", m.day));
        fs::write(&tmp, serde_json::to_vec(m).unwrap_or_default()).map_err(|e| e.to_string())?;
        fs::rename(&tmp, self.p(&format!("{}.manifest", m.day))).map_err(|e| e.to_string())
    }
    fn append_lines(&self, name: &str, lines: impl Iterator<Item = String>) -> Result<(), String> {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.p(name))
            .map_err(|e| format!("{name}: {e}"))?;
        for l in lines {
            writeln!(f, "{l}").map_err(|e| format!("{name}: {e}"))?;
        }
        Ok(())
    }
    /// Record one fetched run: records first, then the outbox, manifest and run id,
    /// so a crash in between re-fetches the run instead of losing it (the hub dedups).
    pub fn add_run(&self, day: &str, run_key: &str, recs: &[JobRecord]) -> Result<(), String> {
        let lines: Vec<String> = recs
            .iter()
            .map(|r| serde_json::to_string(r).unwrap_or_default())
            .collect();
        self.append_lines(&format!("{day}.jsonl"), lines.iter().cloned())?;
        {
            let _g = self.outbox.lock().unwrap_or_else(|e| e.into_inner());
            self.append_lines("outbox.jsonl", lines.into_iter())?;
        }
        let mut m = self.manifest(day);
        for r in recs {
            m.n += 1;
            m.x ^= r.id;
        }
        self.put_manifest(&m)?;
        self.append_lines(&format!("{day}.runs"), std::iter::once(run_key.to_string()))
    }
    pub fn close(&self, day: &str) -> Result<(), String> {
        let mut m = self.manifest(day);
        m.closed = true;
        self.put_manifest(&m)
    }
    /// Re-send up to `max` closed days that were never re-sent, newest first: every
    /// record of the day goes into the outbox once more, then `<day>.resent` marks it.
    /// A record lost on the first send now needs to be lost twice to stay missing.
    pub fn resend_closed(&self, max: usize) -> Vec<String> {
        let mut done = vec![];
        for m in self.manifests(64) {
            if done.len() >= max {
                break;
            }
            let marker = self.p(&format!("{}.resent", m.day));
            if !m.closed || marker.exists() {
                continue;
            }
            let lines = fs::read_to_string(self.p(&format!("{}.jsonl", m.day))).unwrap_or_default();
            let ok = {
                let _g = self.outbox.lock().unwrap_or_else(|e| e.into_inner());
                self.append_lines(
                    "outbox.jsonl",
                    lines
                        .lines()
                        .filter(|l| !l.trim().is_empty())
                        .map(String::from),
                )
                .is_ok()
            };
            if ok && fs::write(&marker, b"").is_ok() {
                done.push(m.day.clone());
            }
        }
        done
    }

    /// Manifests of every day the agent has touched, newest first.
    pub fn manifests(&self, max: usize) -> Vec<DayManifest> {
        let mut days: Vec<String> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".manifest"))
                    .map(String::from)
            })
            .collect();
        days.sort();
        days.into_iter()
            .rev()
            .take(max)
            .map(|d| self.manifest(&d))
            .collect()
    }
    /// Up to `n` records from the outbox. The outbox file is rewritten without them;
    /// the hub dedups, so a crash after this only re-sends, never loses.
    pub fn take_outbox(&self, n: usize) -> Vec<JobRecord> {
        let _g = self.outbox.lock().unwrap_or_else(|e| e.into_inner());
        let path = self.p("outbox.jsonl");
        let all = fs::read_to_string(&path).unwrap_or_default();
        if all.is_empty() {
            return vec![];
        }
        let lines: Vec<&str> = all.lines().filter(|l| !l.trim().is_empty()).collect();
        let take: Vec<JobRecord> = lines
            .iter()
            .take(n)
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        let rest = lines
            .iter()
            .skip(n)
            .map(|l| format!("{l}\n"))
            .collect::<String>();
        let tmp = self.p("outbox.jsonl.tmp");
        if fs::write(&tmp, rest)
            .and_then(|_| fs::rename(&tmp, &path))
            .is_err()
        {
            return vec![]; // keep the outbox intact rather than lose records
        }
        take
    }
}

#[derive(Debug, Default)]
pub struct PassStats {
    pub days: Vec<String>,
    pub runs: u32,
    pub jobs: u32,
    pub calls: u32,
    /// GETs that failed and were skipped (their runs are retried next pass).
    pub errors: u32,
    /// Closed days queued for their one re-send this pass.
    pub resent: Vec<String>,
    /// Job logs read for error signatures, and how many of them matched.
    pub logs: u32,
    pub sigs: u32,
    pub stopped: Option<String>,
}

/// Days to work on this pass: the open window first (newest first), then the
/// newest not-yet-closed day of the backfill range.
pub fn plan(now: i64, backfill_days: i64, closed: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut out: Vec<String> = (0..OPEN_DAYS).map(|i| day_of(now - i * 86400)).collect();
    out.extend(
        (OPEN_DAYS..=backfill_days)
            .map(|i| day_of(now - i * 86400))
            .filter(|d| !closed(d)),
    );
    out
}

/// One ledger pass within `budget` GET calls. Returns what it did; a day is closed
/// only when it is past the open window AND every run of it was fetched.
pub fn pass(
    gh: &Gh,
    store: &Store,
    org: &str,
    repos: &[String],
    now: i64,
    backfill_days: i64,
    budget: u32,
) -> PassStats {
    let mut st = PassStats::default();
    let start_calls = gh.calls.get();
    let over = |st: &mut PassStats| {
        let used = gh.calls.get() - start_calls;
        if used >= budget {
            st.stopped = Some(format!("call budget {budget} used"));
        } else if gh.rate_remaining.get() < RATE_FLOOR {
            st.stopped = Some(format!(
                "rate limit floor: {} remaining",
                gh.rate_remaining.get()
            ));
        }
        st.stopped.is_some()
    };
    let today0 = day_start(&day_of(now)).unwrap_or(now);
    let mut errors = 0u32;
    'days: for day in plan(now, backfill_days, &|d| store.manifest(d).closed) {
        let mut complete = true;
        let fetched = store.fetched(&day);
        let mut ids = store.ids(&day);
        for repo in repos {
            let mut page = 1;
            loop {
                if over(&mut st) {
                    break 'days;
                }
                let v = match gh.get(&format!("/repos/{org}/{repo}/actions/runs?created={day}&status=completed&per_page=100&page={page}")) {
                    Ok(v) => v,
                    Err(e) => {
                        errors += 1;
                        complete = false;
                        if errors >= MAX_ERRORS { st.stopped = Some(format!("{errors} errors, last: {e}")); break 'days; }
                        break;
                    }
                };
                let runs = v["workflow_runs"].as_array().cloned().unwrap_or_default();
                'runs: for run in &runs {
                    let id = run["id"].as_u64().unwrap_or(0);
                    let key = format!("{id}:{}", run["run_attempt"].as_u64().unwrap_or(1));
                    if fetched.contains(&key) {
                        continue;
                    }
                    if over(&mut st) {
                        break 'days;
                    }
                    let attempts = run["run_attempt"].as_u64().unwrap_or(1);
                    let mut recs = match run_jobs(&|p| gh.get(p), org, repo, id, attempts) {
                        Ok(jobs) => jobs
                            .iter()
                            .map(|x| record_of(&day, repo, run, x))
                            .collect::<Vec<_>>(),
                        Err(e) => {
                            // skip this run, leave it unmarked (the next pass retries it)
                            // and keep the day open; only repeated failures stop the pass
                            errors += 1;
                            complete = false;
                            if errors >= MAX_ERRORS {
                                st.stopped = Some(format!("{errors} errors, last: {e}"));
                                break 'days;
                            }
                            continue 'runs;
                        }
                    };
                    recs.retain(|r| ids.insert(r.id));
                    for r in recs.iter_mut() {
                        let recent = r.e.map(|e| now - e <= incidents::RECENT_S).unwrap_or(false);
                        if !incidents::failed(&r.r)
                            || !recent
                            || st.logs as usize >= incidents::MAX_LOGS_PER_PASS
                            || over(&mut st)
                        {
                            continue;
                        }
                        st.logs += 1;
                        // a log that cannot be read leaves the record without a signature
                        if let Ok(log) = gh.log_tail(org, repo, r.id, incidents::LOG_TAIL) {
                            r.sig = incidents::of_log(&log).map(|s| s.encode());
                            st.sigs += r.sig.is_some() as u32;
                        }
                    }
                    if st.stopped.is_some() {
                        break 'days;
                    }
                    if let Err(e) = store.add_run(&day, &key, &recs) {
                        st.stopped = Some(e);
                        break 'days;
                    }
                    st.runs += 1;
                    st.jobs += recs.len() as u32;
                }
                if runs.len() < 100 {
                    break;
                }
                page += 1;
            }
        }
        if !st.days.contains(&day) {
            st.days.push(day.clone())
        }
        let past_window = day_start(&day)
            .map(|d| today0 - d >= OPEN_DAYS * 86400)
            .unwrap_or(false);
        if complete && past_window {
            let _ = store.close(&day);
        }
    }
    st.calls = gh.calls.get() - start_calls;
    st.errors = errors;
    st.resent = store.resend_closed(RESEND_PER_PASS);
    st
}

/// Every job of a run, all attempts. `filter=all` first; when GitHub fails it (it answers 502 for
/// some runs with many re-runs, attempt 7 or 8 of one run kept a ledger day open for weeks),
/// the same jobs attempt by attempt.
fn run_jobs(
    get: &dyn Fn(&str) -> Result<Value, String>,
    org: &str,
    repo: &str,
    id: u64,
    attempts: u64,
) -> Result<Vec<Value>, String> {
    let pages = |base: &str| -> Result<Vec<Value>, String> {
        let mut out = vec![];
        for page in 1.. {
            let j = get(&format!("{base}per_page=100&page={page}"))?;
            let jobs = j["jobs"].as_array().cloned().unwrap_or_default();
            let n = jobs.len();
            out.extend(jobs);
            if n < 100 {
                break;
            }
        }
        Ok(out)
    };
    match pages(&format!(
        "/repos/{org}/{repo}/actions/runs/{id}/jobs?filter=all&"
    )) {
        Ok(jobs) => Ok(jobs),
        Err(first) => {
            let mut all = vec![];
            for a in 1..=attempts.max(1) {
                all.extend(
                    pages(&format!(
                        "/repos/{org}/{repo}/actions/runs/{id}/attempts/{a}/jobs?"
                    ))
                    .map_err(|e| format!("{first}; per attempt: {e}"))?,
                );
            }
            Ok(all)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn run_jobs_falls_back_to_attempts_when_filter_all_fails() {
        let calls = std::cell::RefCell::new(vec![]);
        let get = |p: &str| -> Result<Value, String> {
            calls.borrow_mut().push(p.to_string());
            if p.contains("filter=all") {
                return Err(format!("GET {p}: status code 502"));
            }
            let a: u64 = p
                .split("/attempts/")
                .nth(1)
                .and_then(|r| r.split('/').next())
                .and_then(|n| n.parse().ok())
                .unwrap();
            Ok(json!({"jobs": [{"id": 100 + a, "run_attempt": a}]}))
        };
        let jobs = run_jobs(&get, "example-org", "example-repo", 1000000001, 3).unwrap();
        assert_eq!(
            jobs.iter()
                .map(|j| j["id"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![101, 102, 103]
        );
        assert_eq!(
            calls.borrow().len(),
            4,
            "one failed filter=all, then one page per attempt"
        );
        // filter=all working: no per-attempt calls
        let ok = |_: &str| -> Result<Value, String> { Ok(json!({"jobs": [{"id": 1}]})) };
        assert_eq!(run_jobs(&ok, "o", "r", 1, 5).unwrap().len(), 1);
        // both failing: the error names both
        let bad = |p: &str| -> Result<Value, String> { Err(format!("GET {p}: 502")) };
        let e = run_jobs(&bad, "o", "r", 1, 2).unwrap_err();
        assert!(e.contains("filter=all") && e.contains("per attempt"), "{e}");
    }

    #[test]
    fn records_from_github_shapes() {
        let run = json!({"id": 7, "name": "CI", "event": "pull_request"});
        let ran = json!({"id": 11, "run_id": 7, "run_attempt": 2, "name": "Clippy", "labels": ["self-hosted","linux","x64","rust-cpu"],
            "runner_name": "ci-host-01-3", "created_at": "2026-10-21T10:00:00Z", "started_at": "2026-10-21T10:30:00Z",
            "completed_at": "2026-10-21T10:40:00Z", "conclusion": "success"});
        let r = record_of("2026-10-21", "spar", &run, &ran);
        assert_eq!(
            (
                r.id,
                r.run,
                r.att,
                r.pool.as_str(),
                r.wf.as_str(),
                r.r.as_str()
            ),
            (11, 7, 2, "rust-cpu", "CI", "success")
        );
        assert_eq!(
            (r.s.unwrap() - r.c, r.e.unwrap() - r.s.unwrap()),
            (1800, 600),
            "wait 30m, work 10m"
        );
        // skipped: GitHub fills started/completed but there was never a runner
        let skipped = json!({"id": 12, "name": "deploy", "labels": ["ubuntu-latest"], "runner_name": null,
            "created_at": "2026-10-21T10:00:00Z", "started_at": "2026-10-21T10:00:01Z", "completed_at": "2026-10-21T10:00:01Z", "conclusion": "skipped"});
        let s = record_of("2026-10-21", "spar", &run, &skipped);
        assert!(s.s.is_none() && s.e.is_none() && s.runner.is_empty());
        assert_eq!(
            (s.pool.as_str(), s.run),
            ("hosted:ubuntu-latest", 7),
            "run id falls back to the run"
        );
    }

    #[test]
    fn plan_reads_open_window_then_unclosed_backfill() {
        let now = day_start("2026-10-22").unwrap() + 3600;
        let p = plan(now, 6, &|d| d == "2026-10-18");
        assert_eq!(
            p,
            vec![
                "2026-10-22",
                "2026-10-21",
                "2026-10-20",
                "2026-10-19",
                "2026-10-17",
                "2026-10-16"
            ]
        );
    }

    #[test]
    fn store_manifest_outbox_round_trip() {
        let dir =
            std::env::temp_dir().join(format!("warte-ciagent-ledger-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let st = Store::open(dir.to_str().unwrap()).unwrap();
        let rec = |id: u64| JobRecord {
            d: "2026-10-21".into(),
            id,
            run: 7,
            repo: "spar".into(),
            ..Default::default()
        };
        st.add_run("2026-10-21", "7:1", &[rec(1), rec(2), rec(4)])
            .unwrap();
        st.add_run("2026-10-21", "8:1", &[rec(8)]).unwrap();
        let m = st.manifest("2026-10-21");
        assert_eq!((m.n, m.x, m.closed), (4, 1 ^ 2 ^ 4 ^ 8, false));
        assert_eq!(
            st.fetched("2026-10-21"),
            ["7:1".to_string(), "8:1".to_string()].into_iter().collect()
        );
        assert_eq!(st.ids("2026-10-21"), [1, 2, 4, 8].into_iter().collect());
        let a = st.take_outbox(3);
        let b = st.take_outbox(3);
        assert_eq!(
            (a.len(), b.len(), st.take_outbox(3).len()),
            (3, 1, 0),
            "outbox drains in order, nothing lost"
        );
        assert_eq!(b[0].id, 8);
        st.close("2026-10-21").unwrap();
        assert!(st.manifests(10)[0].closed);
        // closed day: re-sent exactly once, all four records back in the outbox
        assert_eq!(st.resend_closed(2), vec!["2026-10-21".to_string()]);
        assert_eq!(
            st.take_outbox(10).iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![1, 2, 4, 8]
        );
        assert!(
            st.resend_closed(2).is_empty(),
            "the marker stops a second re-send"
        );
        // an open day is never re-sent, and the per-pass cap holds
        st.add_run("2026-10-22", "9:1", &[rec(9)]).unwrap();
        let _ = st.take_outbox(10);
        assert!(st.resend_closed(2).is_empty(), "open days are not re-sent");
        let _ = fs::remove_dir_all(&dir);
    }
}

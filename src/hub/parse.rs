//! Pure parsers. No I/O here, so every rule that bit us during manual
//! investigation is pinned by a unit test instead of re-learned by hand.
use crate::hub::model::{Disk, JobContext, PsiLine};
use std::collections::BTreeMap;

/// `/proc/loadavg` -> [load1, load5, load15]
pub fn loadavg(s: &str) -> Option<[f64; 3]> {
    let mut it = s.split_whitespace().map(|x| x.parse::<f64>());
    Some([it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?])
}

/// `/proc/meminfo` -> (total, available, swap_total, swap_free) in MiB.
pub fn meminfo(s: &str) -> (u64, u64, u64, u64) {
    let mut m: BTreeMap<&str, u64> = BTreeMap::new();
    for line in s.lines() {
        let mut p = line.split_whitespace();
        if let (Some(k), Some(v)) = (p.next(), p.next()) {
            if let Ok(v) = v.parse::<u64>() {
                m.insert(k.trim_end_matches(':'), v / 1024);
            }
        }
    }
    let g = |k: &str| *m.get(k).unwrap_or(&0);
    (
        g("MemTotal"),
        g("MemAvailable"),
        g("SwapTotal"),
        g("SwapFree"),
    )
}

/// First (`some`) line of `/proc/pressure/<res>`.
pub fn psi(s: &str) -> PsiLine {
    let mut out = PsiLine::default();
    if let Some(line) = s.lines().find(|l| l.starts_with("some")) {
        for kv in line.split_whitespace() {
            if let Some((k, v)) = kv.split_once('=') {
                let v = v.parse::<f64>().unwrap_or(0.0);
                match k {
                    "avg10" => out.avg10 = v,
                    "avg60" => out.avg60 = v,
                    "avg300" => out.avg300 = v,
                    _ => {}
                }
            }
        }
    }
    out
}

/// `df -P -B1 <mounts...>` -> disks. Header line is skipped.
pub fn df(s: &str) -> Vec<Disk> {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    s.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 6 {
                return None;
            }
            let size = f[1].parse::<f64>().ok()?;
            let used = f[2].parse::<f64>().ok()?;
            let pct = f[4].trim_end_matches('%').parse::<f64>().ok()?;
            Some(Disk {
                mount: f[5].to_string(),
                size_gb: size / GB,
                used_gb: used / GB,
                used_pct: pct,
            })
        })
        .collect()
}

/// `systemctl show -p A -p B <unit>` -> map.
pub fn systemctl_show(s: &str) -> BTreeMap<String, String> {
    s.lines()
        .filter_map(|l| {
            l.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect()
}

/// First `key value` line of a cgroup flat-keyed file (cpu.stat, memory.events).
pub fn cg_key(s: &str, key: &str) -> Option<u64> {
    s.lines().find_map(|l| {
        l.split_once(' ')
            .filter(|(k, _)| *k == key)
            .and_then(|(_, v)| v.trim().parse().ok())
    })
}

/// Bytes in a cgroup file to MB; `max` (unlimited) and garbage are `None`.
pub fn cg_mb(s: &str) -> Option<u32> {
    s.trim()
        .parse::<u64>()
        .ok()
        .map(|b| (b / (1024 * 1024)) as u32)
}

/// Cores used between two `usage_usec` samples taken `dt_s` seconds apart.
/// A counter that went BACKWARDS means the service restarted: no rate.
pub fn cpu_cores(prev_usec: u64, now_usec: u64, dt_s: f64) -> Option<f64> {
    if dt_s <= 0.5 || now_usec < prev_usec {
        return None;
    }
    Some((now_usec - prev_usec) as f64 / 1e6 / dt_s)
}

#[derive(Debug, Default, PartialEq)]
pub struct JobState {
    pub job: Option<String>,
    pub started: Option<i64>,
    pub last_result: Option<String>,
}

/// Current job from `journalctl -u github-runner@runnerN -o short-unix` lines.
/// The listener logs "Running job: X" as soon as it accepts work (before the
/// worker spawns) and "Job X completed with result: R" at the end; the LAST of
/// those two kinds of line decides whether the runner is mid-job.
pub fn job_state<'a>(lines: impl IntoIterator<Item = &'a str>) -> JobState {
    let mut st = JobState::default();
    for line in lines {
        let epoch = line
            .split_whitespace()
            .next()
            .and_then(|t| t.split('.').next())
            .and_then(|t| t.parse::<i64>().ok());
        if let Some(i) = line.find("Running job: ") {
            st.job = Some(line[i + "Running job: ".len()..].trim().to_string());
            st.started = epoch;
        } else if let Some(i) = line.find(" completed with result: ") {
            st.last_result = Some(
                line[i + " completed with result: ".len()..]
                    .trim()
                    .to_string(),
            );
            st.job = None;
            st.started = None;
        }
    }
    st
}

const GENERIC: [&str; 4] = ["self-hosted", "linux", "x64", "hetzner"];

/// Pool a job (or runner) belongs to, from its LABELS — not from a runner's
/// class. A self-hosted job needing `rust-cpu` and `light` lives in pool
/// "light+rust-cpu", served only by runners carrying both.
pub fn pool_of(labels: &[String]) -> String {
    let lower: Vec<String> = labels.iter().map(|l| l.to_ascii_lowercase()).collect();
    if !lower.iter().any(|l| l == "self-hosted") {
        return "hosted".to_string();
    }
    let mut specific: Vec<String> = lower
        .into_iter()
        .filter(|l| !GENERIC.contains(&l.as_str()))
        .collect();
    specific.sort();
    specific.dedup();
    if specific.is_empty() {
        "self-hosted".to_string()
    } else {
        specific.join("+")
    }
}

/// `relatime` / `noatime` / `atime` for a mount point in /proc/<pid>/mountinfo.
pub fn mount_atime(mountinfo: &str, mount_point: &str) -> Option<String> {
    let needle = format!(" {} ", mount_point);
    let line = mountinfo.lines().find(|l| l.contains(&needle))?;
    let opts = line.split_whitespace().nth(5)?;
    Some(
        if opts.contains("noatime") {
            "noatime"
        } else if opts.contains("relatime") {
            "relatime"
        } else {
            "atime"
        }
        .to_string(),
    )
}

/// (oldest, median) of ages in seconds; (0, 0) when empty.
pub fn age_stats(ages: &mut [i64]) -> (i64, i64) {
    if ages.is_empty() {
        return (0, 0);
    }
    ages.sort_unstable();
    (ages[ages.len() - 1], ages[ages.len() / 2])
}

/// Parse an RFC 3339 UTC timestamp like "2026-10-22T07:10:39Z" to epoch seconds.
pub fn rfc3339_utc(s: &str) -> Option<i64> {
    let s = s.trim_end_matches('Z');
    let (date, time) = s.split_once('T')?;
    let d: Vec<i64> = date
        .split('-')
        .map(|x| x.parse().ok())
        .collect::<Option<_>>()?;
    let t: Vec<i64> = time
        .split(':')
        .map(|x| x.split('.').next().unwrap_or(x).parse().ok())
        .collect::<Option<_>>()?;
    if d.len() != 3 || t.len() != 3 {
        return None;
    }
    let (y, m, day) = (d[0], d[1], d[2]);
    // days since 1970-01-01 (civil-from-days inverse, Howard Hinnant's algorithm)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + t[0] * 3600 + t[1] * 60 + t[2])
}

/// Value of the first `"k": "<key>"` entry at or after `from` whose value is a
/// string or number (object/array values are skipped: e.g. `repository` is
/// both the "owner/name" string and a nested object in the event payload).
fn ctx_value(text: &str, key: &str, from: usize) -> Option<(String, usize)> {
    let needles = [format!("\"k\": \"{key}\""), format!("\"k\":\"{key}\"")];
    let mut pos = from;
    loop {
        let (i, n) = needles
            .iter()
            .filter_map(|n| {
                text.get(pos..)?
                    .find(n.as_str())
                    .map(|i| (pos + i, n.len()))
            })
            .min()?;
        let after = i + n;
        let rest = text.get(after..)?;
        let vpos = rest.find("\"v\"")?;
        if vpos > 120 {
            pos = after;
            continue;
        }
        let v = rest[vpos + 3..]
            .trim_start()
            .strip_prefix(':')?
            .trim_start();
        if let Some(s) = v.strip_prefix('"') {
            let mut out = String::new();
            let mut esc = false;
            for c in s.chars() {
                if esc {
                    out.push(match c {
                        'n' => ' ',
                        't' => ' ',
                        o => o,
                    });
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    return Some((out, after));
                } else {
                    out.push(c);
                }
            }
            return None;
        }
        if v.starts_with('{') || v.starts_with('[') {
            pos = after;
            continue;
        }
        let end = v.find([',', '\n', '}']).unwrap_or(v.len());
        return Some((v[..end].trim().to_string(), after));
    }
}

/// Byte offset just past the first `"k": "<key>"` KEY (any value type). Never
/// matches a value: `"pull_request"` also appears as the value of event_name.
fn key_pos(text: &str, key: &str) -> Option<usize> {
    [format!("\"k\": \"{key}\""), format!("\"k\":\"{key}\"")]
        .iter()
        .filter_map(|n| text.find(n.as_str()).map(|i| i + n.len()))
        .min()
}

fn num(s: &str) -> Option<u64> {
    s.split('.').next()?.trim().parse().ok()
}

/// Job context from a runner's `_diag/Worker_*.log` (GitHub's contextData,
/// encoded as `{"k": name, "v": value}` pairs). Best effort: missing fields stay empty.
pub fn job_context(text: &str) -> Option<JobContext> {
    let g = |k: &str| ctx_value(text, k, 0).map(|(v, _)| v).unwrap_or_default();
    let repo = g("repository");
    if repo.is_empty() || !repo.contains('/') {
        return None;
    }
    let reference = g("ref");
    // refs/pull/511/merge (or /head) -> 511; fall back to pull_request.number
    let pr = reference
        .strip_prefix("refs/pull/")
        .and_then(|r| r.split('/').next())
        .and_then(|n| n.parse::<u32>().ok())
        .or_else(|| {
            let at = key_pos(text, "pull_request")?;
            ctx_value(text, "number", at)
                .and_then(|(v, _)| num(&v))
                .map(|n| n as u32)
        });
    let pr_title = if pr.is_some() {
        key_pos(text, "pull_request")
            .and_then(|at| ctx_value(text, "title", at))
            .map(|(v, _)| v)
            .filter(|v| !v.is_empty())
    } else {
        None
    };
    let head = g("head_ref");
    Some(JobContext {
        workflow: g("workflow"),
        run_id: num(&g("run_id")).unwrap_or(0),
        run_number: num(&g("run_number")).unwrap_or(0) as u32,
        run_attempt: num(&g("run_attempt")).unwrap_or(0) as u32,
        event: g("event_name"),
        branch: if head.is_empty() { g("ref_name") } else { head },
        base: g("base_ref"),
        sha: g("sha"),
        actor: g("actor"),
        pr,
        pr_title,
        repo,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgroup_counters() {
        assert_eq!(
            cg_key("usage_usec 7062086344767\nuser_usec 1\n", "usage_usec"),
            Some(7062086344767)
        );
        assert_eq!(
            cg_key("low 0\nhigh 0\nmax 3\noom 2\noom_kill 1\n", "oom_kill"),
            Some(1)
        );
        assert_eq!(cg_key("oom 2\n", "oom_kill"), None);
        assert_eq!(cg_mb("51539607552\n"), Some(49152));
        assert_eq!(cg_mb("max\n"), None);
        assert_eq!(cpu_cores(1_000_000, 121_000_000, 60.0), Some(2.0));
        assert_eq!(cpu_cores(5, 1, 60.0), None, "counter reset = restart");
        assert_eq!(cpu_cores(1, 5, 0.0), None);
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn loadavg_parses() {
        assert_eq!(
            loadavg("98.11 85.65 80.75 3/2131 4711\n"),
            Some([98.11, 85.65, 80.75])
        );
        assert_eq!(loadavg("garbage"), None);
    }

    #[test]
    fn meminfo_parses_to_mib() {
        let m = "MemTotal:       131072000 kB\nMemFree: 1 kB\nMemAvailable:    95420416 kB\nSwapTotal: 16777216 kB\nSwapFree: 12582912 kB\n";
        assert_eq!(meminfo(m), (128000, 93184, 16384, 12288));
    }

    #[test]
    fn psi_reads_the_some_line_not_full() {
        let p = "some avg10=69.87 avg60=66.62 avg300=59.49 total=2226373594367\nfull avg10=1.00 avg60=2.00 avg300=3.00 total=5\n";
        assert_eq!(
            psi(p),
            PsiLine {
                avg10: 69.87,
                avg60: 66.62,
                avg300: 59.49
            }
        );
    }

    #[test]
    fn df_parses_and_skips_header() {
        let d = "Filesystem 1-blocks Used Available Capacity Mounted on\n/dev/mapper/vg_data-lv_runners 1056561627136 597000000000 412000000000 60% /var/lib/runners\n";
        let v = df(d);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].mount, "/var/lib/runners");
        assert_eq!(v[0].used_pct, 60.0);
    }

    #[test]
    fn job_state_running_then_completed_then_running() {
        let lines = [
            "1792653039.1 h run.sh[1]: 2026-10-22 07:10:39Z: Running job: Kani (relay-dronecan)",
            "1792653049.2 h run.sh[1]: 2026-10-22 07:10:49Z: Job Kani (relay-dronecan) completed with result: Succeeded",
            "1792653051.3 h run.sh[1]: 2026-10-22 07:10:51Z: Running job: Kani (relay-nid)",
        ];
        let st = job_state(lines.iter().copied());
        assert_eq!(st.job.as_deref(), Some("Kani (relay-nid)"));
        assert_eq!(st.started, Some(1792653051));
        assert_eq!(st.last_result.as_deref(), Some("Succeeded"));
    }

    #[test]
    fn job_state_idle_after_completion() {
        let lines = [
            "1792653039.1 h run.sh[1]: Running job: Test",
            "1792653049.2 h run.sh[1]: Job Test completed with result: Failed",
        ];
        assert_eq!(
            job_state(lines.iter().copied()),
            JobState {
                job: None,
                started: None,
                last_result: Some("Failed".into())
            }
        );
    }

    #[test]
    fn pool_comes_from_labels_not_class() {
        // runner5 / runner10 carry BOTH; a job needing both is its own pool
        assert_eq!(
            pool_of(&s(&[
                "self-hosted",
                "Linux",
                "X64",
                "hetzner",
                "rust-cpu",
                "light"
            ])),
            "light+rust-cpu"
        );
        assert_eq!(
            pool_of(&s(&["self-hosted", "linux", "x64", "lean-mem"])),
            "lean-mem"
        );
        assert_eq!(
            pool_of(&s(&["self-hosted", "linux", "x64", "rust-cpu", "podman"])),
            "podman+rust-cpu"
        );
        assert_eq!(pool_of(&s(&["ubuntu-latest"])), "hosted");
        assert_eq!(pool_of(&s(&["self-hosted"])), "self-hosted");
    }

    #[test]
    fn mount_atime_finds_the_runner_rustup_mount() {
        let mi = "310 290 253:2 /runner8/.rustup /var/lib/runners/runner8/.rustup rw,relatime shared:1 - ext4 /dev/x rw\n\
                  311 290 253:2 / /var/lib/runners rw,noatime shared:2 - ext4 /dev/x rw\n";
        assert_eq!(
            mount_atime(mi, "/var/lib/runners/runner8/.rustup").as_deref(),
            Some("relatime")
        );
        assert_eq!(
            mount_atime(mi, "/var/lib/runners").as_deref(),
            Some("noatime")
        );
        assert_eq!(mount_atime(mi, "/nope"), None);
    }

    #[test]
    fn age_stats_oldest_and_median() {
        let mut a = vec![300, 60, 63720, 1800, 120];
        assert_eq!(age_stats(&mut a), (63720, 300));
        assert_eq!(age_stats(&mut []), (0, 0));
    }

    // Shape copied from a real runner1 Worker log (spar PR #511, 2026-10-22):
    // k and v on separate lines, numbers as floats, `repository` ALSO present
    // as a nested object, which must be skipped.
    const WORKER: &str = r#"
      "reference": { "type": "repository", "name": "actions/checkout", "ref": "v4" },
              { "k": "repository",
                "v": { "t": 2, "d": [ { "k": "full_name", "v": "pulseengine/spar" } ] } },
              { "k": "repository",
                "v": "pulseengine/spar" },
              { "k": "workflow",
                "v": "CI" },
              { "k": "run_id",
                "v": "39739849534" },
              { "k": "run_number",
                "v": "1106" },
              { "k": "run_attempt",
                "v": "1" },
              { "k": "event_name",
                "v": "pull_request" },
              { "k": "ref",
                "v": "refs/pull/511/merge" },
              { "k": "head_ref",
                "v": "fix/binding-consumers-codegen" },
              { "k": "base_ref",
                "v": "main" },
              { "k": "sha",
                "v": "ce7e0ac49009ce57dc120104003d0feda16fee68" },
              { "k": "actor",
                "v": "avrabe" },
              { "k": "pull_request",
                "v": { "t": 2, "d": [
              { "k": "number",
                "v": 511.0
              },
              { "k": "title",
                "v": "fix(codegen): bind \"consumers\" correctly" } ] } }
"#;

    #[test]
    fn job_context_from_worker_log() {
        let c = job_context(WORKER).unwrap();
        assert_eq!(c.repo, "pulseengine/spar"); // the string, not the nested object
        assert_eq!(c.workflow, "CI");
        assert_eq!(
            (c.run_id, c.run_number, c.run_attempt),
            (39739849534, 1106, 1)
        );
        assert_eq!(c.event, "pull_request");
        assert_eq!(c.pr, Some(511));
        assert_eq!(
            c.pr_title.as_deref(),
            Some("fix(codegen): bind \"consumers\" correctly")
        );
        assert_eq!(c.branch, "fix/binding-consumers-codegen");
        assert_eq!(c.base, "main");
        assert_eq!(c.actor, "avrabe");
        assert_eq!(c.sha.len(), 40);
    }

    #[test]
    fn job_context_push_event_has_no_pr() {
        let push = WORKER
            .replace("refs/pull/511/merge", "refs/heads/main")
            .replace("\"k\": \"pull_request\"", "\"k\": \"x\"");
        let c = job_context(&push).unwrap();
        assert_eq!((c.pr, c.pr_title), (None, None));
    }

    #[test]
    fn job_context_none_without_repository() {
        assert!(job_context("no context here").is_none());
    }

    #[test]
    fn rfc3339_utc_matches_known_epochs() {
        assert_eq!(rfc3339_utc("1970-01-01T00:00:00Z"), Some(0));
        // verified independently: calendar.timegm(2026-10-22T07:10:39Z) == 1792653039
        assert_eq!(rfc3339_utc("2026-10-22T07:10:39Z"), Some(1792653039));
        // UTC parsing must not depend on local time or DST (the token-cache bug class)
        assert_eq!(
            rfc3339_utc("2026-03-29T01:00:00Z").unwrap()
                - rfc3339_utc("2026-03-28T01:00:00Z").unwrap(),
            86400
        );
    }
}

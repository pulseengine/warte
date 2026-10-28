//! Oracle for the CI/release workflows (v1.3.1, #34 #35 #39).
//!
//! The workflows are configuration, so nothing else checks them: a gate that
//! installs the wrong tool or masks its own failure stays green-looking until
//! someone reads the logs. These tests read the YAML and fail on the shapes
//! that broke the rivet gate on every release PR up to v1.3.0.

use serde_yaml::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn workflows() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
    let mut out: Vec<(String, String)> = fs::read_dir(&dir)
        .expect("read .github/workflows")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&p).unwrap())
        })
        .collect();
    // Fail closed: "no violations" must not mean "nothing was checked".
    assert!(
        !out.is_empty(),
        "no workflow files found in {}",
        dir.display()
    );
    out.sort();
    out
}

fn workflow(name: &str) -> Value {
    let (_, text) = workflows()
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("{name} not found"));
    serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn jobs(wf: &Value) -> Vec<(String, Value)> {
    wf["jobs"]
        .as_mapping()
        .expect("workflow has jobs")
        .iter()
        .map(|(k, v)| (k.as_str().unwrap().to_string(), v.clone()))
        .collect()
}

/// Every shell command in a workflow, with `\` continuations joined so a
/// command split over several lines is checked as one.
fn commands(wf: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for (_, job) in jobs(wf) {
        for step in job["steps"].as_sequence().into_iter().flatten() {
            let Some(run) = step["run"].as_str() else {
                continue;
            };
            let mut cmd = String::new();
            for line in run.lines() {
                let l = line.trim();
                if let Some(head) = l.strip_suffix('\\') {
                    cmd.push_str(head);
                    cmd.push(' ');
                } else {
                    cmd.push_str(l);
                    out.push(std::mem::take(&mut cmd));
                }
            }
            if !cmd.is_empty() {
                out.push(cmd);
            }
        }
    }
    out
}

fn is_toolchain(cmd: &str) -> bool {
    cmd.contains("rivet") || cmd.contains("varve") || cmd.contains("cargo install")
}

// rivet: verifies DD-CI-001
// rivet: verifies REQ-CI-001
#[test]
fn rivet_is_installed_from_pulseengine_git_only() {
    let mut installs = 0;
    for (name, _) in workflows() {
        for cmd in commands(&workflow(&name)) {
            assert!(
                !cmd.contains("varve-cli"),
                "{name}: installs `varve-cli`, which is not a published crate: {cmd}"
            );
            if cmd.contains("cargo install") && cmd.contains("rivet") {
                installs += 1;
                assert!(
                    cmd.contains("--git https://github.com/pulseengine/rivet")
                        && cmd.contains("--rev \"$RIVET_REV\"")
                        && !cmd.contains("||"),
                    "{name}: rivet must be installed only from pulseengine/rivet git at \
                     --rev \"$RIVET_REV\", with no fallback (crates.io `rivet-cli` is a \
                     different project): {cmd}"
                );
            }
        }
    }
    assert!(
        installs > 0,
        "no rivet install step found — oracle checked nothing"
    );
}

// rivet: verifies REQ-CI-004
#[test]
fn superseded_pr_runs_are_cancelled_but_main_never_is() {
    let wf = workflow("ci.yml");
    let c = &wf["concurrency"];
    let group = c["group"].as_str().expect("ci.yml has a concurrency group");
    assert!(
        group.contains("github.event.pull_request.number"),
        "group must key PR runs by PR number: {group}"
    );
    assert!(
        group.contains("github.sha"),
        "non-PR runs must get a per-commit group, so one push never cancels another: {group}"
    );
    assert_eq!(
        c["cancel-in-progress"].as_str(),
        Some("${{ github.event_name == 'pull_request' }}"),
        "only pull_request runs may be cancelled"
    );
}

// rivet: verifies REQ-RELEASE-005
#[test]
fn every_cargo_install_is_pinned() {
    let mut installs = 0;
    for (name, _) in workflows() {
        for cmd in commands(&workflow(&name)) {
            if !cmd.contains("cargo install") {
                continue;
            }
            installs += 1;
            assert!(
                cmd.contains("--version ") || cmd.contains("--rev "),
                "{name}: unpinned `cargo install` picks up whatever was published last: {cmd}"
            );
        }
    }
    assert!(
        installs > 0,
        "no cargo install found — oracle checked nothing"
    );
}

// rivet: verifies REQ-CI-001
#[test]
fn rivet_rev_is_one_full_sha_across_workflows() {
    let revs: Vec<(String, String)> = workflows()
        .into_iter()
        .filter_map(|(name, _)| {
            let rev = workflow(&name)["env"]["RIVET_REV"].as_str()?.to_string();
            Some((name, rev))
        })
        .collect();
    assert!(!revs.is_empty(), "no RIVET_REV pinned in any workflow");
    for (name, rev) in &revs {
        assert!(
            rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit()),
            "{name}: RIVET_REV must be a full commit SHA, got {rev:?}"
        );
    }
    assert!(
        revs.iter().all(|(_, r)| r == &revs[0].1),
        "RIVET_REV differs between workflows: {revs:?}"
    );
}

// rivet: verifies REQ-CI-001
// rivet: verifies REQ-CI-002
#[test]
fn toolchain_steps_are_not_masked() {
    for (name, _) in workflows() {
        let wf = workflow(&name);
        for (job_name, job) in jobs(&wf) {
            assert!(
                job.get("continue-on-error").is_none(),
                "{name}: job {job_name} sets continue-on-error"
            );
            for step in job["steps"].as_sequence().into_iter().flatten() {
                assert!(
                    step.get("continue-on-error").is_none(),
                    "{name}: a step in {job_name} sets continue-on-error"
                );
            }
        }
        for cmd in commands(&wf) {
            assert!(
                !(is_toolchain(&cmd) && cmd.contains("||")),
                "{name}: a toolchain command has a `||` fallback that can mask failure: {cmd}"
            );
        }
    }
}

// rivet: verifies REQ-CI-002
#[test]
fn release_gate_checks_the_tag_and_gates_every_job() {
    let wf = workflow("release.yml");
    let cmds = commands(&wf);
    assert!(
        cmds.iter().any(|c| c == "rivet release status \"$TAG\""),
        "release gate must run `rivet release status \"$TAG\"` as its own command, \
         so rivet's exit code decides"
    );
    assert!(
        !cmds.iter().any(|c| c.contains("Cuttable")),
        "release gate must use rivet's exit code, not grep its prose"
    );
    // Every other job must depend on the gate, directly or transitively.
    let all = jobs(&wf);
    let needs = |job: &Value| -> Vec<String> {
        match &job["needs"] {
            Value::String(s) => vec![s.clone()],
            Value::Sequence(v) => v
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect(),
            _ => vec![],
        }
    };
    let mut gated: BTreeSet<String> = ["release-gate".to_string()].into();
    loop {
        let before = gated.len();
        for (n, j) in &all {
            if needs(j).iter().any(|d| gated.contains(d)) {
                gated.insert(n.clone());
            }
        }
        if gated.len() == before {
            break;
        }
    }
    for (n, _) in &all {
        assert!(
            gated.contains(n),
            "release.yml: job {n} does not depend on release-gate"
        );
    }
}

// rivet: verifies REQ-RELEASE-003
#[test]
fn release_publishes_exactly_what_the_pipeline_produces() {
    let wf = workflow("release.yml");
    let publish = jobs(&wf)
        .into_iter()
        .flat_map(|(_, j)| j["steps"].as_sequence().cloned().unwrap_or_default())
        .find(|s| {
            s["uses"]
                .as_str()
                .is_some_and(|u| u.starts_with("softprops/action-gh-release@"))
        })
        .expect("release.yml has no GitHub release step");
    assert_eq!(
        publish["with"]["fail_on_unmatched_files"].as_bool(),
        Some(true),
        "a missing asset must fail the release, not be skipped"
    );
    let files: Vec<&str> = publish["with"]["files"]
        .as_str()
        .expect("release step lists files")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    // Artifacts are downloaded flat into release-artifacts/, so every asset
    // must be named at that level — a subdirectory glob silently matches nothing.
    for f in &files {
        let rest = f
            .strip_prefix("release-artifacts/")
            .unwrap_or_else(|| panic!("asset outside release-artifacts/: {f}"));
        assert!(
            !rest.contains('/'),
            "asset pattern points into a subdirectory: {f}"
        );
    }
    let uploads: Vec<String> = jobs(&wf)
        .into_iter()
        .flat_map(|(_, j)| j["steps"].as_sequence().cloned().unwrap_or_default())
        .filter(|s| {
            s["uses"]
                .as_str()
                .is_some_and(|u| u.starts_with("actions/upload-artifact@"))
        })
        .filter_map(|s| s["with"]["path"].as_str().map(String::from))
        .collect();
    for u in &uploads {
        assert!(
            !u.contains('*'),
            "upload path {u} uses a glob; its layout on download is not predictable"
        );
    }
}

fn runners(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("runs-on:"))
        .map(|v| v.trim().to_string())
        .collect()
}

/// smithy's rust-cpu pool, with fork PRs routed to a GitHub-hosted runner by
/// default. Routing only: a fork PR can edit the workflow, so isolation itself
/// is the org's runner-group policy and fork-PR approval, not this expression.
const SMITHY_CPU_FORK_GUARDED: &str = "${{ github.event.pull_request.head.repo.fork && 'ubuntu-latest' || fromJSON('[\"self-hosted\",\"linux\",\"x64\",\"rust-cpu\"]') }}";

// rivet: verifies DD-CI-001
// rivet: verifies REQ-CI-003
#[test]
fn ci_jobs_run_on_smithy_with_forks_guarded() {
    let (_, text) = workflows()
        .into_iter()
        .find(|(n, _)| n == "ci.yml")
        .expect("ci.yml");
    let rs = runners(&text);
    assert!(
        !rs.is_empty(),
        "ci.yml has no runs-on — oracle checked nothing"
    );
    for r in rs {
        assert_eq!(r, SMITHY_CPU_FORK_GUARDED, "ci.yml: unexpected runner");
    }
}

// rivet: verifies DD-CI-001
// rivet: verifies REQ-CI-003
#[test]
fn release_jobs_run_on_github_hosted_runners() {
    let (_, text) = workflows()
        .into_iter()
        .find(|(n, _)| n == "release.yml")
        .expect("release.yml");
    assert!(
        !text.contains("self-hosted"),
        "release.yml signs and publishes; it must not run on the shared self-hosted pool"
    );
    for cache in ["actions/cache@", "Swatinem/rust-cache@"] {
        assert!(
            !text.contains(cache),
            "release.yml must restore no caches ({cache}): main's smithy runs write \
             caches a tag run could restore"
        );
    }
    assert!(!runners(&text).is_empty(), "release.yml has no runs-on");
}

// rivet: verifies DD-CI-001
// rivet: verifies REQ-CI-001
#[test]
fn rivet_is_never_trusted_from_home() {
    for (name, text) in workflows() {
        assert!(
            !text.contains(".cargo/bin/rivet"),
            "{name}: rivet must be installed job-scoped, not reused from $HOME/.cargo/bin"
        );
        if text.contains("pulseengine/rivet") {
            assert!(
                text.contains("--root \"$ROOT\"") && text.contains("ROOT=\"$RUNNER_TEMP/rivet\""),
                "{name}: rivet must be installed into $RUNNER_TEMP/rivet"
            );
        }
    }
}

// rivet: verifies REQ-CI-001
#[test]
fn third_party_actions_are_pinned_to_a_sha() {
    let mut checked = 0;
    for (name, text) in workflows() {
        for line in text.lines() {
            let Some(u) = line.trim().trim_start_matches("- ").strip_prefix("uses:") else {
                continue;
            };
            let u = u.split('#').next().unwrap().trim();
            if u.starts_with("actions/") {
                continue;
            }
            checked += 1;
            let rev = u.rsplit('@').next().unwrap();
            assert!(
                rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit()),
                "{name}: third-party action not pinned to a commit SHA: {u}"
            );
        }
    }
    assert!(
        checked > 0,
        "no third-party actions found — oracle checked nothing"
    );
}

// rivet: verifies REQ-RELEASE-003
#[test]
fn release_metadata_is_not_in_the_repo_root() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let stray: Vec<String> = fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("RELEASE_NOTES_") || n.starts_with("SHA256SUMS"))
        .collect();
    assert!(
        stray.is_empty(),
        "release metadata in the repo root: {stray:?}"
    );
}

// rivet: verifies REQ-RELEASE-003
#[test]
fn changelog_top_entry_matches_cargo_version() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let changelog = fs::read_to_string(root.join("CHANGELOG.md")).expect("CHANGELOG.md");
    let top = changelog
        .lines()
        .find_map(|l| l.strip_prefix("## v"))
        .expect("CHANGELOG.md has no `## v<version>` entry");
    let top = top.split_whitespace().next().unwrap();
    assert_eq!(
        top,
        env!("CARGO_PKG_VERSION"),
        "CHANGELOG.md's newest entry and Cargo.toml's version disagree"
    );
}

// rivet: verifies REQ-RELEASE-003
#[test]
fn release_asserts_binary_version_equals_tag() {
    let (_, text) = workflows()
        .into_iter()
        .find(|(n, _)| n == "release.yml")
        .expect("release.yml");
    assert!(
        text.contains("/release/warte\" --version") && text.contains("\"warte ${TAG#v}\""),
        "release.yml must check that the built `warte --version` reports the tag"
    );
}

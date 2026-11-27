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
            // jig's advisory canary is the one job allowed to fail, by name
            // in the verdict (every_rust_toolchain_is_one_pinned_version_across_workflows
            // holds it to exactly one); its steps are still checked below
            assert!(
                job.get("continue-on-error").is_none() || advisory_canary(&wf, &job_name),
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

/// the self-hosted rust-cpu pool, with fork PRs routed to a GitHub-hosted runner by
/// default. Fail-closed: a pull request runs self-hosted only when its head repo
/// is this repo. The earlier `head.repo.fork && hosted || self-hosted` form sent
/// a PR whose fork was deleted (head.repo null) to self-hosted. Routing only: a fork PR can edit the workflow, so isolation itself
/// is the org's runner-group policy and fork-PR approval, not this expression.
const SELF_HOSTED_CPU_SAME_REPO_ONLY: &str = "${{ (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository) && fromJSON('[\"self-hosted\",\"linux\",\"x64\",\"rust-cpu\"]') || 'ubuntu-24.04' }}";
/// The light pool, for the classifier: same fail-closed form, label `light`.
const SELF_HOSTED_LIGHT_SAME_REPO_ONLY: &str = "${{ (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository) && fromJSON('[\"self-hosted\",\"linux\",\"x64\",\"light\"]') || 'ubuntu-24.04' }}";

// rivet: verifies DD-CI-001
// rivet: verifies REQ-CI-003
#[test]
fn ci_jobs_run_self_hosted_with_forks_guarded() {
    // Jobs that must run hosted, each with its reason. Every other job runs self-hosted.
    const HOSTED: &[(&str, &str, &str)] = &[(
        "playwright",
        "ubuntu-24.04",
        "playwright install --with-deps needs apt-get (sudo); self-hosted runners have none",
    )];
    let wf = workflow("ci.yml");
    let mut checked = 0;
    for (job, def) in jobs(&wf) {
        // a reusable-workflow call has no runs-on: jig pins its own fork guard
        // and plant guard checks it (trusted_workflows)
        if def["uses"].is_string() {
            assert!(def["runs-on"].is_null(), "ci.yml job {job}");
            continue;
        }
        checked += 1;
        let runner = def["runs-on"].as_str().unwrap_or_default();
        match HOSTED.iter().find(|(j, _, _)| *j == job) {
            Some((_, want, why)) => assert_eq!(runner, *want, "ci.yml job {job} ({why})"),
            None if job == "classify" => assert_eq!(
                runner, SELF_HOSTED_LIGHT_SAME_REPO_ONLY,
                "ci.yml job classify: the light pool, fail-closed"
            ),
            None => assert_eq!(
                runner, SELF_HOSTED_CPU_SAME_REPO_ONLY,
                "ci.yml job {job}: unexpected runner"
            ),
        }
    }
    assert!(checked > 0, "ci.yml has no jobs — oracle checked nothing");
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
            "release.yml must restore no caches ({cache}): main's self-hosted runs write \
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

/// Runner labels that move to a new image without a change here.
fn floating_ubuntu_labels(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter(|l| l.contains("ubuntu-latest"))
        .map(|l| l.trim().to_string())
        .collect()
}

// rivet: verifies REQ-CI-006
#[test]
fn hosted_ubuntu_jobs_are_pinned_never_ubuntu_latest() {
    // planted: a job line and the fork-guard fallback, both caught
    let planted = "    runs-on: ubuntu-latest\n    runs-on: ${{ x && 'a' || 'ubuntu-latest' }}\n    # ubuntu-latest in a comment is fine\n";
    assert_eq!(floating_ubuntu_labels(planted).len(), 2);
    let all = workflows();
    assert!(all.len() >= 2, "found too few workflows: {}", all.len());
    for (name, text) in &all {
        let bad = floating_ubuntu_labels(text);
        assert!(
            bad.is_empty(),
            "{name}: pin ubuntu-24.04, not ubuntu-latest: {bad:?}"
        );
        let pinned = text.matches("ubuntu-24.04").count();
        assert!(
            pinned > 0,
            "{name}: no pinned ubuntu runner, so the check compared nothing"
        );
    }
}

/// Cargo commands that resolve dependencies and must keep Cargo.lock as committed.
fn resolving_cargo_without_locked(cmd: &str) -> bool {
    let resolves = [
        "cargo build",
        "cargo test",
        "cargo clippy",
        "cargo check",
        "cargo run",
        "cargo install",
    ]
    .iter()
    .any(|c| cmd.contains(c));
    resolves && !cmd.contains("--locked")
}

// rivet: verifies REQ-CI-007
#[test]
fn every_cargo_build_in_ci_and_release_is_locked() {
    // planted: an unlocked build is caught, a locked one and a non-resolving step are not
    assert!(resolving_cargo_without_locked("cargo build --release"));
    assert!(!resolving_cargo_without_locked(
        "cargo build --release --locked"
    ));
    assert!(!resolving_cargo_without_locked("cargo fmt --check"));
    let mut checked = 0;
    for (name, _) in workflows() {
        for cmd in commands(&workflow(&name)) {
            if cmd.contains("cargo ") {
                checked += 1;
            }
            assert!(
                !resolving_cargo_without_locked(&cmd),
                "{name}: a cargo build without --locked re-resolves an out-of-date \
                 Cargo.lock instead of failing: {cmd}"
            );
        }
    }
    assert!(checked >= 5, "found only {checked} cargo commands");
}

/// Tracked manifests and the lockfile each one needs, as repo paths.
fn lockfiles() -> Vec<(String, String)> {
    let out = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("git ls-files");
    let files: BTreeSet<String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    // a member of the root workspace resolves through the root Cargo.lock
    let root: toml::Value = toml::from_str(
        &fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap(),
    )
    .unwrap();
    let members: BTreeSet<String> = root["workspace"]["members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m.as_str())
        .filter(|m| *m != ".")
        .map(|m| format!("{m}/Cargo.toml"))
        .collect();
    let mut pairs = Vec::new();
    for f in &files {
        let (dir, base) = match f.rsplit_once('/') {
            Some((d, b)) => (format!("{d}/"), b),
            None => (String::new(), f.as_str()),
        };
        let lock = match base {
            "Cargo.toml" => "Cargo.lock",
            "package.json" => "package-lock.json",
            _ => continue,
        };
        let lock = if members.contains(f) {
            "Cargo.lock".to_string()
        } else {
            format!("{dir}{lock}")
        };
        assert!(
            files.contains(&lock),
            "{f} has no committed {lock}: its dependencies float"
        );
        pairs.push((f.clone(), lock));
    }
    pairs
}

// rivet: verifies REQ-CI-007
#[test]
fn every_lockfile_is_committed_and_audited_with_planted_controls() {
    let locks = lockfiles();
    assert!(
        locks.iter().any(|(_, l)| l == "Cargo.lock")
            && locks.iter().any(|(_, l)| l.ends_with("package-lock.json")),
        "expected Cargo.lock and a package-lock.json: {locks:?}"
    );
    let wf = workflow("ci.yml");
    let audit = jobs(&wf)
        .into_iter()
        .find(|(n, _)| n == "audit")
        .expect("ci.yml has an audit job")
        .1;
    let steps: Vec<(String, String)> = audit["steps"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["working-directory"].as_str().unwrap_or("").to_string(),
                s["run"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    for (_, lock) in &locks {
        let audited = if let Some(dir) = lock.strip_suffix("/package-lock.json") {
            steps
                .iter()
                .any(|(wd, run)| wd == dir && run.contains("npm audit") && !run.contains("||"))
        } else {
            steps.iter().any(|(_, run)| {
                run.contains(&format!("cargo audit --file {lock}")) && !run.contains("||")
            })
        };
        assert!(audited, "ci.yml's audit job does not audit {lock}");
    }
    // the controls run in the same job, before the real audits
    let first_real = steps
        .iter()
        .position(|(_, r)| r.contains("cargo audit --file Cargo.lock"))
        .unwrap();
    let controls = steps
        .iter()
        .position(|(_, r)| {
            r.contains("--locked") && r.contains("RUSTSEC-2021-0003") && r.contains("minimist")
        })
        .expect("the audit job runs the planted controls");
    assert!(controls < first_real, "the planted controls must run first");
}

/// The `toolchain:` of every rust-toolchain step, with its workflow.
fn toolchains() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for (name, _) in workflows() {
        for (id, job) in jobs(&workflow(&name)) {
            for step in job["steps"].as_sequence().into_iter().flatten() {
                if step["uses"]
                    .as_str()
                    .is_some_and(|u| u.starts_with("dtolnay/rust-toolchain@"))
                {
                    let t = step["with"]["toolchain"].as_str().unwrap_or("").to_string();
                    out.push((name.clone(), id.clone(), t));
                }
            }
        }
    }
    out
}

/// An exact Rust version, `1.100.0`, or a dated nightly. `1.100` is refused:
/// it follows patch releases.
fn pinned_toolchain(t: &str) -> bool {
    let release = t.split('.').count() == 3
        && t.split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    let nightly = t
        .strip_prefix("nightly-")
        .is_some_and(|d| d.len() == 10 && chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok());
    release || nightly
}

/// jig pins' advisory canary (REQ-021 in jig): job-level
/// `continue-on-error: true` as a literal boolean, needed by no job except
/// `required`, and named in `required`'s `advisory` input.
fn advisory_canary(wf: &Value, id: &str) -> bool {
    let all = jobs(wf);
    let Some((_, job)) = all.iter().find(|(j, _)| j == id) else {
        return false;
    };
    let needs_it = |def: &Value| match &def["needs"] {
        Value::String(s) => s == id,
        Value::Sequence(v) => v.iter().any(|n| n.as_str() == Some(id)),
        _ => false,
    };
    let only_required_needs_it = all
        .iter()
        .filter(|(j, def)| j != "required" && needs_it(def))
        .count()
        == 0;
    let advisory = all
        .iter()
        .find(|(j, _)| j == "required")
        .and_then(|(_, def)| def["with"]["advisory"].as_str().map(str::to_string))
        .unwrap_or_default();
    job["continue-on-error"].as_bool() == Some(true)
        && only_required_needs_it
        && advisory.split_whitespace().any(|a| a == id)
}

// rivet: verifies REQ-CI-007
#[test]
fn every_rust_toolchain_is_one_pinned_version_across_workflows() {
    // planted: floating channels and expressions are refused
    for bad in [
        "stable",
        "beta",
        "nightly",
        "${{ env.RUST }}",
        "${{ matrix.rust }}",
        "1",
        "1.x",
        "1.100",
    ] {
        assert!(!pinned_toolchain(bad), "{bad}");
    }
    for good in ["1.100.0", "1.99.1", "nightly-2026-11-01"] {
        assert!(pinned_toolchain(good), "{good}");
    }
    // planted: the canary exception holds only in jig's advisory shape
    let canary = |yaml: &str| {
        let wf: Value = serde_yaml::from_str(yaml).unwrap();
        advisory_canary(&wf, "canary-stable")
    };
    let shape = |coe: &str, other_needs: &str, advisory: &str| {
        format!(
            "jobs:\n  canary-stable:\n    continue-on-error: {coe}\n  build:\n    needs: [{other_needs}]\n  required:\n    needs: [build, canary-stable]\n    with:\n      advisory: \"{advisory}\"\n"
        )
    };
    assert!(canary(&shape("true", "", "canary-stable")));
    assert!(!canary(&shape("false", "", "canary-stable")));
    assert!(!canary(&shape("\"true\"", "", "canary-stable")));
    assert!(!canary(&shape("${{ true }}", "", "canary-stable")));
    assert!(!canary(&shape("true", "canary-stable", "canary-stable")));
    assert!(!canary(&shape("true", "", "")));
    assert!(!canary(&shape("true", "", "canary-stable-2")));
    let all = toolchains();
    assert!(all.len() >= 7, "found only {} toolchain steps", all.len());
    let mut canaries = vec![];
    for (wf, id, t) in &all {
        if t == "stable" && advisory_canary(&workflow(wf), id) {
            canaries.push(format!("{wf} {id}"));
            continue;
        }
        assert!(
            pinned_toolchain(t),
            "{wf}: rust-toolchain `{t}` in job {id} floats; pin a version so a Rust release is a reviewed diff"
        );
    }
    assert_eq!(
        canaries,
        ["ci.yml canary-stable"],
        "exactly one advisory canary"
    );
    let versions: BTreeSet<&str> = all
        .iter()
        .filter(|(wf, id, _)| !canaries.contains(&format!("{wf} {id}")))
        .map(|(_, _, t)| t.as_str())
        .collect();
    assert_eq!(
        versions.len(),
        1,
        "CI and release build with different Rust: {versions:?}"
    );
}

// rivet: verifies REQ-CI-008
#[test]
fn the_jig_shadow_pilot_has_the_caller_shape_and_changes_no_required_context() {
    let wf = workflow("ci.yml");
    let all = jobs(&wf);
    let job = |id: &str| {
        all.iter()
            .find(|(j, _)| j == id)
            .unwrap_or_else(|| panic!("ci.yml has no job {id}"))
            .1
            .clone()
    };
    // both jig calls pin one full SHA
    let sha = |id: &str, file: &str| {
        let u = job(id)["uses"].as_str().unwrap_or_default().to_string();
        let (path, sha) = u.split_once('@').unwrap_or_else(|| panic!("{id}: {u}"));
        assert_eq!(
            path,
            format!("pulseengine/jig/.github/workflows/{file}"),
            "{id}"
        );
        assert!(
            sha.len() == 40
                && sha
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "{id}: {sha} is not a full SHA"
        );
        sha.to_string()
    };
    assert_eq!(
        sha("baseline", "baseline.yml"),
        sha("required", "required.yml")
    );
    // shadow mode: enforce-skips is never set, and no job reads the classifier yet
    assert!(job("baseline")["with"]["enforce-skips"].is_null());
    for (id, def) in &all {
        if id != "required" {
            assert!(def["if"].is_null(), "{id}: shadow mode gates nothing");
        }
    }
    // required waits for every other job, always
    let req = job("required");
    assert_eq!(req["if"].as_str(), Some("always()"));
    let needs: BTreeSet<&str> = req["needs"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let others: BTreeSet<&str> = all
        .iter()
        .map(|(j, _)| j.as_str())
        .filter(|j| *j != "required")
        .collect();
    assert_eq!(needs, others);
    // least privilege: the token reads contents and nothing else, and no job widens it
    let perms = wf["permissions"]
        .as_mapping()
        .expect("ci.yml sets top-level permissions");
    assert_eq!(perms.len(), 1, "{perms:?}");
    assert_eq!(wf["permissions"]["contents"].as_str(), Some("read"));
    for (id, def) in &all {
        assert!(
            def.get("permissions").is_none(),
            "{id}: widens permissions without a reason here"
        );
    }
    // every pull request and the merge queue, no filter
    assert!(wf["on"]["pull_request"].is_null() && wf["on"].get("pull_request").is_some());
    assert!(wf["on"].get("merge_group").is_some());
    // the controls ci.yml runs exist in the manifest
    let manifest =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("ci-controls.toml")).unwrap();
    let only = job("baseline")["with"]["controls-only"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(only.split_whitespace().count() >= 4, "{only}");
    for id in only.split_whitespace() {
        assert!(
            manifest.contains(&format!("id = \"{id}\"")),
            "{id} not in ci-controls.toml"
        );
    }
    // sieve is fetched by version and sha256
    let env = &job("classify")["env"];
    assert!(env["SIEVE_VERSION"]
        .as_str()
        .is_some_and(|v| v.starts_with('v')));
    assert_eq!(env["SIEVE_SHA256"].as_str().map(str::len), Some(64));
    // the required contexts mirror main's protection (owner, 2026-11-26)
    let ctx = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/required-contexts.txt"),
    )
    .unwrap();
    assert!(ctx.starts_with("# Mirrors the required status checks"));
    assert_eq!(
        ctx.lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>(),
        [
            "Build",
            "Clippy",
            "Rivet traceability check",
            "Playwright E2E",
            "Dependency audit",
            "required / verdict",
            "baseline / guard"
        ]
    );
}

// rivet: verifies REQ-CI-007
/// jig's verdict control (ci-controls.toml) replays `required` against fixture
/// needs files; a job added to `required` but missing there turns main red on
/// push only, where the controls job runs (the v1.6.54 merge, 2026-11-27).
#[test]
fn the_verdict_control_fixtures_cover_every_job_required_needs() {
    let wf = workflow("ci.yml");
    let required = jobs(&wf)
        .into_iter()
        .find(|(n, _)| n == "required")
        .expect("ci.yml has a required job")
        .1;
    let needs: BTreeSet<String> = required["needs"]
        .as_sequence()
        .expect("required lists its needs")
        .iter()
        .filter_map(|n| n.as_str().map(str::to_string))
        .collect();
    assert!(needs.len() > 3, "required needs nothing: {needs:?}");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for f in [
        "ci-controls/fixtures/needs.json",
        "ci-controls/verdict-sees-failure/ci-controls/fixtures/needs.json",
    ] {
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join(f)).unwrap()).unwrap();
        let keys: BTreeSet<String> = v.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, needs, "{f} must name exactly the jobs required needs");
    }
    // the control's never-skip list is ci.yml's
    let never_skip = required["with"]["never-skip"].as_str().unwrap();
    let controls = fs::read_to_string(root.join("ci-controls.toml")).unwrap();
    assert!(
        controls.contains(&format!("--never-skip \"{never_skip}\"")),
        "ci-controls.toml's verdict control must pass --never-skip \"{never_skip}\""
    );
}

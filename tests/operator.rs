//! Oracles for the operator kit (REQ-OPS-001..003).

use clap::Parser;
use std::path::Path;
use warte::operator::{judge, Inputs, State, BRIEFING_GRACE_S, CI_LATE_S, DUTIES, TRIAGE_LATE_S};

const NOW: i64 = 1_800_000_000;
const DOC: &str = include_str!("../docs/topics/operator.md");

fn state(i: &Inputs, key: &str) -> (State, Option<i64>, String) {
    let d = judge(i).into_iter().find(|d| d.key == key).expect("duty");
    (d.state, d.since, d.detail)
}

fn base() -> Inputs {
    Inputs {
        now: NOW,
        ..Default::default()
    }
}

// rivet: verifies REQ-OPS-003
#[test]
fn nothing_configured_is_not_set_up_never_met() {
    let all = judge(&base());
    assert_eq!(all.len(), DUTIES.len());
    for d in &all {
        assert_eq!(d.state, State::NotSetUp, "{d:?}");
    }
}

// rivet: verifies REQ-OPS-003
#[test]
fn each_duty_is_late_from_warte_s_own_data() {
    let mut i = base();
    // CI source: configured but silent, then stale, then fresh
    i.ci = Some(None);
    assert_eq!(state(&i, "ci-source").0, State::Late);
    i.ci = Some(Some(NOW - CI_LATE_S - 1));
    assert_eq!(state(&i, "ci-source").0, State::Late);
    i.ci = Some(Some(NOW - 60));
    assert_eq!(state(&i, "ci-source").0, State::Met);

    // probes: one machine quiet, named, since its last report
    i.probes = Some(vec![("a".into(), NOW - 30), ("b".into(), NOW - 3600)]);
    let (s, since, detail) = state(&i, "probes");
    assert_eq!((s, since), (State::Late, Some(NOW - 3600)));
    assert!(
        detail.contains("1 of 2") && detail.contains('b'),
        "{detail}"
    );
    i.probes = Some(vec![("a".into(), NOW - 30)]);
    assert_eq!(state(&i, "probes").0, State::Met);
    i.probes = Some(vec![]);
    assert_eq!(state(&i, "probes").0, State::NotSetUp);

    // costs: old, or on time with old data (the input stopped)
    i.costs = Some(Some((NOW - 27 * 3600, None)));
    assert_eq!(state(&i, "costs").0, State::Late);
    i.costs = Some(Some((NOW - 3600, Some(3))));
    let (s, _, detail) = state(&i, "costs");
    assert_eq!(s, State::Late);
    assert!(detail.contains("3 day(s) old"), "{detail}");
    i.costs = Some(Some((NOW - 3600, None)));
    assert_eq!(state(&i, "costs").0, State::Met);
    i.costs = Some(None);
    assert_eq!(state(&i, "costs").0, State::NotSetUp);

    // skills
    i.skills = Some(Some(NOW - 27 * 3600));
    assert_eq!(state(&i, "skills").0, State::Late);
    i.skills = Some(Some(NOW - 3600));
    assert_eq!(state(&i, "skills").0, State::Met);

    // briefing: the plain version gets a grace period, then the job is late
    i.briefing = Some(Some(("template".into(), NOW - BRIEFING_GRACE_S + 60)));
    assert_eq!(state(&i, "briefing").0, State::Met);
    i.briefing = Some(Some(("template".into(), NOW - BRIEFING_GRACE_S - 60)));
    assert_eq!(state(&i, "briefing").0, State::Late);
    i.briefing = Some(Some(("model".into(), NOW - 3 * 86_400)));
    assert_eq!(state(&i, "briefing").0, State::Met);

    // triage: a hand-out left waiting
    i.triage = Some(Some(NOW - TRIAGE_LATE_S - 1));
    assert_eq!(state(&i, "triage").0, State::Late);
    i.triage = Some(Some(NOW - 60));
    assert_eq!(state(&i, "triage").0, State::Met);
    i.triage = Some(None);
    assert_eq!(state(&i, "triage").0, State::Met);

    // login
    assert_eq!(state(&i, "login").0, State::NotSetUp);
    i.login = true;
    assert_eq!(state(&i, "login").0, State::Met);
}

// rivet: verifies REQ-OPS-001
#[test]
fn the_document_names_every_duty_and_every_right() {
    for d in DUTIES.iter() {
        assert!(
            DOC.contains(&format!("\n## {}\n", d.key)),
            "operator.md has no section for duty {}",
            d.key
        );
        assert!(DOC.contains(d.every), "{} cadence {}", d.key, d.every);
        if let Some(r) = d.right {
            assert!(
                warte::auth::is_known_right(r),
                "{} names unknown right {r}",
                d.key
            );
        }
    }
    for r in warte::auth::known_rights() {
        assert!(
            DOC.contains(&format!("`{r}`")),
            "right {r} is neither a duty's nor listed as not an operator duty"
        );
    }
    assert!(warte::docs::get_topic("operator").is_some());
}

/// Dotted words: candidate host names (and file names, versions).
fn dotted(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        .map(|w| w.trim_matches('.'))
        .filter(|w| w.contains('.'))
        .map(String::from)
        .collect()
}

fn is_host(w: &str) -> bool {
    let last = w.rsplit('.').next().unwrap_or("");
    last.len() >= 2 && last.chars().all(|c| c.is_ascii_lowercase())
}

fn is_ip(w: &str) -> bool {
    let parts: Vec<&str> = w.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()))
}

/// Every `/api/...` path in a text.
fn api_paths(text: &str) -> Vec<String> {
    text.match_indices("/api/")
        .map(|(i, _)| {
            text[i..]
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || "/_-".contains(*c))
                .collect::<String>()
        })
        .collect()
}

fn templates() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push((
                    p.display().to_string(),
                    std::fs::read_to_string(&p).unwrap(),
                ));
            }
        }
    }
    let mut v = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("deploy/operator"),
        &mut v,
    );
    v.sort();
    v
}

// rivet: verifies REQ-OPS-002
#[test]
fn templates_hold_placeholders_only() {
    let files = templates();
    assert!(files.len() >= 10, "{files:?}");
    let allowed = |h: &str| {
        h.ends_with("example.internal")
            || h == "www.apple.com" // the plist DTD
            || h.starts_with("org.example")
            || [
                ".sh", ".json", ".pem", ".toml", ".service", ".timer", ".plist", ".md", ".dtd",
                ".env", ".token", ".snippet", ".target",
            ]
            .iter()
            .any(|e| h.ends_with(e))
    };
    for (name, text) in &files {
        for w in dotted(text) {
            if is_ip(&w) {
                assert_eq!(w, "127.0.0.1", "{name}");
            } else if is_host(&w) {
                assert!(allowed(&w), "{name}: {w}");
            }
        }
        for secret in ["PRIVATE KEY", "ghp_", "github_pat_", "Bearer ey"] {
            assert!(!text.contains(secret), "{name}: {secret}");
        }
    }
}

// rivet: verifies REQ-OPS-002
#[test]
fn templates_call_only_routes_and_commands_warte_has() {
    let openapi: serde_json::Value =
        serde_json::from_str(include_str!("../src/admission/openapi.json")).unwrap();
    let mut routes: Vec<String> = openapi["paths"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    routes.extend(
        warte::auth::MUTATING_ROUTES
            .iter()
            .map(|r| r.pattern.to_string()),
    );
    let mut commands = 0;
    for (name, text) in templates() {
        for p in api_paths(&text) {
            assert!(routes.contains(&p), "{name}: {p}");
        }
        let mut argvs: Vec<Vec<String>> = Vec::new();
        if name.ends_with(".plist") {
            // each argument is its own <string> element, spaces and all
            let args: Vec<String> = text
                .split("<string>")
                .skip(1)
                .filter_map(|x| x.split_once("</string>").map(|(a, _)| a.to_string()))
                .collect();
            let i = args.iter().position(|a| a.ends_with("/bin/warte")).unwrap();
            let mut argv = vec!["warte".to_string()];
            argv.extend(args[i + 1..].iter().cloned());
            argvs.push(argv);
        } else {
            for rest in text
                .lines()
                .filter_map(|l| l.split_once("/usr/local/bin/warte ").map(|(_, r)| r))
            {
                let mut argv = vec!["warte".to_string()];
                argv.extend(rest.split_whitespace().map(String::from));
                argvs.push(argv);
            }
        }
        for argv in argvs {
            assert!(
                warte::cli::Cli::try_parse_from(&argv).is_ok(),
                "{name}: {argv:?}"
            );
            commands += 1;
        }
    }
    assert!(commands >= 3, "found {commands} warte commands");
    // the commands the document shows parse too
    let mut in_code = false;
    let mut shown = 0;
    for l in DOC.lines() {
        if l.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code && l.starts_with("warte ") {
            let words: Vec<&str> = l.split('#').next().unwrap().split_whitespace().collect();
            assert!(warte::cli::Cli::try_parse_from(&words).is_ok(), "{l}");
            shown += 1;
        }
    }
    assert!(shown >= 3, "found {shown} commands in operator.md");
}

// rivet: verifies REQ-OPS-004
#[test]
fn every_release_carries_the_kit_summed_and_signed() {
    let wf = include_str!("../.github/workflows/release.yml");
    // packaged from the tagged commit, not from a working tree
    assert!(
        wf.contains("git archive --format=tar.gz --prefix=\"operator-kit-$v/\""),
        "the kit is not made with git archive"
    );
    assert!(wf.contains("HEAD deploy/operator"));
    // summed, so the cosign signature over SHA256SUMS.txt covers it
    let sums = wf
        .lines()
        .find(|l| l.contains("> SHA256SUMS.txt"))
        .expect("the sums line");
    assert!(sums.contains("operator-kit-*.tar.gz"), "{sums}");
    assert!(wf.contains("test -s operator-kit-*.tar.gz"));
    // published, and never under a name varve reads as a binary
    assert!(wf.contains("release-artifacts/operator-kit-*.tar.gz"));
    assert!(!wf.contains("warte-operator-kit"));
    assert!(wf.contains("needs: [build, sbom, operator-kit]"));
}

// rivet: verifies REQ-DEPLOY-003
#[test]
fn the_named_config_is_read_and_a_missing_one_stops_warte() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("elsewhere.toml");
    std::fs::write(&p, "bind = \"127.0.0.1:18089\"\n").unwrap();
    let c = warte::Config::load_at(Some(&p)).unwrap();
    assert_eq!(c.bind, "127.0.0.1:18089");
    let missing = dir.path().join("missing.toml");
    let e = warte::Config::load_at(Some(&missing))
        .unwrap_err()
        .to_string();
    assert!(e.contains("does not exist"), "{e}");
    // the flag reaches the CLI
    let cli =
        warte::cli::Cli::try_parse_from(["warte", "--config", "/x/warte.toml", "status"]).unwrap();
    assert_eq!(cli.config.as_deref(), Some(Path::new("/x/warte.toml")));
    // and every command in main uses it: no call reads ./warte.toml regardless
    let main = include_str!("../src/main.rs");
    assert!(
        !main.contains("Config::load()"),
        "a command ignores --config"
    );
    assert!(main.contains("Config::load_at(config_path.as_deref())"));
}

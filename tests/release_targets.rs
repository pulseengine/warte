//! The release builds every platform a probe runs on (REQ-RELEASE-006):
//! a target dropped from the matrix would leave those hosts on hand builds.

// rivet: partially-verifies REQ-RELEASE-006
#[test]
fn the_release_matrix_builds_every_probe_platform() {
    let wf = include_str!("../.github/workflows/release.yml");
    for (target, asset) in [
        ("x86_64-unknown-linux-gnu", "linux-amd64"),
        ("aarch64-unknown-linux-gnu", "linux-arm64"),
        ("aarch64-apple-darwin", "macos-arm64"),
    ] {
        let at = wf
            .find(&format!("target: {target}"))
            .unwrap_or_else(|| panic!("release.yml builds no {target}"));
        let next = wf[at..].lines().nth(1).unwrap_or_default().trim();
        assert_eq!(
            next,
            format!("asset: {asset}"),
            "{target} must ship as {asset}"
        );
    }
    // every built asset is checksummed and published (the globs cover them)
    // ...and the SBOM is summed too, under the names varve's deposit reads
    assert!(wf.contains("sha256sum warte-* sbom.json > SHA256SUMS.txt"));
    assert!(wf.contains("--bundle dist/SHA256SUMS.txt.cosign.bundle dist/SHA256SUMS.txt"));
    assert!(wf.contains("release-artifacts/SHA256SUMS.txt.cosign.bundle"));
    assert!(wf.contains("subject-path: \"dist/warte-*\""));
    assert!(wf.contains("release-artifacts/warte-*"));
}

// rivet: partially-verifies REQ-RELEASE-006
#[test]
fn the_release_refuses_a_probe_that_cannot_enroll_or_send() {
    let wf = include_str!("../.github/workflows/release.yml");
    assert!(
        wf.contains("for cmd in enroll send hook; do"),
        "release.yml must check the probe's subcommands"
    );
    // and the check matches what the binary prints
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_warte"))
        .args(["probe", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8(out.stdout).unwrap();
    for cmd in ["enroll", "send", "hook"] {
        assert!(
            help.lines()
                .any(|l| l.starts_with(&format!("  {cmd} ")) || l == format!("  {cmd}")),
            "probe --help lists no '{cmd}' in the form the release checks:\n{help}"
        );
    }
}

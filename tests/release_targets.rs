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
    assert!(wf.contains("sha256sum warte-* > SHA256SUMS"));
    assert!(wf.contains("release-artifacts/warte-*"));
}

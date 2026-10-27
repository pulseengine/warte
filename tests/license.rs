//! The license is declared in three places; they must agree (DD-LICENSE-001).

use std::fs;
use std::path::Path;

// rivet: verifies REQ-RELEASE-004
// rivet: verifies DD-LICENSE-001
#[test]
fn license_declarations_agree_on_apache_2() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(
        env!("CARGO_PKG_LICENSE"),
        "Apache-2.0",
        "Cargo.toml license"
    );
    let license = fs::read_to_string(root.join("LICENSE")).expect("LICENSE file");
    assert!(
        license.contains("Apache License") && license.contains("Version 2.0, January 2004"),
        "LICENSE is not the Apache-2.0 text"
    );
    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    let section = readme
        .split("## License")
        .nth(1)
        .expect("README has a License section");
    assert!(
        section.contains("Apache-2.0") && !section.contains("MIT"),
        "README's License section must say Apache-2.0"
    );
}

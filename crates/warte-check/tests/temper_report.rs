//! The fixture files the component job runs agree with the parser
//! (REQ-TEMPER-002): the good report is accepted and the planted one, whose
//! repository name is markup, is refused without quoting it.

use warte_check::temper_report::parse;

// rivet: verifies REQ-TEMPER-002
#[test]
fn the_good_fixture_is_accepted_and_the_planted_one_refused() {
    let good = include_bytes!("fixtures/temper-report.good.json");
    assert!(parse(good).is_ok(), "{:?}", parse(good).err());
    let bad = include_bytes!("fixtures/temper-report.planted-bad.json");
    let e = parse(bad).expect_err("markup as a repository name must be refused");
    assert!(
        !e.contains("onerror") && !e.contains("<img"),
        "reason quotes the input: {e}"
    );
}

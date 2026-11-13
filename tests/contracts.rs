//! The interfaces a deployment feeds warte are versioned contracts in the docs
//! (REQ-TEMPLATE-002). These oracles fail when the code drifts from the doc.

use warte::hub::model::JobRecord;

const WIRE: &str = include_str!("../docs/topics/ground-station.md");
const JOBS: &str = include_str!("../docs/topics/job-state.md");
const COSTS: &str = include_str!("../docs/topics/costs-schema.md");
const OPS: &str = include_str!("../docs/topics/operator.md");
const NAV: &str = include_str!("../docs/topics/nav.md");

/// Field names in the first column of a doc table: "| `a`, `b` | ...".
fn table_fields(doc: &str, after: &str) -> Vec<String> {
    let i = doc.find(after).unwrap_or_else(|| panic!("no {after:?}"));
    doc[i..]
        .lines()
        .skip_while(|l| !l.starts_with("| Field"))
        .skip(2)
        .take_while(|l| l.starts_with('|'))
        .flat_map(|l| {
            let first = l.split('|').nth(1).unwrap_or("");
            first
                .split(',')
                .map(|f| f.trim().trim_matches('`').to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

// rivet: verifies REQ-TEMPLATE-002
#[test]
fn every_contract_states_its_version() {
    for (name, doc) in [
        ("wire", WIRE),
        ("jobs", JOBS),
        ("costs", COSTS),
        ("operator", OPS),
        ("nav", NAV),
    ] {
        assert!(
            doc.contains("ersion **"),
            "{name} states no contract version"
        );
        assert!(
            doc.contains("since warte v"),
            "{name} states no first release"
        );
    }
}

// rivet: verifies REQ-TEMPLATE-002
#[test]
fn the_ledger_record_fields_are_the_documented_ones() {
    let full = JobRecord {
        s: Some(1),
        e: Some(2),
        sig: Some("resolver:example.org".into()),
        ..Default::default()
    };
    let mut got: Vec<String> = serde_json::to_value(&full)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let mut doc = table_fields(WIRE, "A ledger record");
    got.sort();
    doc.sort();
    assert_eq!(got, doc, "JobRecord and ground-station.md disagree");
}

// rivet: verifies REQ-TEMPLATE-002
#[test]
fn the_wire_apids_are_documented() {
    use warte::hub::telemetry::*;
    for (apid, s) in [
        (APID_HOST, "`0x100`"),
        (APID_RUNNER_BASE, "`0x110`"),
        (APID_QUEUE, "`0x200`"),
        (APID_QUEUE_DETAIL, "`0x201`"),
        (APID_HOSTED_BASE, "`0x210`"),
        (APID_QUEUED_BASE, "`0x220`"),
        (APID_LEDGER, "`0x230`"),
        (APID_MANIFEST, "`0x231`"),
        (APID_TIMERS, "`0x300`"),
        (APID_TRACE, "`0x304`"),
        (APID_FEED_TEMPER, "`0x400`"),
    ] {
        assert_eq!(format!("`{apid:#05x}`"), s, "the constant moved");
        assert!(WIRE.contains(s), "{s} is not in the wire contract");
    }
}

// rivet: verifies REQ-TEMPLATE-002
#[test]
fn the_job_state_fields_are_the_documented_ones() {
    let mut doc = table_fields(JOBS, "## State file");
    let mut got: Vec<String> = warte::jobs::STATE_FILE_FIELDS
        .iter()
        .map(|s| s.to_string())
        .collect();
    doc.sort();
    got.sort();
    assert_eq!(got, doc);
}

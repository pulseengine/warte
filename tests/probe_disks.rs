//! Oracle for REQ-PROBE-DISK-001: which mounts the probe reports.

use warte::probe::shown;

// rivet: verifies REQ-PROBE-DISK-001
#[test]
fn a_sandboxed_probe_still_reports_data_volumes() {
    // Linux: a data volume seen read-only through the unit's sandbox is reported
    assert!(shown("ext4", true, false));
    assert!(shown("xfs", true, false));
    assert!(shown("ext4", false, false));
    // packaged images are not data volumes, writable or not
    for fs in ["squashfs", "iso9660", "udf", "erofs", "cramfs", "romfs"] {
        assert!(!shown(fs, true, false), "{fs}");
        assert!(!shown(fs, false, false), "{fs}");
    }
    // macOS: the sealed, read-only system snapshot stays hidden; data volumes show
    assert!(!shown("apfs", true, true));
    assert!(shown("apfs", false, true));
}

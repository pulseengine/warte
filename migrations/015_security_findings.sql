-- Security findings reported by hosts (REQ-FINDINGS-001, warte#114): the
-- latest set per host, the hosts' last report, and the accepted baseline.
-- Metadata only: there is no column for file contents or secret values.
CREATE TABLE IF NOT EXISTS security_findings (
    host       TEXT NOT NULL,
    source     TEXT NOT NULL,
    rule       TEXT NOT NULL,
    subject    TEXT NOT NULL,
    severity   TEXT NOT NULL,
    first_seen TEXT NOT NULL,
    last_seen  TEXT NOT NULL,
    count      INTEGER NOT NULL,
    PRIMARY KEY (host, source, rule, subject)
);
CREATE TABLE IF NOT EXISTS security_reports (
    host        TEXT PRIMARY KEY,
    at          INTEGER NOT NULL,
    received_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS security_baseline (
    host        TEXT NOT NULL,
    source      TEXT NOT NULL,
    rule        TEXT NOT NULL,
    subject     TEXT NOT NULL,
    accepted_by TEXT NOT NULL,
    accepted_at TEXT NOT NULL,
    PRIMARY KEY (host, source, rule, subject)
);

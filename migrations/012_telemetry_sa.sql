-- Transport v2 security associations (REQ-SEC-001, #41): one per enrolled
-- agent, issued at `warte probe enroll`. k_id is relay-sec's long-term SA key
-- (32 hex characters); last_epoch is the highest initiator epoch accepted, so
-- a handshake can never go back. Re-enrolment replaces the row.
CREATE TABLE IF NOT EXISTS telemetry_sa (
    host       TEXT PRIMARY KEY,
    spi        INTEGER NOT NULL UNIQUE,
    k_id       TEXT NOT NULL,
    last_epoch INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

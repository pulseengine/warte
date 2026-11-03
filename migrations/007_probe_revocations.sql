-- Revoked probe hosts (REQ-PROBE-007). A certificate for `host` issued at or
-- before `revoked_at` is refused for reports and renewal; the machine needs a
-- new enrollment token.
CREATE TABLE IF NOT EXISTS probe_revocations (
    host       TEXT PRIMARY KEY,
    revoked_at INTEGER NOT NULL
);

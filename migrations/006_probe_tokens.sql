-- One-time probe enrollment tokens (DD-PROBE-002). Only the SHA-256 of the
-- token is stored; the raw token is printed once at mint time.
CREATE TABLE IF NOT EXISTS probe_tokens (
    token_sha256 TEXT PRIMARY KEY,
    host         TEXT NOT NULL,
    expires_at   INTEGER NOT NULL,
    used         INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_probe_tokens_host ON probe_tokens(host);

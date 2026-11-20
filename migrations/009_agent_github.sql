-- Per-agent GitHub token scope (REQ-GHTOKEN-002): set only by the operator
-- CLI on the host, never by an agent. Keyed by the agent name a warte token
-- names. repos is a JSON array of "owner/name".
CREATE TABLE IF NOT EXISTS agent_github (
    name          TEXT PRIMARY KEY,
    repos         TEXT NOT NULL,
    actions_write INTEGER NOT NULL DEFAULT 0,
    updated_at    TEXT NOT NULL
);

-- Access policy tombstones (OR-Set: add-wins revocation)
-- A right is active if there is a grant WITHOUT a matching tombstone.
-- Concurrent grant+revoke resolves to REVOKED (tombstone wins).
CREATE TABLE IF NOT EXISTS access_policy_tombstones (
    id TEXT PRIMARY KEY,
    policy_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    resource TEXT NOT NULL,
    action TEXT NOT NULL,
    revoked_at TEXT NOT NULL DEFAULT (datetime('now')),
    revoked_by TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    version_vector TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY (policy_id) REFERENCES access_policies(id)
);

CREATE INDEX IF NOT EXISTS idx_tombstones_policy ON access_policy_tombstones(policy_id);
CREATE INDEX IF NOT EXISTS idx_tombstones_agent ON access_policy_tombstones(agent_id);
CREATE INDEX IF NOT EXISTS idx_tombstones_resource ON access_policy_tombstones(resource, action);

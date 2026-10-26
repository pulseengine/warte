-- Agent registrations
CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    repo TEXT,
    identity_type TEXT NOT NULL,
    public_key BLOB NOT NULL,
    cert_pem BLOB,
    cert_expires_at TEXT,
    access_rights TEXT NOT NULL DEFAULT '[]',
    last_heartbeat TEXT,
    health_status TEXT NOT NULL DEFAULT 'unknown',
    instance_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version_vector TEXT NOT NULL DEFAULT '{}'
);

-- Access control policies
CREATE TABLE IF NOT EXISTS access_policies (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    resource TEXT NOT NULL,
    action TEXT NOT NULL,
    granted_at TEXT NOT NULL DEFAULT (datetime('now')),
    granted_by TEXT NOT NULL,
    revoked_at TEXT,
    version_vector TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY (agent_id) REFERENCES agents(id)
);

-- Artifacts
CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    content BLOB NOT NULL,
    integrity_proof BLOB NOT NULL,
    integrity_type TEXT NOT NULL,
    access_scope TEXT NOT NULL DEFAULT '[]',
    description TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    version_vector TEXT NOT NULL DEFAULT '{}'
);

-- Captured agent data
CREATE TABLE IF NOT EXISTS agent_data (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    data_type TEXT NOT NULL,
    content TEXT NOT NULL,
    captured_at TEXT NOT NULL DEFAULT (datetime('now')),
    instance_id TEXT NOT NULL,
    version_vector TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY (agent_id) REFERENCES agents(id)
);

-- Sync state (version vectors per instance)
CREATE TABLE IF NOT EXISTS sync_state (
    instance_id TEXT PRIMARY KEY,
    version_vector TEXT NOT NULL DEFAULT '{}',
    last_sync_at TEXT,
    divergence_detected INTEGER NOT NULL DEFAULT 0
);

-- Instances
CREATE TABLE IF NOT EXISTS instances (
    id TEXT PRIMARY KEY,
    address TEXT NOT NULL,
    last_seen TEXT,
    health_status TEXT NOT NULL DEFAULT 'unknown'
);

-- CI status cache (for dashboard)
CREATE TABLE IF NOT EXISTS ci_status (
    repo TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    run_url TEXT,
    queue_depth INTEGER NOT NULL DEFAULT 0
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_agents_health ON agents(health_status);
CREATE INDEX IF NOT EXISTS idx_agents_heartbeat ON agents(last_heartbeat);
CREATE INDEX IF NOT EXISTS idx_access_agent ON access_policies(agent_id);
CREATE INDEX IF NOT EXISTS idx_access_revoked ON access_policies(revoked_at);
CREATE INDEX IF NOT EXISTS idx_agent_data_agent ON agent_data(agent_id);
CREATE INDEX IF NOT EXISTS idx_agent_data_type ON agent_data(data_type);

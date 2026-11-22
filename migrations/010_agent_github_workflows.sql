-- Per-agent grant of `workflows: write` (REQ-GHTOKEN-005): like actions_write,
-- set only by the operator CLI on the host, never by an agent, and never part
-- of the configured permission set.
ALTER TABLE agent_github ADD COLUMN workflows_write INTEGER NOT NULL DEFAULT 0;

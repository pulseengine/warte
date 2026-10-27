-- Wait history for ETA estimation (the "ask" feature)
CREATE TABLE IF NOT EXISTS wait_history (
    id TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    pr_number INTEGER,
    runner_pool TEXT NOT NULL,
    queued_at TEXT NOT NULL,
    started_at TEXT NOT NULL,
    wait_seconds INTEGER NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_wait_history_pool ON wait_history(runner_pool);
CREATE INDEX IF NOT EXISTS idx_wait_history_started ON wait_history(started_at);
CREATE INDEX IF NOT EXISTS idx_wait_history_repo ON wait_history(repo);

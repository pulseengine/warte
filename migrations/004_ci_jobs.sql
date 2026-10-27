-- CI job records (from GitHub/GitLab webhooks or polling)
CREATE TABLE IF NOT EXISTS ci_jobs (
    id TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    run_id INTEGER NOT NULL,
    run_number INTEGER NOT NULL,
    branch TEXT NOT NULL,
    sha TEXT NOT NULL,
    status TEXT NOT NULL,
    conclusion TEXT,
    queued_at TEXT,
    started_at TEXT,
    completed_at TEXT,
    runner_pool TEXT NOT NULL DEFAULT 'default',
    url TEXT NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(repo, run_id)
);

CREATE INDEX IF NOT EXISTS idx_ci_jobs_repo ON ci_jobs(repo);
CREATE INDEX IF NOT EXISTS idx_ci_jobs_status ON ci_jobs(status);
CREATE INDEX IF NOT EXISTS idx_ci_jobs_started ON ci_jobs(started_at);

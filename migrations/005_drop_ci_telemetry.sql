-- CI telemetry now lives in the ground station (absorbed smithy hub,
-- DD-SMITHY-001): hub-state.json plus the per-day JSONL ledger, with GitHub
-- as the store of record. These tables were fed by the removed webhooks and
-- poller and duplicated that data (REQ-STORE-002).
DROP INDEX IF EXISTS idx_ci_jobs_repo;
DROP INDEX IF EXISTS idx_ci_jobs_status;
DROP INDEX IF EXISTS idx_ci_jobs_started;
DROP TABLE IF EXISTS ci_jobs;

DROP INDEX IF EXISTS idx_wait_history_pool;
DROP INDEX IF EXISTS idx_wait_history_started;
DROP INDEX IF EXISTS idx_wait_history_repo;
DROP TABLE IF EXISTS wait_history;

DROP TABLE IF EXISTS ci_status;

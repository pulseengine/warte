-- Agent triage of alerts (REQ-ALERT-006, warte#219): each opened or escalated
-- alert, the context the triage agent reads, the agent's verdict and the
-- owner's signal. Shadow mode stores only; gate mode reads the verdict.
CREATE TABLE IF NOT EXISTS alert_triage (
    id          TEXT PRIMARY KEY,
    key         TEXT NOT NULL,
    sev         INTEGER NOT NULL,
    text        TEXT NOT NULL,
    opened_at   INTEGER NOT NULL,
    context     TEXT NOT NULL,
    verdict     TEXT,
    reason      TEXT,
    model       TEXT,
    verdict_at  INTEGER,
    owner       TEXT,
    owner_by    TEXT,
    owner_at    INTEGER
);
CREATE INDEX IF NOT EXISTS alert_triage_opened ON alert_triage (opened_at);
-- the triage runner's last poll, its heartbeat (one row)
CREATE TABLE IF NOT EXISTS alert_triage_runner (
    id        INTEGER PRIMARY KEY CHECK (id = 1),
    last_poll INTEGER NOT NULL
);

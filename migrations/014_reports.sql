-- Reports posted by producers (REQ-REPORTS-001, warte#87, seam
-- pulseengine-warte/reports): the latest of each kind, as received.
CREATE TABLE IF NOT EXISTS reports (
    kind        TEXT PRIMARY KEY,
    body        TEXT NOT NULL,
    received_at TEXT NOT NULL
);

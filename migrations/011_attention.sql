-- One attention schedule per warte login (REQ-ATTN-001, warte#181), keyed by
-- the identity provider's subject. doc is attention.schema.json schema_version
-- 1 as validated and served.
CREATE TABLE IF NOT EXISTS attention_schedules (
    subject    TEXT PRIMARY KEY,
    doc        TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

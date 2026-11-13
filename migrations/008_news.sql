-- News items from public feeds (REQ-NEWS-001, contract news v1).
-- One row per (source, key); a changed entry updates its row and keeps its state.
CREATE TABLE IF NOT EXISTS news_items (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    source     TEXT NOT NULL,
    key        TEXT NOT NULL,
    published  TEXT NOT NULL,
    title      TEXT NOT NULL,
    link       TEXT NOT NULL,
    excerpt    TEXT NOT NULL,
    hash       TEXT NOT NULL,
    first_seen TEXT NOT NULL,
    state      TEXT NOT NULL DEFAULT 'found',
    note       TEXT NOT NULL DEFAULT '',
    state_at   TEXT,
    UNIQUE (source, key)
);
CREATE INDEX IF NOT EXISTS idx_news_published ON news_items(published);

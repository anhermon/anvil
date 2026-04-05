-- Episodes: core episodic memory store
CREATE TABLE IF NOT EXISTS episodes (
    id          TEXT PRIMARY KEY NOT NULL,
    session_id  TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK(kind IN ('turn', 'tool_call', 'fact', 'summary')),
    role        TEXT NOT NULL,
    content     TEXT NOT NULL,
    metadata    TEXT,  -- JSON blob
    created_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_episodes_session ON episodes(session_id);
CREATE INDEX IF NOT EXISTS idx_episodes_created ON episodes(created_at);

-- FTS5 virtual table for full-text search over episode content
CREATE VIRTUAL TABLE IF NOT EXISTS episodes_fts USING fts5(
    id UNINDEXED,
    content,
    content='episodes',
    content_rowid='rowid'
);

-- Keep FTS index in sync via triggers
CREATE TRIGGER IF NOT EXISTS episodes_ai AFTER INSERT ON episodes BEGIN
    INSERT INTO episodes_fts(rowid, id, content) VALUES (new.rowid, new.id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS episodes_au AFTER UPDATE ON episodes BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, id, content) VALUES ('delete', old.rowid, old.id, old.content);
    INSERT INTO episodes_fts(rowid, id, content) VALUES (new.rowid, new.id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS episodes_ad AFTER DELETE ON episodes BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, id, content) VALUES ('delete', old.rowid, old.id, old.content);
END;

-- Sessions table
CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY NOT NULL,
    goal        TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'running',
    started_at  TEXT NOT NULL,
    finished_at TEXT
);

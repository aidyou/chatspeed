-- Development-only workflow schema repair for early unreleased v5 databases.
-- Run this manually only if you have a local/test database created from an
-- intermediate workflow schema before the first public workflow release.
--
-- Example:
-- sqlite3 /path/to/chatspeed.db < scripts/v5_ensure.sql

BEGIN TRANSACTION;

-- Workflows table compatibility.
ALTER TABLE workflows ADD COLUMN parent_session_id TEXT REFERENCES workflows(id);
CREATE INDEX IF NOT EXISTS idx_workflows_parent_session_id
ON workflows(parent_session_id);

-- Workflow messages compatibility.
ALTER TABLE workflow_messages ADD COLUMN message_kind TEXT NOT NULL DEFAULT 'message';
ALTER TABLE workflow_messages ADD COLUMN message_subtype TEXT;
ALTER TABLE workflow_messages ADD COLUMN segment_id INTEGER NOT NULL DEFAULT 1;
ALTER TABLE workflow_messages ADD COLUMN source_event_type TEXT;

-- AI-only context projection compatibility.
CREATE TABLE IF NOT EXISTS workflow_context_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    segment_id INTEGER NOT NULL,
    role TEXT NOT NULL,
    message TEXT NOT NULL,
    reasoning TEXT,
    message_kind TEXT NOT NULL DEFAULT 'message',
    message_subtype TEXT,
    metadata TEXT,
    source_message_id INTEGER,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (session_id) REFERENCES workflows(id),
    FOREIGN KEY (source_message_id) REFERENCES workflow_messages(id)
);
ALTER TABLE workflow_context_messages ADD COLUMN message_kind TEXT NOT NULL DEFAULT 'message';
ALTER TABLE workflow_context_messages ADD COLUMN message_subtype TEXT;
CREATE INDEX IF NOT EXISTS idx_workflow_context_messages_session_segment_id
ON workflow_context_messages(session_id, segment_id, id);

-- Memory candidate compatibility.
CREATE TABLE IF NOT EXISTS memory_candidates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope TEXT NOT NULL,
    category TEXT NOT NULL,
    content TEXT NOT NULL,
    normalized_content TEXT NOT NULL,
    project_key TEXT NOT NULL DEFAULT '',
    project_root TEXT,
    source_session_id TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0.0,
    explicitness INTEGER NOT NULL DEFAULT 0,
    occurrence_count INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    last_seen_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
ALTER TABLE memory_candidates ADD COLUMN project_key TEXT NOT NULL DEFAULT '';
ALTER TABLE memory_candidates ADD COLUMN project_root TEXT;
ALTER TABLE memory_candidates ADD COLUMN confidence REAL NOT NULL DEFAULT 0.0;
ALTER TABLE memory_candidates ADD COLUMN explicitness INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memory_candidates ADD COLUMN occurrence_count INTEGER NOT NULL DEFAULT 1;
ALTER TABLE memory_candidates ADD COLUMN status TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE memory_candidates ADD COLUMN last_seen_at DATETIME DEFAULT CURRENT_TIMESTAMP;
ALTER TABLE memory_candidates ADD COLUMN updated_at DATETIME DEFAULT CURRENT_TIMESTAMP;
CREATE INDEX IF NOT EXISTS idx_memory_candidates_scope_project_status
ON memory_candidates(scope, project_key, status, updated_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_candidates_unique_content
ON memory_candidates(scope, category, normalized_content, project_key);

COMMIT;

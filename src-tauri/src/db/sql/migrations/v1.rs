use rusqlite::Connection;

use crate::db::sql::schema::*;
use crate::db::StoreError;

/// Initial database schema creation SQL statements
pub const INIT_SQL: &[(&str, &str)] = &[
    (
        "db_version",
        "CREATE TABLE IF NOT EXISTS db_version (
            version INTEGER PRIMARY KEY,
            applied_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    ),
    (
        CONFIG_TABLE,
        "CREATE TABLE IF NOT EXISTS config (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
    ),
    // ALTER TABLE ai_model RENAME COLUMN api_provider TO api_protocol;
    (
        AI_MODEL_TABLE,
        "CREATE TABLE IF NOT EXISTS ai_model (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            models TEXT NOT NULL,
            default_model TEXT NOT NULL,
            api_protocol TEXT NOT NULL,
            base_url TEXT NOT NULL,
            api_key TEXT NOT NULL,
            max_tokens INTEGER NOT NULL DEFAULT 4096,
            temperature REAL NOT NULL DEFAULT 1.0,
            top_p REAL NOT NULL DEFAULT 1.0,
            top_k INTEGER NOT NULL DEFAULT 40,
            sort_index INTEGER NOT NULL DEFAULT 0,
            is_default BOOLEAN NOT NULL DEFAULT FALSE,
            disabled BOOLEAN NOT NULL DEFAULT FALSE,
            is_official BOOLEAN NOT NULL DEFAULT FALSE,
            official_id TEXT NOT NULL DEFAULT '',
            metadata TEXT
        )",
    ),
    (
        "idx_sort_index_ai_model",
        "CREATE INDEX IF NOT EXISTS idx_sort_index ON ai_model (sort_index)",
    ),
    (
        "idx_official_id",
        "CREATE INDEX IF NOT EXISTS idx_official_id ON ai_model (official_id)",
    ),
    (
        AI_SKILL_TABLE,
        "CREATE TABLE IF NOT EXISTS ai_skill (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            icon TEXT,
            logo TEXT,
            prompt TEXT NOT NULL,
            share_id TEXT,
            sort_index INTEGER NOT NULL DEFAULT 0,
            disabled BOOLEAN NOT NULL DEFAULT FALSE,
            metadata TEXT
        )",
    ),
    (
        "idx_sort_index_ai_skill",
        "CREATE INDEX IF NOT EXISTS idx_sort_index ON ai_skill (sort_index)",
    ),
    (
        "idx_share_id",
        "CREATE INDEX IF NOT EXISTS idx_share_id ON ai_skill (share_id)",
    ),
    (
        CONVERSATIONS_TABLE,
        "CREATE TABLE IF NOT EXISTS conversations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            is_favorite BOOLEAN DEFAULT FALSE
        )",
    ),
    (
        "idx_title",
        "CREATE INDEX IF NOT EXISTS idx_title ON conversations (title)",
    ),
    (
        MESSAGES_TABLE,
        "CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_id INTEGER,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
            metadata TEXT,
            FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
        )",
    ),
    (
        "idx_conversation_id",
        "CREATE INDEX IF NOT EXISTS idx_conversation_id ON messages (conversation_id)",
    ),
    // Notes related tables
    (
        "notes",
        "CREATE TABLE IF NOT EXISTS notes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tags TEXT,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            conversation_id INTEGER,
            message_id INTEGER,
            created_at INTEGER NOT NULL DEFAULT (unixepoch()),
            updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
            deleted_at INTEGER,
            metadata TEXT,
            FOREIGN KEY (conversation_id) REFERENCES conversations (id) ON DELETE SET NULL,
            FOREIGN KEY (message_id) REFERENCES messages (id) ON DELETE SET NULL
        )",
    ),
    (
        "note_tag_items",
        "CREATE TABLE IF NOT EXISTS note_tag_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            note_count INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        )",
    ),
    (
        "note_tag_relations",
        "CREATE TABLE IF NOT EXISTS note_tag_relations (
            tag_id INTEGER NOT NULL,
            note_id INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (tag_id, note_id),
            FOREIGN KEY (note_id) REFERENCES notes (id),
            FOREIGN KEY (tag_id) REFERENCES note_tag_items (id)
        )",
    ),
    // Indexes for notes
    (
        "idx_notes_title",
        "CREATE INDEX IF NOT EXISTS idx_notes_title ON notes (title)",
    ),
    (
        "idx_notes_content_hash",
        "CREATE INDEX IF NOT EXISTS idx_notes_content_hash ON notes (content_hash)",
    ),
    (
        "idx_notes_source",
        "CREATE INDEX IF NOT EXISTS idx_notes_source ON notes (conversation_id, message_id)",
    ),
    (
        "idx_notes_created_at",
        "CREATE INDEX IF NOT EXISTS idx_notes_created_at ON notes (created_at) WHERE deleted_at IS NULL",
    ),
    // Index for note tags
    (
        "idx_note_tag_items_name",
        "CREATE INDEX IF NOT EXISTS idx_note_tag_items_name ON note_tag_items (name)",
    ),
    (
        "mcp",
        "CREATE TABLE IF NOT EXISTS mcp (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            config TEXT NOT NULL,
            disabled BOOLEAN NOT NULL DEFAULT FALSE
        )",
    ),
    (
        "mcp_name_key",
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_mcp_name ON mcp (name)",
    ),
    (
        "proxy_group",
        "CREATE TABLE IF NOT EXISTS proxy_group (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            description TEXT NOT NULL,
            prompt_injection TEXT NOT NULL,
            prompt_text TEXT NOT NULL,
            tool_filter TEXT NOT NULL,
            temperature FLOAT NOT NULL,
            metadata TEXT,
            disabled BOOLEAN NOT NULL DEFAULT FALSE
        )",
    ),
    (
        "proxy_group_name_key",
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_proxy_group_name ON proxy_group (name)",
    )
];

/// Executes initial database schema creation
pub fn run_migration(conn: &mut Connection) -> Result<(), StoreError> {
    // start transaction
    let tx = conn.transaction()?;

    for (_name, sql) in INIT_SQL {
        tx.execute(sql, [])?;
    }

    // insert database version
    tx.execute("INSERT OR REPLACE INTO db_version (version) VALUES (1)", [])?;

    // commit transaction
    tx.commit()?;

    Ok(())
}

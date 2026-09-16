use super::common::MigrationDefinition;
use crate::db::StoreError;
use rusqlite::Connection;

/// Name of the statement that seeds the preset ChatHub entries.
///
/// The seed must only run when the migration is applied, never from
/// [`ensure_chat_hub_table`], otherwise preset entries would come back after the
/// user deletes them.
const SEED_STATEMENT_NAME: &str = "seed_default_chat_hubs";

pub const MIGRATION_SQL: &[(&str, &str)] = &[
    (
        "chat_hubs",
        "CREATE TABLE IF NOT EXISTS chat_hubs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            logo TEXT NOT NULL DEFAULT '',
            url TEXT NOT NULL,
            sort_index INTEGER NOT NULL DEFAULT 0,
            is_default INTEGER NOT NULL DEFAULT 0
        )",
    ),
    (
        "idx_chat_hubs_sort_index",
        "CREATE INDEX IF NOT EXISTS idx_chat_hubs_sort_index ON chat_hubs(sort_index, id)",
    ),
    (
        SEED_STATEMENT_NAME,
        // Only seed while the table is still empty. This statement is part of the
        // one-time migration, so deleting every entry (including presets) is
        // permanent: the migration never runs again.
        "INSERT INTO chat_hubs (name, logo, url, sort_index, is_default)
        SELECT * FROM (
            SELECT 'ChatGPT' AS name, 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fchatgpt.com%2F' AS logo, 'https://chatgpt.com/' AS url, 0 AS sort_index, 1 AS is_default
            UNION ALL SELECT 'Claude', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fclaude.ai%2Fnew', 'https://claude.ai/new', 1, 1
            UNION ALL SELECT 'Gemini', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fgemini.google.com%2Fapp', 'https://gemini.google.com/app', 2, 1
            UNION ALL SELECT 'DeepSeek', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fchat.deepseek.com%2F', 'https://chat.deepseek.com/', 3, 1
            UNION ALL SELECT 'Grok', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fgrok.com%2F', 'https://grok.com/', 4, 1
            UNION ALL SELECT 'Qwen', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fchat.qwen.ai%2F', 'https://chat.qwen.ai/', 5, 1
            UNION ALL SELECT 'Perplexity', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fwww.perplexity.ai%2F', 'https://www.perplexity.ai/', 6, 1
        )
        WHERE NOT EXISTS (SELECT 1 FROM chat_hubs)",
    ),
];

/// Re-applies the idempotent schema statements on every startup without ever
/// running the preset seed again.
fn ensure_chat_hub_table(conn: &Connection) -> Result<(), StoreError> {
    for (name, sql) in MIGRATION_SQL {
        if *name == SEED_STATEMENT_NAME {
            continue;
        }
        conn.execute(sql, [])?;
    }
    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 18,
    description: "v18 migration: Add chat_hubs table for ChatHub web chat entries",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_chat_hub_table),
    apply: None,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM chat_hubs", [], |row| row.get(0))
            .expect("failed to count chat_hubs rows")
    }

    fn apply_migration(conn: &Connection) {
        for (_, sql) in MIGRATION_SQL {
            conn.execute(sql, []).expect("failed to apply migration sql");
        }
    }

    #[test]
    fn creates_table_and_seeds_presets_once() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        apply_migration(&conn);

        assert_eq!(count(&conn), 7, "preset entries should be seeded");
        let presets: i64 = conn
            .query_row("SELECT COUNT(*) FROM chat_hubs WHERE is_default = 1", [], |row| {
                row.get(0)
            })
            .expect("failed to count preset entries");
        assert_eq!(presets, 7, "every seeded entry should be marked as a preset");

        let ordered: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM chat_hubs ORDER BY sort_index, id")
                .expect("failed to prepare ordered query");
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .expect("failed to query ordered rows");
            rows.map(|row| row.expect("failed to read name")).collect()
        };
        assert_eq!(ordered.first().map(String::as_str), Some("ChatGPT"));
    }

    #[test]
    fn seed_is_skipped_when_table_already_has_rows() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        conn.execute(
            "CREATE TABLE chat_hubs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                logo TEXT NOT NULL DEFAULT '',
                url TEXT NOT NULL,
                sort_index INTEGER NOT NULL DEFAULT 0,
                is_default INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )
        .expect("failed to create table");
        conn.execute(
            "INSERT INTO chat_hubs (name, logo, url, sort_index, is_default)
             VALUES ('Custom', '', 'https://example.com/', 0, 0)",
            [],
        )
        .expect("failed to insert custom entry");

        apply_migration(&conn);

        assert_eq!(count(&conn), 1, "existing entries must not be duplicated");
    }

    #[test]
    fn ensure_never_reseeds_deleted_entries() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        apply_migration(&conn);
        conn.execute("DELETE FROM chat_hubs", [])
            .expect("failed to delete entries");

        // Simulates every later startup, where the migration is not re-applied.
        ensure_chat_hub_table(&conn).expect("ensure should succeed");
        ensure_chat_hub_table(&conn).expect("ensure should be idempotent");

        assert_eq!(count(&conn), 0, "deleted entries must never be restored");
    }
}
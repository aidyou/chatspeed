use super::automation_schema::ensure_automation_concurrency_schema;
use super::common::{column_exists, MigrationDefinition};
use super::tool_compatibility::ensure_v22_data;
use super::capability_schema::ensure_capability_journal;
use crate::db::StoreError;
use rusqlite::{Connection, Transaction};

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
            SELECT 'Gemini' AS name, 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fgemini.google.com%2Fapp' AS logo, 'https://gemini.google.com/app' AS url, 0 AS sort_index, 1 AS is_default
            UNION ALL SELECT 'DeepSeek', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fchat.deepseek.com%2F', 'https://chat.deepseek.com/', 1, 1
            UNION ALL SELECT 'ChatGPT', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fchatgpt.com%2F', 'https://chatgpt.com/', 2, 1
            UNION ALL SELECT 'Qwen', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fchat.qwen.ai%2F', 'https://chat.qwen.ai/', 3, 1
            UNION ALL SELECT '豆包', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fwww.doubao.com%2Fchat%2F', 'https://www.doubao.com/chat/', 4, 1
            UNION ALL SELECT 'Kimi', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fwww.kimi.com%2F', 'https://www.kimi.com/', 5, 1
            UNION ALL SELECT 'Z.ai', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fchat.z.ai%2F', 'https://chat.z.ai/', 6, 1
            UNION ALL SELECT 'ChatGLM', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fchatglm.cn%2F', 'https://chatglm.cn/', 7, 1
            UNION ALL SELECT 'Meta AI', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fmeta.ai%2F', 'https://meta.ai/', 8, 1
            UNION ALL SELECT '文心一言', 'https://www.google.com/s2/favicons?sz=64&domain_url=https%3A%2F%2Fernie.baidu.com%2F', 'https://wenxin.baidu.com/', 9, 1
        )
        WHERE NOT EXISTS (SELECT 1 FROM chat_hubs)",
    ),
    (
        "agents_report_required_sections",
        "ALTER TABLE agents ADD COLUMN report_required_sections TEXT",
    ),
    (
        "capability_operations",
        "CREATE TABLE IF NOT EXISTS capability_operations (
            operation_id TEXT PRIMARY KEY,
            capability TEXT NOT NULL CHECK (capability IN ('skill','mcp')),
            operation_kind TEXT NOT NULL,
            actor_scope TEXT NOT NULL,
            idempotency_key TEXT NOT NULL,
            request_hash TEXT NOT NULL,
            request_json TEXT NOT NULL,
            resource_key TEXT NOT NULL,
            state TEXT NOT NULL CHECK (state IN ('planned','staging','checking','applying','completed','blocked','failed','needs_reconcile')),
            phase TEXT,
            result_json TEXT,
            error_code TEXT,
            error_message TEXT,
            reconcile_reason TEXT,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            completed_at_ms INTEGER,
            UNIQUE (capability, actor_scope, idempotency_key)
        )",
    ),
    (
        "capability_operations_state_idx",
        "CREATE INDEX IF NOT EXISTS capability_operations_state_idx ON capability_operations (capability, state, created_at_ms)",
    ),
    (
        "capability_operations_resource_idx",
        "CREATE INDEX IF NOT EXISTS capability_operations_resource_idx ON capability_operations (capability, resource_key, created_at_ms)",
    ),
    (
        "capability_operation_effects",
        "CREATE TABLE IF NOT EXISTS capability_operation_effects (
            effect_id TEXT PRIMARY KEY,
            operation_id TEXT NOT NULL REFERENCES capability_operations(operation_id),
            effect_key TEXT NOT NULL,
            target TEXT,
            intent TEXT NOT NULL CHECK (intent IN ('not_started','intent_recorded','completed')),
            outcome TEXT NOT NULL CHECK (outcome IN ('pending','applied','skipped','blocked','failed','unknown')),
            detail_json TEXT,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            UNIQUE (operation_id, effect_key)
        )",
    ),
    (
        "capability_operation_effects_outcome_idx",
        "CREATE INDEX IF NOT EXISTS capability_operation_effects_outcome_idx ON capability_operation_effects (operation_id, outcome)",
    ),
    (
        "skill_installations",
        "CREATE TABLE IF NOT EXISTS skill_installations (
            installation_id TEXT PRIMARY KEY,
            skill_name TEXT NOT NULL,
            target_id TEXT NOT NULL,
            install_path TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            source_ref TEXT NOT NULL,
            checker_version TEXT NOT NULL,
            verdict TEXT NOT NULL,
            content_digest TEXT NOT NULL,
            file_manifest_json TEXT NOT NULL,
            marker_nonce TEXT NOT NULL,
            manifest_digest TEXT NOT NULL,
            state TEXT NOT NULL CHECK (state IN ('installing','installed','quarantined','removed','drifted')),
            operation_id TEXT,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            UNIQUE (target_id, skill_name)
        )",
    ),
    (
        "skill_installations_state_idx",
        "CREATE INDEX IF NOT EXISTS skill_installations_state_idx ON skill_installations (state, target_id)",
    ),
    (
        "automation_operations",
        "CREATE TABLE IF NOT EXISTS automation_operations (
            operation_id TEXT PRIMARY KEY,
            actor_scope TEXT NOT NULL,
            operation TEXT NOT NULL,
            idempotency_key TEXT NOT NULL,
            request_hash TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('in_progress','completed','failed')),
            result_json TEXT,
            error_code TEXT,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            UNIQUE (actor_scope, idempotency_key)
        )",
    ),
    (
        "idx_automation_operations_status",
        "CREATE INDEX IF NOT EXISTS idx_automation_operations_status ON automation_operations (status, created_at_ms)",
    ),
];

/// Re-applies the idempotent schema statements on every startup without ever
/// running the preset seed again.
fn ensure_chat_hub_table(conn: &Connection) -> Result<(), StoreError> {
    for (name, sql) in MIGRATION_SQL {
        if *name == SEED_STATEMENT_NAME {
            continue;
        }
        if *name == "agents_report_required_sections"
            && column_exists(conn, "agents", "report_required_sections")?
        {
            continue;
        }
        conn.execute(sql, [])?;
    }
    ensure_capability_journal(conn)?;
    ensure_automation_concurrency_schema(conn)?;
    ensure_v22_data(conn)?;
    Ok(())
}

fn apply_post_v18_data(tx: &Transaction<'_>) -> Result<(), StoreError> {
    super::tool_compatibility::apply_v22_data(tx)
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 18,
    description: "v18 migration: Add chat_hubs table for ChatHub web chat entries",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_chat_hub_table),
    apply: Some(apply_post_v18_data),
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
            conn.execute(sql, [])
                .expect("failed to apply migration sql");
        }
    }

    #[test]
    fn creates_table_and_seeds_presets_once() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        apply_migration(&conn);

        assert_eq!(count(&conn), 10, "preset entries should be seeded");
        let presets: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chat_hubs WHERE is_default = 1",
                [],
                |row| row.get(0),
            )
            .expect("failed to count preset entries");
        assert_eq!(
            presets, 10,
            "every seeded entry should be marked as a preset"
        );

        let ordered: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM chat_hubs ORDER BY sort_index, id")
                .expect("failed to prepare ordered query");
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .expect("failed to query ordered rows");
            rows.map(|row| row.expect("failed to read name")).collect()
        };
        assert_eq!(ordered.first().map(String::as_str), Some("Gemini"));
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

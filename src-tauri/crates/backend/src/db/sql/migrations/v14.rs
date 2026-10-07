use super::common::MigrationDefinition;
use crate::db::StoreError;
use rusqlite::{params, Connection, Transaction};
use serde_json::Value;
use std::collections::HashMap;

const JAVASCRIPT_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

pub const MIGRATION_SQL: &[(&str, &str)] = &[];

fn remap_message_id(value: &mut Value, id_map: &HashMap<i64, i64>) -> bool {
    let Some(old_id) = value.as_i64() else {
        return false;
    };
    let Some(new_id) = id_map.get(&old_id) else {
        return false;
    };
    if *new_id == old_id {
        return false;
    }

    *value = Value::from(*new_id);
    true
}

fn remap_pending_completion_report_ids(
    execution_context: &mut Value,
    id_map: &HashMap<i64, i64>,
) -> bool {
    let Some(reports) = execution_context
        .as_object_mut()
        .and_then(|context| context.get_mut("pending_completion_reports"))
        .and_then(Value::as_array_mut)
    else {
        return false;
    };

    reports.iter_mut().fold(false, |changed, report| {
        let remapped = report
            .as_object_mut()
            .and_then(|report| report.get_mut("source_message_id"))
            .is_some_and(|source_message_id| remap_message_id(source_message_id, id_map));
        changed || remapped
    })
}

fn remap_workflow_message_metadata(metadata: &mut Value, id_map: &HashMap<i64, i64>) -> bool {
    let Some(metadata) = metadata.as_object_mut() else {
        return false;
    };

    let mut changed = metadata
        .get_mut("compressed_until_message_id")
        .is_some_and(|boundary_id| remap_message_id(boundary_id, id_map));
    if let Some(previous_execution_context) = metadata.get_mut("previous_execution_context") {
        changed |= remap_pending_completion_report_ids(previous_execution_context, id_map);
    }
    changed
}

fn has_unsafe_workflow_message_ids(conn: &Connection) -> Result<bool, StoreError> {
    let max_message_id: Option<i64> =
        conn.query_row("SELECT MAX(id) FROM workflow_messages", [], |row| {
            row.get(0)
        })?;
    Ok(max_message_id.is_some_and(|message_id| message_id > JAVASCRIPT_MAX_SAFE_INTEGER))
}

fn has_unsafe_workflow_message_ids_in_transaction(
    transaction: &Transaction<'_>,
) -> Result<bool, StoreError> {
    let max_message_id: Option<i64> =
        transaction.query_row("SELECT MAX(id) FROM workflow_messages", [], |row| {
            row.get(0)
        })?;
    Ok(max_message_id.is_some_and(|message_id| message_id > JAVASCRIPT_MAX_SAFE_INTEGER))
}

fn normalize_workflow_message_ids_in_transaction(
    transaction: &Transaction<'_>,
) -> Result<(), StoreError> {
    if !has_unsafe_workflow_message_ids_in_transaction(transaction)? {
        return Ok(());
    }

    transaction.execute_batch("PRAGMA defer_foreign_keys = ON")?;

    let message_rows = {
        let mut statement =
            transaction.prepare("SELECT id, metadata FROM workflow_messages ORDER BY id ASC")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let id_map = message_rows
        .iter()
        .enumerate()
        .map(|(index, (old_id, _))| {
            Ok((
                *old_id,
                i64::try_from(index + 1).map_err(|_| {
                    StoreError::InvalidData(
                        "Too many workflow messages to normalize IDs".to_string(),
                    )
                })?,
            ))
        })
        .collect::<Result<HashMap<_, _>, StoreError>>()?;

    if message_rows.iter().any(|(id, _)| *id <= 0) {
        return Err(StoreError::InvalidData(
            "Workflow message ID normalization requires positive message IDs".to_string(),
        ));
    }

    let mut updated_metadata = Vec::new();
    for (old_id, metadata_json) in &message_rows {
        let Some(metadata_json) = metadata_json else {
            continue;
        };
        let mut metadata: Value = serde_json::from_str(metadata_json)?;
        if remap_workflow_message_metadata(&mut metadata, &id_map) {
            updated_metadata.push((*old_id, serde_json::to_string(&metadata)?));
        }
    }
    for (old_id, metadata_json) in updated_metadata {
        transaction.execute(
            "UPDATE workflow_messages SET metadata = ?1 WHERE id = ?2",
            params![metadata_json, old_id],
        )?;
    }

    let snapshots = {
        let mut statement =
            transaction.prepare("SELECT session_id, context_json FROM workflow_snapshots")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (session_id, context_json) in snapshots {
        let mut execution_context: Value = serde_json::from_str(&context_json)?;
        if remap_pending_completion_report_ids(&mut execution_context, &id_map) {
            transaction.execute(
                "UPDATE workflow_snapshots SET context_json = ?1 WHERE session_id = ?2",
                params![serde_json::to_string(&execution_context)?, session_id],
            )?;
        }
    }

    // workflow_context_messages is a rebuildable AI-context cache. Clearing it removes its
    // foreign-key and copied metadata references so the durable transcript remains authoritative.
    transaction.execute("DELETE FROM workflow_context_messages", [])?;
    transaction.execute_batch(
        "CREATE TEMP TABLE workflow_message_id_map (
             old_id INTEGER PRIMARY KEY,
             new_id INTEGER NOT NULL UNIQUE
         );",
    )?;
    {
        let mut statement = transaction
            .prepare("INSERT INTO workflow_message_id_map (old_id, new_id) VALUES (?1, ?2)")?;
        for (old_id, new_id) in &id_map {
            statement.execute(params![old_id, new_id])?;
        }
    }

    // Move every existing key outside the positive AUTOINCREMENT range before assigning its new
    // sequential value, avoiding collisions with rows that already occupy a target ID.
    transaction.execute("UPDATE workflow_messages SET id = -id", [])?;
    transaction.execute(
        "UPDATE workflow_messages
         SET id = (
             SELECT new_id FROM workflow_message_id_map
             WHERE old_id = -workflow_messages.id
         )",
        [],
    )?;
    transaction.execute("DROP TABLE workflow_message_id_map", [])?;

    transaction.execute(
        "DELETE FROM sqlite_sequence WHERE name = 'workflow_messages'",
        [],
    )?;
    if let Some(sequence) = i64::try_from(message_rows.len())
        .ok()
        .filter(|sequence| *sequence > 0)
    {
        transaction.execute(
            "INSERT INTO sqlite_sequence (name, seq) VALUES ('workflow_messages', ?1)",
            [sequence],
        )?;
    }

    let has_context_cache_foreign_key_violation = {
        let mut statement =
            transaction.prepare("PRAGMA foreign_key_check(workflow_context_messages)")?;
        let mut rows = statement.query([])?;
        rows.next()?.is_some()
    };
    if has_context_cache_foreign_key_violation {
        return Err(StoreError::InvalidData(
            "Workflow context cache has invalid source message references after ID normalization"
                .to_string(),
        ));
    }

    log::info!(
        "Normalized {} workflow message IDs and reset the AUTOINCREMENT sequence",
        message_rows.len()
    );
    Ok(())
}

fn normalize_workflow_message_ids(conn: &Connection) -> Result<(), StoreError> {
    if !has_unsafe_workflow_message_ids(conn)? {
        return Ok(());
    }

    let transaction = conn.unchecked_transaction()?;
    normalize_workflow_message_ids_in_transaction(&transaction)?;
    transaction.commit()?;
    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 14,
    description: "v14 migration: Normalize workflow message IDs to SQLite AUTOINCREMENT values",
    sql: MIGRATION_SQL,
    ensure: Some(normalize_workflow_message_ids),
    apply: Some(normalize_workflow_message_ids_in_transaction),
};

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_database() -> Connection {
        let conn = Connection::open_in_memory().expect("failed to open database");
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE workflow_messages (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 metadata TEXT
             );
             CREATE TABLE workflow_context_messages (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 source_message_id INTEGER NOT NULL REFERENCES workflow_messages(id),
                 metadata TEXT
             );
             CREATE TABLE workflow_snapshots (
                 session_id TEXT PRIMARY KEY,
                 context_json TEXT NOT NULL
             );",
        )
        .expect("failed to create migration fixture");
        conn
    }

    #[test]
    fn normalizes_message_ids_rewrites_durable_references_and_resets_sequence() {
        let conn = setup_database();
        let high_id = 873149882892816384_i64;
        conn.execute(
            "INSERT INTO workflow_messages (id, metadata) VALUES (?1, ?2)",
            params![
                42_i64,
                r#"{"previous_context_tokens":42,"compressed_until_message_id":42}"#
            ],
        )
        .expect("failed to insert first workflow message");
        conn.execute(
            "INSERT INTO workflow_messages (id, metadata) VALUES (?1, ?2)",
            params![
                high_id,
                format!(
                    r#"{{"compressed_until_message_id":42,"previous_execution_context":{{"pending_completion_reports":[{{"source_message_id":{high_id}}}]}}}}"#
                )
            ],
        )
        .expect("failed to insert high-ID workflow message");
        conn.execute(
            "INSERT INTO workflow_context_messages (source_message_id, metadata) VALUES (?1, ?2)",
            params![high_id, r#"{"compressed_until_message_id":42}"#],
        )
        .expect("failed to insert context cache message");
        conn.execute(
            "INSERT INTO workflow_snapshots (session_id, context_json) VALUES (?1, ?2)",
            params![
                "session",
                format!(r#"{{"pending_completion_reports":[{{"source_message_id":{high_id}}}]}}"#)
            ],
        )
        .expect("failed to insert workflow snapshot");

        normalize_workflow_message_ids(&conn).expect("migration should succeed");

        let ids = conn
            .prepare("SELECT id FROM workflow_messages ORDER BY id ASC")
            .expect("failed to prepare message query")
            .query_map([], |row| row.get::<_, i64>(0))
            .expect("failed to query messages")
            .collect::<Result<Vec<_>, _>>()
            .expect("failed to collect message IDs");
        assert_eq!(ids, vec![1, 2]);

        let metadata_json: String = conn
            .query_row(
                "SELECT metadata FROM workflow_messages WHERE id = 2",
                [],
                |row| row.get(0),
            )
            .expect("failed to load rewritten metadata");
        let metadata: Value =
            serde_json::from_str(&metadata_json).expect("rewritten metadata must be valid JSON");
        assert_eq!(metadata["compressed_until_message_id"], 1);
        assert_eq!(
            metadata["previous_execution_context"]["pending_completion_reports"][0]
                ["source_message_id"],
            2
        );
        let first_metadata_json: String = conn
            .query_row(
                "SELECT metadata FROM workflow_messages WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("failed to load first rewritten metadata");
        let first_metadata: Value = serde_json::from_str(&first_metadata_json)
            .expect("first rewritten metadata must be valid JSON");
        assert_eq!(first_metadata["compressed_until_message_id"], 1);
        assert_eq!(first_metadata["previous_context_tokens"], 42);

        let snapshot_json: String = conn
            .query_row(
                "SELECT context_json FROM workflow_snapshots WHERE session_id = 'session'",
                [],
                |row| row.get(0),
            )
            .expect("failed to load rewritten snapshot");
        let snapshot: Value =
            serde_json::from_str(&snapshot_json).expect("rewritten snapshot must be valid JSON");
        assert_eq!(
            snapshot["pending_completion_reports"][0]["source_message_id"],
            2
        );

        let cache_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM workflow_context_messages",
                [],
                |row| row.get(0),
            )
            .expect("failed to count context cache rows");
        assert_eq!(cache_count, 0, "rebuildable context cache must be cleared");

        let sequence: i64 = conn
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name = 'workflow_messages'",
                [],
                |row| row.get(0),
            )
            .expect("failed to read sequence");
        assert_eq!(sequence, 2);
        conn.execute("INSERT INTO workflow_messages (metadata) VALUES (NULL)", [])
            .expect("failed to verify next AUTOINCREMENT value");
        assert_eq!(conn.last_insert_rowid(), 3);

        normalize_workflow_message_ids(&conn).expect("second migration run should be a no-op");
        assert!(!has_unsafe_workflow_message_ids(&conn)
            .expect("failed to check normalized message IDs"));
    }

    #[test]
    fn skips_databases_without_unsafe_workflow_message_ids() {
        let conn = setup_database();
        conn.execute(
            "INSERT INTO workflow_messages (id, metadata) VALUES (1, NULL), (3, NULL)",
            [],
        )
        .expect("failed to insert normal workflow messages");
        conn.execute(
            "INSERT INTO workflow_context_messages (source_message_id, metadata) VALUES (3, NULL)",
            [],
        )
        .expect("failed to insert context cache message");

        normalize_workflow_message_ids(&conn)
            .expect("normal-ID database should skip the normalization migration");

        let ids = conn
            .prepare("SELECT id FROM workflow_messages ORDER BY id ASC")
            .expect("failed to prepare message query")
            .query_map([], |row| row.get::<_, i64>(0))
            .expect("failed to query messages")
            .collect::<Result<Vec<_>, _>>()
            .expect("failed to collect message IDs");
        assert_eq!(ids, vec![1, 3]);

        let context_source_message_id: i64 = conn
            .query_row(
                "SELECT source_message_id FROM workflow_context_messages",
                [],
                |row| row.get(0),
            )
            .expect("context cache should not be cleared for normal IDs");
        assert_eq!(context_source_message_id, 3);
    }
}

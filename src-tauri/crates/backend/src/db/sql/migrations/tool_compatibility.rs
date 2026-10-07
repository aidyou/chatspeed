use crate::db::StoreError;
use rusqlite::{params, Connection, Transaction};
use serde_json::Value;
use std::collections::HashSet;

const LEGACY_GLOB_TOOL: &str = "glob";
const GREP_TOOL: &str = "grep";
const MIGRATION_BATCH_SIZE: i64 = 500;

/// Rewrites a JSON array of tool ids in place: the legacy `glob` id becomes
/// `grep`, and duplicate ids are dropped while preserving first-seen order.
///
/// A plain string REPLACE cannot be used here because it would leave duplicates
/// such as `["grep","grep"]` behind. Returns `true` when the array changed.
fn rewrite_tool_id_array(array: &mut Value) -> bool {
    let Some(items) = array.as_array_mut() else {
        return false;
    };
    if !items
        .iter()
        .any(|item| item.as_str() == Some(LEGACY_GLOB_TOOL))
    {
        return false;
    }

    let mut rewritten: Vec<Value> = Vec::with_capacity(items.len());
    let mut seen: HashSet<String> = HashSet::new();

    for item in items.iter() {
        match item {
            Value::String(name) => {
                let name = if name.as_str() == LEGACY_GLOB_TOOL {
                    GREP_TOOL.to_string()
                } else {
                    name.clone()
                };
                if !seen.insert(name.clone()) {
                    continue;
                }
                rewritten.push(Value::String(name));
            }
            // Only string ids identify tools; keep anything else untouched.
            other => rewritten.push(other.clone()),
        }
    }

    if *items == rewritten {
        return false;
    }

    *items = rewritten;
    true
}

/// Rewrites a stored tool-id array (for example `agents.available_tools`) and
/// returns the new JSON text when it changed. Unparsable or non-array values are
/// left alone so a single bad row never aborts the migration.
fn rewrite_tool_ids_json(raw: &str) -> Option<String> {
    let mut value: Value = serde_json::from_str(raw).ok()?;
    if !rewrite_tool_id_array(&mut value) {
        return None;
    }
    serde_json::to_string(&value).ok()
}

/// Rewrites `availableTools` / `autoApprove` inside an `agent_config` object and
/// returns the new JSON text when it changed. Every other key, including
/// `shellPolicy`, is preserved.
fn rewrite_agent_config_json(raw: &str) -> Option<String> {
    let mut value: Value = serde_json::from_str(raw).ok()?;
    let Some(object) = value.as_object_mut() else {
        return None;
    };

    let mut changed = false;
    for key in ["availableTools", "autoApprove"] {
        if let Some(array) = object.get_mut(key) {
            if rewrite_tool_id_array(array) {
                changed = true;
            }
        }
    }

    if !changed {
        return None;
    }

    serde_json::to_string(&value).ok()
}

fn ensure_agent_tool_columns(conn: &Connection) -> Result<(), StoreError> {
    let rows = {
        let mut statement =
            conn.prepare("SELECT rowid, available_tools, auto_approve FROM agents")?;
        let mapped = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        mapped.collect::<Result<Vec<_>, rusqlite::Error>>()?
    };

    for (rowid, available_tools, auto_approve) in rows {
        let updated_available = available_tools.as_deref().and_then(rewrite_tool_ids_json);
        let updated_auto_approve = auto_approve.as_deref().and_then(rewrite_tool_ids_json);
        if updated_available.is_none() && updated_auto_approve.is_none() {
            continue;
        }

        conn.execute(
            "UPDATE agents
             SET available_tools = COALESCE(?1, available_tools),
                 auto_approve = COALESCE(?2, auto_approve)
             WHERE rowid = ?3",
            params![updated_available, updated_auto_approve, rowid],
        )?;
    }

    Ok(())
}

fn ensure_agent_config_columns(conn: &Connection, table: &str) -> Result<(), StoreError> {
    let mut last_rowid = 0_i64;
    loop {
        let rows = {
            let mut statement = conn.prepare(&format!(
                "SELECT rowid, agent_config FROM {table}
                 WHERE rowid > ?1 AND agent_config LIKE '%glob%'
                 ORDER BY rowid LIMIT ?2"
            ))?;
            let mapped = statement.query_map(params![last_rowid, MIGRATION_BATCH_SIZE], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            mapped.collect::<Result<Vec<_>, rusqlite::Error>>()?
        };
        if rows.is_empty() {
            break;
        }
        for (rowid, agent_config) in rows {
            last_rowid = rowid;
            if let Some(updated) = rewrite_agent_config_json(&agent_config) {
                conn.execute(
                    &format!("UPDATE {table} SET agent_config = ?1 WHERE rowid = ?2"),
                    params![updated, rowid],
                )?;
            }
        }
    }

    Ok(())
}

/// The ensure pass runs on every startup for versions in range, so it only
/// checks tool settings and workflow config columns. Transcript and event
/// scans live in `apply`, which runs once when a database crosses version 22.
pub(crate) fn ensure_v22_data(conn: &Connection) -> Result<(), StoreError> {
    ensure_agent_tool_columns(conn)?;
    ensure_agent_config_columns(conn, "workflows")?;
    ensure_agent_config_columns(conn, "workflow_automations")?;
    Ok(())
}

/// Moves a legacy path glob out of `pattern` and into `glob`.
///
/// The merged grep tool keeps its path filter in `glob` and selects path-only
/// mode when `pattern` is absent, so a legacy `{"pattern": <glob>}` argument
/// object must become `{"glob": <glob>}`. Old calls could omit `path`, while
/// the merged tool requires it, so those calls get the old default of `"."`.
/// An existing `glob` key means the row is already migrated.
fn move_pattern_to_glob(value: &mut Value) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    if object.contains_key(LEGACY_GLOB_TOOL) {
        return false;
    }
    let Some(pattern) = object.remove("pattern") else {
        return false;
    };
    object.insert(LEGACY_GLOB_TOOL.to_string(), pattern);
    object.entry("path").or_insert_with(|| Value::from("."));
    true
}

fn move_arguments_to_glob(value: &mut Value) -> bool {
    if value.is_object() {
        return move_pattern_to_glob(value);
    }
    let Some(raw) = value.as_str() else {
        return false;
    };
    let Ok(mut arguments) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    if !move_pattern_to_glob(&mut arguments) {
        return false;
    }
    let Ok(updated) = serde_json::to_string(&arguments) else {
        return false;
    };
    *value = Value::String(updated);
    true
}

/// Rewrites one `workflow_events.event_data` payload from the legacy glob tool
/// to the merged grep tool. Only the tool name and the path-filter argument are
/// touched; result content and error text are preserved.
fn rewrite_event_data(raw: &str) -> Option<String> {
    let mut value: Value = serde_json::from_str(raw).ok()?;
    let object = value.as_object_mut()?;

    if object.get("tool_name").and_then(Value::as_str) != Some(LEGACY_GLOB_TOOL) {
        return None;
    }
    object.insert("tool_name".to_string(), Value::from(GREP_TOOL));

    if let Some(arguments) = object.get_mut("arguments") {
        move_arguments_to_glob(arguments);
    }

    // `structured_content` is a rendering mirror of the call; keep its argument
    // naming consistent. `result.content` is human/LLM-facing text and is left
    // untouched.
    if let Some(structured) = object
        .get_mut("result")
        .and_then(|result| result.get_mut("structured_content"))
    {
        move_pattern_to_glob(structured);
    }

    serde_json::to_string(&value).ok()
}

/// Normalizes structured legacy tool references without touching user text.
fn rename_embedded_tool_name(value: &mut Value, is_tool_call: bool) -> bool {
    match value {
        Value::Object(object) => {
            let mut changed = false;
            let is_legacy_tool =
                object.get("tool_name").and_then(Value::as_str) == Some(LEGACY_GLOB_TOOL);
            let is_legacy_call = is_tool_call
                && object.get("name").and_then(Value::as_str) == Some(LEGACY_GLOB_TOOL);
            if is_legacy_tool || is_legacy_call {
                object.insert(
                    if is_legacy_tool { "tool_name" } else { "name" }.to_string(),
                    Value::from(GREP_TOOL),
                );
                if let Some(arguments) = object.get_mut("arguments") {
                    move_arguments_to_glob(arguments);
                }
                if let Some(structured) = object
                    .get_mut("result")
                    .and_then(|result| result.get_mut("structured_content"))
                {
                    move_pattern_to_glob(structured);
                }
                changed = true;
            }
            for (key, child) in object.iter_mut() {
                let child_is_tool_call = matches!(
                    key.as_str(),
                    "function" | "tool_call" | "tool_calls" | "pending_tool_call" | "pending_tools"
                );
                if rename_embedded_tool_name(child, child_is_tool_call) {
                    changed = true;
                }
            }
            changed
        }
        Value::Array(items) => {
            let mut changed = false;
            for item in items.iter_mut() {
                if rename_embedded_tool_name(item, is_tool_call) {
                    changed = true;
                }
            }
            changed
        }
        _ => false,
    }
}

/// Rewrites an embedded tool name in a metadata / snapshot JSON payload and
/// returns the new text when it changed. Unparsable rows are left alone.
fn rewrite_embedded_tool_names(raw: &str) -> Option<String> {
    let mut value: Value = serde_json::from_str(raw).ok()?;
    if !rename_embedded_tool_name(&mut value, false) {
        return None;
    }
    serde_json::to_string(&value).ok()
}

fn migrate_workflow_events(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let mut last_id = 0_i64;
    loop {
        let rows = {
            let mut statement = tx.prepare(
                "SELECT id, event_data FROM workflow_events
                 WHERE id > ?1 AND event_data LIKE '%glob%'
                 ORDER BY id LIMIT ?2",
            )?;
            let mapped = statement.query_map(params![last_id, MIGRATION_BATCH_SIZE], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            mapped.collect::<Result<Vec<_>, rusqlite::Error>>()?
        };
        if rows.is_empty() {
            break;
        }
        for (id, event_data) in rows {
            last_id = id;
            if let Some(updated) = rewrite_event_data(&event_data) {
                tx.execute(
                    "UPDATE workflow_events SET event_data = ?1 WHERE id = ?2",
                    params![updated, id],
                )?;
            }
        }
    }

    Ok(())
}

fn migrate_metadata_column(tx: &Transaction<'_>, table: &str) -> Result<(), StoreError> {
    let mut last_id = 0_i64;
    loop {
        let rows = {
            let mut statement = tx.prepare(&format!(
                "SELECT id, metadata FROM {table}
                 WHERE id > ?1 AND metadata LIKE '%glob%'
                 ORDER BY id LIMIT ?2"
            ))?;
            let mapped = statement.query_map(params![last_id, MIGRATION_BATCH_SIZE], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            mapped.collect::<Result<Vec<_>, rusqlite::Error>>()?
        };
        if rows.is_empty() {
            break;
        }
        for (id, metadata) in rows {
            last_id = id;
            if let Some(updated) = rewrite_embedded_tool_names(&metadata) {
                tx.execute(
                    &format!("UPDATE {table} SET metadata = ?1 WHERE id = ?2"),
                    params![updated, id],
                )?;
            }
        }
    }

    Ok(())
}

fn migrate_workflow_snapshots(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let mut last_session_id = String::new();
    loop {
        let rows = {
            let mut statement = tx.prepare(
                "SELECT session_id, context_json FROM workflow_snapshots
                 WHERE session_id > ?1 AND context_json LIKE '%glob%'
                 ORDER BY session_id LIMIT ?2",
            )?;
            let mapped = statement
                .query_map(params![last_session_id, MIGRATION_BATCH_SIZE], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
            mapped.collect::<Result<Vec<_>, rusqlite::Error>>()?
        };
        if rows.is_empty() {
            break;
        }
        for (session_id, context_json) in rows {
            last_session_id = session_id.clone();
            if let Some(updated) = rewrite_embedded_tool_names(&context_json) {
                tx.execute(
                    "UPDATE workflow_snapshots SET context_json = ?1 WHERE session_id = ?2",
                    params![updated, session_id],
                )?;
            }
        }
    }

    Ok(())
}

/// One-time pass for a database crossing version 22: it walks the large
/// `workflow_events` / `workflow_messages` / `workflow_context_messages` /
/// `workflow_snapshots` tables once and never runs on a normal startup.
pub(crate) fn apply_v22_data(tx: &Transaction<'_>) -> Result<(), StoreError> {
    migrate_workflow_events(tx)?;
    migrate_metadata_column(tx, "workflow_messages")?;
    migrate_metadata_column(tx, "workflow_context_messages")?;
    migrate_workflow_snapshots(tx)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Legacy `glob` tool rows must keep rendering and replaying correctly after
    //! the tool merge: available-tool ids collapse to `grep`, argument shapes
    //! follow the merged grep contract, and a bad row never aborts the pass.

    use super::*;
    use serde_json::json;

    fn create_tables(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE agents (
                id TEXT PRIMARY KEY,
                available_tools TEXT,
                auto_approve TEXT
            );
            CREATE TABLE workflows (id TEXT PRIMARY KEY, agent_config TEXT);
            CREATE TABLE workflow_automations (id TEXT PRIMARY KEY, agent_config TEXT);
            CREATE TABLE workflow_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT,
                event_data TEXT
            );
            CREATE TABLE workflow_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                metadata TEXT
            );
            CREATE TABLE workflow_context_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                metadata TEXT
            );
            CREATE TABLE workflow_snapshots (
                session_id TEXT PRIMARY KEY,
                context_json TEXT NOT NULL
            );",
        )
        .expect("failed to create migration test tables");
    }

    fn apply_migration(conn: &mut Connection) {
        let tx = conn.transaction().expect("failed to begin transaction");
        apply_v22_data(&tx).expect("apply pass should succeed");
        tx.commit().expect("failed to commit apply pass");
    }

    fn read_json(conn: &Connection, query: &str) -> Value {
        let raw: String = conn
            .query_row(query, [], |row| row.get(0))
            .expect("failed to read migrated value");
        serde_json::from_str(&raw).expect("migrated value must stay valid JSON")
    }

    #[test]
    fn rewrites_and_dedupes_tool_id_arrays_in_small_tables() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);

        conn.execute(
            "INSERT INTO agents (id, available_tools, auto_approve)
             VALUES ('a1', '[\"bash\",\"glob\",\"grep\",\"list_dir\",\"glob\"]', '[\"glob\",\"glob\",\"edit_file\"]')",
            [],
        )
        .expect("failed to seed agent");

        let unrelated_policy = json!({"restrictPaths": true, "maxDepth": 4});
        conn.execute(
            "INSERT INTO workflows (id, agent_config) VALUES ('w1', ?1)",
            [json!({
                "availableTools": ["glob", "grep"],
                "autoApprove": ["glob"],
                "shellPolicy": unrelated_policy,
            })
            .to_string()],
        )
        .expect("failed to seed workflow");

        conn.execute(
            "INSERT INTO workflow_automations (id, agent_config) VALUES ('auto1', ?1)",
            [json!({
                "availableTools": ["glob"],
                "autoApprove": ["grep", "glob", "grep"],
            })
            .to_string()],
        )
        .expect("failed to seed workflow automation");

        ensure_v22_data(&conn).expect("ensure pass should succeed");

        assert_eq!(
            read_json(&conn, "SELECT available_tools FROM agents WHERE id = 'a1'"),
            json!(["bash", "grep", "list_dir"])
        );
        assert_eq!(
            read_json(&conn, "SELECT auto_approve FROM agents WHERE id = 'a1'"),
            json!(["grep", "edit_file"])
        );

        let workflow_config =
            read_json(&conn, "SELECT agent_config FROM workflows WHERE id = 'w1'");
        assert_eq!(workflow_config["availableTools"], json!(["grep"]));
        assert_eq!(workflow_config["autoApprove"], json!(["grep"]));
        assert_eq!(workflow_config["shellPolicy"], unrelated_policy);

        let automation_config = read_json(
            &conn,
            "SELECT agent_config FROM workflow_automations WHERE id = 'auto1'",
        );
        assert_eq!(automation_config["availableTools"], json!(["grep"]));
        assert_eq!(automation_config["autoApprove"], json!(["grep"]));
    }

    #[test]
    fn ensure_pass_is_idempotent() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);

        conn.execute(
            "INSERT INTO agents (id, available_tools, auto_approve)
             VALUES ('a1', '[\"glob\",\"grep\",\"bash\"]', '[\"glob\"]')",
            [],
        )
        .expect("failed to seed agent");
        conn.execute(
            "INSERT INTO workflows (id, agent_config) VALUES ('w1', ?1)",
            [json!({"availableTools": ["glob", "grep"], "autoApprove": ["glob"]}).to_string()],
        )
        .expect("failed to seed workflow");

        ensure_v22_data(&conn).expect("first ensure pass should succeed");
        let first_available: String = conn
            .query_row(
                "SELECT available_tools FROM agents WHERE id = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("failed to read available_tools");
        let first_config: String = conn
            .query_row(
                "SELECT agent_config FROM workflows WHERE id = 'w1'",
                [],
                |row| row.get(0),
            )
            .expect("failed to read agent_config");

        ensure_v22_data(&conn).expect("second ensure pass should succeed");

        let second_available: String = conn
            .query_row(
                "SELECT available_tools FROM agents WHERE id = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("failed to read available_tools");
        let second_config: String = conn
            .query_row(
                "SELECT agent_config FROM workflows WHERE id = 'w1'",
                [],
                |row| row.get(0),
            )
            .expect("failed to read agent_config");

        assert_eq!(first_available, second_available);
        assert_eq!(first_config, second_config);
        assert_eq!(first_available, "[\"grep\",\"bash\"]");
    }

    #[test]
    fn reshapes_glob_tool_events() {
        let mut conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);

        for (event_type, data) in [
            (
                "tool_started",
                json!({
                    "tool_call_id": "c1",
                    "tool_name": "glob",
                    "arguments": {"pattern": "**/*.rs", "path": "src"}
                }),
            ),
            (
                "tool_completed",
                json!({
                    "tool_call_id": "c1",
                    "tool_name": "glob",
                    "result": {
                        "content": "src/main.rs",
                        "structured_content": {"pattern": "**/*.rs", "path": "src", "count": 1}
                    }
                }),
            ),
            (
                "tool_failed",
                json!({
                    "tool_call_id": "c2",
                    "tool_name": "glob",
                    "error": "boom"
                }),
            ),
        ] {
            conn.execute(
                "INSERT INTO workflow_events (event_type, event_data) VALUES (?1, ?2)",
                params![event_type, data.to_string()],
            )
            .expect("failed to seed workflow event");
        }

        apply_migration(&mut conn);

        let started = read_json(
            &conn,
            "SELECT event_data FROM workflow_events WHERE event_type = 'tool_started'",
        );
        assert_eq!(started["tool_name"], json!("grep"));
        assert_eq!(
            started["arguments"],
            json!({"glob": "**/*.rs", "path": "src"})
        );
        assert!(started["arguments"].get("pattern").is_none());

        let completed = read_json(
            &conn,
            "SELECT event_data FROM workflow_events WHERE event_type = 'tool_completed'",
        );
        assert_eq!(completed["tool_name"], json!("grep"));
        assert_eq!(completed["result"]["content"], json!("src/main.rs"));
        assert_eq!(
            completed["result"]["structured_content"],
            json!({"glob": "**/*.rs", "path": "src", "count": 1})
        );

        let failed = read_json(
            &conn,
            "SELECT event_data FROM workflow_events WHERE event_type = 'tool_failed'",
        );
        assert_eq!(failed["tool_name"], json!("grep"));
        assert_eq!(failed["error"], json!("boom"));

        // Re-running the apply pass must not change anything further.
        apply_migration(&mut conn);
        let started_again = read_json(
            &conn,
            "SELECT event_data FROM workflow_events WHERE event_type = 'tool_started'",
        );
        assert_eq!(started, started_again);
    }

    #[test]
    fn leaves_grep_events_and_unrelated_names_unchanged() {
        let mut conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);
        let grep_event = json!({
            "tool_name": "grep",
            "arguments": {"pattern": "glob", "glob": "*.rs"}
        });
        conn.execute(
            "INSERT INTO workflow_events (event_type, event_data) VALUES ('tool_started', ?1)",
            [grep_event.to_string()],
        )
        .expect("failed to seed grep event");
        let metadata = json!({
            "display": {"name": "glob"},
            "tool_call": {"function": {"name": "glob", "arguments": "{\"pattern\":\"*.rs\",\"path\":\"src\"}"}}
        });
        conn.execute(
            "INSERT INTO workflow_messages (metadata) VALUES (?1)",
            [metadata.to_string()],
        )
        .expect("failed to seed metadata");

        apply_migration(&mut conn);

        let event = read_json(&conn, "SELECT event_data FROM workflow_events WHERE id = 1");
        assert_eq!(event, grep_event);
        let migrated = read_json(&conn, "SELECT metadata FROM workflow_messages WHERE id = 1");
        assert_eq!(migrated["display"]["name"], "glob");
        assert_eq!(migrated["tool_call"]["function"]["name"], "grep");
        let arguments: Value = serde_json::from_str(
            migrated["tool_call"]["function"]["arguments"]
                .as_str()
                .expect("arguments must stay serialized JSON"),
        )
        .expect("arguments must remain valid JSON");
        assert_eq!(arguments, json!({"glob": "*.rs", "path": "src"}));
    }

    #[test]
    fn migrates_past_a_batch_boundary() {
        let mut conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);
        let event = json!({
            "tool_name": "glob",
            "arguments": {"pattern": "*.rs", "path": "src"}
        })
        .to_string();
        for _ in 0..=MIGRATION_BATCH_SIZE {
            conn.execute(
                "INSERT INTO workflow_events (event_type, event_data) VALUES ('tool_started', ?1)",
                [&event],
            )
            .expect("failed to seed event");
        }

        apply_migration(&mut conn);

        let last = read_json(
            &conn,
            "SELECT event_data FROM workflow_events ORDER BY id DESC LIMIT 1",
        );
        assert_eq!(last["tool_name"], "grep");
        assert_eq!(last["arguments"]["glob"], "*.rs");
    }

    #[test]
    fn old_glob_default_path_survives_in_pending_calls() {
        let migrated = rewrite_embedded_tool_names(
            &json!({
                "tool_call": {
                    "function": {
                        "name": "glob",
                        "arguments": "{\"pattern\":\"**/*.rs\"}"
                    }
                }
            })
            .to_string(),
        )
        .expect("legacy call should change");
        let value: Value = serde_json::from_str(&migrated).expect("metadata should remain JSON");
        let arguments: Value = serde_json::from_str(
            value["tool_call"]["function"]["arguments"]
                .as_str()
                .expect("arguments should remain serialized JSON"),
        )
        .expect("arguments should remain JSON");
        assert_eq!(arguments, json!({"glob": "**/*.rs", "path": "."}));
    }

    #[test]
    fn renames_embedded_glob_tool_names() {
        let mut conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);

        conn.execute(
            "INSERT INTO workflow_messages (metadata) VALUES (?1)",
            [json!({"tool_call": {"function": {"name": "glob", "arguments": "{}"}}}).to_string()],
        )
        .expect("failed to seed workflow message");
        conn.execute(
            "INSERT INTO workflow_messages (metadata) VALUES (?1)",
            [json!({"tool_calls": [{"function": {"name": "glob"}}]}).to_string()],
        )
        .expect("failed to seed workflow message");
        conn.execute(
            "INSERT INTO workflow_context_messages (metadata) VALUES (?1)",
            [json!({"tool_name": "glob", "tool_call_id": "c1"}).to_string()],
        )
        .expect("failed to seed context message");
        conn.execute(
            "INSERT INTO workflow_snapshots (session_id, context_json) VALUES ('s1', ?1)",
            [json!({
                "pending_tool_call": {"function": {"name": "glob"}},
                "messages": [{"metadata": {"tool_name": "glob"}}]
            })
            .to_string()],
        )
        .expect("failed to seed snapshot");

        apply_migration(&mut conn);

        let tool_call = read_json(&conn, "SELECT metadata FROM workflow_messages WHERE id = 1");
        assert_eq!(tool_call["tool_call"]["function"]["name"], json!("grep"));
        assert_eq!(tool_call["tool_call"]["function"]["arguments"], json!("{}"));

        let tool_calls = read_json(&conn, "SELECT metadata FROM workflow_messages WHERE id = 2");
        assert_eq!(
            tool_calls["tool_calls"][0]["function"]["name"],
            json!("grep")
        );

        let context = read_json(
            &conn,
            "SELECT metadata FROM workflow_context_messages WHERE id = 1",
        );
        assert_eq!(context["tool_name"], json!("grep"));
        assert_eq!(context["tool_call_id"], json!("c1"));

        let snapshot = read_json(
            &conn,
            "SELECT context_json FROM workflow_snapshots WHERE session_id = 's1'",
        );
        assert_eq!(
            snapshot["pending_tool_call"]["function"]["name"],
            json!("grep")
        );
        assert_eq!(
            snapshot["messages"][0]["metadata"]["tool_name"],
            json!("grep")
        );
    }

    #[test]
    fn leaves_unparsable_rows_untouched() {
        let mut conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);

        let bad_metadata = "not json { glob";
        let bad_event = "{invalid glob";
        let bad_config = "[\"glob\"";

        conn.execute(
            "INSERT INTO agents (id, available_tools, auto_approve) VALUES ('a1', ?1, NULL)",
            [bad_config],
        )
        .expect("failed to seed agent");
        conn.execute(
            "INSERT INTO workflow_messages (metadata) VALUES (?1)",
            [bad_metadata],
        )
        .expect("failed to seed workflow message");
        conn.execute(
            "INSERT INTO workflow_events (event_type, event_data) VALUES ('tool_started', ?1)",
            [bad_event],
        )
        .expect("failed to seed workflow event");

        ensure_v22_data(&conn).expect("ensure must tolerate a bad row");
        apply_migration(&mut conn);

        let stored_config: String = conn
            .query_row(
                "SELECT available_tools FROM agents WHERE id = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("failed to read available_tools");
        let stored_metadata: String = conn
            .query_row(
                "SELECT metadata FROM workflow_messages WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("failed to read metadata");
        let stored_event: String = conn
            .query_row(
                "SELECT event_data FROM workflow_events WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("failed to read event_data");

        assert_eq!(stored_config, bad_config);
        assert_eq!(stored_metadata, bad_metadata);
        assert_eq!(stored_event, bad_event);
    }
}

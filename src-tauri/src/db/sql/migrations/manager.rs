use crate::db::sql::migrations::{
    common::MigrationDefinition, v1, v10, v11, v12, v13, v14, v15, v16, v17, v18, v19, v2, v20,
    v21, v3, v4, v5, v6, v7, v8, v9,
};
use crate::db::StoreError;
use rusqlite::Connection;

// Register all migrations with their corresponding SQL.
const MIGRATIONS: &[MigrationDefinition] = &[
    v2::MIGRATION,
    v3::MIGRATION,
    v4::MIGRATION,
    v5::MIGRATION,
    v6::MIGRATION,
    v7::MIGRATION,
    v8::MIGRATION,
    v9::MIGRATION,
    v10::MIGRATION,
    v11::MIGRATION,
    v12::MIGRATION,
    v13::MIGRATION,
    v14::MIGRATION,
    v15::MIGRATION,
    v16::MIGRATION,
    v17::MIGRATION,
    v18::MIGRATION,
    v19::MIGRATION,
    v20::MIGRATION,
    v21::MIGRATION,
];

fn latest_migration_version() -> i32 {
    MIGRATIONS.last().map_or(1, |migration| migration.version)
}

fn latest_schema_statements() -> Vec<(&'static str, &'static str)> {
    let mut statements = Vec::new();
    statements.extend_from_slice(v1::INIT_SQL);
    for migration in MIGRATIONS {
        statements.extend_from_slice(migration.sql);
    }
    statements
}

/// Executes a given set of SQL statements within a transaction and updates the db version.
fn execute_migration_statements<I>(
    conn: &mut Connection,
    sql_statements: I,
    version: i32,
) -> Result<(), StoreError>
where
    I: IntoIterator<Item = (&'static str, &'static str)>,
{
    let tx = conn.transaction()?;
    for (_name, sql) in sql_statements {
        tx.execute(sql, [])?;
    }

    // Insert or replace the database version.
    tx.execute(
        "INSERT OR REPLACE INTO db_version (version) VALUES (?1)",
        [version],
    )?;

    tx.commit()?;
    Ok(())
}

fn execute_migration(
    conn: &mut Connection,
    migration: &MigrationDefinition,
) -> Result<(), StoreError> {
    let tx = conn.transaction()?;
    for (_name, sql) in migration.sql {
        tx.execute(sql, [])?;
    }
    if let Some(apply) = migration.apply {
        apply(&tx)?;
    }
    tx.execute(
        "INSERT OR REPLACE INTO db_version (version) VALUES (?1)",
        [migration.version],
    )?;
    tx.commit()?;
    Ok(())
}

fn run_post_migration_ensures(conn: &Connection, current_version: i32) -> Result<(), StoreError> {
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version <= current_version)
    {
        if let Some(ensure) = migration.ensure {
            ensure(conn)?;
        }
    }

    Ok(())
}

/// Gets the current database version
pub fn get_db_version(conn: &Connection) -> Result<i32, StoreError> {
    let result: Result<i32, rusqlite::Error> = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM db_version",
        [],
        |row| row.get(0),
    );

    match result {
        Ok(v) => Ok(v),
        Err(e) => {
            if e.to_string().contains("no such table: db_version") {
                Ok(0)
            } else {
                Err(StoreError::from(e))
            }
        }
    }
}

/// Runs all necessary migrations to update the database to the latest version
pub fn run_migrations(conn: &mut Connection) -> Result<(), StoreError> {
    let mut current_version = get_db_version(conn)?;
    let latest_version = latest_migration_version();

    // A brand-new database should install the latest schema in one pass instead of
    // replaying every historical migration step.
    if current_version < 1 {
        log::info!(
            "Database not initialized. Installing latest schema at version {}...",
            latest_version
        );
        execute_migration_statements(conn, latest_schema_statements(), latest_version)?;
        current_version = get_db_version(conn)?;
        log::info!(
            "Fresh database installation complete at v{}.",
            current_version
        );

        run_post_migration_ensures(conn, current_version)?;
        return Ok(());
    }

    if current_version >= latest_version {
        log::info!(
            "Database is already up to date at version {}.",
            current_version
        );
        run_post_migration_ensures(conn, current_version)?;
        return Ok(());
    }

    log::info!(
        "Current DB version: {}. Checking for pending migrations...",
        current_version
    );

    // Filter out all migrations that need to be executed and sort them by version.
    let mut pending_migrations: Vec<&MigrationDefinition> = MIGRATIONS
        .iter()
        .filter(|m| m.version > current_version)
        .collect();
    pending_migrations.sort_by_key(|m| m.version);

    // Execute all pending migrations in order.
    for migration in pending_migrations {
        log::info!(
            "Applying migration version {}... ({})",
            migration.version,
            migration.description
        );
        execute_migration(conn, migration)?;
        log::info!(
            "Successfully applied migration version {}.",
            migration.version
        );
    }

    let final_version = get_db_version(conn)?;
    run_post_migration_ensures(conn, final_version)?;
    log::info!(
        "All migrations applied. Database is now at version {}.",
        final_version
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_exists(conn: &Connection, table_name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table_name],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count == 1)
        .expect("failed to query sqlite_master for table existence")
    }

    fn has_column(conn: &Connection, table_name: &str, column_name: &str) -> bool {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({})", table_name))
            .expect("failed to prepare pragma table_info");
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("failed to read pragma table_info rows");

        for column in columns {
            if column.expect("failed to read column name") == column_name {
                return true;
            }
        }

        false
    }

    #[test]
    fn fresh_install_builds_latest_schema_directly() {
        let mut conn = Connection::open_in_memory().expect("failed to open sqlite connection");

        run_migrations(&mut conn).expect("fresh install migrations should succeed");

        assert_eq!(
            get_db_version(&conn).expect("db version should be readable"),
            latest_migration_version()
        );
        assert!(table_exists(&conn, "agents"));
        assert!(table_exists(&conn, "workflows"));
        assert!(table_exists(&conn, "workflow_events"));
        assert!(table_exists(&conn, "memory_candidates"));
        assert!(has_column(&conn, "ccproxy_stats", "provider_id"));
        assert!(has_column(&conn, "agents", "mcp_tool_exposure"));
        assert!(has_column(&conn, "agents", "sub_agent_role"));
        assert!(has_column(&conn, "agents", "sandbox_execution_mode"));
        assert!(has_column(&conn, "agents", "sandbox_scheme_id"));
        assert!(has_column(&conn, "agents", "personality"));
        assert!(table_exists(&conn, "sandbox_schemes"));
        assert!(has_column(
            &conn,
            "experiment_campaign_schedules",
            "profile_hash"
        ));
        assert!(has_column(
            &conn,
            "experiment_campaign_jobs",
            "profile_hash"
        ));
        assert!(table_exists(&conn, "capability_operations"));
        assert!(table_exists(&conn, "capability_operation_effects"));
        assert!(table_exists(&conn, "skill_installations"));

        let recorded_versions: i64 = conn
            .query_row("SELECT COUNT(1) FROM db_version", [], |row| row.get(0))
            .expect("failed to count db_version rows");
        assert_eq!(
            recorded_versions, 1,
            "fresh installs should record only the latest schema version"
        );
    }

    #[test]
    fn migration_apply_failure_rolls_back_schema_changes_and_version_marker() {
        fn fail_after_schema_change(tx: &rusqlite::Transaction<'_>) -> Result<(), StoreError> {
            tx.execute(
                "CREATE TABLE migration_apply_should_rollback (id INTEGER)",
                [],
            )?;
            Err(StoreError::InvalidData(
                "intentional migration failure".to_string(),
            ))
        }

        const MIGRATION_SQL: &[(&str, &str)] = &[(
            "migration_sql_should_rollback",
            "CREATE TABLE migration_sql_should_rollback (id INTEGER)",
        )];
        let migration = MigrationDefinition {
            version: 14,
            description: "test migration",
            sql: MIGRATION_SQL,
            ensure: None,
            apply: Some(fail_after_schema_change),
        };
        let mut conn = Connection::open_in_memory().expect("failed to open sqlite connection");
        conn.execute("CREATE TABLE db_version (version INTEGER PRIMARY KEY)", [])
            .expect("failed to create db version fixture");

        let error = execute_migration(&mut conn, &migration)
            .expect_err("failing migration apply should roll back");
        assert!(error.to_string().contains("intentional migration failure"));
        assert_eq!(
            get_db_version(&conn).expect("db version should be readable"),
            0,
            "failed migration must not record its version"
        );
        assert!(
            !table_exists(&conn, "migration_sql_should_rollback"),
            "migration SQL should roll back with the failed apply hook"
        );
        assert!(
            !table_exists(&conn, "migration_apply_should_rollback"),
            "apply-hook SQL should roll back with the failed migration"
        );
    }

    #[test]
    fn existing_database_upgrades_incrementally() {
        let mut conn = Connection::open_in_memory().expect("failed to open sqlite connection");

        execute_migration_statements(&mut conn, v1::INIT_SQL.iter().copied(), 1)
            .expect("v1 bootstrap should succeed");
        assert_eq!(
            get_db_version(&conn).expect("db version should be readable"),
            1
        );

        run_migrations(&mut conn).expect("incremental migrations should succeed");

        assert_eq!(
            get_db_version(&conn).expect("db version should be readable"),
            latest_migration_version()
        );
        assert!(table_exists(&conn, "agents"));
        assert!(table_exists(&conn, "ccproxy_stats"));
        assert!(table_exists(&conn, "workflows"));
        assert!(table_exists(&conn, "workflow_context_messages"));
        assert!(has_column(&conn, "ccproxy_stats", "provider_id"));
        assert!(has_column(&conn, "agents", "sub_agent_role"));
        assert!(has_column(&conn, "agents", "sandbox_execution_mode"));
        assert!(has_column(&conn, "agents", "sandbox_scheme_id"));
        assert!(has_column(&conn, "agents", "personality"));
        assert!(table_exists(&conn, "sandbox_schemes"));
        assert!(table_exists(&conn, "capability_operations"));
        assert!(table_exists(&conn, "capability_operation_effects"));
        assert!(table_exists(&conn, "skill_installations"));

        let has_v3_marker: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM db_version WHERE version = 3",
                [],
                |row| row.get(0),
            )
            .expect("failed to query placeholder migration marker");
        assert_eq!(
            has_v3_marker, 1,
            "placeholder migrations should still advance db_version"
        );
    }

    /// Builds a database at exactly `version` by replaying the historical
    /// statements, which is how the pre-v18 shape is reproduced for the
    /// consolidated CLI upgrade test.
    fn build_at_version(conn: &mut Connection, version: i32) {
        let mut statements: Vec<(&'static str, &'static str)> = Vec::new();
        statements.extend_from_slice(v1::INIT_SQL);
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= version)
        {
            statements.extend_from_slice(migration.sql);
        }
        execute_migration_statements(conn, statements, version)
            .expect("historical schema should build");
    }

    /// The v18 CLI migration is consolidated and therefore upgrades an older
    /// v17 database directly to the complete CLI schema, and every later
    /// additive migration applies on top of it.
    #[test]
    fn v17_database_upgrades_to_v18_without_touching_existing_rows() {
        let mut conn = Connection::open_in_memory().expect("failed to open sqlite connection");
        build_at_version(&mut conn, 17);
        assert_eq!(get_db_version(&conn).expect("version"), 17);
        assert!(!table_exists(&conn, "experiment_campaign_jobs"));
        assert!(!table_exists(&conn, "experiment_domain"));

        conn.execute(
            "INSERT INTO agents (id, name, system_prompt, created_at, updated_at)
             VALUES ('agent-1', 'kept', 'prompt', '0', '0')",
            [],
        )
        .expect("seed an existing row");

        run_migrations(&mut conn).expect("v17 -> latest should succeed");

        // The assertion tracks the latest registered migration instead of a
        // literal, so adding an additive migration does not make this
        // consolidated-upgrade test stale.
        assert_eq!(
            get_db_version(&conn).expect("version"),
            latest_migration_version()
        );
        // The pre-existing row survived untouched.
        let name: String = conn
            .query_row("SELECT name FROM agents WHERE id = 'agent-1'", [], |row| {
                row.get(0)
            })
            .expect("existing row survives");
        assert_eq!(name, "kept");
        for table in [
            "experiment_domain",
            "experiment_domain_lease",
            "experiment_campaign_schedules",
            "experiment_campaign_jobs",
            "experiment_job_journal",
            "experiment_job_bundles",
            "experiment_job_artifacts",
            "experiment_promotions",
            "experiment_promotion_journal",
            "experiment_promotion_canary_results",
        ] {
            assert!(
                table_exists(&conn, table),
                "missing consolidated v18 table {table}"
            );
        }
        assert!(has_column(
            &conn,
            "experiment_campaign_schedules",
            "profile_hash"
        ));
        assert!(has_column(
            &conn,
            "experiment_campaign_jobs",
            "profile_hash"
        ));
        let markers: i64 = conn
            .query_row("SELECT COUNT(1) FROM experiment_domain", [], |row| {
                row.get(0)
            })
            .expect("count markers");
        assert_eq!(markers, 0, "an upgraded database is never auto-marked");
    }

    /// The v20 capability journal is additive: a v19 database gains the new
    /// tables without losing or rewriting existing rows.
    #[test]
    fn v19_database_upgrades_to_v20_without_touching_existing_rows() {
        let mut conn = Connection::open_in_memory().expect("failed to open sqlite connection");
        build_at_version(&mut conn, 19);
        assert_eq!(get_db_version(&conn).expect("version"), 19);
        assert!(!table_exists(&conn, "capability_operations"));

        conn.execute(
            "INSERT INTO agents (id, name, system_prompt, created_at, updated_at)
             VALUES ('agent-v20', 'kept', 'prompt', '0', '0')",
            [],
        )
        .expect("seed an existing row");

        run_migrations(&mut conn).expect("v19 -> v20 should succeed");

        assert_eq!(
            get_db_version(&conn).expect("version"),
            latest_migration_version(),
            "v19 upgrades to the current head"
        );
        let name: String = conn
            .query_row("SELECT name FROM agents WHERE id = 'agent-v20'", [], |row| {
                row.get(0)
            })
            .expect("existing row survives");
        assert_eq!(name, "kept");
        assert!(table_exists(&conn, "capability_operations"));
        assert!(table_exists(&conn, "capability_operation_effects"));
        assert!(table_exists(&conn, "skill_installations"));
        assert!(has_column(&conn, "capability_operations", "request_hash"));
        assert!(has_column(&conn, "capability_operations", "reconcile_reason"));
        assert!(has_column(&conn, "skill_installations", "marker_nonce"));
    }

    /// A database that records a version *ahead* of the newest migration still
    /// gains the capability journal.
    ///
    /// The CLI experiment migrations were consolidated, so a database created by
    /// an older build can already record a higher number than anything this tree
    /// knows about. `run_migrations` then skips every step, so an additive
    /// migration that only ships `sql` would silently never run and the
    /// capability service would fail on an otherwise healthy database. The
    /// idempotent `ensure` hook is what makes that case work.
    #[test]
    fn a_database_recorded_ahead_of_the_latest_version_still_gets_the_capability_journal() {
        let mut conn = Connection::open_in_memory().expect("failed to open sqlite connection");
        build_at_version(&mut conn, 19);
        let ahead = latest_migration_version() + 1;
        conn.execute("INSERT INTO db_version (version) VALUES (?1)", [ahead])
            .expect("record a consolidated ahead version");
        assert_eq!(get_db_version(&conn).expect("version"), ahead);
        assert!(
            ahead > latest_migration_version(),
            "the seeded version must be ahead of the current head"
        );
        assert!(!table_exists(&conn, "capability_operations"));

        run_migrations(&mut conn)
            .expect("an ahead-of-latest database stays usable");

        assert_eq!(
            get_db_version(&conn).expect("version"),
            ahead,
            "a newer recorded version is never downgraded or rewritten"
        );
        assert!(table_exists(&conn, "capability_operations"));
        assert!(table_exists(&conn, "capability_operation_effects"));
        assert!(table_exists(&conn, "skill_installations"));

        // Running it twice is still a no-op, because startup calls this path on
        // every launch and the journal must keep its rows.
        conn.execute(
            "INSERT INTO skill_installations (installation_id, skill_name, target_id,
                install_path, source_kind, source_ref, checker_version, verdict,
                content_digest, file_manifest_json, marker_nonce, manifest_digest,
                state, operation_id, created_at_ms, updated_at_ms)
             VALUES ('ins-keep', 'demo', 'chatspeed', '/x', 'local_directory',
                'local_directory:x', 'skill-checker.v1', 'pass', 'digest', '[]',
                'nonce', 'manifest', 'installed', 'op-1', 0, 0)",
            [],
        )
        .expect("seed an ownership row");
        run_migrations(&mut conn).expect("a second startup still succeeds");
        let kept: i64 = conn
            .query_row("SELECT COUNT(1) FROM skill_installations", [], |row| row.get(0))
            .expect("count ownership rows");
        assert_eq!(kept, 1, "the ensure hook never rewrites existing rows");
    }
}

use super::common::MigrationDefinition;
use crate::db::StoreError;
use rusqlite::Connection;

/// Phase 3 capability-management schema (Agent Skills + MCP).
///
/// This is the durable authority for every capability mutation. One
/// `capability_operations` row records the mutation envelope (idempotency
/// scope, canonical request hash, structured state and redacted result), one
/// `capability_operation_effects` row records the intent and observation of a
/// single externally visible effect, and one `skill_installations` row records
/// the ownership proof of an installed Agent Skill.
///
/// The migration is additive only: it creates new tables and indexes and never
/// touches an existing table or row. Only redacted projections are persisted,
/// so the journal can never become a second copy of a secret (AC-13/INV-6).
pub const MIGRATION_SQL: &[(&str, &str)] = &[
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
            state TEXT NOT NULL CHECK (state IN (
                'planned','staging','checking','applying',
                'completed','blocked','failed','needs_reconcile'
            )),
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
        "CREATE INDEX IF NOT EXISTS capability_operations_state_idx
            ON capability_operations (capability, state, created_at_ms)",
    ),
    (
        "capability_operations_resource_idx",
        "CREATE INDEX IF NOT EXISTS capability_operations_resource_idx
            ON capability_operations (capability, resource_key, created_at_ms)",
    ),
    (
        "capability_operation_effects",
        "CREATE TABLE IF NOT EXISTS capability_operation_effects (
            effect_id TEXT PRIMARY KEY,
            operation_id TEXT NOT NULL REFERENCES capability_operations(operation_id),
            effect_key TEXT NOT NULL,
            target TEXT,
            intent TEXT NOT NULL CHECK (intent IN (
                'not_started','intent_recorded','completed'
            )),
            outcome TEXT NOT NULL CHECK (outcome IN (
                'pending','applied','skipped','blocked','failed','unknown'
            )),
            detail_json TEXT,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            UNIQUE (operation_id, effect_key)
        )",
    ),
    (
        "capability_operation_effects_outcome_idx",
        "CREATE INDEX IF NOT EXISTS capability_operation_effects_outcome_idx
            ON capability_operation_effects (operation_id, outcome)",
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
            state TEXT NOT NULL CHECK (state IN (
                'installing','installed','quarantined','removed','drifted'
            )),
            operation_id TEXT,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            UNIQUE (target_id, skill_name)
        )",
    ),
    (
        "skill_installations_state_idx",
        "CREATE INDEX IF NOT EXISTS skill_installations_state_idx
            ON skill_installations (state, target_id)",
    ),
];

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 20,
    description: "v20 migration: Add Phase 3 capability operation, effect and Skill ownership journal",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_capability_journal),
    apply: None,
};

/// Re-applies the capability journal schema on every startup.
///
/// Earlier CLI experiment migrations were consolidated into an existing step, so
/// a database created by that build can already record a version *higher* than
/// this one. `run_migrations` then skips this step entirely, and without this
/// hook the journal tables would be missing while the capability service is
/// running — every mutation would fail on a perfectly healthy database.
///
/// Every statement is `IF NOT EXISTS`, so re-applying the whole list is a no-op
/// once the schema exists; nothing is seeded and nothing is rewritten.
fn ensure_capability_journal(conn: &Connection) -> Result<(), StoreError> {
    for (name, sql) in MIGRATION_SQL {
        conn.execute(sql, [])
            .map_err(|e| StoreError::Query(format!("v20 ensure {name} failed: {e}")))?;
    }
    Ok(())
}

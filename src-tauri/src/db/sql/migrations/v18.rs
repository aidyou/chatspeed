use super::common::MigrationDefinition;

/// Phase 2B budget ledger schema. All externally visible identifiers are
/// opaque strings; every balance/counter is a non-negative SQLite INTEGER
/// column so admission checks are conditional integer updates, never JSON
/// arithmetic. `cap_*` columns are NULL when a dimension is explicitly
/// not applicable; committed/reserved counters default to zero.
pub const MIGRATION_SQL: &[(&str, &str)] = &[
    (
        "experiment_budget_scopes",
        "CREATE TABLE IF NOT EXISTS experiment_budget_scopes (
            scope_id TEXT PRIMARY KEY,
            scope_kind TEXT NOT NULL CHECK (scope_kind IN ('request','trial','candidate','campaign')),
            parent_scope_id TEXT REFERENCES experiment_budget_scopes(scope_id),
            status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','paused','closed')),
            currency_code TEXT,
            cap_input_tokens INTEGER,
            cap_output_tokens INTEGER,
            cap_cache_read_tokens INTEGER,
            cap_cache_write_tokens INTEGER,
            cap_wall_time_ms INTEGER,
            cap_tool_calls INTEGER,
            cap_processes INTEGER,
            cap_disk_bytes INTEGER,
            cap_network_bytes INTEGER,
            cap_concurrency INTEGER,
            cap_money_micros INTEGER,
            committed_input_tokens INTEGER NOT NULL DEFAULT 0 CHECK (committed_input_tokens >= 0),
            committed_output_tokens INTEGER NOT NULL DEFAULT 0 CHECK (committed_output_tokens >= 0),
            committed_cache_read_tokens INTEGER NOT NULL DEFAULT 0 CHECK (committed_cache_read_tokens >= 0),
            committed_cache_write_tokens INTEGER NOT NULL DEFAULT 0 CHECK (committed_cache_write_tokens >= 0),
            committed_wall_time_ms INTEGER NOT NULL DEFAULT 0 CHECK (committed_wall_time_ms >= 0),
            committed_tool_calls INTEGER NOT NULL DEFAULT 0 CHECK (committed_tool_calls >= 0),
            committed_processes INTEGER NOT NULL DEFAULT 0 CHECK (committed_processes >= 0),
            committed_disk_bytes INTEGER NOT NULL DEFAULT 0 CHECK (committed_disk_bytes >= 0),
            committed_network_bytes INTEGER NOT NULL DEFAULT 0 CHECK (committed_network_bytes >= 0),
            committed_concurrency INTEGER NOT NULL DEFAULT 0 CHECK (committed_concurrency >= 0),
            committed_money_micros INTEGER NOT NULL DEFAULT 0 CHECK (committed_money_micros >= 0),
            reserved_input_tokens INTEGER NOT NULL DEFAULT 0 CHECK (reserved_input_tokens >= 0),
            reserved_output_tokens INTEGER NOT NULL DEFAULT 0 CHECK (reserved_output_tokens >= 0),
            reserved_cache_read_tokens INTEGER NOT NULL DEFAULT 0 CHECK (reserved_cache_read_tokens >= 0),
            reserved_cache_write_tokens INTEGER NOT NULL DEFAULT 0 CHECK (reserved_cache_write_tokens >= 0),
            reserved_wall_time_ms INTEGER NOT NULL DEFAULT 0 CHECK (reserved_wall_time_ms >= 0),
            reserved_tool_calls INTEGER NOT NULL DEFAULT 0 CHECK (reserved_tool_calls >= 0),
            reserved_processes INTEGER NOT NULL DEFAULT 0 CHECK (reserved_processes >= 0),
            reserved_disk_bytes INTEGER NOT NULL DEFAULT 0 CHECK (reserved_disk_bytes >= 0),
            reserved_network_bytes INTEGER NOT NULL DEFAULT 0 CHECK (reserved_network_bytes >= 0),
            reserved_concurrency INTEGER NOT NULL DEFAULT 0 CHECK (reserved_concurrency >= 0),
            reserved_money_micros INTEGER NOT NULL DEFAULT 0 CHECK (reserved_money_micros >= 0),
            infra_failure_count INTEGER NOT NULL DEFAULT 0 CHECK (infra_failure_count >= 0),
            infra_failure_threshold INTEGER NOT NULL DEFAULT 0,
            pause_reason TEXT,
            envelope_json TEXT NOT NULL,
            version INTEGER NOT NULL DEFAULT 0,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            CHECK (
                (scope_kind = 'campaign' AND parent_scope_id IS NULL)
                OR (scope_kind != 'campaign' AND parent_scope_id IS NOT NULL)
            )
        )",
    ),
    (
        "experiment_budget_reservations",
        "CREATE TABLE IF NOT EXISTS experiment_budget_reservations (
            reservation_id TEXT PRIMARY KEY,
            effect_id TEXT NOT NULL,
            idempotency_key TEXT NOT NULL UNIQUE,
            request_scope_id TEXT NOT NULL REFERENCES experiment_budget_scopes(scope_id),
            trial_scope_id TEXT NOT NULL REFERENCES experiment_budget_scopes(scope_id),
            candidate_scope_id TEXT NOT NULL REFERENCES experiment_budget_scopes(scope_id),
            campaign_scope_id TEXT NOT NULL REFERENCES experiment_budget_scopes(scope_id),
            effect_kind TEXT NOT NULL CHECK (effect_kind IN ('llm_completion','embedding','tool_call','process')),
            attempt INTEGER NOT NULL CHECK (attempt >= 1),
            state TEXT NOT NULL CHECK (state IN ('reserved','committed','released','unknown','overrun')),
            est_input_tokens INTEGER NOT NULL,
            est_output_tokens INTEGER NOT NULL,
            est_cache_read_tokens INTEGER NOT NULL,
            est_cache_write_tokens INTEGER NOT NULL,
            est_wall_time_ms INTEGER NOT NULL,
            est_tool_calls INTEGER NOT NULL,
            est_processes INTEGER NOT NULL,
            est_disk_bytes INTEGER NOT NULL,
            est_network_bytes INTEGER NOT NULL,
            est_concurrency INTEGER NOT NULL,
            est_money_micros INTEGER NOT NULL,
            actual_input_tokens INTEGER,
            actual_output_tokens INTEGER,
            actual_cache_read_tokens INTEGER,
            actual_cache_write_tokens INTEGER,
            actual_wall_time_ms INTEGER,
            actual_tool_calls INTEGER,
            actual_processes INTEGER,
            actual_disk_bytes INTEGER,
            actual_network_bytes INTEGER,
            actual_concurrency INTEGER,
            actual_money_micros INTEGER,
            pricing_snapshot_json TEXT,
            operation_id TEXT UNIQUE,
            overrun INTEGER NOT NULL DEFAULT 0,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            lease_expires_at_ms INTEGER NOT NULL
        )",
    ),
    (
        "experiment_budget_ledger_entries",
        "CREATE TABLE IF NOT EXISTS experiment_budget_ledger_entries (
            entry_id TEXT PRIMARY KEY,
            scope_id TEXT NOT NULL,
            reservation_id TEXT,
            effect_id TEXT,
            operation TEXT NOT NULL CHECK (operation IN ('reserve','commit','release','mark_unknown','infra_failure','pause','overrun')),
            vector_json TEXT NOT NULL,
            operation_id TEXT,
            idempotency_key TEXT,
            reason TEXT,
            created_at_ms INTEGER NOT NULL
        )",
    ),
    (
        "idx_budget_entries_scope",
        "CREATE INDEX IF NOT EXISTS idx_budget_entries_scope
         ON experiment_budget_ledger_entries(scope_id, created_at_ms)",
    ),
    (
        "idx_budget_entries_reservation",
        "CREATE INDEX IF NOT EXISTS idx_budget_entries_reservation
         ON experiment_budget_ledger_entries(reservation_id)",
    ),
    (
        "idx_budget_reservations_campaign_state",
        "CREATE INDEX IF NOT EXISTS idx_budget_reservations_campaign_state
         ON experiment_budget_reservations(campaign_scope_id, state)",
    ),
    (
        "idx_budget_reservations_lease",
        "CREATE INDEX IF NOT EXISTS idx_budget_reservations_lease
         ON experiment_budget_reservations(state, lease_expires_at_ms)",
    ),
];

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 18,
    description:
        "v18 migration: Add experiment budget scopes, reservations and append-only ledger entries",
    sql: MIGRATION_SQL,
    ensure: None,
    apply: None,
};

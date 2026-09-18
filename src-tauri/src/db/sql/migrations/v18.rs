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
    (
        "experiment_domain",
        "CREATE TABLE IF NOT EXISTS experiment_domain (
            domain_id TEXT PRIMARY KEY,
            domain_kind TEXT NOT NULL CHECK (domain_kind IN ('experiment.v1')),
            marker_schema_version TEXT NOT NULL,
            singleton INTEGER NOT NULL DEFAULT 1 CHECK (singleton = 1),
            created_at_ms INTEGER NOT NULL,
            UNIQUE (singleton)
        )",
    ),
    (
        "experiment_domain_lease",
        "CREATE TABLE IF NOT EXISTS experiment_domain_lease (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            owner_id TEXT NOT NULL,
            lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
            pid INTEGER NOT NULL,
            lease_expires_at_ms INTEGER NOT NULL,
            heartbeat_at_ms INTEGER NOT NULL,
            started_at_ms INTEGER NOT NULL
        )",
    ),
    (
        "experiment_campaign_schedules",
        "CREATE TABLE IF NOT EXISTS experiment_campaign_schedules (
            campaign_id TEXT PRIMARY KEY,
            campaign_key TEXT NOT NULL,
            plan_hash TEXT NOT NULL,
            schedule_hash TEXT NOT NULL,
            plan_json TEXT NOT NULL,
            fixture_refs_json TEXT NOT NULL,
            execution_profile_ref TEXT NOT NULL,
            bundle_refs_json TEXT NOT NULL,
            concurrency INTEGER NOT NULL CHECK (concurrency = 1),
            status TEXT NOT NULL CHECK (status IN ('active','closed','cancelled')),
            idempotency_key TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            profile_hash TEXT,
            UNIQUE (campaign_key),
            UNIQUE (idempotency_key)
        )",
    ),
    (
        "experiment_campaign_jobs",
        "CREATE TABLE IF NOT EXISTS experiment_campaign_jobs (
            job_id TEXT PRIMARY KEY,
            campaign_id TEXT NOT NULL REFERENCES experiment_campaign_schedules(campaign_id),
            ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
            candidate_key TEXT NOT NULL,
            task_id TEXT NOT NULL,
            suite TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            dataset_version INTEGER NOT NULL,
            split TEXT NOT NULL,
            manifest_digest TEXT NOT NULL,
            task_digest TEXT NOT NULL,
            instruction_hash TEXT NOT NULL,
            execution_profile_ref TEXT NOT NULL,
            profile_hash TEXT,
            state TEXT NOT NULL CHECK (state IN (
                'queued','preparing','prepared','dispatching','running','collecting',
                'succeeded','failed_precondition','failed','cancelled','unknown_manual'
            )),
            dispatch_marker TEXT NOT NULL CHECK (dispatch_marker IN (
                'not_dispatched','intent_recorded','confirmed'
            )),
            run_id TEXT,
            session_id TEXT,
            attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt >= 0),
            owner_id TEXT,
            lease_generation INTEGER NOT NULL DEFAULT 0 CHECK (lease_generation >= 0),
            lease_expires_at_ms INTEGER,
            heartbeat_at_ms INTEGER,
            last_stage TEXT,
            error_code TEXT,
            artifact_dir TEXT,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            UNIQUE (campaign_id, ordinal),
            CHECK (
                (dispatch_marker = 'confirmed' AND run_id IS NOT NULL)
                OR (dispatch_marker != 'confirmed' AND run_id IS NULL)
            )
        )",
    ),
    (
        "experiment_campaign_jobs_claim_idx",
        "CREATE INDEX IF NOT EXISTS experiment_campaign_jobs_claim_idx
            ON experiment_campaign_jobs (state, lease_expires_at_ms, campaign_id, ordinal)",
    ),
    (
        "experiment_campaign_jobs_campaign_idx",
        "CREATE INDEX IF NOT EXISTS experiment_campaign_jobs_campaign_idx
            ON experiment_campaign_jobs (campaign_id, ordinal)",
    ),
    (
        "experiment_job_journal",
        "CREATE TABLE IF NOT EXISTS experiment_job_journal (
            journal_id INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id TEXT NOT NULL REFERENCES experiment_campaign_jobs(job_id),
            stage TEXT NOT NULL,
            owner_id TEXT,
            lease_generation INTEGER NOT NULL CHECK (lease_generation >= 0),
            detail_json TEXT,
            created_at_ms INTEGER NOT NULL
        )",
    ),
    (
        "experiment_job_journal_job_idx",
        "CREATE INDEX IF NOT EXISTS experiment_job_journal_job_idx
            ON experiment_job_journal (job_id, journal_id)",
    ),
    (
        "experiment_job_bundles",
        "CREATE TABLE IF NOT EXISTS experiment_job_bundles (
            bundle_install_id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES experiment_campaign_jobs(job_id),
            bundle_ref TEXT NOT NULL,
            bundle_version TEXT NOT NULL,
            content_digest TEXT NOT NULL,
            staged_dir TEXT NOT NULL,
            verified INTEGER NOT NULL DEFAULT 0 CHECK (verified IN (0,1)),
            registered INTEGER NOT NULL DEFAULT 0 CHECK (registered IN (0,1)),
            owner_id TEXT,
            lease_generation INTEGER NOT NULL DEFAULT 0 CHECK (lease_generation >= 0),
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            UNIQUE (job_id, bundle_ref)
        )",
    ),
    (
        "experiment_job_artifacts",
        "CREATE TABLE IF NOT EXISTS experiment_job_artifacts (
            artifact_id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES experiment_campaign_jobs(job_id),
            kind TEXT NOT NULL CHECK (kind IN (
                'output_patch','patch_manifest','bundle_manifest','job_summary'
            )),
            relative_path TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
            base_revision TEXT,
            run_id TEXT,
            session_id TEXT,
            candidate_key TEXT,
            created_at_ms INTEGER NOT NULL,
            UNIQUE (job_id, kind)
        )",
    ),
    (
        "experiment_promotions",
        "CREATE TABLE IF NOT EXISTS experiment_promotions (
            promotion_id TEXT PRIMARY KEY,
            campaign_id TEXT NOT NULL,
            candidate_key TEXT NOT NULL,
            target_ref TEXT NOT NULL,
            state TEXT NOT NULL CHECK (state IN (
                'queued','evidence_validating','rejected','checkpointing','checkpointed',
                'canary_running','canary_failed','ready_to_advance','advancing','promoted',
                'rolled_back','unknown_manual'
            )),
            request_hash TEXT NOT NULL,
            evidence_hash TEXT NOT NULL,
            evidence_json TEXT NOT NULL,
            target_hash TEXT NOT NULL,
            policy_hash TEXT NOT NULL,
            base_revision TEXT NOT NULL,
            patch_sha256 TEXT NOT NULL,
            patch_manifest_hash TEXT NOT NULL,
            expected_old_head TEXT,
            observed_head TEXT,
            checkpoint_commit TEXT,
            checkpoint_ref TEXT,
            checkpoint_intent TEXT NOT NULL DEFAULT 'not_started' CHECK (checkpoint_intent IN (
                'not_started','intent_recorded','completed'
            )),
            branch_intent TEXT NOT NULL DEFAULT 'not_started' CHECK (branch_intent IN (
                'not_started','intent_recorded','completed'
            )),
            canary_result_hash TEXT,
            canary_result_json TEXT,
            decision_outcome TEXT,
            decision_code TEXT,
            decision_detail TEXT,
            decision_json TEXT,
            error_code TEXT,
            owner_id TEXT,
            lease_generation INTEGER NOT NULL DEFAULT 0 CHECK (lease_generation >= 0),
            lease_expires_at_ms INTEGER,
            heartbeat_at_ms INTEGER,
            attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt >= 0),
            idempotency_key TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            UNIQUE (idempotency_key)
        )",
    ),
    (
        "experiment_promotions_target_active_idx",
        "CREATE UNIQUE INDEX IF NOT EXISTS experiment_promotions_target_active_idx
            ON experiment_promotions (target_ref)
            WHERE state NOT IN (
                'rejected','canary_failed','promoted','rolled_back','unknown_manual'
            )",
    ),
    (
        "experiment_promotions_claim_idx",
        "CREATE INDEX IF NOT EXISTS experiment_promotions_claim_idx
            ON experiment_promotions (state, lease_expires_at_ms, created_at_ms)",
    ),
    (
        "experiment_promotions_campaign_idx",
        "CREATE INDEX IF NOT EXISTS experiment_promotions_campaign_idx
            ON experiment_promotions (campaign_id, candidate_key)",
    ),
    (
        "experiment_promotion_journal",
        "CREATE TABLE IF NOT EXISTS experiment_promotion_journal (
            journal_id INTEGER PRIMARY KEY AUTOINCREMENT,
            promotion_id TEXT NOT NULL REFERENCES experiment_promotions(promotion_id),
            stage TEXT NOT NULL,
            owner_id TEXT,
            lease_generation INTEGER NOT NULL CHECK (lease_generation >= 0),
            detail_json TEXT,
            created_at_ms INTEGER NOT NULL
        )",
    ),
    (
        "experiment_promotion_journal_idx",
        "CREATE INDEX IF NOT EXISTS experiment_promotion_journal_idx
            ON experiment_promotion_journal (promotion_id, journal_id)",
    ),
    (
        "experiment_promotion_canary_results",
        "CREATE TABLE IF NOT EXISTS experiment_promotion_canary_results (
            result_id TEXT PRIMARY KEY,
            promotion_id TEXT NOT NULL REFERENCES experiment_promotions(promotion_id),
            stage_index INTEGER NOT NULL CHECK (stage_index >= 0),
            stage_id TEXT NOT NULL,
            metric TEXT NOT NULL,
            samples INTEGER NOT NULL CHECK (samples >= 0),
            baseline_passed INTEGER NOT NULL CHECK (baseline_passed >= 0),
            candidate_passed INTEGER NOT NULL CHECK (candidate_passed >= 0),
            baseline_mean REAL NOT NULL,
            candidate_mean REAL NOT NULL,
            declared_status TEXT NOT NULL CHECK (declared_status IN ('pass','fail')),
            recomputed_status TEXT NOT NULL CHECK (recomputed_status IN ('pass','fail')),
            output_sha256 TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            UNIQUE (promotion_id, stage_id)
        )",
    ),
    (
        "experiment_promotion_canary_results_idx",
        "CREATE INDEX IF NOT EXISTS experiment_promotion_canary_results_idx
            ON experiment_promotion_canary_results (promotion_id, stage_index)",
    ),
];

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 18,
    description: "v18 migration: Add CLI experiment budget, schedule and promotion schemas",
    sql: MIGRATION_SQL,
    ensure: None,
    apply: None,
};

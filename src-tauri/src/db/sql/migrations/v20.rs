use super::common::MigrationDefinition;

/// Phase 2I promotion schema.
///
/// Purely additive: no existing table, column, index or row is touched, so a
/// database upgraded to v20 merely gains empty tables and a rollback to an
/// older binary keeps working (the old code ignores unknown tables). There is
/// no down migration and no automatic drop.
///
/// Identity rules (see `db/CONSTITUTION.md` / INV-9):
/// - Every externally visible identifier is an opaque TEXT id minted by the
///   backend. SQLite allocates the only auto-increment column
///   (`experiment_promotion_journal.journal_id`); no TSID or large integer is
///   ever assigned to it.
/// - Durable rows store refs, digests and typed state only: never an
///   instruction, a raw prompt/response/transcript, a token/API key, a resolved
///   secret or a raw canary stdout/stderr (INV-8). A canary's structured
///   per-stage numbers and the sha256 of its raw bytes are the most that is
///   kept.
/// - The promotion FSM is the promotion authority; the 2G job FSM is untouched,
///   so a promotion can never be inferred from, or perturb, a job row.
///
/// The single-flight rule is enforced by a *partial* unique index: at most one
/// non-terminal promotion may exist per registered target. Terminal states are
/// excluded from the index, so the same target may be promoted again later
/// while its history stays queryable.
pub const MIGRATION_SQL: &[(&str, &str)] = &[
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
    version: 20,
    description: "v20 migration: Add the Phase 2I promotion FSM, journal and canary-result tables",
    sql: MIGRATION_SQL,
    ensure: None,
    apply: None,
};

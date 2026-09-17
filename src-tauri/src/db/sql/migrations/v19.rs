use super::common::MigrationDefinition;

/// Phase 2G+2H experiment-domain and durable campaign-schedule schema.
///
/// Purely additive: no existing table, column, index or row is touched, so a
/// desktop database upgraded to v19 merely gains empty tables and a rollback to
/// an older binary keeps working (the old code ignores unknown tables). There is
/// no down migration and no automatic drop.
///
/// Identity rules (see `db/CONSTITUTION.md` / INV-9):
/// - Every externally visible identifier is an opaque TEXT id minted by the
///   backend. SQLite allocates the only auto-increment column
///   (`experiment_job_journal.journal_id`); no TSID or large integer is ever
///   assigned to it.
/// - Durable rows store refs, digests and typed state only: never an
///   instruction, a raw prompt/response/transcript, a token/API key or a
///   complete environment (INV-6).
///
/// The tables are created for every database that runs the shared migration
/// runner, but the scheduler only ever starts for a database carrying the
/// `experiment_domain` marker and holding the singleton domain lease (AC-1).
pub const MIGRATION_SQL: &[(&str, &str)] = &[
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
];

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 19,
    description:
        "v19 migration: Add experiment domain marker/lease and durable campaign schedule/job tables",
    sql: MIGRATION_SQL,
    ensure: None,
    apply: None,
};

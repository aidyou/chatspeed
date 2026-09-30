-- ChatSpeed post-self-improvement database cleanup
--
-- This script is intentionally manual. It is not executed by the application.
-- Review and back up the database before running it.
--
-- The application migration history has been consolidated into v18. Ordinary
-- capability, automation, and tool-compatibility schema remains supported by
-- the application; the tables below are no longer used by the application.

BEGIN TRANSACTION;

-- Self-improvement / experiment budget tables.
DROP TABLE IF EXISTS experiment_budget_ledger_entries;
DROP TABLE IF EXISTS experiment_budget_reservations;
DROP TABLE IF EXISTS experiment_budget_scopes;

-- Headless experiment-domain and durable campaign scheduler tables.
DROP TABLE IF EXISTS experiment_job_artifacts;
DROP TABLE IF EXISTS experiment_job_bundles;
DROP TABLE IF EXISTS experiment_job_journal;
DROP TABLE IF EXISTS experiment_campaign_jobs;
DROP TABLE IF EXISTS experiment_campaign_schedules;
DROP TABLE IF EXISTS experiment_domain_lease;
DROP TABLE IF EXISTS experiment_domain;

-- Self-improvement promotion tables. Drop child rows before the parent table.
DROP TABLE IF EXISTS experiment_promotion_canary_results;
DROP TABLE IF EXISTS experiment_promotion_journal;
DROP TABLE IF EXISTS experiment_promotions;

-- If an older build created any standalone candidate/campaign tables, remove
-- them only after confirming they are not used by another local tool.
DROP TABLE IF EXISTS experiment_candidates;
DROP TABLE IF EXISTS experiment_campaigns;

-- Removed experiment-only workflow columns. These ALTER statements are
-- SQLite-compatible only when the column exists; inspect PRAGMA table_info
-- first and run the applicable statements manually if needed.
-- SQLite has no portable DROP COLUMN fallback for all supported older builds.
-- ALTER TABLE workflows DROP COLUMN experiment_agent_prompt_ref;
-- ALTER TABLE workflows DROP COLUMN experiment_agent_prompt_hash;
-- ALTER TABLE workflows DROP COLUMN experiment_prompt_catalog_digest;

COMMIT;

-- Historical v22 glob -> grep data normalization is not included here because
-- it is ordinary tool compatibility and must be preserved. If an existing
-- database is pre-v22, apply that normalization separately after review of
-- workflow_events, workflow_messages, workflow_context_messages, and
-- workflow_snapshots; do not delete those tables.

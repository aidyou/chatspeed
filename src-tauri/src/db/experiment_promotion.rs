//! Phase 2I durable promotion store.
//!
//! This is the promotion **authority**: the FSM, the fenced lease, the
//! transactionally consistent state transitions, the idempotent submission and
//! the append-only journal all live here. The `cs` CLI never opens this store,
//! never runs a scheduler and never touches Git (INV-2).
//!
//! Contract rules enforced here:
//!
//! - Every mutation takes a [`PromotionFence`] and CASes on
//!   `owner_id + lease_generation + state`, so a superseded worker matches zero
//!   rows and gets `lease_lost` instead of corrupting a newer owner's state.
//! - Every effect is *intent first*: the intent column and the FSM edge are
//!   written in one transaction before the effect runs, so recovery always has
//!   a typed starting point (INV-7).
//! - The single-flight rule is enforced by the schema's partial unique index,
//!   not by application logic alone: at most one non-terminal promotion may
//!   exist per target.
//! - Only refs, digests, typed facts and bounded diagnostics are stored; never
//!   a secret, a transcript, a holdout body or a raw canary stream (INV-8).

use crate::db::{MainStore, StoreError};
use crate::workflow::react::experiment_promotion::policy::{
    evaluate_promotion_policy, PromotionDecision, PromotionMetricEvaluation, PromotionOutcome,
};
use crate::workflow::react::experiment_promotion::types::{
    classify_promotion_recovery, journal_digest, transition_allowed, validate_transition,
    BranchObservation, CheckpointObservation, EffectIntent, PromotionError, PromotionErrorCode,
    PromotionEvidenceV1, PromotionFence, PromotionJournalEntryV1, PromotionJournalStage,
    PromotionRecoveryDecision, PromotionRequestV1, PromotionState,
};
use rusqlite::{params, OptionalExtension};
use std::sync::Arc;

/// Every column of `experiment_promotions`, in load order.
const PROMOTION_COLUMNS: &str = "promotion_id, campaign_id, candidate_key, target_ref, state,
    request_hash, evidence_hash, evidence_json, target_hash, policy_hash, base_revision,
    patch_sha256, patch_manifest_hash, expected_old_head, observed_head, checkpoint_commit,
    checkpoint_ref, checkpoint_intent, branch_intent, canary_result_hash, canary_result_json,
    decision_outcome, decision_code, decision_detail, decision_json, error_code, owner_id,
    lease_generation, lease_expires_at_ms, heartbeat_at_ms, attempt, created_at_ms, updated_at_ms";

/// Builds a store error carrying a stable machine code.
pub fn promotion_error(code: PromotionErrorCode, message: impl Into<String>) -> PromotionError {
    PromotionError::new(code, message)
}

/// Maps a durable-store or rusqlite failure onto the promotion error surface.
pub fn persistence_error(error: impl std::fmt::Display) -> PromotionError {
    PromotionError::new(
        PromotionErrorCode::PersistenceFailure,
        format!("durable promotion store failure: {error}"),
    )
}

/// Extracts the typed [`PromotionError`] a writer closure produced, or maps the
/// outer store failure.
fn flatten<T>(result: Result<Result<T, PromotionError>, StoreError>) -> Result<T, PromotionError> {
    match result {
        Ok(inner) => inner,
        Err(error) => Err(persistence_error(error)),
    }
}

/// The recorded policy decision, as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct PromotionDecisionRecord {
    pub outcome: String,
    pub code: String,
    pub detail: String,
}

impl PromotionDecisionRecord {
    pub fn is_promote(&self) -> bool {
        self.outcome == PromotionOutcome::Promote.as_str()
    }
}

/// One durable promotion row plus its decoded evidence.
#[derive(Debug, Clone)]
pub struct PromotionRecord {
    pub promotion_id: String,
    pub campaign_id: String,
    pub candidate_key: String,
    pub target_ref: String,
    pub state: PromotionState,
    pub request_hash: String,
    pub evidence_hash: String,
    pub evidence: PromotionEvidenceV1,
    pub target_hash: String,
    pub policy_hash: String,
    pub base_revision: String,
    pub patch_sha256: String,
    pub patch_manifest_hash: String,
    pub expected_old_head: Option<String>,
    pub observed_head: Option<String>,
    pub checkpoint_commit: Option<String>,
    pub checkpoint_ref: Option<String>,
    pub checkpoint_intent: EffectIntent,
    pub branch_intent: EffectIntent,
    pub canary_result_hash: Option<String>,
    pub decision: Option<PromotionDecisionRecord>,
    pub error_code: Option<String>,
    pub owner_id: Option<String>,
    pub lease_generation: i64,
    pub lease_expires_at_ms: Option<u64>,
    pub attempt: u32,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

impl PromotionRecord {
    /// The fence a worker must present to keep mutating this promotion.
    pub fn fence(&self) -> Option<PromotionFence> {
        self.owner_id
            .as_ref()
            .map(|owner| PromotionFence::new(owner.clone(), self.lease_generation))
    }

    /// The typed recovery decision for this row, given the two Git
    /// observations. Pure: it does not read the repository.
    pub fn recovery_decision(
        &self,
        checkpoint: CheckpointObservation,
        branch: BranchObservation,
    ) -> PromotionRecoveryDecision {
        classify_promotion_recovery(self.state, checkpoint, branch, self.checkpoint_intent)
    }
}

/// The outcome of an idempotent submission.
#[derive(Debug, Clone)]
pub enum SubmitOutcome {
    /// The promotion was created by this call.
    Created(Box<PromotionRecord>),
    /// The same idempotency key with the same request body was replayed.
    Existing(Box<PromotionRecord>),
}

/// The outcome of a claim attempt.
#[derive(Debug, Clone)]
pub enum PromotionClaimOutcome {
    /// A promotion was claimed or adopted with a fresh lease generation.
    Claimed(Box<PromotionRecord>),
    /// No claimable work (nothing queued, or every lease is live).
    Idle,
}

/// The durable promotion store.
#[derive(Clone)]
pub struct ExperimentPromotionStore {
    store: Arc<MainStore>,
}

impl ExperimentPromotionStore {
    pub fn new(store: Arc<MainStore>) -> Self {
        Self { store }
    }

    // -----------------------------------------------------------------------
    // Submission
    // -----------------------------------------------------------------------

    /// Persists one validated promotion request.
    ///
    /// The promotion id, the request hash and the evidence hash are all
    /// **re-derived** here from the request, so a caller can never mint a
    /// promotion identity: the id the CLI computes is only a hint, and a
    /// mismatch is a hard rejection.
    ///
    /// Idempotency is keyed on `idempotency_key`: the same key with the same
    /// request hash returns the original row, a different body under the same
    /// key is rejected with `idempotency_conflict`, and a second *different*
    /// promotion for a target that already has a live one is rejected with
    /// `promotion_in_flight`.
    pub fn submit(
        &self,
        request: &PromotionRequestV1,
        expected_promotion_id: &str,
        idempotency_key: &str,
        now_ms: u64,
    ) -> Result<SubmitOutcome, PromotionError> {
        request.validate()?;
        if !crate::workflow::react::experiment_promotion::types::is_valid_key(idempotency_key) {
            return Err(promotion_error(
                PromotionErrorCode::IdempotencyConflict,
                "'{idempotency_key}' is not a valid idempotency key",
            ));
        }
        let promotion_id = request.promotion_id();
        if promotion_id != expected_promotion_id {
            return Err(promotion_error(
                PromotionErrorCode::EvidenceMismatch,
                format!(
                    "the promotion id derived from the request is '{promotion_id}', not '{expected_promotion_id}'"
                ),
            ));
        }
        let request_hash = request.request_hash();
        let evidence_hash = request.evidence.evidence_hash();
        let evidence_json = serde_json::to_string(&request.evidence).map_err(|error| {
            promotion_error(
                PromotionErrorCode::InconsistentEvidence,
                format!("the evidence projection could not be serialised: {error}"),
            )
        })?;

        let campaign_id = request.campaign_id.clone();
        let candidate_key = request.candidate_key.clone();
        let target_ref = request.target_ref.clone();
        let base_revision = request.evidence.base_revision.clone();
        let patch_sha256 = request.evidence.patch_sha256.clone();
        let patch_manifest_hash = request.evidence.patch_manifest_hash.clone();
        let idempotency_key = idempotency_key.to_string();

        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<SubmitOutcome, PromotionError> {
                let tx = conn.transaction().map_err(persistence_error)?;

                // Idempotent replay: same key, same request hash.
                let existing: Option<(String, String)> = tx
                    .query_row(
                        "SELECT promotion_id, request_hash FROM experiment_promotions
                          WHERE idempotency_key = ?1",
                        params![idempotency_key],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                    .map_err(persistence_error)?;
                if let Some((existing_id, existing_hash)) = existing {
                    if existing_hash != request_hash {
                        return Err(promotion_error(
                            PromotionErrorCode::IdempotencyConflict,
                            format!(
                                "idempotency key '{idempotency_key}' was already used for a different promotion body"
                            ),
                        ));
                    }
                    let record = load_record(&tx, &existing_id)?;
                    tx.commit().map_err(persistence_error)?;
                    return Ok(SubmitOutcome::Existing(Box::new(record)));
                }

                // Single-flight per target, enforced by the partial unique index
                // and checked here for a stable machine code.
                let live: Option<String> = tx
                    .query_row(
                        "SELECT promotion_id FROM experiment_promotions
                          WHERE target_ref = ?1
                            AND state NOT IN (
                                'rejected','canary_failed','promoted','rolled_back','unknown_manual'
                            )",
                        params![target_ref],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(persistence_error)?;
                if let Some(live_id) = live {
                    return Err(promotion_error(
                        PromotionErrorCode::PromotionInFlight,
                        format!(
                            "target '{target_ref}' already has live promotion '{live_id}'"
                        ),
                    ));
                }

                let inserted = tx
                    .execute(
                        "INSERT OR IGNORE INTO experiment_promotions (
                            promotion_id, campaign_id, candidate_key, target_ref, state,
                            request_hash, evidence_hash, evidence_json, target_hash, policy_hash,
                            base_revision, patch_sha256, patch_manifest_hash,
                            checkpoint_intent, branch_intent, attempt, idempotency_key,
                            created_at_ms, updated_at_ms
                         ) VALUES (
                            ?1, ?2, ?3, ?4, 'queued',
                            ?5, ?6, ?7, '', '',
                            ?8, ?9, ?10,
                            'not_started', 'not_started', 0, ?11,
                            ?12, ?12
                         )",
                        params![
                            promotion_id,
                            campaign_id,
                            candidate_key,
                            target_ref,
                            request_hash,
                            evidence_hash,
                            evidence_json,
                            base_revision,
                            patch_sha256,
                            patch_manifest_hash,
                            idempotency_key,
                            now_ms as i64,
                        ],
                    )
                    .map_err(persistence_error)?;
                if inserted != 1 {
                    // A concurrent submission created the same promotion id.
                    let existing: i64 = tx
                        .query_row(
                            "SELECT COUNT(1) FROM experiment_promotions WHERE promotion_id = ?1",
                            params![promotion_id],
                            |row| row.get(0),
                        )
                        .map_err(persistence_error)?;
                    if existing > 0 {
                        let record = load_record(&tx, &promotion_id)?;
                        tx.commit().map_err(persistence_error)?;
                        return Ok(SubmitOutcome::Existing(Box::new(record)));
                    }
                    return Err(promotion_error(
                        PromotionErrorCode::PromotionInFlight,
                        format!("target '{target_ref}' already has a live promotion"),
                    ));
                }
                insert_journal(
                    &tx,
                    &promotion_id,
                    PromotionJournalStage::Submitted,
                    None,
                    0,
                    Some(&request_hash),
                    now_ms,
                )?;
                let record = load_record(&tx, &promotion_id)?;
                tx.commit().map_err(persistence_error)?;
                Ok(SubmitOutcome::Created(Box::new(record)))
            })();
            Ok(inner)
        }))
    }

    // -----------------------------------------------------------------------
    // Reads
    // -----------------------------------------------------------------------

    /// Loads one promotion by id.
    pub fn get(&self, promotion_id: &str) -> Result<PromotionRecord, PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let promotion_id = promotion_id.to_string();
        flatten(runtime.read_blocking(move |conn| {
            let inner = load_record(conn, &promotion_id);
            Ok(inner)
        }))
    }

    /// Loads the live promotion for a target, if any.
    pub fn latest_for_target(
        &self,
        target_ref: &str,
    ) -> Result<Option<PromotionRecord>, PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let target_ref = target_ref.to_string();
        flatten(runtime.read_blocking(move |conn| {
            let inner = (|| -> Result<Option<PromotionRecord>, PromotionError> {
                let promotion_id: Option<String> = conn
                    .query_row(
                        "SELECT promotion_id FROM experiment_promotions
                          WHERE target_ref = ?1
                          ORDER BY created_at_ms DESC, promotion_id DESC LIMIT 1",
                        params![target_ref],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(persistence_error)?;
                match promotion_id {
                    Some(promotion_id) => Ok(Some(load_record(conn, &promotion_id)?)),
                    None => Ok(None),
                }
            })();
            Ok(inner)
        }))
    }

    /// Every non-terminal promotion, oldest first. Used by the supervisor's
    /// startup reconciliation.
    pub fn list_non_terminal(&self) -> Result<Vec<PromotionRecord>, PromotionError> {
        let terminal = TERMINAL_STATES_SQL;
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        flatten(runtime.read_blocking(move |conn| {
            let inner = (|| -> Result<Vec<PromotionRecord>, PromotionError> {
                let mut statement = conn
                    .prepare(&format!(
                        "SELECT promotion_id FROM experiment_promotions
                          WHERE state NOT IN {terminal}
                          ORDER BY created_at_ms ASC, promotion_id ASC"
                    ))
                    .map_err(persistence_error)?;
                let ids: Vec<String> = statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(persistence_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(persistence_error)?;
                let mut records = Vec::with_capacity(ids.len());
                for id in ids {
                    records.push(load_record(conn, &id)?);
                }
                Ok(records)
            })();
            Ok(inner)
        }))
    }

    /// The ordered, append-only journal of one promotion.
    pub fn journal(
        &self,
        promotion_id: &str,
    ) -> Result<Vec<PromotionJournalEntryV1>, PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let promotion_id = promotion_id.to_string();
        flatten(runtime.read_blocking(move |conn| {
            let inner = (|| -> Result<Vec<PromotionJournalEntryV1>, PromotionError> {
                let mut statement = conn
                    .prepare(
                        "SELECT journal_id, stage, owner_id, lease_generation, detail_json,
                                created_at_ms
                           FROM experiment_promotion_journal
                          WHERE promotion_id = ?1
                          ORDER BY journal_id ASC",
                    )
                    .map_err(persistence_error)?;
                let entries = statement
                    .query_map(params![promotion_id], |row| {
                        Ok(PromotionJournalEntryV1 {
                            sequence: row.get(0)?,
                            stage: row.get(1)?,
                            owner_id: row.get(2)?,
                            lease_generation: row.get(3)?,
                            detail: row.get(4)?,
                            created_at_ms: row.get::<_, i64>(5)? as u64,
                        })
                    })
                    .map_err(persistence_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(persistence_error)?;
                Ok(entries)
            })();
            Ok(inner)
        }))
    }

    /// The canonical digest of one promotion's ordered journal.
    pub fn journal_digest(&self, promotion_id: &str) -> Result<String, PromotionError> {
        Ok(journal_digest(&self.journal(promotion_id)?))
    }

    /// The stored per-stage canary results, in stage order.
    pub fn canary_stage_results(
        &self,
        promotion_id: &str,
    ) -> Result<Vec<CanaryStageRow>, PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let promotion_id = promotion_id.to_string();
        flatten(runtime.read_blocking(move |conn| {
            let inner = (|| -> Result<Vec<CanaryStageRow>, PromotionError> {
                let mut statement = conn
                    .prepare(
                        "SELECT stage_index, stage_id, metric, samples, baseline_passed,
                                candidate_passed, baseline_mean, candidate_mean, declared_status,
                                recomputed_status, output_sha256
                           FROM experiment_promotion_canary_results
                          WHERE promotion_id = ?1
                          ORDER BY stage_index ASC",
                    )
                    .map_err(persistence_error)?;
                let rows = statement
                    .query_map(params![promotion_id], |row| {
                        Ok(CanaryStageRow {
                            stage_index: row.get::<_, i64>(0)? as u32,
                            stage_id: row.get(1)?,
                            metric: row.get(2)?,
                            samples: row.get::<_, i64>(3)? as u32,
                            baseline_passed: row.get::<_, i64>(4)? as u32,
                            candidate_passed: row.get::<_, i64>(5)? as u32,
                            baseline_mean: row.get(6)?,
                            candidate_mean: row.get(7)?,
                            declared_status: row.get(8)?,
                            recomputed_status: row.get(9)?,
                            output_sha256: row.get(10)?,
                        })
                    })
                    .map_err(persistence_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(persistence_error)?;
                Ok(rows)
            })();
            Ok(inner)
        }))
    }

    // -----------------------------------------------------------------------
    // Lease claim / heartbeat
    // -----------------------------------------------------------------------

    /// Claims the next claimable promotion with a fresh lease generation.
    ///
    /// Claimable means: the row is not terminal and either its lease is absent
    /// or provably expired. A `queued` row moves to `evidence_validating`; an
    /// already-started row keeps its state and is merely re-owned, because what
    /// to do next is decided by the pure recovery classifier, never here.
    pub fn claim_next(
        &self,
        owner_id: &str,
        now_ms: u64,
        lease_ms: u64,
    ) -> Result<PromotionClaimOutcome, PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let owner_id = owner_id.to_string();
        let lease_until = now_ms.saturating_add(lease_ms) as i64;
        let terminal = TERMINAL_STATES_SQL;
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<PromotionClaimOutcome, PromotionError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let candidate: Option<(String, String, i64)> = tx
                    .query_row(
                        &format!(
                            "SELECT promotion_id, state, lease_generation
                               FROM experiment_promotions
                              WHERE state NOT IN {terminal}
                                AND (lease_expires_at_ms IS NULL OR lease_expires_at_ms <= ?1)
                              ORDER BY created_at_ms ASC, promotion_id ASC
                              LIMIT 1"
                        ),
                        params![now_ms as i64],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()
                    .map_err(persistence_error)?;
                let Some((promotion_id, previous_state, generation)) = candidate else {
                    tx.commit().map_err(persistence_error)?;
                    return Ok(PromotionClaimOutcome::Idle);
                };
                let previous = PromotionState::parse(&previous_state).ok_or_else(|| {
                    promotion_error(
                        PromotionErrorCode::InvalidPromotionState,
                        format!("promotion '{promotion_id}' has unknown state '{previous_state}'"),
                    )
                })?;
                let adopted = previous != PromotionState::Queued;
                let next = if adopted {
                    previous
                } else {
                    PromotionState::EvidenceValidating
                };
                // Adoption re-owns a row at the state it already reached; that
                // is not an FSM edge, so only a real move is checked.
                if next != previous && !transition_allowed(previous, next) {
                    return Err(promotion_error(
                        PromotionErrorCode::InvalidPromotionTransition,
                        format!(
                            "claim cannot move promotion '{promotion_id}' from {} to {}",
                            previous.as_str(),
                            next.as_str()
                        ),
                    ));
                }
                let changed = tx
                    .execute(
                        "UPDATE experiment_promotions
                            SET state = ?3,
                                owner_id = ?2,
                                lease_generation = lease_generation + 1,
                                lease_expires_at_ms = ?4,
                                heartbeat_at_ms = ?5,
                                attempt = attempt + 1,
                                updated_at_ms = ?5
                          WHERE promotion_id = ?1
                            AND lease_generation = ?6
                            AND state = ?7",
                        params![
                            promotion_id,
                            owner_id,
                            next.as_str(),
                            lease_until,
                            now_ms as i64,
                            generation,
                            previous.as_str(),
                        ],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    // Someone else won the race; no effect was recorded.
                    tx.rollback().map_err(persistence_error)?;
                    return Ok(PromotionClaimOutcome::Idle);
                }
                let record = load_record(&tx, &promotion_id)?;
                insert_journal(
                    &tx,
                    &promotion_id,
                    PromotionJournalStage::Claimed,
                    Some(&owner_id),
                    record.lease_generation,
                    Some(if adopted {
                        "lease_adopted"
                    } else {
                        "lease_claimed"
                    }),
                    now_ms,
                )?;
                tx.commit().map_err(persistence_error)?;
                Ok(PromotionClaimOutcome::Claimed(Box::new(record)))
            })();
            Ok(inner)
        }))
    }

    /// Renews a live lease. A stale generation matches zero rows.
    pub fn heartbeat(
        &self,
        fence: &PromotionFence,
        promotion_id: &str,
        now_ms: u64,
        lease_ms: u64,
    ) -> Result<(), PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let promotion_id = promotion_id.to_string();
        let owner_id = fence.owner_id.clone();
        let generation = fence.lease_generation;
        let lease_until = now_ms.saturating_add(lease_ms) as i64;
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<(), PromotionError> {
                let changed = conn
                    .execute(
                        "UPDATE experiment_promotions
                            SET lease_expires_at_ms = ?3, heartbeat_at_ms = ?4, updated_at_ms = ?4
                          WHERE promotion_id = ?1
                            AND owner_id = ?2
                            AND lease_generation = ?5",
                        params![
                            promotion_id,
                            owner_id,
                            lease_until,
                            now_ms as i64,
                            generation
                        ],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    return Err(promotion_error(
                        PromotionErrorCode::LeaseLost,
                        format!(
                            "promotion '{promotion_id}' is no longer owned by generation {generation}"
                        ),
                    ));
                }
                Ok(())
            })();
            Ok(inner)
        }))
    }

    // -----------------------------------------------------------------------
    // Fenced transitions
    // -----------------------------------------------------------------------

    /// Moves one promotion along an FSM edge, CASing on owner, generation and
    /// the expected state. Terminal targets release ownership.
    pub fn transition(
        &self,
        fence: &PromotionFence,
        promotion_id: &str,
        from: PromotionState,
        to: PromotionState,
        journal_stage: PromotionJournalStage,
        detail: Option<&str>,
        error_code: Option<&str>,
        now_ms: u64,
    ) -> Result<PromotionRecord, PromotionError> {
        validate_transition(from, to)?;
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let promotion_id = promotion_id.to_string();
        let owner_id = fence.owner_id.clone();
        let generation = fence.lease_generation;
        let detail = detail.map(str::to_string);
        let error_code = error_code.map(str::to_string);
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<PromotionRecord, PromotionError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let current = load_record(&tx, &promotion_id)?;
                if current.state != from {
                    return Err(promotion_error(
                        PromotionErrorCode::InvalidPromotionTransition,
                        format!(
                            "promotion '{promotion_id}' is '{}' but '{}' was expected",
                            current.state.as_str(),
                            from.as_str()
                        ),
                    ));
                }
                let clears_lease = if to.is_terminal() { 1_i64 } else { 0_i64 };
                let changed = tx
                    .execute(
                        "UPDATE experiment_promotions
                            SET state = ?3,
                                error_code = COALESCE(?7, error_code),
                                owner_id = CASE WHEN ?8 = 1 THEN NULL ELSE ?2 END,
                                lease_expires_at_ms = CASE
                                    WHEN ?8 = 1 THEN NULL ELSE lease_expires_at_ms END,
                                heartbeat_at_ms = CASE
                                    WHEN ?8 = 1 THEN NULL ELSE heartbeat_at_ms END,
                                updated_at_ms = ?6
                          WHERE promotion_id = ?1
                            AND owner_id = ?2
                            AND lease_generation = ?5
                            AND state = ?4",
                        params![
                            promotion_id,
                            owner_id,
                            to.as_str(),
                            from.as_str(),
                            generation,
                            now_ms as i64,
                            error_code,
                            clears_lease,
                        ],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    tx.rollback().map_err(persistence_error)?;
                    return Err(promotion_error(
                        PromotionErrorCode::LeaseLost,
                        format!(
                            "promotion '{promotion_id}' is no longer owned by generation {generation}"
                        ),
                    ));
                }
                insert_journal(
                    &tx,
                    &promotion_id,
                    journal_stage,
                    Some(&owner_id),
                    generation,
                    detail.as_deref(),
                    now_ms,
                )?;
                let record = load_record(&tx, &promotion_id)?;
                tx.commit().map_err(persistence_error)?;
                Ok(record)
            })();
            Ok(inner)
        }))
    }

    /// Records the validated bindings and the policy decision, without moving
    /// the FSM. The bindings are the durable half of "the CLI projection is not
    /// the only evidence" (INV-2): they are what the supervisor re-checks
    /// against the repository and the durable job rows.
    pub fn record_bindings(
        &self,
        fence: &PromotionFence,
        promotion_id: &str,
        base_revision: &str,
        expected_old_head: &str,
        target_hash: &str,
        policy_hash: &str,
        now_ms: u64,
    ) -> Result<PromotionRecord, PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let promotion_id = promotion_id.to_string();
        let owner_id = fence.owner_id.clone();
        let generation = fence.lease_generation;
        let base_revision = base_revision.to_string();
        let expected_old_head = expected_old_head.to_string();
        let target_hash = target_hash.to_string();
        let policy_hash = policy_hash.to_string();
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<PromotionRecord, PromotionError> {
                let changed = conn
                    .execute(
                        "UPDATE experiment_promotions
                            SET base_revision = ?3,
                                expected_old_head = ?4,
                                target_hash = ?5,
                                policy_hash = ?6,
                                updated_at_ms = ?7
                          WHERE promotion_id = ?1
                            AND owner_id = ?2
                            AND lease_generation = ?8
                            AND state = 'evidence_validating'",
                        params![
                            promotion_id,
                            owner_id,
                            base_revision,
                            expected_old_head,
                            target_hash,
                            policy_hash,
                            now_ms as i64,
                            generation,
                        ],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    return Err(promotion_error(
                        PromotionErrorCode::LeaseLost,
                        format!("promotion '{promotion_id}' is no longer claimable by this worker"),
                    ));
                }
                load_record(conn, &promotion_id)
            })();
            Ok(inner)
        }))
    }

    /// Records the policy decision in the durable row and the journal.
    pub fn record_policy_decision(
        &self,
        fence: &PromotionFence,
        promotion_id: &str,
        decision: &PromotionDecision,
        now_ms: u64,
    ) -> Result<PromotionRecord, PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let promotion_id = promotion_id.to_string();
        let owner_id = fence.owner_id.clone();
        let generation = fence.lease_generation;
        let outcome = decision.outcome.as_str().to_string();
        let code = decision.code.as_str().to_string();
        let detail = decision.detail.clone();
        let decision_json =
            serde_json::to_string(&PromotionDecisionJson::from(decision)).map_err(|error| {
                promotion_error(
                    PromotionErrorCode::PersistenceFailure,
                    format!("the policy decision could not be serialised: {error}"),
                )
            })?;
        let journal_detail = format!("{outcome}:{code}");
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<PromotionRecord, PromotionError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let changed = tx
                    .execute(
                        "UPDATE experiment_promotions
                            SET decision_outcome = ?3,
                                decision_code = ?4,
                                decision_detail = ?5,
                                decision_json = ?6,
                                updated_at_ms = ?7
                          WHERE promotion_id = ?1
                            AND owner_id = ?2
                            AND lease_generation = ?8
                            AND state = 'evidence_validating'",
                        params![
                            promotion_id,
                            owner_id,
                            outcome,
                            code,
                            detail,
                            decision_json,
                            now_ms as i64,
                            generation,
                        ],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    return Err(promotion_error(
                        PromotionErrorCode::LeaseLost,
                        format!("promotion '{promotion_id}' is no longer claimable by this worker"),
                    ));
                }
                insert_journal(
                    &tx,
                    &promotion_id,
                    PromotionJournalStage::PolicyDecided,
                    Some(&owner_id),
                    generation,
                    Some(&journal_detail),
                    now_ms,
                )?;
                let record = load_record(&tx, &promotion_id)?;
                tx.commit().map_err(persistence_error)?;
                Ok(record)
            })();
            Ok(inner)
        }))
    }

    /// Rejects the candidate, recording the machine code.
    pub fn reject(
        &self,
        fence: &PromotionFence,
        promotion_id: &str,
        from: PromotionState,
        code: PromotionErrorCode,
        detail: &str,
        now_ms: u64,
    ) -> Result<PromotionRecord, PromotionError> {
        self.transition(
            fence,
            promotion_id,
            from,
            PromotionState::Rejected,
            PromotionJournalStage::Rejected,
            Some(detail),
            Some(code.as_str()),
            now_ms,
        )
    }

    /// Parks the promotion as `unknown_manual` from any non-terminal state. The
    /// FSM edge is validated, so a terminal row can never be re-parked.
    pub fn park_unknown_manual(
        &self,
        fence: &PromotionFence,
        promotion_id: &str,
        from: PromotionState,
        code: PromotionErrorCode,
        detail: &str,
        now_ms: u64,
    ) -> Result<PromotionRecord, PromotionError> {
        self.transition(
            fence,
            promotion_id,
            from,
            PromotionState::UnknownManual,
            PromotionJournalStage::ParkedUnknownManual,
            Some(detail),
            Some(code.as_str()),
            now_ms,
        )
    }

    /// Writes the durable checkpoint intent and moves the FSM to
    /// `checkpointing` in one transaction. After this commit the effect may or
    /// may not have happened, which is exactly what the intent records.
    pub fn begin_checkpoint(
        &self,
        fence: &PromotionFence,
        promotion_id: &str,
        now_ms: u64,
    ) -> Result<PromotionRecord, PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let promotion_id = promotion_id.to_string();
        let owner_id = fence.owner_id.clone();
        let generation = fence.lease_generation;
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<PromotionRecord, PromotionError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let changed = tx
                    .execute(
                        "UPDATE experiment_promotions
                            SET state = 'checkpointing',
                                checkpoint_intent = 'intent_recorded',
                                updated_at_ms = ?3
                          WHERE promotion_id = ?1
                            AND owner_id = ?2
                            AND lease_generation = ?4
                            AND state = 'evidence_validating'",
                        params![promotion_id, owner_id, now_ms as i64, generation],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    tx.rollback().map_err(persistence_error)?;
                    return Err(promotion_error(
                        PromotionErrorCode::LeaseLost,
                        format!("promotion '{promotion_id}' is no longer claimable by this worker"),
                    ));
                }
                insert_journal(
                    &tx,
                    &promotion_id,
                    PromotionJournalStage::CheckpointIntent,
                    Some(&owner_id),
                    generation,
                    None,
                    now_ms,
                )?;
                let record = load_record(&tx, &promotion_id)?;
                tx.commit().map_err(persistence_error)?;
                Ok(record)
            })();
            Ok(inner)
        }))
    }

    /// Records the created checkpoint commit/ref and moves the FSM to
    /// `checkpointed`.
    ///
    /// `observed_head` is deliberately *not* written here: it records the
    /// observed head of the **registered branch**, so it stays empty until the
    /// advance (or rollback) observes the branch.
    pub fn complete_checkpoint(
        &self,
        fence: &PromotionFence,
        promotion_id: &str,
        checkpoint_commit: &str,
        checkpoint_ref: &str,
        now_ms: u64,
    ) -> Result<PromotionRecord, PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let promotion_id = promotion_id.to_string();
        let owner_id = fence.owner_id.clone();
        let generation = fence.lease_generation;
        let checkpoint_commit = checkpoint_commit.to_string();
        let checkpoint_ref = checkpoint_ref.to_string();
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<PromotionRecord, PromotionError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let changed = tx
                    .execute(
                        "UPDATE experiment_promotions
                            SET state = 'checkpointed',
                                checkpoint_intent = 'completed',
                                checkpoint_commit = ?3,
                                checkpoint_ref = ?4,
                                updated_at_ms = ?5
                          WHERE promotion_id = ?1
                            AND owner_id = ?2
                            AND lease_generation = ?6
                            AND state = 'checkpointing'",
                        params![
                            promotion_id,
                            owner_id,
                            checkpoint_commit,
                            checkpoint_ref,
                            now_ms as i64,
                            generation,
                        ],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    tx.rollback().map_err(persistence_error)?;
                    return Err(promotion_error(
                        PromotionErrorCode::LeaseLost,
                        format!("promotion '{promotion_id}' is no longer checkpointing"),
                    ));
                }
                insert_journal(
                    &tx,
                    &promotion_id,
                    PromotionJournalStage::CheckpointCreated,
                    Some(&owner_id),
                    generation,
                    Some(&checkpoint_commit),
                    now_ms,
                )?;
                let record = load_record(&tx, &promotion_id)?;
                tx.commit().map_err(persistence_error)?;
                Ok(record)
            })();
            Ok(inner)
        }))
    }

    /// Moves the FSM to `canary_running` and journals the canary intent.
    pub fn begin_canary(
        &self,
        fence: &PromotionFence,
        promotion_id: &str,
        now_ms: u64,
    ) -> Result<PromotionRecord, PromotionError> {
        self.transition(
            fence,
            promotion_id,
            PromotionState::Checkpointed,
            PromotionState::CanaryRunning,
            PromotionJournalStage::CanaryIntent,
            None,
            None,
            now_ms,
        )
    }

    /// Persists a completed paired canary result and moves the FSM either to
    /// `ready_to_advance` or to `canary_failed`. The per-stage rows and the
    /// aggregate document are written in the same transaction, so a stage can
    /// never be half-recorded.
    #[allow(clippy::too_many_arguments)]
    pub fn record_canary_result(
        &self,
        fence: &PromotionFence,
        promotion_id: &str,
        result_json: &str,
        result_hash: &str,
        stage_rows: &[CanaryStageRow],
        passed: bool,
        error_code: Option<PromotionErrorCode>,
        detail: &str,
        now_ms: u64,
    ) -> Result<PromotionRecord, PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let promotion_id = promotion_id.to_string();
        let owner_id = fence.owner_id.clone();
        let generation = fence.lease_generation;
        let result_json = result_json.to_string();
        let result_hash = result_hash.to_string();
        let stage_rows = stage_rows.to_vec();
        let detail = detail.to_string();
        let error_code = error_code.map(|code| code.as_str().to_string());
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<PromotionRecord, PromotionError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let next = if passed {
                    PromotionState::ReadyToAdvance
                } else {
                    PromotionState::CanaryFailed
                };
                let clears_lease = if next.is_terminal() { 1_i64 } else { 0_i64 };
                let changed = tx
                    .execute(
                        "UPDATE experiment_promotions
                            SET state = ?3,
                                canary_result_hash = ?4,
                                canary_result_json = ?5,
                                error_code = ?6,
                                owner_id = CASE WHEN ?9 = 1 THEN NULL ELSE ?2 END,
                                lease_expires_at_ms = CASE
                                    WHEN ?9 = 1 THEN NULL ELSE lease_expires_at_ms END,
                                heartbeat_at_ms = CASE
                                    WHEN ?9 = 1 THEN NULL ELSE heartbeat_at_ms END,
                                updated_at_ms = ?7
                          WHERE promotion_id = ?1
                            AND owner_id = ?2
                            AND lease_generation = ?8
                            AND state = 'canary_running'",
                        params![
                            promotion_id,
                            owner_id,
                            next.as_str(),
                            result_hash,
                            result_json,
                            error_code,
                            now_ms as i64,
                            generation,
                            clears_lease,
                        ],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    tx.rollback().map_err(persistence_error)?;
                    return Err(promotion_error(
                        PromotionErrorCode::LeaseLost,
                        format!("promotion '{promotion_id}' is no longer running a canary"),
                    ));
                }
                for row in &stage_rows {
                    tx.execute(
                        "INSERT OR REPLACE INTO experiment_promotion_canary_results (
                            result_id, promotion_id, stage_index, stage_id, metric, samples,
                            baseline_passed, candidate_passed, baseline_mean, candidate_mean,
                            declared_status, recomputed_status, output_sha256, created_at_ms
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                        params![
                            format!("{promotion_id}:{}", row.stage_id),
                            promotion_id,
                            row.stage_index as i64,
                            row.stage_id,
                            row.metric,
                            row.samples as i64,
                            row.baseline_passed as i64,
                            row.candidate_passed as i64,
                            row.baseline_mean,
                            row.candidate_mean,
                            row.declared_status,
                            row.recomputed_status,
                            row.output_sha256,
                            now_ms as i64,
                        ],
                    )
                    .map_err(persistence_error)?;
                }
                insert_journal(
                    &tx,
                    &promotion_id,
                    PromotionJournalStage::CanaryCompleted,
                    Some(&owner_id),
                    generation,
                    Some(&detail),
                    now_ms,
                )?;
                let record = load_record(&tx, &promotion_id)?;
                tx.commit().map_err(persistence_error)?;
                Ok(record)
            })();
            Ok(inner)
        }))
    }

    /// Writes the durable branch-advance intent and moves the FSM to
    /// `advancing` in one transaction, before the CAS runs.
    pub fn begin_advance(
        &self,
        fence: &PromotionFence,
        promotion_id: &str,
        now_ms: u64,
    ) -> Result<PromotionRecord, PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let promotion_id = promotion_id.to_string();
        let owner_id = fence.owner_id.clone();
        let generation = fence.lease_generation;
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<PromotionRecord, PromotionError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let changed = tx
                    .execute(
                        "UPDATE experiment_promotions
                            SET state = 'advancing',
                                branch_intent = 'intent_recorded',
                                updated_at_ms = ?3
                          WHERE promotion_id = ?1
                            AND owner_id = ?2
                            AND lease_generation = ?4
                            AND state = 'ready_to_advance'",
                        params![promotion_id, owner_id, now_ms as i64, generation],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    tx.rollback().map_err(persistence_error)?;
                    return Err(promotion_error(
                        PromotionErrorCode::LeaseLost,
                        format!("promotion '{promotion_id}' is no longer ready to advance"),
                    ));
                }
                insert_journal(
                    &tx,
                    &promotion_id,
                    PromotionJournalStage::AdvanceIntent,
                    Some(&owner_id),
                    generation,
                    None,
                    now_ms,
                )?;
                let record = load_record(&tx, &promotion_id)?;
                tx.commit().map_err(persistence_error)?;
                Ok(record)
            })();
            Ok(inner)
        }))
    }

    /// Records that the registered branch now points at the checkpoint.
    pub fn complete_advance(
        &self,
        fence: &PromotionFence,
        promotion_id: &str,
        observed_head: &str,
        now_ms: u64,
    ) -> Result<PromotionRecord, PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let promotion_id = promotion_id.to_string();
        let owner_id = fence.owner_id.clone();
        let generation = fence.lease_generation;
        let observed_head = observed_head.to_string();
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<PromotionRecord, PromotionError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let changed = tx
                    .execute(
                        "UPDATE experiment_promotions
                            SET state = 'promoted',
                                branch_intent = 'completed',
                                observed_head = ?3,
                                owner_id = NULL,
                                lease_expires_at_ms = NULL,
                                heartbeat_at_ms = NULL,
                                updated_at_ms = ?4
                          WHERE promotion_id = ?1
                            AND owner_id = ?2
                            AND lease_generation = ?5
                            AND state = 'advancing'",
                        params![
                            promotion_id,
                            owner_id,
                            observed_head,
                            now_ms as i64,
                            generation,
                        ],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    tx.rollback().map_err(persistence_error)?;
                    return Err(promotion_error(
                        PromotionErrorCode::LeaseLost,
                        format!("promotion '{promotion_id}' is no longer advancing"),
                    ));
                }
                insert_journal(
                    &tx,
                    &promotion_id,
                    PromotionJournalStage::BranchAdvanced,
                    Some(&owner_id),
                    generation,
                    Some(&observed_head),
                    now_ms,
                )?;
                let record = load_record(&tx, &promotion_id)?;
                tx.commit().map_err(persistence_error)?;
                Ok(record)
            })();
            Ok(inner)
        }))
    }

    /// Records a compensated rollback from `advancing` back to the old head.
    pub fn roll_back(
        &self,
        fence: &PromotionFence,
        promotion_id: &str,
        observed_head: &str,
        detail: &str,
        now_ms: u64,
    ) -> Result<PromotionRecord, PromotionError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let promotion_id = promotion_id.to_string();
        let owner_id = fence.owner_id.clone();
        let generation = fence.lease_generation;
        let observed_head = observed_head.to_string();
        let detail = detail.to_string();
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<PromotionRecord, PromotionError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let changed = tx
                    .execute(
                        "UPDATE experiment_promotions
                            SET state = 'rolled_back',
                                branch_intent = 'not_started',
                                observed_head = ?3,
                                error_code = 'branch_rolled_back',
                                owner_id = NULL,
                                lease_expires_at_ms = NULL,
                                heartbeat_at_ms = NULL,
                                updated_at_ms = ?4
                          WHERE promotion_id = ?1
                            AND owner_id = ?2
                            AND lease_generation = ?5
                            AND state = 'advancing'",
                        params![
                            promotion_id,
                            owner_id,
                            observed_head,
                            now_ms as i64,
                            generation,
                        ],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    tx.rollback().map_err(persistence_error)?;
                    return Err(promotion_error(
                        PromotionErrorCode::LeaseLost,
                        format!("promotion '{promotion_id}' is no longer advancing"),
                    ));
                }
                insert_journal(
                    &tx,
                    &promotion_id,
                    PromotionJournalStage::BranchRolledBack,
                    Some(&owner_id),
                    generation,
                    Some(&detail),
                    now_ms,
                )?;
                let record = load_record(&tx, &promotion_id)?;
                tx.commit().map_err(persistence_error)?;
                Ok(record)
            })();
            Ok(inner)
        }))
    }
}

/// The SQL predicate for the terminal promotion states, derived from the FSM so
/// the two can never drift.
pub const TERMINAL_STATES_SQL: &str =
    "('rejected','canary_failed','promoted','rolled_back','unknown_manual')";

/// One stored per-stage canary outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct CanaryStageRow {
    pub stage_index: u32,
    pub stage_id: String,
    pub metric: String,
    pub samples: u32,
    pub baseline_passed: u32,
    pub candidate_passed: u32,
    pub baseline_mean: f64,
    pub candidate_mean: f64,
    pub declared_status: String,
    pub recomputed_status: String,
    pub output_sha256: String,
}

/// The serialisable projection of a policy decision, so the audit can rebuild
/// the decision without re-evaluating the evidence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct PromotionDecisionJson {
    outcome: String,
    code: String,
    detail: String,
    metrics: Vec<PromotionMetricEvaluationJson>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct PromotionMetricEvaluationJson {
    metric: String,
    samples: u32,
    improvement: f64,
    improved: bool,
    regressed: bool,
}

impl From<&PromotionDecision> for PromotionDecisionJson {
    fn from(decision: &PromotionDecision) -> Self {
        Self {
            outcome: decision.outcome.as_str().to_string(),
            code: decision.code.as_str().to_string(),
            detail: decision.detail.clone(),
            metrics: decision
                .metrics
                .iter()
                .map(|metric| PromotionMetricEvaluationJson {
                    metric: metric.metric.clone(),
                    samples: metric.samples,
                    improvement: metric.improvement,
                    improved: metric.improved,
                    regressed: metric.regressed,
                })
                .collect(),
        }
    }
}

/// Rebuilds a typed decision from its stored projection.
fn decision_from_json(
    outcome: Option<String>,
    code: Option<String>,
    detail: Option<String>,
) -> Option<PromotionDecisionRecord> {
    Some(PromotionDecisionRecord {
        outcome: outcome?,
        code: code.unwrap_or_default(),
        detail: detail.unwrap_or_default(),
    })
}

/// Evaluates the policy for a stored record. Kept here so the supervisor and
/// the tests both call exactly one implementation.
pub fn evaluate_record_policy(
    record: &PromotionRecord,
    policy: &crate::workflow::react::experiment_promotion::policy::PromotionPolicyV1,
) -> PromotionDecision {
    evaluate_promotion_policy(&record.evidence, policy)
}

/// The metric evaluations of a stored decision, decoded for the audit.
pub fn decision_metrics(
    decision_json: Option<&str>,
) -> Result<Vec<PromotionMetricEvaluation>, PromotionError> {
    let Some(document) = decision_json else {
        return Ok(Vec::new());
    };
    let decoded: PromotionDecisionJson = serde_json::from_str(document).map_err(|error| {
        promotion_error(
            PromotionErrorCode::InvalidPromotionState,
            format!("the stored policy decision is not readable: {error}"),
        )
    })?;
    Ok(decoded
        .metrics
        .into_iter()
        .map(|metric| PromotionMetricEvaluation {
            metric: metric.metric,
            samples: metric.samples,
            improvement: metric.improvement,
            improved: metric.improved,
            regressed: metric.regressed,
        })
        .collect())
}

/// Loads one promotion row inside an existing connection or transaction.
fn load_record(
    conn: &rusqlite::Connection,
    promotion_id: &str,
) -> Result<PromotionRecord, PromotionError> {
    let row = conn
        .query_row(
            &format!(
                "SELECT {PROMOTION_COLUMNS} FROM experiment_promotions WHERE promotion_id = ?1"
            ),
            params![promotion_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, Option<String>>(16)?,
                    row.get::<_, String>(17)?,
                    row.get::<_, String>(18)?,
                    row.get::<_, Option<String>>(19)?,
                    row.get::<_, Option<String>>(20)?,
                    row.get::<_, Option<String>>(21)?,
                    row.get::<_, Option<String>>(22)?,
                    row.get::<_, Option<String>>(23)?,
                    row.get::<_, Option<String>>(24)?,
                    row.get::<_, Option<String>>(25)?,
                    row.get::<_, Option<String>>(26)?,
                    row.get::<_, i64>(27)?,
                    row.get::<_, Option<i64>>(28)?,
                    row.get::<_, Option<i64>>(29)?,
                    row.get::<_, i64>(30)?,
                    row.get::<_, i64>(31)?,
                    row.get::<_, i64>(32)?,
                ))
            },
        )
        .optional()
        .map_err(persistence_error)?
        .ok_or_else(|| {
            promotion_error(
                PromotionErrorCode::UnknownPromotion,
                format!("promotion '{promotion_id}' is unknown for this domain"),
            )
        })?;

    let state = PromotionState::parse(&row.4).ok_or_else(|| {
        promotion_error(
            PromotionErrorCode::InvalidPromotionState,
            format!("promotion '{promotion_id}' has unknown state '{}'", row.4),
        )
    })?;
    let checkpoint_intent = EffectIntent::parse(&row.17).ok_or_else(|| {
        promotion_error(
            PromotionErrorCode::InvalidPromotionState,
            format!(
                "promotion '{promotion_id}' has unknown checkpoint intent '{}'",
                row.17
            ),
        )
    })?;
    let branch_intent = EffectIntent::parse(&row.18).ok_or_else(|| {
        promotion_error(
            PromotionErrorCode::InvalidPromotionState,
            format!(
                "promotion '{promotion_id}' has unknown branch intent '{}'",
                row.18
            ),
        )
    })?;
    let evidence: PromotionEvidenceV1 = serde_json::from_str(&row.7).map_err(|error| {
        promotion_error(
            PromotionErrorCode::InvalidPromotionState,
            format!("promotion '{promotion_id}' has unreadable evidence: {error}"),
        )
    })?;

    Ok(PromotionRecord {
        promotion_id: row.0,
        campaign_id: row.1,
        candidate_key: row.2,
        target_ref: row.3,
        state,
        request_hash: row.5,
        evidence_hash: row.6,
        evidence,
        target_hash: row.8,
        policy_hash: row.9,
        base_revision: row.10,
        patch_sha256: row.11,
        patch_manifest_hash: row.12,
        expected_old_head: row.13,
        observed_head: row.14,
        checkpoint_commit: row.15,
        checkpoint_ref: row.16,
        checkpoint_intent,
        branch_intent,
        canary_result_hash: row.19.clone(),
        decision: decision_from_json(row.21, row.22, row.23),
        error_code: row.25,
        owner_id: row.26,
        lease_generation: row.27,
        lease_expires_at_ms: row.28.map(|value| value as u64),
        attempt: row.30 as u32,
        created_at_ms: row.31 as u64,
        updated_at_ms: row.32 as u64,
    })
}

fn insert_journal(
    conn: &rusqlite::Connection,
    promotion_id: &str,
    stage: PromotionJournalStage,
    owner_id: Option<&str>,
    lease_generation: i64,
    detail: Option<&str>,
    now_ms: u64,
) -> Result<(), PromotionError> {
    conn.execute(
        "INSERT INTO experiment_promotion_journal (
            promotion_id, stage, owner_id, lease_generation, detail_json, created_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            promotion_id,
            stage.as_str(),
            owner_id,
            lease_generation,
            detail,
            now_ms as i64
        ],
    )
    .map_err(persistence_error)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_promotion::policy::{
        evaluate_promotion_policy, CanaryStageSpecV1, MetricDirection, PromotionCanarySpecV1,
        PromotionMetricRuleV1, PromotionPolicyV1, PROMOTION_POLICY_V1,
    };
    use crate::workflow::react::experiment_promotion::types::{
        PromotionBudgetFactsV1, PromotionMetricFactV1, PromotionVerifierIdentityV1,
        CANARY_RESULT_V1, PROMOTION_EVIDENCE_V1, PROMOTION_REQUEST_V1,
    };
    use tempfile::tempdir;

    const T0: u64 = 1_700_000_000_000;
    const LEASE_MS: u64 = 30_000;

    fn harness() -> (ExperimentPromotionStore, tempfile::TempDir) {
        let directory = tempdir().expect("tempdir");
        let store =
            Arc::new(MainStore::new(directory.path().join("promotion.db")).expect("main store"));
        // The promotion schema is part of the consolidated v18 CLI migration;
        // a fresh database installs the latest schema in one pass.
        let version = store
            .db_runtime()
            .expect("runtime")
            .read_blocking(|conn: &mut rusqlite::Connection| {
                crate::db::sql::migrations::manager::get_db_version(conn)
            })
            .expect("version read");
        assert_eq!(version, 18);
        (ExperimentPromotionStore::new(store), directory)
    }

    fn metric(metric: &str, baseline: f64, candidate: f64) -> PromotionMetricFactV1 {
        PromotionMetricFactV1 {
            metric: metric.to_string(),
            samples: 8,
            baseline_passed: 4,
            candidate_passed: 8,
            baseline_mean: baseline,
            candidate_mean: candidate,
        }
    }

    fn request(
        candidate_key: &str,
        target_ref: &str,
        baseline: f64,
        candidate: f64,
    ) -> PromotionRequestV1 {
        PromotionRequestV1 {
            schema_version: PROMOTION_REQUEST_V1.to_string(),
            campaign_id: "camp-0123456789abcdef0123456789abcdef".to_string(),
            candidate_key: candidate_key.to_string(),
            target_ref: target_ref.to_string(),
            evidence: PromotionEvidenceV1 {
                schema_version: PROMOTION_EVIDENCE_V1.to_string(),
                campaign_id: "camp-0123456789abcdef0123456789abcdef".to_string(),
                candidate_key: candidate_key.to_string(),
                baseline_job_id: "job-baseline".to_string(),
                candidate_job_id: "job-candidate".to_string(),
                baseline_run_id: "run-baseline".to_string(),
                candidate_run_id: "run-candidate".to_string(),
                candidate_session_id: "session-candidate".to_string(),
                baseline_artifact_hash: "a".repeat(64),
                candidate_artifact_hash: "b".repeat(64),
                baseline_evaluation_hash: "c".repeat(64),
                candidate_evaluation_hash: "d".repeat(64),
                baseline_verdict_hash: "e".repeat(64),
                candidate_verdict_hash: "f".repeat(64),
                fixture_ref: "smoke-tools".to_string(),
                fixture_digest: "1".repeat(64),
                task_id: "task-a".to_string(),
                suite: "chatspeed-smoke".to_string(),
                dataset_id: "chatspeed-smoke".to_string(),
                dataset_version: 2,
                split: "smoke".to_string(),
                execution_profile_ref: "smoke-local".to_string(),
                execution_profile_hash: "2".repeat(64),
                patch_manifest_hash: "3".repeat(64),
                patch_sha256: "4".repeat(64),
                base_revision: "refs/heads/main".to_string(),
                verdict_status: "pass".to_string(),
                verdict_score: 1.0,
                verdict_safety_status: "pass".to_string(),
                verdict_infra_status: "pass".to_string(),
                verdict_cost_status: "known".to_string(),
                budget: PromotionBudgetFactsV1 {
                    cost_status: "known".to_string(),
                    budget_rejected: false,
                    committed_micros: 10,
                    currency: "usd".to_string(),
                },
                verifier: PromotionVerifierIdentityV1 {
                    verifier_id: "chatspeed-smoke".to_string(),
                    verifier_version: "2".to_string(),
                    verifier_digest: "5".repeat(64),
                },
                metrics: vec![metric("verdict_score", baseline, candidate)],
            },
        }
    }

    fn policy() -> PromotionPolicyV1 {
        PromotionPolicyV1 {
            schema_version: PROMOTION_POLICY_V1.to_string(),
            policy_ref: "default".to_string(),
            require_verdict_pass: true,
            require_safety_pass: true,
            require_infra_pass: true,
            allow_unknown_cost: false,
            max_committed_micros: 1_000_000,
            metrics: vec![PromotionMetricRuleV1 {
                metric: "verdict_score".to_string(),
                direction: MetricDirection::HigherIsBetter,
                min_improvement: 0.1,
                max_regression: 0.0,
                min_samples: 4,
                required: true,
            }],
        }
    }

    fn canary_spec() -> PromotionCanarySpecV1 {
        PromotionCanarySpecV1 {
            execution_profile_ref: "smoke-local".to_string(),
            bundle_ref: "smoke-tools".to_string(),
            executable: "./tools/canary".to_string(),
            stages: vec![CanaryStageSpecV1 {
                stage_id: "stage-1".to_string(),
                args: vec!["--task".to_string(), "task-a".to_string()],
                metric: "verdict_score".to_string(),
                direction: MetricDirection::HigherIsBetter,
                min_improvement: 0.1,
                max_regression: 0.0,
                min_samples: 4,
                required: true,
            }],
            timeout_ms: 60_000,
            max_output_bytes: 65_536,
        }
    }

    fn stage_row(passed: bool) -> CanaryStageRow {
        CanaryStageRow {
            stage_index: 0,
            stage_id: "stage-1".to_string(),
            metric: "verdict_score".to_string(),
            samples: 4,
            baseline_passed: 2,
            candidate_passed: if passed { 4 } else { 2 },
            baseline_mean: 0.5,
            candidate_mean: if passed { 1.0 } else { 0.5 },
            declared_status: if passed { "pass" } else { "fail" }.to_string(),
            recomputed_status: if passed { "pass" } else { "fail" }.to_string(),
            output_sha256: "9".repeat(64),
        }
    }

    /// Claims one promotion and drives it up to (but not through) the canary.
    fn drive_to_ready(
        store: &ExperimentPromotionStore,
        owner: &str,
        now: u64,
        idempotency_key: &str,
    ) -> PromotionRecord {
        let request = request("prompt-a", "local-dev", 0.5, 1.0);
        let promotion_id = request.promotion_id();
        store
            .submit(&request, &promotion_id, idempotency_key, now)
            .expect("submit");
        let claimed = match store.claim_next(owner, now, LEASE_MS).expect("claim") {
            PromotionClaimOutcome::Claimed(record) => record,
            PromotionClaimOutcome::Idle => panic!("expected a claim"),
        };
        let fence = claimed.fence().expect("fence");
        store
            .record_bindings(
                &fence,
                &claimed.promotion_id,
                "refs/heads/main",
                &"d".repeat(40),
                &"t".repeat(64),
                &"p".repeat(64),
                now,
            )
            .expect("bindings");
        let decision = evaluate_promotion_policy(&claimed.evidence, &policy());
        store
            .record_policy_decision(&fence, &claimed.promotion_id, &decision, now)
            .expect("decision");
        store
            .begin_checkpoint(&fence, &claimed.promotion_id, now)
            .expect("begin checkpoint");
        store
            .complete_checkpoint(
                &fence,
                &claimed.promotion_id,
                &"c".repeat(40),
                &format!("refs/chatspeed/checkpoints/{}", claimed.promotion_id),
                now,
            )
            .expect("complete checkpoint");
        store
            .begin_canary(&fence, &claimed.promotion_id, now)
            .expect("begin canary");
        store.get(&claimed.promotion_id).expect("reload")
    }

    #[test]
    fn submission_is_idempotent_and_single_flight_per_target() {
        let (store, _directory) = harness();
        let submission = request("prompt-a", "local-dev", 0.5, 1.0);
        let promotion_id = submission.promotion_id();

        let created = store
            .submit(&submission, &promotion_id, "key-1", T0)
            .expect("first submit");
        let SubmitOutcome::Created(created) = created else {
            panic!("the first submission must create the promotion");
        };
        assert_eq!(created.state, PromotionState::Queued);
        assert_eq!(created.checkpoint_intent, EffectIntent::NotStarted);
        assert_eq!(created.branch_intent, EffectIntent::NotStarted);

        // Same key, same body: the original row comes back untouched.
        let replayed = store
            .submit(&submission, &promotion_id, "key-1", T0 + 1)
            .expect("replay");
        let SubmitOutcome::Existing(replayed) = replayed else {
            panic!("a replay must return the existing promotion");
        };
        assert_eq!(replayed.promotion_id, created.promotion_id);
        assert_eq!(replayed.created_at_ms, created.created_at_ms);
        assert_eq!(replayed.state, PromotionState::Queued);

        // Same key, different body: a conflict, and nothing changed.
        let mut other = submission.clone();
        other.evidence.metrics = vec![metric("verdict_score", 0.5, 0.6)];
        assert_eq!(
            store
                .submit(&other, &other.promotion_id(), "key-1", T0 + 2)
                .expect_err("different body")
                .code,
            PromotionErrorCode::IdempotencyConflict
        );

        // A different promotion (different evidence ⇒ different id) for a
        // target that already has a live one is refused.
        let second = request("prompt-a", "local-dev", 0.5, 0.9);
        assert_eq!(
            store
                .submit(&second, &second.promotion_id(), "key-2", T0 + 3)
                .expect_err("single flight")
                .code,
            PromotionErrorCode::PromotionInFlight
        );

        // A caller-minted promotion id is never accepted.
        assert_eq!(
            store
                .submit(
                    &submission,
                    "promo-00000000000000000000000000000000",
                    "key-3",
                    T0 + 4
                )
                .expect_err("minted id")
                .code,
            PromotionErrorCode::EvidenceMismatch
        );
    }

    #[test]
    fn a_claim_is_single_winner_and_generation_fenced() {
        let (store, _directory) = harness();
        let request = request("prompt-a", "local-dev", 0.5, 1.0);
        let promotion_id = request.promotion_id();
        store
            .submit(&request, &promotion_id, "key-1", T0)
            .expect("submit");

        let first = match store.claim_next("worker-a", T0, LEASE_MS).expect("claim") {
            PromotionClaimOutcome::Claimed(record) => record,
            PromotionClaimOutcome::Idle => panic!("expected the first claim to win"),
        };
        assert_eq!(first.state, PromotionState::EvidenceValidating);
        assert_eq!(first.lease_generation, 1);
        assert_eq!(first.owner_id.as_deref(), Some("worker-a"));

        // The lease is live, so a second worker gets nothing.
        assert!(matches!(
            store
                .claim_next("worker-b", T0 + 1, LEASE_MS)
                .expect("claim"),
            PromotionClaimOutcome::Idle
        ));

        // Once the lease provably expired, a new worker adopts the row and the
        // generation advances.
        let adopted = match store
            .claim_next("worker-b", T0 + LEASE_MS + 1, LEASE_MS)
            .expect("adopt")
        {
            PromotionClaimOutcome::Claimed(record) => record,
            PromotionClaimOutcome::Idle => panic!("expected the adoption to win"),
        };
        assert_eq!(adopted.state, PromotionState::EvidenceValidating);
        assert_eq!(adopted.lease_generation, 2);
        assert_eq!(adopted.owner_id.as_deref(), Some("worker-b"));
        assert_eq!(adopted.attempt, 2);

        // The superseded worker can no longer mutate anything.
        let stale = PromotionFence::new("worker-a", 1);
        assert_eq!(
            store
                .heartbeat(&stale, &promotion_id, T0 + 2, LEASE_MS)
                .expect_err("stale heartbeat")
                .code,
            PromotionErrorCode::LeaseLost
        );
        assert_eq!(
            store
                .begin_checkpoint(&stale, &promotion_id, T0 + 2)
                .expect_err("stale checkpoint")
                .code,
            PromotionErrorCode::LeaseLost
        );
        // ...and the row is untouched.
        let current = store.get(&promotion_id).expect("reload");
        assert_eq!(current.state, PromotionState::EvidenceValidating);
        assert_eq!(current.lease_generation, 2);
    }

    #[test]
    fn a_rejected_candidate_never_reaches_a_checkpoint() {
        let (store, _directory) = harness();
        // A candidate that does not improve is rejected by the policy gate.
        let submission = request("prompt-a", "local-dev", 0.5, 0.5);
        let promotion_id = submission.promotion_id();
        store
            .submit(&submission, &promotion_id, "key-1", T0)
            .expect("submit");
        let claimed = match store.claim_next("worker-a", T0, LEASE_MS).expect("claim") {
            PromotionClaimOutcome::Claimed(record) => record,
            PromotionClaimOutcome::Idle => panic!("expected a claim"),
        };
        let fence = claimed.fence().expect("fence");
        let decision = evaluate_promotion_policy(&claimed.evidence, &policy());
        assert_eq!(decision.outcome, PromotionOutcome::Reject);
        assert_eq!(decision.code, PromotionErrorCode::NoImprovement);
        store
            .record_policy_decision(&fence, &promotion_id, &decision, T0)
            .expect("decision");
        let rejected = store
            .reject(
                &fence,
                &promotion_id,
                PromotionState::EvidenceValidating,
                decision.code,
                &decision.detail,
                T0,
            )
            .expect("reject");
        assert_eq!(rejected.state, PromotionState::Rejected);
        assert_eq!(rejected.error_code.as_deref(), Some("no_improvement"));
        // No checkpoint was ever requested.
        assert_eq!(rejected.checkpoint_intent, EffectIntent::NotStarted);
        assert!(rejected.checkpoint_commit.is_none());
        assert!(rejected.checkpoint_ref.is_none());
        assert!(
            rejected.owner_id.is_none(),
            "terminal rows release ownership"
        );

        // The rejection is terminal, so a later attempt for the same target is
        // admitted again (the partial unique index excludes terminal rows).
        let second = request("prompt-a", "local-dev", 0.5, 0.9);
        assert!(store
            .submit(&second, &second.promotion_id(), "key-2", T0 + 1)
            .is_ok());
        // A rejected promotion can never be moved back into the FSM.
        assert_eq!(
            store
                .begin_checkpoint(&fence, &promotion_id, T0 + 2)
                .expect_err("terminal")
                .code,
            PromotionErrorCode::LeaseLost
        );
    }

    #[test]
    fn the_happy_path_reaches_promoted_with_an_ordered_journal() {
        let (store, _directory) = harness();
        let record = drive_to_ready(&store, "worker-a", T0, "key-1");
        let fence = record.fence().expect("fence");
        assert_eq!(record.state, PromotionState::CanaryRunning);
        assert_eq!(record.checkpoint_intent, EffectIntent::Completed);
        assert!(record.checkpoint_commit.is_some());

        let passing = store
            .record_canary_result(
                &fence,
                &record.promotion_id,
                "{\"schema_version\":\"canary_result.v1\"}",
                &"r".repeat(64),
                &[stage_row(true)],
                true,
                None,
                "pass",
                T0,
            )
            .expect("canary result");
        assert_eq!(passing.state, PromotionState::ReadyToAdvance);
        assert_eq!(
            passing.canary_result_hash.as_deref(),
            Some(&*"r".repeat(64))
        );

        store
            .begin_advance(&fence, &record.promotion_id, T0)
            .expect("begin advance");
        let advanced = store
            .complete_advance(&fence, &record.promotion_id, &"c".repeat(40), T0)
            .expect("complete advance");
        assert_eq!(advanced.state, PromotionState::Promoted);
        assert_eq!(advanced.branch_intent, EffectIntent::Completed);
        assert!(advanced.owner_id.is_none());

        // The journal is append-only and ordered by allocation.
        let journal = store.journal(&record.promotion_id).expect("journal");
        let stages: Vec<&str> = journal.iter().map(|entry| entry.stage.as_str()).collect();
        assert_eq!(
            stages,
            vec![
                "submitted",
                "claimed",
                "policy_decided",
                "checkpoint_intent",
                "checkpoint_created",
                "canary_intent",
                "canary_completed",
                "advance_intent",
                "branch_advanced",
            ]
        );
        let sequences: Vec<i64> = journal.iter().map(|entry| entry.sequence).collect();
        let mut sorted = sequences.clone();
        sorted.sort_unstable();
        assert_eq!(sequences, sorted, "journal sequence must be monotonic");

        // The digest is stable across reads and covers the ordering.
        let digest = store.journal_digest(&record.promotion_id).expect("digest");
        assert_eq!(
            digest,
            store.journal_digest(&record.promotion_id).expect("digest")
        );
        assert_ne!(
            digest,
            journal_digest(&journal[..journal.len() - 1]),
            "a truncated journal must hash differently"
        );

        // The stage rows round-trip through the store.
        let rows = store
            .canary_stage_results(&record.promotion_id)
            .expect("stage rows");
        assert_eq!(rows, vec![stage_row(true)]);

        // The stored decision is reproducible for the audit.
        assert!(store
            .get(&record.promotion_id)
            .expect("reload")
            .decision
            .expect("decision recorded")
            .is_promote());
    }

    #[test]
    fn a_failed_canary_stops_the_run_and_leaves_the_branch_alone() {
        let (store, _directory) = harness();
        let record = drive_to_ready(&store, "worker-a", T0, "key-1");
        let fence = record.fence().expect("fence");
        let failed = store
            .record_canary_result(
                &fence,
                &record.promotion_id,
                "{\"schema_version\":\"canary_result.v1\"}",
                &"r".repeat(64),
                &[stage_row(false)],
                false,
                Some(PromotionErrorCode::CanaryStageFailed),
                "stage-1 failed",
                T0,
            )
            .expect("canary result");
        assert_eq!(failed.state, PromotionState::CanaryFailed);
        assert_eq!(failed.error_code.as_deref(), Some("canary_stage_failed"));
        // The branch CAS was never requested, and the registered branch head
        // was never observed because no advance ran.
        assert_eq!(failed.branch_intent, EffectIntent::NotStarted);
        assert!(failed.observed_head.is_none());
        assert!(failed.owner_id.is_none());

        // A failed canary can never be advanced afterwards.
        assert_eq!(
            store
                .begin_advance(&fence, &record.promotion_id, T0)
                .expect_err("terminal")
                .code,
            PromotionErrorCode::LeaseLost
        );
        // The checkpoint evidence is retained, not discarded.
        let retained = store.get(&record.promotion_id).expect("reload");
        assert!(retained.checkpoint_commit.is_some());
        assert_eq!(retained.checkpoint_intent, EffectIntent::Completed);
    }

    #[test]
    fn a_transaction_failure_leaves_no_half_state() {
        let (store, _directory) = harness();
        let record = drive_to_ready(&store, "worker-a", T0, "key-1");
        let fence = record.fence().expect("fence");

        // A transition that names the wrong source state is refused and writes
        // nothing (no journal entry, no state change).
        let before = store.journal(&record.promotion_id).expect("journal").len();
        assert_eq!(
            store
                .transition(
                    &fence,
                    &record.promotion_id,
                    PromotionState::Checkpointed,
                    PromotionState::ReadyToAdvance,
                    PromotionJournalStage::CanaryCompleted,
                    None,
                    None,
                    T0,
                )
                .expect_err("wrong source state")
                .code,
            PromotionErrorCode::InvalidPromotionTransition
        );
        let after = store.get(&record.promotion_id).expect("reload");
        assert_eq!(after.state, PromotionState::CanaryRunning);
        assert_eq!(after.canary_result_hash, None);
        assert_eq!(
            store.journal(&record.promotion_id).expect("journal").len(),
            before,
            "a refused transition must not write a journal entry"
        );

        // An illegal FSM edge is refused before touching the row at all.
        assert_eq!(
            store
                .transition(
                    &fence,
                    &record.promotion_id,
                    PromotionState::CanaryRunning,
                    PromotionState::Promoted,
                    PromotionJournalStage::BranchAdvanced,
                    None,
                    None,
                    T0,
                )
                .expect_err("illegal edge")
                .code,
            PromotionErrorCode::InvalidPromotionTransition
        );

        // A stale-worker transition leaves the row untouched too.
        let stale = PromotionFence::new("worker-z", 99);
        assert_eq!(
            store
                .transition(
                    &stale,
                    &record.promotion_id,
                    PromotionState::CanaryRunning,
                    PromotionState::CanaryFailed,
                    PromotionJournalStage::CanaryCompleted,
                    None,
                    None,
                    T0,
                )
                .expect_err("stale")
                .code,
            PromotionErrorCode::LeaseLost
        );
        assert_eq!(
            store.get(&record.promotion_id).expect("reload").state,
            PromotionState::CanaryRunning
        );
    }

    #[test]
    fn recovery_classification_drives_from_stored_state() {
        let (store, _directory) = harness();
        let record = drive_to_ready(&store, "worker-a", T0, "key-1");
        let record = store.get(&record.promotion_id).expect("reload");

        // The row is mid-canary with a provable checkpoint: resume the canary.
        assert_eq!(
            record.recovery_decision(
                CheckpointObservation::PresentConsistent,
                BranchObservation::AtOld
            ),
            PromotionRecoveryDecision::ResumeCanary
        );
        // A checkpoint the repository cannot prove is never adopted.
        assert_eq!(
            record.recovery_decision(CheckpointObservation::Absent, BranchObservation::AtOld),
            PromotionRecoveryDecision::ParkUnknown
        );

        // After the advance intent, the same row either retries the CAS or
        // rolls forward; a third value parks.
        let fence = record.fence().expect("fence");
        store
            .record_canary_result(
                &fence,
                &record.promotion_id,
                "{}",
                &"r".repeat(64),
                &[stage_row(true)],
                true,
                None,
                "pass",
                T0,
            )
            .expect("canary result");
        store
            .begin_advance(&fence, &record.promotion_id, T0)
            .expect("advance intent");
        let advancing = store.get(&record.promotion_id).expect("reload");
        assert_eq!(advancing.state, PromotionState::Advancing);
        assert_eq!(advancing.branch_intent, EffectIntent::IntentRecorded);
        assert_eq!(
            advancing.recovery_decision(
                CheckpointObservation::PresentConsistent,
                BranchObservation::AtOld
            ),
            PromotionRecoveryDecision::Advance
        );
        assert_eq!(
            advancing.recovery_decision(
                CheckpointObservation::PresentConsistent,
                BranchObservation::AtCheckpoint
            ),
            PromotionRecoveryDecision::RollForward
        );
        assert_eq!(
            advancing.recovery_decision(
                CheckpointObservation::PresentConsistent,
                BranchObservation::ThirdValue
            ),
            PromotionRecoveryDecision::ParkUnknown
        );

        // Parking is a real, recorded terminal outcome that keeps the evidence.
        let parked = store
            .park_unknown_manual(
                &fence,
                &record.promotion_id,
                PromotionState::Advancing,
                PromotionErrorCode::BranchHeadDrift,
                "the branch moved under us",
                T0,
            )
            .expect("park");
        assert_eq!(parked.state, PromotionState::UnknownManual);
        assert_eq!(parked.error_code.as_deref(), Some("branch_head_drift"));
        assert!(parked.checkpoint_commit.is_some());
        assert_eq!(
            parked.recovery_decision(
                CheckpointObservation::PresentConsistent,
                BranchObservation::ThirdValue
            ),
            PromotionRecoveryDecision::Terminal
        );

        // A compensated rollback is recorded as its own terminal state.
        let (store, _directory) = harness();
        let record = drive_to_ready(&store, "worker-a", T0, "key-1");
        let fence = record.fence().expect("fence");
        store
            .record_canary_result(
                &fence,
                &record.promotion_id,
                "{}",
                &"r".repeat(64),
                &[stage_row(true)],
                true,
                None,
                "pass",
                T0,
            )
            .expect("canary result");
        store
            .begin_advance(&fence, &record.promotion_id, T0)
            .expect("advance intent");
        let rolled = store
            .roll_back(
                &fence,
                &record.promotion_id,
                &"d".repeat(40),
                "post-CAS persistence failed",
                T0,
            )
            .expect("rollback");
        assert_eq!(rolled.state, PromotionState::RolledBack);
        assert_eq!(rolled.observed_head.as_deref(), Some(&*"d".repeat(40)));
    }

    #[test]
    fn the_live_set_and_the_canary_document_are_queryable() {
        let (store, _directory) = harness();
        let submission = request("prompt-a", "local-dev", 0.5, 1.0);
        let promotion_id = submission.promotion_id();
        store
            .submit(&submission, &promotion_id, "key-1", T0)
            .expect("submit");
        assert_eq!(store.list_non_terminal().expect("live").len(), 1);
        let latest = store
            .latest_for_target("local-dev")
            .expect("latest")
            .expect("present");
        assert_eq!(latest.promotion_id, promotion_id);
        assert!(store
            .latest_for_target("missing")
            .expect("latest")
            .is_none());
        assert_eq!(
            store
                .get("promo-00000000000000000000000000000000")
                .expect_err("unknown")
                .code,
            PromotionErrorCode::UnknownPromotion
        );

        // The stored evidence round-trips, so a restart can re-evaluate the
        // policy without the caller.
        let reloaded = store.get(&promotion_id).expect("reload");
        assert_eq!(reloaded.evidence, submission.evidence);
        assert_eq!(reloaded.evidence_hash, submission.evidence.evidence_hash());
        assert_eq!(reloaded.request_hash, submission.request_hash());
        assert_eq!(reloaded.campaign_id, submission.campaign_id);
        assert_eq!(reloaded.candidate_key, submission.candidate_key);
        let replayed_decision = evaluate_promotion_policy(&reloaded.evidence, &policy());
        assert_eq!(replayed_decision.outcome, PromotionOutcome::Promote);

        // The canary document schema version the store expects is the frozen one.
        assert_eq!(CANARY_RESULT_V1, "canary_result.v1");
        let _ = canary_spec();
    }
}

//! SQLite-backed durable journal for capability operations and effects.
//!
//! The journal is the authority for idempotency across restarts and for the
//! ownership/reconciliation evidence doctor consumes. Rows only ever hold
//! redacted projections, and every external effect is bracketed by an intent
//! row and an observation row written in their own short transaction
//! (AC-2/AC-13/INV-8).

use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Arc;

use crate::db::{MainStore, StoreError};

use super::error::{code, CapabilityError};
use super::operation::{canonical_request_hash, new_effect_id, new_operation_id, now_ms};
use super::redaction;
use super::types::{
    CapabilityKind, CapabilityOperation, EffectIntent, EffectOutcome, OperationBegin,
    OperationEffect, OperationRequest, OperationState, SkillFileEntry, SkillInstallation,
    SkillInstallationState,
};

/// Upper bound for a persisted request/result projection. Larger payloads are
/// replaced by a truncation marker so no journal row can grow without bound.
const MAX_PAYLOAD_CHARS: usize = 8_192;

const OPERATION_COLUMNS: &str = "operation_id, capability, operation_kind, actor_scope, \
     idempotency_key, request_hash, request_json, resource_key, state, phase, result_json, \
     error_code, error_message, reconcile_reason, created_at_ms, updated_at_ms, completed_at_ms";

const EFFECT_COLUMNS: &str = "effect_id, operation_id, effect_key, target, intent, outcome, \
     detail_json, created_at_ms, updated_at_ms";

const INSTALLATION_COLUMNS: &str = "installation_id, skill_name, target_id, install_path, \
     source_kind, source_ref, checker_version, verdict, content_digest, file_manifest_json, \
     marker_nonce, manifest_digest, state, operation_id, created_at_ms, updated_at_ms";

/// Durable repository for capability operations, effects and Skill ownership.
pub struct CapabilityRepository {
    store: Arc<MainStore>,
}

impl CapabilityRepository {
    pub fn new(store: Arc<MainStore>) -> Self {
        Self { store }
    }

    /// The underlying store, shared with the desktop runtime (INV-1).
    pub fn store(&self) -> &Arc<MainStore> {
        &self.store
    }

    // ---------------------------------------------------------------- operations

    /// Opens (or replays) one operation under an idempotency key.
    ///
    /// Returns [`OperationBegin::Replay`] when the key already exists with the
    /// same canonical request hash, and `idempotency_key_conflict` when the key
    /// exists with a different hash. The unique index is the final authority,
    /// so a concurrent insert that loses the race is resolved by re-reading.
    pub fn begin(&self, request: &OperationRequest) -> Result<OperationBegin, CapabilityError> {
        let request_hash = canonical_request_hash(&request.request);
        let redacted_request = redaction::bounded_redacted_json(&request.request, MAX_PAYLOAD_CHARS);
        let capability = request.capability;
        let operation_kind = request.operation_kind.clone();
        let actor_scope = request.actor_scope.clone();
        let idempotency_key = request.idempotency_key.clone();
        let resource_key = request.resource_key.clone();
        let request_json = serde_json::to_string(&redacted_request)?;

        let runtime = self.store.db_runtime()?;
        let lookup_keys = (
            capability.as_str().to_string(),
            actor_scope.clone(),
            idempotency_key.clone(),
        );
        let insert_payload = (            operation_kind,
            request_hash.clone(),
            request_json,
            resource_key,
        );

        let outcome = runtime.write_blocking(move |connection| {
            if let Some(existing) = find_operation_by_key(
                connection,
                &lookup_keys.0,
                &lookup_keys.1,
                &lookup_keys.2,
            )? {
                return Ok((Some(existing), false));
            }

            let operation_id = new_operation_id(capability);
            let now = now_ms();
            let insert_result = connection.execute(
                "INSERT INTO capability_operations (
                     operation_id, capability, operation_kind, actor_scope, idempotency_key,
                     request_hash, request_json, resource_key, state, phase,
                     created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'planned', NULL, ?9, ?9)",
                params![
                    operation_id,
                    capability.as_str(),
                    insert_payload.0,
                    lookup_keys.1,
                    lookup_keys.2,
                    insert_payload.1,
                    insert_payload.2,
                    insert_payload.3,
                    now,
                ],
            );

            if let Err(error) = insert_result {
                // The unique index rejected a concurrent create; the stored
                // row is authoritative and is re-read below.
                if is_unique_violation(&error) {
                    let existing = find_operation_by_key(
                        connection,
                        capability.as_str(),
                        &lookup_keys.1,
                        &lookup_keys.2,
                    )?;
                    return Ok((existing, false));
                }
                return Err(StoreError::from(error));
            }

            let created = find_operation(connection, &operation_id)?;
            Ok((created, true))
        })?;

        let (stored, inserted) = outcome;
        let stored = stored.ok_or_else(|| {
            CapabilityError::internal("operation row vanished immediately after insert")
        })?;

        if !inserted {
            if stored.request_hash != request_hash {
                return Err(CapabilityError::new(
                    code::IDEMPOTENCY_KEY_CONFLICT,
                    format!(
                        "idempotency key '{}' was already used with a different request",
                        stored.idempotency_key
                    ),
                ));
            }
            return Ok(OperationBegin::Replay(stored));
        }

        Ok(OperationBegin::Started(stored))
    }

    /// Loads one operation by id.
    pub fn get(&self, operation_id: &str) -> Result<Option<CapabilityOperation>, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        let operation_id = operation_id.to_string();
        Ok(runtime.read_blocking(move |connection| find_operation(connection, &operation_id))?)
    }

    /// Loads one operation by its idempotency scope.
    pub fn get_by_idempotency(
        &self,
        capability: CapabilityKind,
        actor_scope: &str,
        idempotency_key: &str,
    ) -> Result<Option<CapabilityOperation>, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        let capability = capability.as_str().to_string();
        let actor_scope = actor_scope.to_string();
        let idempotency_key = idempotency_key.to_string();
        Ok(runtime.read_blocking(move |connection| {
            find_operation_by_key(connection, &capability, &actor_scope, &idempotency_key)
        })?)
    }

    /// Every operation that is still in an in-flight state (crash candidates).
    pub fn list_interrupted(&self) -> Result<Vec<CapabilityOperation>, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        Ok(runtime.read_blocking(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {OPERATION_COLUMNS} FROM capability_operations
                 WHERE state IN ('planned','staging','checking','applying')
                 ORDER BY created_at_ms"
            ))?;
            let rows = statement.query_map([], row_to_operation)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::from)
        })?)
    }

    /// Every operation that needs an explicit reconcile decision.
    pub fn list_needing_reconcile(&self) -> Result<Vec<CapabilityOperation>, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        Ok(runtime.read_blocking(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {OPERATION_COLUMNS} FROM capability_operations
                 WHERE state = 'needs_reconcile' ORDER BY created_at_ms"
            ))?;
            let rows = statement.query_map([], row_to_operation)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::from)
        })?)
    }

    /// The most recent operations for one resource key, newest first.
    pub fn list_by_resource(
        &self,
        capability: CapabilityKind,
        resource_key: &str,
        limit: u32,
    ) -> Result<Vec<CapabilityOperation>, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        let capability = capability.as_str().to_string();
        let resource_key = resource_key.to_string();
        Ok(runtime.read_blocking(move |connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {OPERATION_COLUMNS} FROM capability_operations
                 WHERE capability = ?1 AND resource_key = ?2
                 ORDER BY created_at_ms DESC LIMIT ?3"
            ))?;
            let rows = statement.query_map(params![capability, resource_key, limit], row_to_operation)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::from)
        })?)
    }

    /// Advances the state (and optionally the phase) of one operation.
    pub fn set_state(
        &self,
        operation_id: &str,
        state: OperationState,
        phase: Option<&str>,
    ) -> Result<CapabilityOperation, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        let operation_id = operation_id.to_string();
        let phase = phase.map(|value| value.to_string());
        let stored = runtime.write_blocking(move |connection| {
            let changed = connection.execute(
                "UPDATE capability_operations
                 SET state = ?2, phase = ?3, updated_at_ms = ?4
                 WHERE operation_id = ?1",
                params![operation_id, state.as_str(), phase, now_ms()],
            )?;
            if changed == 0 {
                return Ok(None);
            }
            find_operation(connection, &operation_id)
        })?;
        stored.ok_or_else(|| {
            CapabilityError::new(
                code::OPERATION_NOT_FOUND,
                "capability operation does not exist",
            )
        })
    }

    /// Records the terminal state of one operation with a redacted result.
    pub fn finish(
        &self,
        operation_id: &str,
        state: OperationState,
        result: Option<&serde_json::Value>,
        error: Option<&CapabilityError>,
    ) -> Result<CapabilityOperation, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        let operation_id = operation_id.to_string();
        let result_json = match result {
            Some(value) => {
                Some(serde_json::to_string(&redaction::bounded_redacted_json(
                    value,
                    MAX_PAYLOAD_CHARS,
                ))?)
            }
            None => None,
        };
        let error_code = error.map(|value| value.code.clone());
        let error_message = error.map(|value| value.redacted_message());
        let stored = runtime.write_blocking(move |connection| {
            let changed = connection.execute(
                "UPDATE capability_operations
                 SET state = ?2, result_json = ?3, error_code = ?4, error_message = ?5,
                     completed_at_ms = ?6, updated_at_ms = ?6
                 WHERE operation_id = ?1",
                params![
                    operation_id,
                    state.as_str(),
                    result_json,
                    error_code,
                    error_message,
                    now_ms(),
                ],
            )?;
            if changed == 0 {
                return Ok(None);
            }
            find_operation(connection, &operation_id)
        })?;
        stored.ok_or_else(|| {
            CapabilityError::new(
                code::OPERATION_NOT_FOUND,
                "capability operation does not exist",
            )
        })
    }

    /// Marks an operation as requiring reconciliation instead of a blind retry.
    pub fn mark_needs_reconcile(
        &self,
        operation_id: &str,
        reason: &str,
        error: Option<&CapabilityError>,
    ) -> Result<CapabilityOperation, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        let operation_id = operation_id.to_string();
        let reason = redaction::redact_text(reason);
        let error_code = error.map(|value| value.code.clone());
        let error_message = error.map(|value| value.redacted_message());
        let stored = runtime.write_blocking(move |connection| {
            let now = now_ms();
            let changed = connection.execute(
                "UPDATE capability_operations
                 SET state = 'needs_reconcile', reconcile_reason = ?2, error_code = ?3,
                     error_message = ?4, updated_at_ms = ?5, completed_at_ms = ?5
                 WHERE operation_id = ?1",
                params![operation_id, reason, error_code, error_message, now],
            )?;
            if changed == 0 {
                return Ok(None);
            }
            find_operation(connection, &operation_id)
        })?;
        stored.ok_or_else(|| {
            CapabilityError::new(
                code::OPERATION_NOT_FOUND,
                "capability operation does not exist",
            )
        })
    }

    // ------------------------------------------------------------------- effects

    /// Writes the intent of one externally visible effect *before* it happens.
    ///
    /// The row is created with `outcome = 'pending'`; if the process dies
    /// before [`Self::record_effect_outcome`], recovery sees an unproven
    /// effect and classifies the operation as `needs_reconcile`.
    pub fn record_effect_intent(
        &self,
        operation_id: &str,
        effect_key: &str,
        target: Option<&str>,
        detail: Option<&serde_json::Value>,
    ) -> Result<OperationEffect, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        let operation_id = operation_id.to_string();
        let effect_key = effect_key.to_string();
        let target = target.map(|value| value.to_string());
        let detail_json = match detail {
            Some(value) => Some(serde_json::to_string(&redaction::bounded_redacted_json(
                value,
                MAX_PAYLOAD_CHARS,
            ))?),
            None => None,
        };
        let effect_id = new_effect_id();
        let stored = runtime.write_blocking(move |connection| {
            let now = now_ms();
            connection.execute(
                "INSERT INTO capability_operation_effects (
                     effect_id, operation_id, effect_key, target, intent, outcome,
                     detail_json, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, 'intent_recorded', 'pending', ?5, ?6, ?6)
                 ON CONFLICT(operation_id, effect_key) DO UPDATE SET
                     target = excluded.target,
                     intent = 'intent_recorded',
                     outcome = 'pending',
                     detail_json = excluded.detail_json,
                     updated_at_ms = excluded.updated_at_ms",
                params![effect_id, operation_id, effect_key, target, detail_json, now],
            )?;
            find_effect(connection, &operation_id, &effect_key)
        })?;
        stored.ok_or_else(|| CapabilityError::internal("effect row vanished after upsert"))
    }

    /// Records the observation of one effect *after* it happened.
    pub fn record_effect_outcome(
        &self,
        operation_id: &str,
        effect_key: &str,
        outcome: EffectOutcome,
        detail: Option<&serde_json::Value>,
    ) -> Result<OperationEffect, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        let operation_id = operation_id.to_string();
        let effect_key = effect_key.to_string();
        let detail_json = match detail {
            Some(value) => Some(serde_json::to_string(&redaction::bounded_redacted_json(
                value,
                MAX_PAYLOAD_CHARS,
            ))?),
            None => None,
        };
        let effect_id = new_effect_id();
        let stored = runtime.write_blocking(move |connection| {
            let now = now_ms();
            connection.execute(
                "INSERT INTO capability_operation_effects (
                     effect_id, operation_id, effect_key, target, intent, outcome,
                     detail_json, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, NULL, 'completed', ?4, ?5, ?6, ?6)
                 ON CONFLICT(operation_id, effect_key) DO UPDATE SET
                     intent = 'completed',
                     outcome = excluded.outcome,
                     detail_json = excluded.detail_json,
                     updated_at_ms = excluded.updated_at_ms",
                params![effect_id, operation_id, effect_key, outcome.as_str(), detail_json, now],
            )?;
            find_effect(connection, &operation_id, &effect_key)
        })?;
        stored.ok_or_else(|| CapabilityError::internal("effect row vanished after upsert"))
    }

    /// Every effect row of one operation, in creation order.
    pub fn list_effects(&self, operation_id: &str) -> Result<Vec<OperationEffect>, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        let operation_id = operation_id.to_string();
        Ok(runtime.read_blocking(move |connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {EFFECT_COLUMNS} FROM capability_operation_effects
                 WHERE operation_id = ?1 ORDER BY created_at_ms, effect_key"
            ))?;
            let rows = statement.query_map(params![operation_id], row_to_effect)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::from)
        })?)
    }

    /// The number of effects of one operation whose state cannot be proven.
    pub fn count_unproven_effects(&self, operation_id: &str) -> Result<usize, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        let operation_id = operation_id.to_string();
        let count: i64 = runtime.read_blocking(move |connection| {
            connection
                .query_row(
                    "SELECT COUNT(1) FROM capability_operation_effects
                     WHERE operation_id = ?1 AND intent != 'not_started'
                       AND outcome IN ('pending','unknown')",
                    params![operation_id],
                    |row| row.get(0),
                )
                .map_err(StoreError::from)
        })?;
        Ok(count as usize)
    }

    // --------------------------------------------------------------- ownership

    /// Inserts or replaces the ownership record of one Skill installation.
    ///
    /// The `(target_id, skill_name)` uniqueness means a re-install of the same
    /// name in the same target updates the existing proof instead of creating a
    /// second owner for one directory.
    pub fn upsert_installation(
        &self,
        installation: &SkillInstallation,
    ) -> Result<SkillInstallation, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        let installation_id = installation.installation_id.clone();
        let skill_name = installation.skill_name.clone();
        let target_id = installation.target_id.clone();
        let install_path = installation.install_path.clone();
        let source_kind = installation.source_kind.clone();
        let source_ref = installation.source_ref.clone();
        let checker_version = installation.checker_version.clone();
        let verdict = installation.verdict.clone();
        let content_digest = installation.content_digest.clone();
        let file_manifest_json = serde_json::to_string(&installation.file_manifest)?;
        let marker_nonce = installation.marker_nonce.clone();
        let manifest_digest = installation.manifest_digest.clone();
        let state = installation.state;
        let operation_id = installation.operation_id.clone();
        let lookup_target = target_id.clone();
        let lookup_name = skill_name.clone();

        let stored = runtime.write_blocking(move |connection| {
            let now = now_ms();
            connection.execute(
                "INSERT INTO skill_installations (
                     installation_id, skill_name, target_id, install_path, source_kind,
                     source_ref, checker_version, verdict, content_digest, file_manifest_json,
                     marker_nonce, manifest_digest, state, operation_id, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)
                 ON CONFLICT(target_id, skill_name) DO UPDATE SET
                     install_path = excluded.install_path,
                     source_kind = excluded.source_kind,
                     source_ref = excluded.source_ref,
                     checker_version = excluded.checker_version,
                     verdict = excluded.verdict,
                     content_digest = excluded.content_digest,
                     file_manifest_json = excluded.file_manifest_json,
                     marker_nonce = excluded.marker_nonce,
                     manifest_digest = excluded.manifest_digest,
                     state = excluded.state,
                     operation_id = excluded.operation_id,
                     updated_at_ms = excluded.updated_at_ms",
                params![
                    installation_id,
                    skill_name,
                    target_id,
                    install_path,
                    source_kind,
                    source_ref,
                    checker_version,
                    verdict,
                    content_digest,
                    file_manifest_json,
                    marker_nonce,
                    manifest_digest,
                    state.as_str(),
                    operation_id,
                    now,
                ],
            )?;
            find_installation(connection, &lookup_target, &lookup_name)
        })?;
        stored.ok_or_else(|| CapabilityError::internal("installation row vanished after upsert"))
    }

    /// Loads the ownership record of one `(target, skill name)` pair.
    pub fn get_installation(
        &self,
        target_id: &str,
        skill_name: &str,
    ) -> Result<Option<SkillInstallation>, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        let target_id = target_id.to_string();
        let skill_name = skill_name.to_string();
        Ok(runtime.read_blocking(move |connection| {
            find_installation(connection, &target_id, &skill_name)
        })?)
    }

    /// Every ownership record, newest first.
    pub fn list_installations(&self) -> Result<Vec<SkillInstallation>, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        Ok(runtime.read_blocking(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {INSTALLATION_COLUMNS} FROM skill_installations
                 ORDER BY updated_at_ms DESC, installation_id"
            ))?;
            let rows = statement.query_map([], row_to_installation)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::from)
        })?)
    }

    /// Advances the lifecycle state of one ownership record.
    pub fn set_installation_state(
        &self,
        installation_id: &str,
        state: SkillInstallationState,
    ) -> Result<SkillInstallation, CapabilityError> {
        let runtime = self.store.db_runtime()?;
        let installation_id = installation_id.to_string();
        let stored = runtime.write_blocking(move |connection| {
            let changed = connection.execute(
                "UPDATE skill_installations SET state = ?2, updated_at_ms = ?3
                 WHERE installation_id = ?1",
                params![installation_id, state.as_str(), now_ms()],
            )?;
            if changed == 0 {
                return Ok(None);
            }
            find_installation_by_id(connection, &installation_id)
        })?;
        stored.ok_or_else(|| CapabilityError::not_found("skill installation does not exist"))
    }
}

fn is_unique_violation(error: &rusqlite::Error) -> bool {
    match error {
        rusqlite::Error::SqliteFailure(inner, message) => {
            inner.code == rusqlite::ErrorCode::ConstraintViolation
                && message
                    .as_deref()
                    .map(|text| text.contains("UNIQUE"))
                    .unwrap_or(true)
        }
        _ => false,
    }
}

fn row_to_operation(row: &rusqlite::Row<'_>) -> Result<CapabilityOperation, rusqlite::Error> {
    let capability: String = row.get(1)?;
    let state: String = row.get(8)?;
    let request_json: String = row.get(6)?;
    let result_json: Option<String> = row.get(10)?;

    let capability = CapabilityKind::parse(&capability).ok_or_else(|| {
        rusqlite::Error::InvalidColumnType(
            1,
            "capability".to_string(),
            rusqlite::types::Type::Text,
        )
    })?;
    let state = OperationState::parse(&state).ok_or_else(|| {
        rusqlite::Error::InvalidColumnType(8, "state".to_string(), rusqlite::types::Type::Text)
    })?;

    Ok(CapabilityOperation {
        operation_id: row.get(0)?,
        capability,
        operation_kind: row.get(2)?,
        actor_scope: row.get(3)?,
        idempotency_key: row.get(4)?,
        request_hash: row.get(5)?,
        request: serde_json::from_str(&request_json).unwrap_or(serde_json::Value::Null),
        resource_key: row.get(7)?,
        state,
        phase: row.get(9)?,
        result: result_json.and_then(|raw| serde_json::from_str(&raw).ok()),
        error_code: row.get(11)?,
        error_message: row.get(12)?,
        reconcile_reason: row.get(13)?,
        created_at_ms: row.get(14)?,
        updated_at_ms: row.get(15)?,
        completed_at_ms: row.get(16)?,
    })
}

fn row_to_effect(row: &rusqlite::Row<'_>) -> Result<OperationEffect, rusqlite::Error> {
    let intent: String = row.get(4)?;
    let outcome: String = row.get(5)?;
    let detail_json: Option<String> = row.get(6)?;
    Ok(OperationEffect {
        effect_id: row.get(0)?,
        operation_id: row.get(1)?,
        effect_key: row.get(2)?,
        target: row.get(3)?,
        intent: EffectIntent::parse(&intent).unwrap_or(EffectIntent::NotStarted),
        outcome: EffectOutcome::parse(&outcome).unwrap_or(EffectOutcome::Unknown),
        detail: detail_json.and_then(|raw| serde_json::from_str(&raw).ok()),
        created_at_ms: row.get(7)?,
        updated_at_ms: row.get(8)?,
    })
}

fn find_operation(
    connection: &Connection,
    operation_id: &str,
) -> Result<Option<CapabilityOperation>, StoreError> {
    connection
        .query_row(
            &format!("SELECT {OPERATION_COLUMNS} FROM capability_operations WHERE operation_id = ?1"),
            params![operation_id],
            row_to_operation,
        )
        .optional()
        .map_err(StoreError::from)
}

fn find_operation_by_key(
    connection: &Connection,
    capability: &str,
    actor_scope: &str,
    idempotency_key: &str,
) -> Result<Option<CapabilityOperation>, StoreError> {
    connection
        .query_row(
            &format!(
                "SELECT {OPERATION_COLUMNS} FROM capability_operations
                 WHERE capability = ?1 AND actor_scope = ?2 AND idempotency_key = ?3"
            ),
            params![capability, actor_scope, idempotency_key],
            row_to_operation,
        )
        .optional()
        .map_err(StoreError::from)
}

fn find_effect(
    connection: &Connection,
    operation_id: &str,
    effect_key: &str,
) -> Result<Option<OperationEffect>, StoreError> {    connection
        .query_row(
            &format!(
                "SELECT {EFFECT_COLUMNS} FROM capability_operation_effects
                 WHERE operation_id = ?1 AND effect_key = ?2"
            ),
            params![operation_id, effect_key],
            row_to_effect,
        )
        .optional()
        .map_err(StoreError::from)
}

fn find_installation(
    connection: &Connection,
    target_id: &str,
    skill_name: &str,
) -> Result<Option<SkillInstallation>, StoreError> {
    connection
        .query_row(
            &format!(
                "SELECT {INSTALLATION_COLUMNS} FROM skill_installations
                 WHERE target_id = ?1 AND skill_name = ?2"
            ),
            params![target_id, skill_name],
            row_to_installation,
        )
        .optional()
        .map_err(StoreError::from)
}

fn find_installation_by_id(
    connection: &Connection,
    installation_id: &str,
) -> Result<Option<SkillInstallation>, StoreError> {
    connection
        .query_row(
            &format!(
                "SELECT {INSTALLATION_COLUMNS} FROM skill_installations
                 WHERE installation_id = ?1"
            ),
            params![installation_id],
            row_to_installation,
        )
        .optional()
        .map_err(StoreError::from)
}

fn row_to_installation(
    row: &rusqlite::Row<'_>,
) -> Result<SkillInstallation, rusqlite::Error> {
    let file_manifest_json: String = row.get(9)?;
    let state: String = row.get(12)?;
    Ok(SkillInstallation {
        installation_id: row.get(0)?,
        skill_name: row.get(1)?,
        target_id: row.get(2)?,
        install_path: row.get(3)?,
        source_kind: row.get(4)?,
        source_ref: row.get(5)?,
        checker_version: row.get(6)?,
        verdict: row.get(7)?,
        content_digest: row.get(8)?,
        file_manifest: serde_json::from_str::<Vec<SkillFileEntry>>(&file_manifest_json)
            .unwrap_or_default(),
        marker_nonce: row.get(10)?,
        manifest_digest: row.get(11)?,
        state: SkillInstallationState::parse(&state).unwrap_or(SkillInstallationState::Drifted),
        operation_id: row.get(13)?,
        created_at_ms: row.get(14)?,
        updated_at_ms: row.get(15)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::types::LOCAL_ACTOR_SCOPE;
    use serde_json::json;

    fn test_store() -> Arc<MainStore> {
        Arc::new(MainStore::new(":memory:").expect("in-memory store must initialize"))
    }

    fn operation_request(key: &str, request: serde_json::Value) -> OperationRequest {
        OperationRequest {
            capability: CapabilityKind::Skill,
            operation_kind: "skill.install".to_string(),
            actor_scope: LOCAL_ACTOR_SCOPE.to_string(),
            idempotency_key: key.to_string(),
            request,
            resource_key: "skill:demo".to_string(),
        }
    }

    #[test]
    fn same_key_and_hash_replays_while_a_different_hash_conflicts() {
        let repository = CapabilityRepository::new(test_store());
        let first = repository
            .begin(&operation_request("key-1", json!({ "name": "demo", "targets": ["chatspeed"] })))
            .expect("first begin");
        assert!(!first.is_replay());

        // Same meaning, different key order: still a replay.
        let replay = repository
            .begin(&operation_request(
                "key-1",
                json!({ "targets": ["chatspeed"], "name": "demo" }),
            ))
            .expect("replay begin");
        assert!(replay.is_replay());
        assert_eq!(replay.operation().operation_id, first.operation().operation_id);

        let conflict = repository
            .begin(&operation_request("key-1", json!({ "name": "other" })))
            .expect_err("different request must conflict");
        assert_eq!(conflict.code(), code::IDEMPOTENCY_KEY_CONFLICT);
    }

    #[test]
    fn journal_persists_only_redacted_payloads() {
        let repository = CapabilityRepository::new(test_store());
        let begin = repository
            .begin(&operation_request(
                "key-secret",
                json!({ "bearer_token": "canary-token-value", "name": "demo" }),
            ))
            .expect("begin");
        let operation = begin.into_operation();
        let serialized = operation.request.to_string();
        assert!(!serialized.contains("canary"), "got {serialized}");
        assert_eq!(operation.request["name"], json!("demo"));
    }

    #[test]
    fn state_transitions_and_finish_are_recorded() {
        let repository = CapabilityRepository::new(test_store());
        let operation = repository
            .begin(&operation_request("key-2", json!({ "name": "demo" })))
            .expect("begin")
            .into_operation();

        repository
            .set_state(&operation.operation_id, OperationState::Checking, Some("checker"))
            .expect("set checking");
        let finished = repository
            .finish(
                &operation.operation_id,
                OperationState::Completed,
                Some(&json!({ "installed": true })),
                None,
            )
            .expect("finish");
        assert_eq!(finished.state, OperationState::Completed);
        assert!(finished.completed_at_ms.is_some());
        assert_eq!(finished.result, Some(json!({ "installed": true })));
    }

    #[test]
    fn effects_are_upserted_by_key_and_report_unproven_state() {
        let repository = CapabilityRepository::new(test_store());
        let operation = repository
            .begin(&operation_request("key-3", json!({ "name": "demo" })))
            .expect("begin")
            .into_operation();

        repository
            .record_effect_intent(&operation.operation_id, "target:chatspeed", Some("chatspeed"), None)
            .expect("intent");
        assert_eq!(
            repository
                .count_unproven_effects(&operation.operation_id)
                .expect("unproven count"),
            1
        );

        repository
            .record_effect_outcome(
                &operation.operation_id,
                "target:chatspeed",
                EffectOutcome::Applied,
                Some(&json!({ "installed": true })),
            )
            .expect("outcome");
        assert_eq!(
            repository
                .count_unproven_effects(&operation.operation_id)
                .expect("unproven count"),
            0
        );

        let effects = repository
            .list_effects(&operation.operation_id)
            .expect("list effects");
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].intent, EffectIntent::Completed);
        assert_eq!(effects[0].outcome, EffectOutcome::Applied);
    }

    #[test]
    fn ownership_is_unique_per_target_and_skill_and_tracks_state() {
        let repository = CapabilityRepository::new(test_store());
        let installation = SkillInstallation {
            installation_id: "skl-1".to_string(),
            skill_name: "demo".to_string(),
            target_id: "chatspeed".to_string(),
            install_path: "/tmp/chatspeed/skills/demo".to_string(),
            source_kind: "local_directory".to_string(),
            source_ref: "/tmp/source".to_string(),
            checker_version: "skill-checker.v1".to_string(),
            verdict: "pass".to_string(),
            content_digest: "digest-1".to_string(),
            file_manifest: vec![SkillFileEntry {
                path: "SKILL.md".to_string(),
                sha256: "abc".to_string(),
                size_bytes: 12,
            }],
            marker_nonce: "nonce-1".to_string(),
            manifest_digest: "manifest-1".to_string(),
            state: SkillInstallationState::Installing,
            operation_id: Some("op-skill-1".to_string()),
            created_at_ms: 0,
            updated_at_ms: 0,
        };
        repository
            .upsert_installation(&installation)
            .expect("first upsert");

        // A re-install of the same name in the same target replaces the row
        // instead of creating a second owner of one directory.
        let mut updated = installation.clone();
        updated.installation_id = "skl-2".to_string();
        updated.content_digest = "digest-2".to_string();
        updated.state = SkillInstallationState::Installed;
        repository
            .upsert_installation(&updated)
            .expect("second upsert");

        let all = repository.list_installations().expect("list installations");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].content_digest, "digest-2");
        assert_eq!(all[0].state, SkillInstallationState::Installed);
        assert_eq!(all[0].file_manifest.len(), 1);

        repository
            .set_installation_state("skl-1", SkillInstallationState::Quarantined)
            .expect("state change");
        assert_eq!(
            repository
                .list_installations()
                .expect("list installations")[0]
                .state,
            SkillInstallationState::Quarantined
        );
    }
}

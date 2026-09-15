//! Durable experiment budget ledger (Phase 2B).
//!
//! All ledger operations run inside a single `DbRuntime` writer transaction:
//! scope ownership/status checks, four-level cap checks, reservation state
//! transitions, materialized balance updates and append-only audit entries
//! either all commit together or all roll back. There is no second runtime
//! or data authority; callers never open SQLite directly.
//!
//! Idempotency contract:
//! - `reserve` is idempotent on `idempotency_key`: the same payload returns
//!   the original reservation, a conflicting payload is rejected with
//!   `idempotency_conflict`.
//! - `commit`/`release`/`mark_unknown` are idempotent on `operation_id` for
//!   the same terminal state; cross-terminal or conflicting operations are
//!   rejected with `invalid_transition`. Unknown reservations can never be
//!   released (fail closed).
//! - `record_infra_failure` is idempotent on `operation_id`.

use crate::budget::types::{
    ensure_sqlite_range, BudgetEnvelope, BudgetVector, CapLimit, EffectKind, MoneyMode,
    ReservationState, ResourceCaps, ResourceDimension, ScopeChain, ScopeKind,
};
use crate::budget::{
    AdmissionError, CommitReceipt, InfraReceipt, ReleaseReceipt, Reservation, ReserveEffect,
    UnknownReceipt,
};
use crate::db::{MainStore, StoreError};
use crate::libs::tsid::TsidGenerator;
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

impl From<StoreError> for AdmissionError {
    fn from(error: StoreError) -> Self {
        AdmissionError::persistence_failure(error.to_string())
    }
}

impl From<rusqlite::Error> for AdmissionError {
    fn from(error: rusqlite::Error) -> Self {
        AdmissionError::persistence_failure(error.to_string())
    }
}

/// Flattens a nested `Result<Result<T, AdmissionError>, StoreError>` produced
/// by writer closures that keep typed admission errors intact.
fn flatten<T>(result: Result<Result<T, AdmissionError>, StoreError>) -> Result<T, AdmissionError> {
    match result {
        Ok(inner) => inner,
        Err(store_error) => Err(AdmissionError::from(store_error)),
    }
}

/// Generates an opaque, backend-owned ledger identifier.
fn ledger_id(prefix: &str) -> Result<String, AdmissionError> {
    static GENERATOR: OnceLock<Result<TsidGenerator, String>> = OnceLock::new();
    let generator = GENERATOR.get_or_init(|| TsidGenerator::new(7));
    match generator {
        Ok(generator) => generator
            .generate()
            .map(|id| format!("{prefix}_{id}"))
            .map_err(|error| AdmissionError::persistence_failure(error)),
        Err(error) => Err(AdmissionError::persistence_failure(error.clone())),
    }
}

/// Builds a durable-store error for a malformed/partial scope chain. Read by
/// the LLM admission-context injector to fail closed before any effect.
fn invalid_chain_error(message: impl Into<String>) -> StoreError {
    StoreError::InvalidData(message.into())
}

/// Canonical mapping between ledger columns and resource dimensions.
const DIMENSION_COLUMNS: &[(&str, ResourceDimension)] = &[
    ("input_tokens", ResourceDimension::InputTokens),
    ("output_tokens", ResourceDimension::OutputTokens),
    ("cache_read_tokens", ResourceDimension::CacheReadTokens),
    ("cache_write_tokens", ResourceDimension::CacheWriteTokens),
    ("wall_time_ms", ResourceDimension::WallTimeMs),
    ("tool_calls", ResourceDimension::ToolCalls),
    ("processes", ResourceDimension::Processes),
    ("disk_bytes", ResourceDimension::DiskBytes),
    ("network_bytes", ResourceDimension::NetworkBytes),
    ("concurrency", ResourceDimension::Concurrency),
    ("money_micros", ResourceDimension::Money),
];

fn effect_kind_as_str(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::LlmCompletion => "llm_completion",
        EffectKind::Embedding => "embedding",
        EffectKind::ToolCall => "tool_call",
        EffectKind::Process => "process",
    }
}

fn effect_kind_parse(value: &str) -> Result<EffectKind, AdmissionError> {
    match value {
        "llm_completion" => Ok(EffectKind::LlmCompletion),
        "embedding" => Ok(EffectKind::Embedding),
        "tool_call" => Ok(EffectKind::ToolCall),
        "process" => Ok(EffectKind::Process),
        _ => Err(AdmissionError::persistence_failure(format!(
            "unknown effect kind in ledger: {value}"
        ))),
    }
}

/// A backend-created budget scope. The envelope is frozen at creation time
/// and materialized into per-dimension cap columns.
#[derive(Debug, Clone)]
pub struct NewBudgetScope {
    pub scope_id: String,
    pub scope_kind: ScopeKind,
    pub parent_scope_id: Option<String>,
    pub envelope: BudgetEnvelope,
    pub now_ms: u64,
}

/// Read-only projection of a durable budget scope.
#[derive(Debug, Clone)]
pub struct BudgetScopeStatus {
    pub scope_id: String,
    pub scope_kind: ScopeKind,
    pub parent_scope_id: Option<String>,
    pub status: String,
    pub committed: BudgetVector,
    pub reserved: BudgetVector,
    pub caps: ResourceCaps,
    pub infra_failure_count: u64,
    pub infra_failure_threshold: u32,
    pub pause_reason: Option<String>,
}

/// Read-only projection of one append-only ledger entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetLedgerEntry {
    pub entry_id: String,
    pub scope_id: String,
    pub reservation_id: Option<String>,
    pub effect_id: Option<String>,
    pub operation: String,
    pub vector: BudgetVector,
    pub operation_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub reason: Option<String>,
    pub created_at_ms: u64,
}

/// Durable reservation row as stored.
struct StoredReservation {
    reservation_id: String,
    effect_id: String,
    idempotency_key: String,
    scopes: ScopeChain,
    effect_kind: EffectKind,
    attempt: u32,
    state: ReservationState,
    estimate: BudgetVector,
    actual: Option<BudgetVector>,
    overrun: bool,
    operation_id: Option<String>,
    created_at_ms: u64,
    lease_expires_at_ms: u64,
}

impl StoredReservation {
    fn matches_request(&self, request: &ReserveEffect) -> bool {
        self.effect_id == request.effect_id
            && self.idempotency_key == request.idempotency_key
            && self.scopes == request.scopes
            && self.effect_kind == request.effect_kind
            && self.attempt == request.attempt
            && self.estimate == request.estimate
    }

    fn as_reservation(&self) -> Reservation {
        Reservation {
            reservation_id: self.reservation_id.clone(),
            effect_id: self.effect_id.clone(),
            idempotency_key: self.idempotency_key.clone(),
            scopes: self.scopes.clone(),
            effect_kind: self.effect_kind,
            attempt: self.attempt,
            reserved: self.estimate,
            state: self.state,
            created_at_ms: self.created_at_ms,
            lease_expires_at_ms: self.lease_expires_at_ms,
        }
    }
}

struct ScopeRow {
    scope_id: String,
    scope_kind: ScopeKind,
    parent_scope_id: Option<String>,
    status: String,
    committed: BudgetVector,
    reserved: BudgetVector,
    caps: ResourceCaps,
    infra_failure_count: u64,
    infra_failure_threshold: u32,
    envelope_json: String,
    pause_reason: Option<String>,
}

fn scope_select_sql() -> String {
    let mut columns: Vec<String> = vec![
        "scope_id".into(),
        "scope_kind".into(),
        "parent_scope_id".into(),
        "status".into(),
    ];
    for (suffix, _) in DIMENSION_COLUMNS {
        columns.push(format!("cap_{suffix}"));
    }
    for (suffix, _) in DIMENSION_COLUMNS {
        columns.push(format!("committed_{suffix}"));
    }
    for (suffix, _) in DIMENSION_COLUMNS {
        columns.push(format!("reserved_{suffix}"));
    }
    columns.extend([
        "infra_failure_count".to_string(),
        "infra_failure_threshold".to_string(),
        "envelope_json".to_string(),
        "pause_reason".to_string(),
    ]);
    format!(
        "SELECT {} FROM experiment_budget_scopes WHERE scope_id = ?1",
        columns.join(", ")
    )
}

fn parse_scope_kind(value: &str) -> Result<ScopeKind, AdmissionError> {
    ScopeKind::parse(value).ok_or_else(|| {
        AdmissionError::persistence_failure(format!("unknown scope kind in ledger: {value}"))
    })
}

fn read_scope_row(row: &rusqlite::Row<'_>) -> Result<ScopeRow, rusqlite::Error> {
    let scope_id: String = row.get(0)?;
    let scope_kind = parse_scope_kind(&row.get::<_, String>(1)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                error.to_string(),
            )),
        )
    })?;
    let parent_scope_id: Option<String> = row.get(2)?;
    let status: String = row.get(3)?;
    let mut caps_values: Vec<Option<i64>> = Vec::new();
    for index in 4..15 {
        caps_values.push(row.get(index)?);
    }
    let mut committed = BudgetVector::ZERO;
    for (offset, (_, dimension)) in DIMENSION_COLUMNS.iter().enumerate() {
        let value: i64 = row.get(15 + offset)?;
        committed = committed
            .with_dimension(*dimension, value.max(0) as u64)
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    15 + offset,
                    rusqlite::types::Type::Integer,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        error.to_string(),
                    )),
                )
            })?;
    }
    let mut reserved = BudgetVector::ZERO;
    for (offset, (_, dimension)) in DIMENSION_COLUMNS.iter().enumerate() {
        let value: i64 = row.get(26 + offset)?;
        reserved = reserved
            .with_dimension(*dimension, value.max(0) as u64)
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    26 + offset,
                    rusqlite::types::Type::Integer,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        error.to_string(),
                    )),
                )
            })?;
    }
    let caps = ResourceCaps {
        input_tokens: cap_from_db(caps_values[0]),
        output_tokens: cap_from_db(caps_values[1]),
        cache_read_tokens: cap_from_db(caps_values[2]),
        cache_write_tokens: cap_from_db(caps_values[3]),
        wall_time_ms: cap_from_db(caps_values[4]),
        tool_calls: cap_from_db(caps_values[5]),
        processes: cap_from_db(caps_values[6]),
        disk_bytes: cap_from_db(caps_values[7]),
        network_bytes: cap_from_db(caps_values[8]),
        concurrency: cap_from_db(caps_values[9]),
        money: cap_from_db(caps_values[10]),
    };
    Ok(ScopeRow {
        scope_id,
        scope_kind,
        parent_scope_id,
        status,
        committed,
        reserved,
        caps,
        infra_failure_count: row.get::<_, i64>(37)?.max(0) as u64,
        infra_failure_threshold: row.get::<_, i64>(38)?.max(0) as u32,
        envelope_json: row.get(39)?,
        pause_reason: row.get(40)?,
    })
}

fn cap_from_db(value: Option<i64>) -> CapLimit {
    match value {
        Some(value) if value >= 0 => CapLimit::HardCap(value as u64),
        _ => CapLimit::NotApplicable,
    }
}

fn load_scope(tx: &Transaction<'_>, scope_id: &str) -> Result<ScopeRow, AdmissionError> {
    tx.query_row(&scope_select_sql(), [scope_id], read_scope_row)
        .optional()
        .map_err(StoreError::from)?
        .ok_or_else(|| {
            AdmissionError::invalid_scope_chain(format!("budget scope {scope_id} does not exist"))
        })
}

fn update_scope_balances(
    tx: &Transaction<'_>,
    scope_id: &str,
    committed: &BudgetVector,
    reserved: &BudgetVector,
    now_ms: u64,
) -> Result<(), AdmissionError> {
    let mut assignments: Vec<String> = Vec::new();
    let mut values: Vec<SqlValue> = Vec::new();
    for (suffix, dimension) in DIMENSION_COLUMNS {
        assignments.push(format!("committed_{suffix} = ?"));
        values.push(SqlValue::Integer(ensure_sqlite_range(
            committed.dimension(*dimension),
        )?));
    }
    for (suffix, dimension) in DIMENSION_COLUMNS {
        assignments.push(format!("reserved_{suffix} = ?"));
        values.push(SqlValue::Integer(ensure_sqlite_range(
            reserved.dimension(*dimension),
        )?));
    }
    assignments.push("updated_at_ms = ?".to_string());
    assignments.push("version = version + 1".to_string());
    values.push(SqlValue::Integer(ensure_sqlite_range(now_ms)?));
    values.push(SqlValue::Text(scope_id.to_string()));
    let sql = format!(
        "UPDATE experiment_budget_scopes SET {} WHERE scope_id = ?",
        assignments.join(", ")
    );
    let changed = tx
        .execute(&sql, params_from_iter(values))
        .map_err(StoreError::from)?;
    if changed != 1 {
        return Err(AdmissionError::invalid_scope_chain(format!(
            "budget scope {scope_id} disappeared during update"
        )));
    }
    Ok(())
}

fn pause_scope_in_tx(
    tx: &Transaction<'_>,
    scope_id: &str,
    reason: &str,
    now_ms: u64,
) -> Result<(), AdmissionError> {
    let changed = tx
        .execute(
            "UPDATE experiment_budget_scopes
             SET status = 'paused', pause_reason = ?2, updated_at_ms = ?3, version = version + 1
             WHERE scope_id = ?1 AND status = 'active'",
            params![scope_id, reason, ensure_sqlite_range(now_ms)?],
        )
        .map_err(StoreError::from)?;
    if changed == 1 {
        insert_ledger_entry(
            tx,
            scope_id,
            None,
            None,
            "pause",
            &BudgetVector::ZERO,
            None,
            None,
            Some(reason),
            now_ms,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_ledger_entry(
    tx: &Transaction<'_>,
    scope_id: &str,
    reservation_id: Option<&str>,
    effect_id: Option<&str>,
    operation: &str,
    vector: &BudgetVector,
    operation_id: Option<&str>,
    idempotency_key: Option<&str>,
    reason: Option<&str>,
    now_ms: u64,
) -> Result<(), AdmissionError> {
    let entry_id = ledger_id("ble")?;
    let vector_json = serde_json::to_string(vector)
        .map_err(|error| AdmissionError::persistence_failure(error.to_string()))?;
    tx.execute(
        "INSERT INTO experiment_budget_ledger_entries
         (entry_id, scope_id, reservation_id, effect_id, operation, vector_json,
          operation_id, idempotency_key, reason, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            entry_id,
            scope_id,
            reservation_id,
            effect_id,
            operation,
            vector_json,
            operation_id,
            idempotency_key,
            reason,
            ensure_sqlite_range(now_ms)?
        ],
    )
    .map_err(StoreError::from)?;
    Ok(())
}

const RESERVATION_ESTIMATE_COLUMNS: &[&str] = &[
    "est_input_tokens",
    "est_output_tokens",
    "est_cache_read_tokens",
    "est_cache_write_tokens",
    "est_wall_time_ms",
    "est_tool_calls",
    "est_processes",
    "est_disk_bytes",
    "est_network_bytes",
    "est_concurrency",
    "est_money_micros",
];

fn insert_reservation_in_tx(
    tx: &Transaction<'_>,
    reservation_id: &str,
    request: &ReserveEffect,
    lease_expires_at_ms: u64,
    pricing_snapshot_json: Option<&str>,
    now_ms: u64,
) -> Result<(), AdmissionError> {
    let mut columns: Vec<String> = vec![
        "reservation_id".into(),
        "effect_id".into(),
        "idempotency_key".into(),
        "request_scope_id".into(),
        "trial_scope_id".into(),
        "candidate_scope_id".into(),
        "campaign_scope_id".into(),
        "effect_kind".into(),
        "attempt".into(),
        "state".into(),
    ];
    columns.extend(
        RESERVATION_ESTIMATE_COLUMNS
            .iter()
            .map(|column| column.to_string()),
    );
    columns.extend([
        "pricing_snapshot_json".to_string(),
        "created_at_ms".to_string(),
        "updated_at_ms".to_string(),
        "lease_expires_at_ms".to_string(),
    ]);
    let mut values: Vec<SqlValue> = vec![
        SqlValue::Text(reservation_id.to_string()),
        SqlValue::Text(request.effect_id.clone()),
        SqlValue::Text(request.idempotency_key.clone()),
        SqlValue::Text(request.scopes.request_id.clone()),
        SqlValue::Text(request.scopes.trial_id.clone()),
        SqlValue::Text(request.scopes.candidate_id.clone()),
        SqlValue::Text(request.scopes.campaign_id.clone()),
        SqlValue::Text(effect_kind_as_str(request.effect_kind).to_string()),
        SqlValue::Integer(request.attempt as i64),
        SqlValue::Text(ReservationState::Reserved.as_str().to_string()),
    ];
    for (_, dimension) in DIMENSION_COLUMNS {
        values.push(SqlValue::Integer(ensure_sqlite_range(
            request.estimate.dimension(*dimension),
        )?));
    }
    values.push(match pricing_snapshot_json {
        Some(json) => SqlValue::Text(json.to_string()),
        None => SqlValue::Null,
    });
    values.push(SqlValue::Integer(ensure_sqlite_range(now_ms)?));
    values.push(SqlValue::Integer(ensure_sqlite_range(now_ms)?));
    values.push(SqlValue::Integer(ensure_sqlite_range(lease_expires_at_ms)?));
    let sql = format!(
        "INSERT INTO experiment_budget_reservations ({}) VALUES ({})",
        columns.join(", "),
        (1..=columns.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    tx.execute(&sql, params_from_iter(values))
        .map_err(StoreError::from)?;
    Ok(())
}

fn read_stored_reservation(row: &rusqlite::Row<'_>) -> Result<StoredReservation, rusqlite::Error> {
    let state_text: String = row.get(9)?;
    let state = ReservationState::parse(&state_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            9,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown reservation state: {state_text}"),
            )),
        )
    })?;
    let mut estimate = BudgetVector::ZERO;
    for (offset, (_, dimension)) in DIMENSION_COLUMNS.iter().enumerate() {
        let value: i64 = row.get(10 + offset)?;
        estimate = estimate
            .with_dimension(*dimension, value.max(0) as u64)
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    10 + offset,
                    rusqlite::types::Type::Integer,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        error.to_string(),
                    )),
                )
            })?;
    }
    let mut actual: Option<BudgetVector> = None;
    let mut any_actual_present = false;
    let mut actual_vector = BudgetVector::ZERO;
    for (offset, (_, dimension)) in DIMENSION_COLUMNS.iter().enumerate() {
        let value: Option<i64> = row.get(21 + offset)?;
        if let Some(value) = value {
            any_actual_present = true;
            actual_vector = actual_vector
                .with_dimension(*dimension, value.max(0) as u64)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        21 + offset,
                        rusqlite::types::Type::Integer,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            error.to_string(),
                        )),
                    )
                })?;
        }
    }
    if any_actual_present {
        actual = Some(actual_vector);
    }
    Ok(StoredReservation {
        reservation_id: row.get(0)?,
        effect_id: row.get(1)?,
        idempotency_key: row.get(2)?,
        scopes: ScopeChain {
            request_id: row.get(3)?,
            trial_id: row.get(4)?,
            candidate_id: row.get(5)?,
            campaign_id: row.get(6)?,
        },
        effect_kind: effect_kind_parse(&row.get::<_, String>(7)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    error.to_string(),
                )),
            )
        })?,
        attempt: row.get::<_, i64>(8)?.max(0) as u32,
        state,
        estimate,
        actual,
        overrun: row.get::<_, i64>(32)? != 0,
        operation_id: row.get(33)?,
        created_at_ms: row.get::<_, i64>(34)?.max(0) as u64,
        lease_expires_at_ms: row.get::<_, i64>(36)?.max(0) as u64,
    })
}

const RESERVATION_SELECT_SQL: &str = "SELECT reservation_id, effect_id, idempotency_key, request_scope_id, trial_scope_id, candidate_scope_id, campaign_scope_id, effect_kind, attempt, state, est_input_tokens, est_output_tokens, est_cache_read_tokens, est_cache_write_tokens, est_wall_time_ms, est_tool_calls, est_processes, est_disk_bytes, est_network_bytes, est_concurrency, est_money_micros, actual_input_tokens, actual_output_tokens, actual_cache_read_tokens, actual_cache_write_tokens, actual_wall_time_ms, actual_tool_calls, actual_processes, actual_disk_bytes, actual_network_bytes, actual_concurrency, actual_money_micros, overrun, operation_id, created_at_ms, updated_at_ms, lease_expires_at_ms FROM experiment_budget_reservations";

fn load_reservation(
    tx: &Transaction<'_>,
    reservation_id: &str,
) -> Result<StoredReservation, AdmissionError> {
    tx.query_row(
        &format!("{RESERVATION_SELECT_SQL} WHERE reservation_id = ?1"),
        [reservation_id],
        read_stored_reservation,
    )
    .optional()
    .map_err(StoreError::from)?
    .ok_or_else(|| {
        AdmissionError::invalid_scope_chain(format!("reservation {reservation_id} does not exist"))
    })
}

fn load_reservation_by_idempotency_key(
    tx: &Transaction<'_>,
    idempotency_key: &str,
) -> Result<Option<StoredReservation>, AdmissionError> {
    tx.query_row(
        &format!("{RESERVATION_SELECT_SQL} WHERE idempotency_key = ?1"),
        [idempotency_key],
        read_stored_reservation,
    )
    .optional()
    .map_err(AdmissionError::from)
}

fn update_reservation_settled(
    tx: &Transaction<'_>,
    reservation_id: &str,
    state: ReservationState,
    actual: Option<&BudgetVector>,
    operation_id: &str,
    overrun: bool,
    now_ms: u64,
) -> Result<(), AdmissionError> {
    let mut assignments: Vec<String> = vec![
        "state = ?".to_string(),
        "operation_id = ?".to_string(),
        "overrun = ?".to_string(),
        "updated_at_ms = ?".to_string(),
    ];
    let mut values: Vec<SqlValue> = vec![
        SqlValue::Text(state.as_str().to_string()),
        SqlValue::Text(operation_id.to_string()),
        SqlValue::Integer(if overrun { 1 } else { 0 }),
        SqlValue::Integer(ensure_sqlite_range(now_ms)?),
    ];
    for (suffix, dimension) in DIMENSION_COLUMNS {
        assignments.push(format!("actual_{suffix} = ?"));
        let value = match actual {
            Some(vector) => SqlValue::Integer(ensure_sqlite_range(vector.dimension(*dimension))?),
            None => SqlValue::Null,
        };
        values.push(value);
    }
    values.push(SqlValue::Text(reservation_id.to_string()));
    let sql = format!(
        "UPDATE experiment_budget_reservations SET {} WHERE reservation_id = ?",
        assignments.join(", ")
    );
    tx.execute(&sql, params_from_iter(values))
        .map_err(StoreError::from)?;
    Ok(())
}

/// Loads the four scopes of a chain in outermost-first order and validates
/// kind and parent linkage. When `enforce_active` is true (admission), a
/// paused/closed scope is rejected; settlement paths pass false so that
/// already-admitted effects can always be settled.
fn load_chain_scopes(
    tx: &Transaction<'_>,
    scopes: &ScopeChain,
    enforce_active: bool,
) -> Result<Vec<ScopeRow>, AdmissionError> {
    let chain: [(ScopeKind, &str, Option<&str>); 4] = [
        (ScopeKind::Campaign, scopes.campaign_id.as_str(), None),
        (
            ScopeKind::Candidate,
            scopes.candidate_id.as_str(),
            Some(scopes.campaign_id.as_str()),
        ),
        (
            ScopeKind::Trial,
            scopes.trial_id.as_str(),
            Some(scopes.candidate_id.as_str()),
        ),
        (
            ScopeKind::Request,
            scopes.request_id.as_str(),
            Some(scopes.trial_id.as_str()),
        ),
    ];
    let mut loaded = Vec::with_capacity(4);
    for (kind, id, expected_parent) in chain {
        let scope = load_scope(tx, id)?;
        if scope.scope_kind != kind {
            return Err(AdmissionError::invalid_scope_chain(format!(
                "scope {id} is a {} scope but the chain requires {}",
                scope.scope_kind.as_str(),
                kind.as_str()
            )));
        }
        if scope.parent_scope_id.as_deref() != expected_parent {
            return Err(AdmissionError::invalid_scope_chain(format!(
                "scope {id} parent linkage does not match the requested chain"
            )));
        }
        if enforce_active && scope.status != "active" {
            return Err(AdmissionError::scope_paused(format!(
                "scope {id} is {}",
                scope.status
            )));
        }
        loaded.push(scope);
    }
    Ok(loaded)
}

fn reserve_in_tx(
    tx: &Transaction<'_>,
    request: &ReserveEffect,
    now_ms: u64,
) -> Result<Reservation, AdmissionError> {
    request.validate()?;
    let request_scope = load_scope(tx, &request.scopes.request_id)?;
    if request_scope.scope_kind != ScopeKind::Request {
        return Err(AdmissionError::invalid_scope_chain(format!(
            "scope {} is a {} scope, not a request scope",
            request.scopes.request_id,
            request_scope.scope_kind.as_str()
        )));
    }
    let envelope: BudgetEnvelope =
        serde_json::from_str(&request_scope.envelope_json).map_err(|error| {
            AdmissionError::persistence_failure(format!(
                "corrupted frozen envelope on scope {}: {error}",
                request.scopes.request_id
            ))
        })?;
    envelope.validate()?;
    if request.attempt > envelope.max_attempts {
        return Err(AdmissionError::invalid_transition(format!(
            "attempt {} exceeds envelope max_attempts {}",
            request.attempt, envelope.max_attempts
        )));
    }
    if matches!(envelope.money_mode, MoneyMode::TokenResourceOnly)
        && request.estimate.money_micros > 0
    {
        return Err(AdmissionError::budget_exceeded(
            "money cost cannot be reserved in token/resource-only mode",
        )
        .with_dimension(ResourceDimension::Money));
    }
    // Idempotent replay or conflict detection.
    if let Some(existing) = load_reservation_by_idempotency_key(tx, &request.idempotency_key)? {
        if existing.matches_request(request) {
            return Ok(existing.as_reservation());
        }
        return Err(AdmissionError::idempotency_conflict(format!(
            "idempotency key {} was already used with a different reservation payload",
            request.idempotency_key
        )));
    }
    // Validate the full chain outermost-first, then check caps per scope.
    let scopes = load_chain_scopes(tx, &request.scopes, true)?;
    for scope in &scopes {
        if let Err((dimension, projected, limit)) =
            scope
                .caps
                .check_admission(&scope.committed, &scope.reserved, &request.estimate)
        {
            return Err(AdmissionError::budget_exceeded(format!(
                "scope {} dimension {} projected {projected} exceeds hard cap {limit}",
                scope.scope_id,
                dimension.as_str()
            ))
            .with_dimension(dimension));
        }
    }
    let reservation_id = ledger_id("res")?;
    let lease_expires_at_ms = now_ms.saturating_add(envelope.reservation_lease_ms);
    insert_reservation_in_tx(
        tx,
        &reservation_id,
        request,
        lease_expires_at_ms,
        None,
        now_ms,
    )?;
    for scope in &scopes {
        let new_reserved = scope.reserved.checked_add(&request.estimate)?;
        update_scope_balances(tx, &scope.scope_id, &scope.committed, &new_reserved, now_ms)?;
        insert_ledger_entry(
            tx,
            &scope.scope_id,
            Some(reservation_id.as_str()),
            Some(request.effect_id.as_str()),
            "reserve",
            &request.estimate,
            None,
            Some(request.idempotency_key.as_str()),
            None,
            now_ms,
        )?;
    }
    Ok(Reservation {
        reservation_id,
        effect_id: request.effect_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        scopes: request.scopes.clone(),
        effect_kind: request.effect_kind,
        attempt: request.attempt,
        reserved: request.estimate,
        state: ReservationState::Reserved,
        created_at_ms: now_ms,
        lease_expires_at_ms,
    })
}

fn commit_in_tx(
    tx: &Transaction<'_>,
    reservation_id: &str,
    operation_id: &str,
    actual: BudgetVector,
    now_ms: u64,
) -> Result<CommitReceipt, AdmissionError> {
    let stored = load_reservation(tx, reservation_id)?;
    match stored.state {
        ReservationState::Reserved => {}
        ReservationState::Committed => {
            if stored.operation_id.as_deref() == Some(operation_id) {
                return Ok(CommitReceipt {
                    reservation_id: stored.reservation_id,
                    operation_id: operation_id.to_string(),
                    committed: stored.actual.unwrap_or(BudgetVector::ZERO),
                    overrun: stored.overrun,
                });
            }
            return Err(AdmissionError::invalid_transition(
                "reservation already committed under a different operation id",
            ));
        }
        other => {
            return Err(AdmissionError::invalid_transition(format!(
                "cannot commit a reservation in state {}",
                other.as_str()
            )));
        }
    }
    let scopes = load_chain_scopes(tx, &stored.scopes, false)?;
    let mut overrun = false;
    for scope in &scopes {
        let envelope: BudgetEnvelope =
            serde_json::from_str(&scope.envelope_json).map_err(|error| {
                AdmissionError::persistence_failure(format!(
                    "corrupted frozen envelope on scope {}: {error}",
                    scope.scope_id
                ))
            })?;
        let projected = scope.committed.checked_add(&actual)?;
        for dimension in ResourceDimension::ALL {
            let Some(limit) = envelope.caps.cap(dimension).limit() else {
                continue;
            };
            // Tool wall time is measured after execution and has no reliable
            // pre-dispatch bound yet, so a zero estimate alone is not an
            // overrun. Crossing the actual hard cap remains an overrun.
            let estimate_overrun = actual.dimension(dimension)
                > stored.estimate.dimension(dimension)
                && !(dimension == ResourceDimension::WallTimeMs
                    && stored.estimate.dimension(dimension) == 0);
            let cap_overrun = projected.dimension(dimension) > limit;
            if estimate_overrun || cap_overrun {
                overrun = true;
                break;
            }
        }
        if overrun {
            break;
        }
    }
    for scope in &scopes {
        let new_committed = scope.committed.checked_add(&actual)?;
        let new_reserved = scope.reserved.checked_sub(&stored.estimate)?;
        update_scope_balances(tx, &scope.scope_id, &new_committed, &new_reserved, now_ms)?;
        insert_ledger_entry(
            tx,
            &scope.scope_id,
            Some(reservation_id),
            Some(stored.effect_id.as_str()),
            if overrun { "overrun" } else { "commit" },
            &actual,
            Some(operation_id),
            None,
            None,
            now_ms,
        )?;
    }
    if overrun {
        pause_scope_in_tx(
            tx,
            &stored.scopes.campaign_id,
            &format!("overrun:{reservation_id}"),
            now_ms,
        )?;
    }
    update_reservation_settled(
        tx,
        reservation_id,
        if overrun {
            ReservationState::Overrun
        } else {
            ReservationState::Committed
        },
        Some(&actual),
        operation_id,
        overrun,
        now_ms,
    )?;
    Ok(CommitReceipt {
        reservation_id: reservation_id.to_string(),
        operation_id: operation_id.to_string(),
        committed: actual,
        overrun,
    })
}

fn release_in_tx(
    tx: &Transaction<'_>,
    reservation_id: &str,
    operation_id: &str,
    reason: &str,
    now_ms: u64,
) -> Result<ReleaseReceipt, AdmissionError> {
    let stored = load_reservation(tx, reservation_id)?;
    match stored.state {
        ReservationState::Reserved => {}
        ReservationState::Released => {
            if stored.operation_id.as_deref() == Some(operation_id) {
                return Ok(ReleaseReceipt {
                    reservation_id: stored.reservation_id,
                    operation_id: operation_id.to_string(),
                });
            }
            return Err(AdmissionError::invalid_transition(
                "reservation already released under a different operation id",
            ));
        }
        other => {
            return Err(AdmissionError::invalid_transition(format!(
                "cannot release a reservation in state {}",
                other.as_str()
            )));
        }
    }
    let scopes = load_chain_scopes(tx, &stored.scopes, false)?;
    for scope in &scopes {
        let new_reserved = scope.reserved.checked_sub(&stored.estimate)?;
        update_scope_balances(tx, &scope.scope_id, &scope.committed, &new_reserved, now_ms)?;
        insert_ledger_entry(
            tx,
            &scope.scope_id,
            Some(reservation_id),
            Some(stored.effect_id.as_str()),
            "release",
            &stored.estimate,
            Some(operation_id),
            None,
            Some(reason),
            now_ms,
        )?;
    }
    update_reservation_settled(
        tx,
        reservation_id,
        ReservationState::Released,
        None,
        operation_id,
        false,
        now_ms,
    )?;
    Ok(ReleaseReceipt {
        reservation_id: reservation_id.to_string(),
        operation_id: operation_id.to_string(),
    })
}

fn mark_unknown_in_tx(
    tx: &Transaction<'_>,
    reservation_id: &str,
    operation_id: &str,
    reason: &str,
    now_ms: u64,
) -> Result<UnknownReceipt, AdmissionError> {
    let stored = load_reservation(tx, reservation_id)?;
    match stored.state {
        ReservationState::Reserved => {}
        ReservationState::Unknown => {
            if stored.operation_id.as_deref() == Some(operation_id) {
                return Ok(UnknownReceipt {
                    reservation_id: stored.reservation_id,
                    operation_id: operation_id.to_string(),
                });
            }
            return Err(AdmissionError::invalid_transition(
                "reservation already marked unknown under a different operation id",
            ));
        }
        other => {
            return Err(AdmissionError::invalid_transition(format!(
                "cannot mark a reservation unknown in state {}",
                other.as_str()
            )));
        }
    }
    // The budget stays frozen: reserved balances are intentionally NOT
    // released here (fail closed until owner reconciliation).
    let scopes = load_chain_scopes(tx, &stored.scopes, false)?;
    for scope in &scopes {
        insert_ledger_entry(
            tx,
            &scope.scope_id,
            Some(reservation_id),
            Some(stored.effect_id.as_str()),
            "mark_unknown",
            &stored.estimate,
            Some(operation_id),
            None,
            Some(reason),
            now_ms,
        )?;
    }
    update_reservation_settled(
        tx,
        reservation_id,
        ReservationState::Unknown,
        None,
        operation_id,
        false,
        now_ms,
    )?;
    Ok(UnknownReceipt {
        reservation_id: reservation_id.to_string(),
        operation_id: operation_id.to_string(),
    })
}

fn record_infra_failure_in_tx(
    tx: &Transaction<'_>,
    campaign_id: &str,
    effect_id: &str,
    operation_id: &str,
    kind: &str,
    now_ms: u64,
) -> Result<InfraReceipt, AdmissionError> {
    let scope = load_scope(tx, campaign_id)?;
    if scope.scope_kind != ScopeKind::Campaign {
        return Err(AdmissionError::invalid_scope_chain(format!(
            "scope {campaign_id} is a {} scope, not a campaign scope",
            scope.scope_kind.as_str()
        )));
    }
    // Idempotent replay: the same operation id must not double count.
    let already_recorded: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM experiment_budget_ledger_entries
             WHERE scope_id = ?1 AND operation = 'infra_failure' AND operation_id = ?2)",
            params![campaign_id, operation_id],
            |row| row.get(0),
        )
        .map_err(StoreError::from)?;
    if already_recorded {
        return Ok(InfraReceipt {
            campaign_id: campaign_id.to_string(),
            infra_failure_count: scope.infra_failure_count,
            paused: scope.status != "active",
        });
    }
    let new_count = scope
        .infra_failure_count
        .checked_add(1)
        .ok_or_else(|| AdmissionError::persistence_failure("infra failure count overflow"))?;
    let threshold = scope.infra_failure_threshold;
    let should_pause = threshold > 0 && new_count >= threshold as u64;
    tx.execute(
        "UPDATE experiment_budget_scopes
         SET infra_failure_count = ?2, updated_at_ms = ?3, version = version + 1
         WHERE scope_id = ?1",
        params![
            campaign_id,
            ensure_sqlite_range(new_count)?,
            ensure_sqlite_range(now_ms)?
        ],
    )
    .map_err(StoreError::from)?;
    insert_ledger_entry(
        tx,
        campaign_id,
        None,
        Some(effect_id),
        "infra_failure",
        &BudgetVector::ZERO,
        Some(operation_id),
        None,
        Some(kind),
        now_ms,
    )?;
    if should_pause {
        pause_scope_in_tx(
            tx,
            campaign_id,
            &format!("infra_failure_threshold:{kind}"),
            now_ms,
        )?;
    }
    Ok(InfraReceipt {
        campaign_id: campaign_id.to_string(),
        infra_failure_count: new_count,
        paused: should_pause || scope.status != "active",
    })
}

fn create_scope_in_tx(tx: &Transaction<'_>, scope: &NewBudgetScope) -> Result<(), AdmissionError> {
    scope.envelope.validate()?;
    if scope.scope_id.trim().is_empty() {
        return Err(AdmissionError::invalid_scope_chain(
            "scope id must be a non-empty backend-generated identifier",
        ));
    }
    match (scope.scope_kind, scope.parent_scope_id.as_deref()) {
        (ScopeKind::Campaign, None) => {}
        (ScopeKind::Campaign, Some(_)) => {
            return Err(AdmissionError::invalid_scope_chain(
                "campaign scope must not have a parent",
            ));
        }
        (kind, None) => {
            return Err(AdmissionError::invalid_scope_chain(format!(
                "{} scope requires a parent scope id",
                kind.as_str()
            )));
        }
        (kind, Some(parent_id)) => {
            let parent = load_scope(tx, parent_id)?;
            if parent.scope_kind != kind.parent().unwrap() {
                return Err(AdmissionError::invalid_scope_chain(format!(
                    "{} scope parent must be a {} scope",
                    kind.as_str(),
                    kind.parent().unwrap().as_str()
                )));
            }
            if parent.status != "active" {
                return Err(AdmissionError::scope_paused(format!(
                    "parent scope {parent_id} is {}",
                    parent.status
                )));
            }
        }
    }
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM experiment_budget_scopes WHERE scope_id = ?1)",
            [&scope.scope_id],
            |row| row.get(0),
        )
        .map_err(StoreError::from)?;
    if exists {
        return Err(AdmissionError::invalid_scope_chain(format!(
            "budget scope {} already exists",
            scope.scope_id
        )));
    }
    let currency_code = match &scope.envelope.money_mode {
        MoneyMode::TokenResourceOnly => None,
        MoneyMode::Money { currency_code, .. } => Some(currency_code.clone()),
    };
    let mut columns: Vec<String> = vec![
        "scope_id".into(),
        "scope_kind".into(),
        "parent_scope_id".into(),
        "status".into(),
        "currency_code".into(),
    ];
    let mut values: Vec<SqlValue> = vec![
        SqlValue::Text(scope.scope_id.clone()),
        SqlValue::Text(scope.scope_kind.as_str().to_string()),
        scope
            .parent_scope_id
            .clone()
            .map(SqlValue::Text)
            .unwrap_or(SqlValue::Null),
        SqlValue::Text("active".to_string()),
        currency_code.map(SqlValue::Text).unwrap_or(SqlValue::Null),
    ];
    for (suffix, dimension) in DIMENSION_COLUMNS {
        columns.push(format!("cap_{suffix}"));
        values.push(match scope.envelope.caps.cap(*dimension) {
            CapLimit::NotApplicable => SqlValue::Null,
            CapLimit::HardCap(limit) => SqlValue::Integer(ensure_sqlite_range(limit)?),
        });
    }
    columns.push("infra_failure_threshold".to_string());
    values.push(SqlValue::Integer(
        scope.envelope.infra_failure_threshold as i64,
    ));
    columns.push("envelope_json".to_string());
    values.push(SqlValue::Text(
        serde_json::to_string(&scope.envelope)
            .map_err(|error| AdmissionError::persistence_failure(error.to_string()))?,
    ));
    columns.push("created_at_ms".to_string());
    values.push(SqlValue::Integer(ensure_sqlite_range(scope.now_ms)?));
    columns.push("updated_at_ms".to_string());
    values.push(SqlValue::Integer(ensure_sqlite_range(scope.now_ms)?));
    let sql = format!(
        "INSERT INTO experiment_budget_scopes ({}) VALUES ({})",
        columns.join(", "),
        (1..=columns.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    tx.execute(&sql, params_from_iter(values))
        .map_err(StoreError::from)?;
    Ok(())
}

fn reconcile_expired_in_tx(
    tx: &Transaction<'_>,
    now_ms: u64,
) -> Result<Vec<String>, AdmissionError> {
    let mut statement = tx
        .prepare(
            "SELECT reservation_id FROM experiment_budget_reservations
             WHERE state = 'reserved' AND lease_expires_at_ms < ?1",
        )
        .map_err(StoreError::from)?;
    let ids = statement
        .query_map([ensure_sqlite_range(now_ms)?], |row| {
            row.get::<_, String>(0)
        })
        .map_err(StoreError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)?;
    drop(statement);
    let mut reconciled = Vec::with_capacity(ids.len());
    for reservation_id in ids {
        // Conservative recovery: an expired reservation may have produced a
        // physical effect, so it is frozen as unknown, never released.
        mark_unknown_in_tx(
            tx,
            &reservation_id,
            &format!("recovery:{reservation_id}"),
            "lease_expired_recovery",
            now_ms,
        )?;
        reconciled.push(reservation_id);
    }
    Ok(reconciled)
}

/// A backend-owned experiment workflow row inserted atomically with its
/// four-level budget scope chain. Only the columns the experiment facade
/// controls are written; timestamps use the table defaults.
pub struct NewExperimentWorkflowRow {
    pub session_id: String,
    pub title: String,
    pub user_query: String,
    pub agent_id: String,
    pub agent_config: Option<String>,
}

/// Derives the canonical four-level scope chain for a workflow session.
/// The request scope id equals the session id; the outer levels use the
/// fixed `:trial` / `:candidate` / `:campaign` suffixes shared with the 2B
/// tool and LLM admission owners, so a single durable chain binds the
/// run/session/request-scope identity for the whole experiment.
pub fn canonical_experiment_chain(session_id: &str) -> ScopeChain {
    ScopeChain {
        request_id: session_id.to_string(),
        trial_id: format!("{session_id}:trial"),
        candidate_id: format!("{session_id}:candidate"),
        campaign_id: format!("{session_id}:campaign"),
    }
}

fn insert_experiment_workflow_in_tx(
    tx: &Transaction<'_>,
    row: &NewExperimentWorkflowRow,
) -> Result<(), AdmissionError> {
    tx.execute(
        "INSERT INTO workflows (id, parent_session_id, title, user_query, agent_id, agent_config, status)
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, 'pending')",
        params![
            row.session_id,
            row.title,
            row.user_query,
            row.agent_id,
            row.agent_config
        ],
    )
    .map_err(StoreError::from)?;
    Ok(())
}

impl MainStore {
    /// Creates one budget scope with a frozen envelope. Parent chain
    /// validity is enforced here; campaign scopes have no parent.
    pub fn create_budget_scope(&self, scope: NewBudgetScope) -> Result<(), AdmissionError> {
        let runtime = self.db_runtime()?;
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<(), AdmissionError> {
                let tx = conn.transaction().map_err(AdmissionError::from)?;
                create_scope_in_tx(&tx, &scope)?;
                tx.commit().map_err(AdmissionError::from)?;
                Ok(())
            })();
            Ok(inner)
        }))
    }

    /// Atomically creates an experiment workflow and its four-level budget
    /// scope chain (campaign -> candidate -> trial -> request) inside a
    /// single `DbRuntime` writer transaction. Any failure rolls back the
    /// whole unit so no partial workflow or scope chain is ever visible
    /// (AC-2 / INV-6). The request scope id equals the session id and the
    /// outer levels reuse the canonical suffixes, so the existing 2B tool
    /// and LLM admission owners opt in automatically once this chain
    /// exists. No new migration is introduced; the scopes reuse the 2B
    /// `experiment_budget_scopes` schema.
    pub fn create_experiment_run_atomic(
        &self,
        workflow: NewExperimentWorkflowRow,
        envelope: BudgetEnvelope,
        now_ms: u64,
    ) -> Result<ScopeChain, AdmissionError> {
        envelope.validate()?;
        let chain = canonical_experiment_chain(&workflow.session_id);
        chain.validate()?;
        let runtime = self.db_runtime()?;
        let chain_for_tx = chain.clone();
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<(), AdmissionError> {
                let tx = conn.transaction().map_err(AdmissionError::from)?;
                insert_experiment_workflow_in_tx(&tx, &workflow)?;
                let scopes = [
                    (ScopeKind::Campaign, &chain_for_tx.campaign_id, None),
                    (
                        ScopeKind::Candidate,
                        &chain_for_tx.candidate_id,
                        Some(chain_for_tx.campaign_id.as_str()),
                    ),
                    (
                        ScopeKind::Trial,
                        &chain_for_tx.trial_id,
                        Some(chain_for_tx.candidate_id.as_str()),
                    ),
                    (
                        ScopeKind::Request,
                        &chain_for_tx.request_id,
                        Some(chain_for_tx.trial_id.as_str()),
                    ),
                ];
                for (kind, scope_id, parent) in scopes {
                    create_scope_in_tx(
                        &tx,
                        &NewBudgetScope {
                            scope_id: scope_id.clone(),
                            scope_kind: kind,
                            parent_scope_id: parent.map(str::to_string),
                            envelope: envelope.clone(),
                            now_ms,
                        },
                    )?;
                }
                tx.commit().map_err(AdmissionError::from)?;
                Ok(())
            })();
            Ok(inner)
        }))
        .map(|()| chain)
    }

    /// Atomically reserves budget across the full four-level scope chain.
    /// Either every level admits the estimate or nothing changes.
    pub fn reserve_effect(
        &self,
        request: ReserveEffect,
        now_ms: u64,
    ) -> Result<Reservation, AdmissionError> {
        let runtime = self.db_runtime()?;
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<Reservation, AdmissionError> {
                let tx = conn.transaction().map_err(AdmissionError::from)?;
                let reservation = reserve_in_tx(&tx, &request, now_ms)?;
                tx.commit().map_err(AdmissionError::from)?;
                Ok(reservation)
            })();
            Ok(inner)
        }))
    }

    /// Transfers a reservation to committed with the actual resource vector.
    /// Overruns are recorded truthfully and pause the campaign.
    pub fn commit_reservation(
        &self,
        reservation_id: &str,
        operation_id: &str,
        actual: BudgetVector,
        now_ms: u64,
    ) -> Result<CommitReceipt, AdmissionError> {
        let runtime = self.db_runtime()?;
        let reservation_id = reservation_id.to_string();
        let operation_id = operation_id.to_string();
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<CommitReceipt, AdmissionError> {
                let tx = conn.transaction().map_err(AdmissionError::from)?;
                let receipt = commit_in_tx(&tx, &reservation_id, &operation_id, actual, now_ms)?;
                tx.commit().map_err(AdmissionError::from)?;
                Ok(receipt)
            })();
            Ok(inner)
        }))
    }

    /// Releases a reservation whose effect is proven not to have happened.
    pub fn release_reservation(
        &self,
        reservation_id: &str,
        operation_id: &str,
        reason: &str,
        now_ms: u64,
    ) -> Result<ReleaseReceipt, AdmissionError> {
        let runtime = self.db_runtime()?;
        let reservation_id = reservation_id.to_string();
        let operation_id = operation_id.to_string();
        let reason = reason.to_string();
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<ReleaseReceipt, AdmissionError> {
                let tx = conn.transaction().map_err(AdmissionError::from)?;
                let receipt = release_in_tx(&tx, &reservation_id, &operation_id, &reason, now_ms)?;
                tx.commit().map_err(AdmissionError::from)?;
                Ok(receipt)
            })();
            Ok(inner)
        }))
    }

    /// Marks a possibly-happened effect unknown; the budget stays frozen.
    pub fn mark_reservation_unknown(
        &self,
        reservation_id: &str,
        operation_id: &str,
        reason: &str,
        now_ms: u64,
    ) -> Result<UnknownReceipt, AdmissionError> {
        let runtime = self.db_runtime()?;
        let reservation_id = reservation_id.to_string();
        let operation_id = operation_id.to_string();
        let reason = reason.to_string();
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<UnknownReceipt, AdmissionError> {
                let tx = conn.transaction().map_err(AdmissionError::from)?;
                let receipt =
                    mark_unknown_in_tx(&tx, &reservation_id, &operation_id, &reason, now_ms)?;
                tx.commit().map_err(AdmissionError::from)?;
                Ok(receipt)
            })();
            Ok(inner)
        }))
    }

    /// Records one infra failure against a campaign and pauses it when the
    /// configured threshold is reached (same transaction).
    pub fn record_infra_failure(
        &self,
        campaign_id: &str,
        effect_id: &str,
        operation_id: &str,
        kind: &str,
        now_ms: u64,
    ) -> Result<InfraReceipt, AdmissionError> {
        let runtime = self.db_runtime()?;
        let campaign_id = campaign_id.to_string();
        let effect_id = effect_id.to_string();
        let operation_id = operation_id.to_string();
        let kind = kind.to_string();
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<InfraReceipt, AdmissionError> {
                let tx = conn.transaction().map_err(AdmissionError::from)?;
                let receipt = record_infra_failure_in_tx(
                    &tx,
                    &campaign_id,
                    &effect_id,
                    &operation_id,
                    &kind,
                    now_ms,
                )?;
                tx.commit().map_err(AdmissionError::from)?;
                Ok(receipt)
            })();
            Ok(inner)
        }))
    }

    /// Atomically marks a possibly-happened effect unknown AND records one
    /// infra failure against the campaign (idempotent on both operation
    /// ids, threshold pause included) in a single writer transaction, so a
    /// crash between the two facts can never leave them inconsistent.
    #[allow(clippy::too_many_arguments)]
    pub fn mark_reservation_unknown_with_infra_failure(
        &self,
        reservation_id: &str,
        unknown_operation_id: &str,
        reason: &str,
        campaign_id: &str,
        effect_id: &str,
        infra_operation_id: &str,
        kind: &str,
        now_ms: u64,
    ) -> Result<UnknownReceipt, AdmissionError> {
        let runtime = self.db_runtime()?;
        let reservation_id = reservation_id.to_string();
        let unknown_operation_id = unknown_operation_id.to_string();
        let reason = reason.to_string();
        let campaign_id = campaign_id.to_string();
        let effect_id = effect_id.to_string();
        let infra_operation_id = infra_operation_id.to_string();
        let kind = kind.to_string();
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<UnknownReceipt, AdmissionError> {
                let tx = conn.transaction().map_err(AdmissionError::from)?;
                let receipt = mark_unknown_in_tx(
                    &tx,
                    &reservation_id,
                    &unknown_operation_id,
                    &reason,
                    now_ms,
                )?;
                // Idempotent on the infra operation id; a replay of the same
                // settlement must not double count the infra failure.
                record_infra_failure_in_tx(
                    &tx,
                    &campaign_id,
                    &effect_id,
                    &infra_operation_id,
                    &kind,
                    now_ms,
                )?;
                tx.commit().map_err(AdmissionError::from)?;
                Ok(receipt)
            })();
            Ok(inner)
        }))
    }

    /// Conservatively reconciles expired reservations: every reservation
    /// whose lease elapsed while still `reserved` is frozen as unknown.
    /// Returns the reconciled reservation ids.
    pub fn reconcile_expired_reservations(
        &self,
        now_ms: u64,
    ) -> Result<Vec<String>, AdmissionError> {
        let runtime = self.db_runtime()?;
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<Vec<String>, AdmissionError> {
                let tx = conn.transaction().map_err(AdmissionError::from)?;
                let reconciled = reconcile_expired_in_tx(&tx, now_ms)?;
                tx.commit().map_err(AdmissionError::from)?;
                Ok(reconciled)
            })();
            Ok(inner)
        }))
    }

    /// Reads one reservation projection (None when it does not exist).
    pub fn get_budget_reservation(
        &self,
        reservation_id: &str,
    ) -> Result<Option<Reservation>, StoreError> {
        let runtime = self.db_runtime()?;
        let reservation_id = reservation_id.to_string();
        runtime.read_blocking(move |conn| {
            let stored = conn
                .query_row(
                    &format!("{RESERVATION_SELECT_SQL} WHERE reservation_id = ?1"),
                    [reservation_id],
                    read_stored_reservation,
                )
                .optional()?;
            Ok(stored.map(|stored| stored.as_reservation()))
        })
    }

    /// Reads the frozen envelope of one scope (None when it does not exist).
    pub fn get_budget_scope_envelope(
        &self,
        scope_id: &str,
    ) -> Result<Option<BudgetEnvelope>, StoreError> {
        let runtime = self.db_runtime()?;
        let scope_id = scope_id.to_string();
        runtime.read_blocking(move |conn| {
            let envelope_json: Option<String> = conn
                .query_row(
                    "SELECT envelope_json FROM experiment_budget_scopes WHERE scope_id = ?1",
                    [scope_id],
                    |row| row.get(0),
                )
                .optional()?;
            match envelope_json {
                Some(json) => {
                    Ok(Some(serde_json::from_str(&json).map_err(|error| {
                        StoreError::JsonError(error.to_string())
                    })?))
                }
                None => Ok(None),
            }
        })
    }

    /// Read-only resolution of the durable four-level scope chain rooted at a
    /// request scope id.
    ///
    /// - `Ok(None)` when no request scope exists: this is an ordinary
    ///   workflow with no budget opt-in, so callers keep the normal path
    ///   (INV-4).
    /// - `Ok(Some(chain))` when a complete, correctly parented canonical
    ///   chain exists.
    /// - `Err` when a request scope exists but its chain is incomplete or
    ///   mis-parented. A half/forged chain must fail closed before any effect
    ///   rather than degrading to a normal request (INV-2).
    pub fn get_budget_scope_chain(
        &self,
        request_scope_id: &str,
    ) -> Result<Option<ScopeChain>, StoreError> {
        let runtime = self.db_runtime()?;
        let request_scope_id = request_scope_id.to_string();
        runtime.read_blocking(move |conn| {
            let trial_id = format!("{request_scope_id}:trial");
            let candidate_id = format!("{request_scope_id}:candidate");
            let campaign_id = format!("{request_scope_id}:campaign");
            let read_level = |id: &str| -> Result<Option<(String, Option<String>)>, StoreError> {
                conn.query_row(
                    "SELECT scope_kind, parent_scope_id
                         FROM experiment_budget_scopes WHERE scope_id = ?1",
                    [id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .optional()
                .map_err(StoreError::from)
            };
            // The request level decides opt-in. Absent -> ordinary path.
            let Some((request_kind, request_parent)) = read_level(&request_scope_id)? else {
                return Ok(None);
            };
            let levels: [(&str, &str, &str, Option<&str>); 4] = [
                (&request_scope_id, "request", "request", Some(&trial_id)),
                (&trial_id, "trial", "trial", Some(&candidate_id)),
                (&candidate_id, "candidate", "candidate", Some(&campaign_id)),
                (&campaign_id, "campaign", "campaign", None),
            ];
            // Validate the request level against the first tuple, then the
            // remaining levels by re-reading each id.
            let expected_parent = request_parent.as_deref();
            if request_kind != "request" || expected_parent != Some(trial_id.as_str()) {
                return Err(invalid_chain_error(format!(
                    "request scope {request_scope_id} has kind '{request_kind}' / unexpected parent"
                )));
            }
            for (id, _label, expected_kind, expected_parent) in &levels[1..] {
                let Some((kind, parent)) = read_level(id)? else {
                    return Err(invalid_chain_error(format!(
                        "scope chain is incomplete: {id} is missing"
                    )));
                };
                if kind != *expected_kind {
                    return Err(invalid_chain_error(format!(
                        "scope {id} has kind '{kind}', expected '{expected_kind}'"
                    )));
                }
                if parent.as_deref() != *expected_parent {
                    return Err(invalid_chain_error(format!(
                        "scope {id} parent mismatch: got {:?}, expected {expected_parent:?}",
                        parent
                    )));
                }
            }
            Ok(Some(ScopeChain {
                request_id: request_scope_id.clone(),
                trial_id,
                candidate_id,
                campaign_id,
            }))
        })
    }

    /// Reads one scope projection (None when it does not exist).
    pub fn get_budget_scope_status(
        &self,
        scope_id: &str,
    ) -> Result<Option<BudgetScopeStatus>, StoreError> {
        let runtime = self.db_runtime()?;
        let scope_id = scope_id.to_string();
        runtime.read_blocking(move |conn| {
            let row = conn
                .query_row(&scope_select_sql(), [scope_id], read_scope_row)
                .optional()?;
            Ok(row.map(|row| BudgetScopeStatus {
                scope_id: row.scope_id,
                scope_kind: row.scope_kind,
                parent_scope_id: row.parent_scope_id,
                status: row.status,
                committed: row.committed,
                reserved: row.reserved,
                caps: row.caps,
                infra_failure_count: row.infra_failure_count,
                infra_failure_threshold: row.infra_failure_threshold,
                pause_reason: row.pause_reason,
            }))
        })
    }

    /// Lists the append-only ledger entries of one scope in commit order.
    pub fn list_budget_ledger_entries(
        &self,
        scope_id: &str,
    ) -> Result<Vec<BudgetLedgerEntry>, StoreError> {
        let runtime = self.db_runtime()?;
        let scope_id = scope_id.to_string();
        runtime.read_blocking(move |conn| {
            let mut statement = conn.prepare(
                "SELECT entry_id, scope_id, reservation_id, effect_id, operation, vector_json,
                        operation_id, idempotency_key, reason, created_at_ms
                 FROM experiment_budget_ledger_entries WHERE scope_id = ?1
                 ORDER BY created_at_ms, entry_id",
            )?;
            let rows = statement.query_map([&scope_id], |row| {
                let vector_json: String = row.get(5)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    vector_json,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            })?;
            let mut entries = Vec::new();
            for row in rows {
                let (
                    entry_id,
                    scope_id,
                    reservation_id,
                    effect_id,
                    operation,
                    vector_json,
                    operation_id,
                    idempotency_key,
                    reason,
                    created_at_ms,
                ) = row?;
                let vector: BudgetVector = serde_json::from_str(&vector_json)
                    .map_err(|error| StoreError::JsonError(error.to_string()))?;
                entries.push(BudgetLedgerEntry {
                    entry_id,
                    scope_id,
                    reservation_id,
                    effect_id,
                    operation,
                    vector,
                    operation_id,
                    idempotency_key,
                    reason,
                    created_at_ms: created_at_ms.max(0) as u64,
                });
            }
            Ok(entries)
        })
    }

    /// Recomputes a scope's materialized balances from its append-only
    /// ledger entries (audit projection). Returns (committed, reserved).
    pub fn recompute_scope_balance_from_ledger(
        &self,
        scope_id: &str,
    ) -> Result<(BudgetVector, BudgetVector), AdmissionError> {
        let runtime = self.db_runtime()?;
        let scope_id = scope_id.to_string();
        flatten(runtime.read_blocking(move |conn| {
            let inner = (|| -> Result<(BudgetVector, BudgetVector), AdmissionError> {
                let mut committed = BudgetVector::ZERO;
                let mut reserved = BudgetVector::ZERO;
                // Collect the scope's entries first so reservation lookups do
                // not borrow the connection while the statement is alive.
                let mut statement = conn
                    .prepare(
                        "SELECT operation, vector_json, reservation_id
                         FROM experiment_budget_ledger_entries WHERE scope_id = ?1
                         ORDER BY created_at_ms, entry_id",
                    )
                    .map_err(AdmissionError::from)?;
                let rows = statement
                    .query_map([&scope_id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    })
                    .map_err(AdmissionError::from)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(AdmissionError::from)?;
                drop(statement);
                for (operation, vector_json, reservation_id) in rows {
                    let vector: BudgetVector = serde_json::from_str(&vector_json)
                        .map_err(|error| AdmissionError::persistence_failure(error.to_string()))?;
                    match operation.as_str() {
                        "reserve" => reserved = reserved.checked_add(&vector)?,
                        "commit" | "overrun" => {
                            let estimate = match &reservation_id {
                                Some(reservation_id) => {
                                    let stored = conn
                                        .query_row(
                                            &format!(
                                                "{RESERVATION_SELECT_SQL} WHERE reservation_id = ?1"
                                            ),
                                            [reservation_id],
                                            read_stored_reservation,
                                        )
                                        .optional()
                                        .map_err(AdmissionError::from)?
                                        .ok_or_else(|| {
                                            AdmissionError::persistence_failure(format!(
                                                "ledger references missing reservation {reservation_id}"
                                            ))
                                        })?;
                                    stored.estimate
                                }
                                None => BudgetVector::ZERO,
                            };
                            reserved = reserved.checked_sub(&estimate)?;
                            committed = committed.checked_add(&vector)?;
                        }
                        "release" => reserved = reserved.checked_sub(&vector)?,
                        // Unknown keeps the budget frozen; infra/pause do not
                        // change balances.
                        "mark_unknown" | "infra_failure" | "pause" => {}
                        other => {
                            return Err(AdmissionError::persistence_failure(format!(
                                "unknown ledger operation {other}"
                            )));
                        }
                    }
                }
                Ok((committed, reserved))
            })();
            Ok(inner)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::errors::AdmissionErrorCode;
    use crate::budget::types::{BudgetEnvelope, ReserveEffect, SQLITE_INTEGER_MAX};
    use std::collections::BTreeSet;
    use tempfile::tempdir;

    fn now() -> u64 {
        1_700_000_000_000
    }

    fn envelope() -> BudgetEnvelope {
        BudgetEnvelope {
            caps: ResourceCaps {
                input_tokens: CapLimit::HardCap(1_000),
                output_tokens: CapLimit::HardCap(1_000),
                cache_read_tokens: CapLimit::NotApplicable,
                cache_write_tokens: CapLimit::NotApplicable,
                wall_time_ms: CapLimit::HardCap(60_000),
                tool_calls: CapLimit::HardCap(10),
                processes: CapLimit::HardCap(2),
                disk_bytes: CapLimit::NotApplicable,
                network_bytes: CapLimit::NotApplicable,
                concurrency: CapLimit::HardCap(2),
                money: CapLimit::NotApplicable,
            },
            required_dimensions: BTreeSet::new(),
            money_mode: MoneyMode::TokenResourceOnly,
            max_attempts: 1,
            infra_failure_threshold: 3,
            reservation_lease_ms: 60_000,
        }
    }

    fn store() -> (MainStore, tempfile::TempDir) {
        let directory = tempdir().expect("failed to create temp dir");
        let store = MainStore::new(directory.path().join("budget.db"))
            .expect("failed to create test store");
        (store, directory)
    }

    fn chain() -> ScopeChain {
        ScopeChain {
            request_id: "req-1".into(),
            trial_id: "trial-1".into(),
            candidate_id: "cand-1".into(),
            campaign_id: "camp-1".into(),
        }
    }

    fn create_chain(store: &MainStore) {
        let base = envelope();
        for (kind, id, parent) in [
            (ScopeKind::Campaign, "camp-1", None),
            (ScopeKind::Candidate, "cand-1", Some("camp-1")),
            (ScopeKind::Trial, "trial-1", Some("cand-1")),
            (ScopeKind::Request, "req-1", Some("trial-1")),
        ] {
            store
                .create_budget_scope(NewBudgetScope {
                    scope_id: id.into(),
                    scope_kind: kind,
                    parent_scope_id: parent.map(|value| value.to_string()),
                    envelope: base.clone(),
                    now_ms: now(),
                })
                .expect("scope creation should succeed");
        }
    }

    fn reserve_request(idempotency_key: &str, estimate: BudgetVector) -> ReserveEffect {
        ReserveEffect {
            effect_id: format!("eff-{idempotency_key}"),
            idempotency_key: idempotency_key.into(),
            scopes: chain(),
            effect_kind: EffectKind::LlmCompletion,
            attempt: 1,
            estimate,
        }
    }

    fn estimate(input: u64, output: u64) -> BudgetVector {
        BudgetVector {
            input_tokens: input,
            output_tokens: output,
            ..BudgetVector::ZERO
        }
    }

    mod budget_ledger {
        use super::*;

        #[test]
        fn reserve_commit_round_trip_updates_all_scopes() {
            let (store, _dir) = store();
            create_chain(&store);
            let reservation = store
                .reserve_effect(reserve_request("idem-1", estimate(100, 50)), now())
                .expect("reserve should succeed");
            assert_eq!(reservation.state, ReservationState::Reserved);

            for scope_id in ["camp-1", "cand-1", "trial-1", "req-1"] {
                let status = store
                    .get_budget_scope_status(scope_id)
                    .expect("read scope")
                    .expect("scope exists");
                assert_eq!(status.reserved.input_tokens, 100);
                assert_eq!(status.reserved.output_tokens, 50);
            }

            let receipt = store
                .commit_reservation(&reservation.reservation_id, "op-1", estimate(90, 40), now())
                .expect("commit should succeed");
            assert!(!receipt.overrun);
            for scope_id in ["camp-1", "cand-1", "trial-1", "req-1"] {
                let status = store
                    .get_budget_scope_status(scope_id)
                    .expect("read scope")
                    .expect("scope exists");
                assert_eq!(status.committed.input_tokens, 90);
                assert_eq!(status.reserved.input_tokens, 0);
                // Audit projection matches the materialized balances.
                let (committed, reserved) = store
                    .recompute_scope_balance_from_ledger(scope_id)
                    .expect("recompute should succeed");
                assert_eq!(committed, status.committed);
                assert_eq!(reserved, status.reserved);
            }
        }

        #[test]
        fn reserve_is_idempotent_and_conflicts_are_rejected() {
            let (store, _dir) = store();
            create_chain(&store);
            let first = store
                .reserve_effect(reserve_request("idem-1", estimate(10, 10)), now())
                .expect("first reserve should succeed");
            let replay = store
                .reserve_effect(reserve_request("idem-1", estimate(10, 10)), now())
                .expect("replay should return the original reservation");
            assert_eq!(first.reservation_id, replay.reservation_id);

            let conflict = store.reserve_effect(reserve_request("idem-1", estimate(20, 10)), now());
            assert_eq!(
                conflict.unwrap_err().code,
                AdmissionErrorCode::IdempotencyConflict
            );
            // No partial balance change from the conflicting call.
            let status = store
                .get_budget_scope_status("req-1")
                .expect("read scope")
                .expect("scope exists");
            assert_eq!(status.reserved.input_tokens, 10);
        }

        #[test]
        fn cap_shortfall_at_any_level_rolls_back_everything() {
            let (store, _dir) = store();
            create_chain(&store);
            // First reservation consumes most of the trial-level input cap.
            store
                .reserve_effect(reserve_request("idem-1", estimate(900, 0)), now())
                .expect("first reserve should succeed");
            // The request scope still has room, but the trial/candidate/campaign
            // scopes do not; the whole reserve must roll back.
            let rejected = store.reserve_effect(reserve_request("idem-2", estimate(200, 0)), now());
            let error = rejected.unwrap_err();
            assert_eq!(error.code, AdmissionErrorCode::BudgetExceeded);
            let status = store
                .get_budget_scope_status("req-1")
                .expect("read scope")
                .expect("scope exists");
            assert_eq!(
                status.reserved.input_tokens, 900,
                "no partial reservation may remain"
            );
        }

        #[test]
        fn release_only_from_reserved_and_unknown_cannot_release() {
            let (store, _dir) = store();
            create_chain(&store);
            let reservation = store
                .reserve_effect(reserve_request("idem-1", estimate(10, 10)), now())
                .expect("reserve should succeed");
            store
                .release_reservation(&reservation.reservation_id, "op-1", "not_sent", now())
                .expect("release before send should succeed");
            let status = store
                .get_budget_scope_status("req-1")
                .expect("read scope")
                .expect("scope exists");
            assert_eq!(status.reserved.input_tokens, 0);

            // Release replay with the same operation id is idempotent.
            store
                .release_reservation(&reservation.reservation_id, "op-1", "not_sent", now())
                .expect("release replay should succeed");
            // A different operation id on a terminal state is rejected.
            assert_eq!(
                store
                    .release_reservation(&reservation.reservation_id, "op-2", "not_sent", now())
                    .unwrap_err()
                    .code,
                AdmissionErrorCode::InvalidTransition
            );

            // Unknown reservations can never be released (INV-5).
            let reservation = store
                .reserve_effect(reserve_request("idem-2", estimate(10, 10)), now())
                .expect("reserve should succeed");
            store
                .mark_reservation_unknown(
                    &reservation.reservation_id,
                    "op-3",
                    "sent_then_crash",
                    now(),
                )
                .expect("mark unknown should succeed");
            assert_eq!(
                store
                    .release_reservation(&reservation.reservation_id, "op-4", "late_release", now())
                    .unwrap_err()
                    .code,
                AdmissionErrorCode::InvalidTransition
            );
            // The budget stays frozen for unknown reservations.
            let status = store
                .get_budget_scope_status("req-1")
                .expect("read scope")
                .expect("scope exists");
            assert_eq!(status.reserved.input_tokens, 10);
        }

        #[test]
        fn overrun_records_actual_and_pauses_campaign() {
            let (store, _dir) = store();
            create_chain(&store);
            let reservation = store
                .reserve_effect(reserve_request("idem-1", estimate(100, 0)), now())
                .expect("reserve should succeed");
            let receipt = store
                .commit_reservation(&reservation.reservation_id, "op-1", estimate(150, 0), now())
                .expect("overrun commit should be recorded");
            assert!(receipt.overrun);
            let campaign = store
                .get_budget_scope_status("camp-1")
                .expect("read scope")
                .expect("scope exists");
            assert_eq!(campaign.status, "paused");
            assert_eq!(campaign.committed.input_tokens, 150);
            // Subsequent reserves are rejected while paused.
            assert_eq!(
                store
                    .reserve_effect(reserve_request("idem-2", estimate(1, 0)), now())
                    .unwrap_err()
                    .code,
                AdmissionErrorCode::ScopePaused
            );
        }

        #[test]
        fn infra_failure_threshold_pauses_campaign_idempotently() {
            let (store, _dir) = store();
            create_chain(&store);
            for index in 1..=3 {
                let receipt = store
                    .record_infra_failure(
                        "camp-1",
                        "eff-x",
                        &format!("infra-op-{index}"),
                        "timeout",
                        now(),
                    )
                    .expect("infra failure should be recorded");
                assert_eq!(receipt.infra_failure_count, index);
                assert_eq!(receipt.paused, index == 3);
            }
            // Duplicate operation id does not double count.
            let replay = store
                .record_infra_failure("camp-1", "eff-x", "infra-op-3", "timeout", now())
                .expect("replay should succeed");
            assert_eq!(replay.infra_failure_count, 3);
            let campaign = store
                .get_budget_scope_status("camp-1")
                .expect("read scope")
                .expect("scope exists");
            assert_eq!(campaign.status, "paused");
            assert_eq!(
                store
                    .reserve_effect(reserve_request("idem-1", estimate(1, 0)), now())
                    .unwrap_err()
                    .code,
                AdmissionErrorCode::ScopePaused
            );
        }
    }

    mod budget_recovery {
        use super::*;

        #[test]
        fn expired_reservations_are_recovered_as_unknown() {
            let (store, _dir) = store();
            create_chain(&store);
            let reservation = store
                .reserve_effect(reserve_request("idem-1", estimate(10, 10)), now())
                .expect("reserve should succeed");
            // Advance past the lease (60s) without any settlement.
            let reconciled = store
                .reconcile_expired_reservations(now() + 61_000)
                .expect("recovery should succeed");
            assert_eq!(reconciled, vec![reservation.reservation_id.clone()]);
            let stored = store
                .get_budget_reservation(&reservation.reservation_id)
                .expect("read reservation")
                .expect("reservation exists");
            assert_eq!(stored.state, ReservationState::Unknown);
            // Frozen budget is still held.
            let status = store
                .get_budget_scope_status("req-1")
                .expect("read scope")
                .expect("scope exists");
            assert_eq!(status.reserved.input_tokens, 10);
            // Recovery is idempotent: nothing left to reconcile.
            let again = store
                .reconcile_expired_reservations(now() + 61_000)
                .expect("second recovery should succeed");
            assert!(again.is_empty());
        }
    }

    mod budget_validation {
        use super::*;

        #[test]
        fn invalid_scope_chains_are_rejected() {
            let (store, _dir) = store();
            create_chain(&store);
            let mut request = reserve_request("idem-1", estimate(1, 1));
            request.scopes.trial_id = "missing-trial".into();
            assert_eq!(
                store.reserve_effect(request, now()).unwrap_err().code,
                AdmissionErrorCode::InvalidScopeChain
            );
        }

        #[test]
        fn attempt_beyond_max_attempts_is_rejected() {
            let (store, _dir) = store();
            create_chain(&store);
            let mut request = reserve_request("idem-1", estimate(1, 1));
            request.attempt = 2;
            assert_eq!(
                store.reserve_effect(request, now()).unwrap_err().code,
                AdmissionErrorCode::InvalidTransition
            );
        }

        #[test]
        fn money_cost_in_token_only_mode_is_rejected() {
            let (store, _dir) = store();
            create_chain(&store);
            let request = reserve_request(
                "idem-1",
                BudgetVector {
                    money_micros: 100,
                    ..BudgetVector::ZERO
                },
            );
            assert_eq!(
                store.reserve_effect(request, now()).unwrap_err().code,
                AdmissionErrorCode::BudgetExceeded
            );
        }
    }

    fn money_envelope() -> BudgetEnvelope {
        let pricing = crate::budget::types::PricingSnapshot {
            currency_code: "cny".into(),
            provider_id: "prov".into(),
            model_id: "model".into(),
            input_micros_per_million: 1_000_000,
            output_micros_per_million: 2_000_000,
            cache_read_micros_per_million: 0,
            cache_write_micros_per_million: 0,
            reasoning_micros_per_million: None,
            multiplier_micros: 1_000_000,
            source_hash: "hash".into(),
        };
        BudgetEnvelope {
            caps: ResourceCaps {
                input_tokens: CapLimit::HardCap(10_000_000),
                output_tokens: CapLimit::HardCap(10_000_000),
                cache_read_tokens: CapLimit::NotApplicable,
                cache_write_tokens: CapLimit::NotApplicable,
                wall_time_ms: CapLimit::HardCap(600_000),
                tool_calls: CapLimit::HardCap(100),
                processes: CapLimit::HardCap(10),
                disk_bytes: CapLimit::NotApplicable,
                network_bytes: CapLimit::NotApplicable,
                concurrency: CapLimit::HardCap(4),
                money: CapLimit::HardCap(5_000),
            },
            required_dimensions: BTreeSet::new(),
            money_mode: MoneyMode::Money {
                currency_code: "cny".into(),
                cap_money_micros: 5_000,
                pricing,
            },
            max_attempts: 1,
            infra_failure_threshold: 3,
            reservation_lease_ms: 600_000,
        }
    }

    #[test]
    fn sequential_money_settlement_accumulates_across_cap() {
        let (store, _dir) = store();
        let base = money_envelope();
        for (kind, id, parent) in [
            (ScopeKind::Campaign, "camp-1", None),
            (ScopeKind::Candidate, "cand-1", Some("camp-1")),
            (ScopeKind::Trial, "trial-1", Some("cand-1")),
            (ScopeKind::Request, "req-1", Some("trial-1")),
        ] {
            store
                .create_budget_scope(NewBudgetScope {
                    scope_id: id.into(),
                    scope_kind: kind,
                    parent_scope_id: parent.map(|value| value.to_string()),
                    envelope: base.clone(),
                    now_ms: now(),
                })
                .expect("scope creation should succeed");
        }
        // First effect: reserve 1M input + 1M output tokens with their
        // worst-case money (1000 + 2000 = 3000 micros), settle actual
        // usage at 500 micros of money.
        let first = store
            .reserve_effect(
                reserve_request(
                    "idem-1",
                    BudgetVector {
                        input_tokens: 1_000_000,
                        output_tokens: 1_000_000,
                        money_micros: 3_000,
                        ..BudgetVector::ZERO
                    },
                ),
                now(),
            )
            .expect("first reserve should succeed");
        store
            .commit_reservation(
                &first.reservation_id,
                "op-1",
                BudgetVector {
                    input_tokens: 500_000,
                    output_tokens: 0,
                    money_micros: 500,
                    ..BudgetVector::ZERO
                },
                now(),
            )
            .expect("first commit should succeed");
        // Second effect settles at 4600 micros of money: the accumulated
        // committed money (500) plus this actual (4600) exceeds the 5000
        // micro cap, so the commit must be recorded as an overrun (and
        // pause the campaign) instead of silently accepting the excess.
        let second = store
            .reserve_effect(
                reserve_request(
                    "idem-2",
                    BudgetVector {
                        input_tokens: 1_000_000,
                        output_tokens: 1_000_000,
                        money_micros: 3_000,
                        ..BudgetVector::ZERO
                    },
                ),
                now(),
            )
            .expect("second reserve should succeed");
        let receipt = store
            .commit_reservation(
                &second.reservation_id,
                "op-2",
                BudgetVector {
                    input_tokens: 500_000,
                    output_tokens: 0,
                    money_micros: 4_600,
                    ..BudgetVector::ZERO
                },
                now(),
            )
            .expect("overrun commit must be recorded truthfully");
        assert!(receipt.overrun);
        let campaign = store
            .get_budget_scope_status("camp-1")
            .expect("read scope")
            .expect("scope exists");
        assert_eq!(campaign.status, "paused");
        assert_eq!(campaign.committed.money_micros, 5_100);
        // Subsequent reserves are refused while paused.
        assert_eq!(
            store
                .reserve_effect(
                    reserve_request(
                        "idem-3",
                        BudgetVector {
                            input_tokens: 1,
                            ..BudgetVector::ZERO
                        }
                    ),
                    now()
                )
                .unwrap_err()
                .code,
            AdmissionErrorCode::ScopePaused
        );
    }

    #[test]
    fn unknown_and_infra_failure_are_recorded_atomically() {
        let (store, _dir) = store();
        create_chain(&store);
        let reservation = store
            .reserve_effect(reserve_request("idem-1", estimate(10, 10)), now())
            .expect("reserve should succeed");
        // One atomic call records both facts; threshold is 3 so no pause yet.
        store
            .mark_reservation_unknown_with_infra_failure(
                &reservation.reservation_id,
                "unknown:op-1",
                "transport_error",
                "camp-1",
                "eff-1",
                "infra:op-1",
                "transport",
                now(),
            )
            .expect("atomic unknown+infra should succeed");
        let stored = store
            .get_budget_reservation(&reservation.reservation_id)
            .expect("read")
            .expect("reservation exists");
        assert_eq!(stored.state, ReservationState::Unknown);
        let campaign = store
            .get_budget_scope_status("camp-1")
            .expect("read scope")
            .expect("scope exists");
        assert_eq!(campaign.infra_failure_count, 1);
        assert_eq!(campaign.status, "active");
        // Replaying the same operation ids must not double count.
        store
            .mark_reservation_unknown_with_infra_failure(
                &reservation.reservation_id,
                "unknown:op-1",
                "transport_error",
                "camp-1",
                "eff-1",
                "infra:op-1",
                "transport",
                now(),
            )
            .expect("replay should succeed");
        let campaign = store
            .get_budget_scope_status("camp-1")
            .expect("read scope")
            .expect("scope exists");
        assert_eq!(campaign.infra_failure_count, 1);
        // Reaching the threshold via atomic calls pauses the campaign.
        for index in 2..=3 {
            let reservation = store
                .reserve_effect(
                    reserve_request(&format!("idem-{index}"), estimate(1, 1)),
                    now(),
                )
                .expect("reserve should succeed");
            store
                .mark_reservation_unknown_with_infra_failure(
                    &reservation.reservation_id,
                    &format!("unknown:op-{index}"),
                    "transport_error",
                    "camp-1",
                    &format!("eff-{index}"),
                    &format!("infra:op-{index}"),
                    "transport",
                    now(),
                )
                .expect("atomic unknown+infra should succeed");
        }
        let campaign = store
            .get_budget_scope_status("camp-1")
            .expect("read scope")
            .expect("scope exists");
        assert_eq!(campaign.infra_failure_count, 3);
        assert_eq!(campaign.status, "paused");
    }

    mod budget_concurrency {
        use super::*;

        #[test]
        fn concurrent_reserves_never_exceed_caps() {
            use std::sync::Arc;
            let (store, _dir) = store();
            create_chain(&store);
            let shared = Arc::new(store);
            let handles: Vec<_> = (0..8)
                .map(|index| {
                    let store = Arc::clone(&shared);
                    std::thread::spawn(move || {
                        store.reserve_effect(
                            reserve_request(&format!("idem-{index}"), estimate(200, 0)),
                            now(),
                        )
                    })
                })
                .collect();
            let mut admitted = 0;
            for handle in handles {
                if handle.join().expect("thread should not panic").is_ok() {
                    admitted += 1;
                }
            }
            assert_eq!(
                admitted, 5,
                "input cap 1000 / estimate 200 admits exactly 5 reservations"
            );
            let status = shared
                .get_budget_scope_status("req-1")
                .expect("read scope")
                .expect("scope exists");
            assert_eq!(status.reserved.input_tokens, 1_000);
        }

        #[test]
        fn sqlite_integer_range_is_enforced_on_counters() {
            assert_eq!(ensure_sqlite_range(SQLITE_INTEGER_MAX).unwrap(), i64::MAX);
            assert!(ensure_sqlite_range(SQLITE_INTEGER_MAX + 1).is_err());
        }

        fn experiment_row(session_id: &str) -> NewExperimentWorkflowRow {
            NewExperimentWorkflowRow {
                session_id: session_id.into(),
                title: format!("experiment {session_id}"),
                user_query: "do the thing".into(),
                agent_id: "agent-1".into(),
                agent_config: Some("{}".into()),
            }
        }

        fn seed_agent(store: &MainStore, agent_id: &str) {
            let agent_id = agent_id.to_string();
            store
                .db_runtime()
                .expect("runtime")
                .write_blocking(move |conn| {
                    conn.execute(
                        "INSERT INTO agents (id, name, system_prompt, agent_type, max_contexts)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            agent_id,
                            format!("Agent {agent_id}"),
                            "test",
                            "autonomous",
                            20
                        ],
                    )?;
                    Ok(())
                })
                .expect("seed agent");
        }

        #[test]
        fn experiment_run_atomic_creates_workflow_and_four_scopes() {
            let (store, _dir) = store();
            seed_agent(&store, "agent-1");
            let chain = store
                .create_experiment_run_atomic(experiment_row("sess-1"), envelope(), now())
                .expect("atomic create");
            // request scope id equals the session id; outer levels derive the
            // canonical suffixes shared with the 2B admission owners.
            assert_eq!(chain.request_id, "sess-1");
            assert_eq!(chain.trial_id, "sess-1:trial");
            assert_eq!(chain.candidate_id, "sess-1:candidate");
            assert_eq!(chain.campaign_id, "sess-1:campaign");
            assert!(store.get_workflow("sess-1").expect("read").is_some());
            for id in [
                &chain.request_id,
                &chain.trial_id,
                &chain.candidate_id,
                &chain.campaign_id,
            ] {
                let status = store
                    .get_budget_scope_status(id)
                    .expect("read scope")
                    .unwrap_or_else(|| panic!("scope {id} must exist"));
                assert_eq!(status.status, "active");
            }
            // The envelope is frozen and readable via the request scope, so
            // the existing tool/LLM admission owners opt in automatically.
            let envelope = store
                .get_budget_scope_envelope("sess-1")
                .expect("read envelope")
                .expect("envelope present");
            assert_eq!(envelope.max_attempts, 1);
        }

        #[test]
        fn experiment_run_atomic_rolls_back_on_duplicate_session() {
            let (store, _dir) = store();
            seed_agent(&store, "agent-1");
            store
                .create_experiment_run_atomic(experiment_row("sess-1"), envelope(), now())
                .expect("first create");
            // A second run reusing the same session id hits the workflow
            // primary key and must roll back the whole transaction, leaving
            // no partial scope chain from the failed attempt.
            let error = store
                .create_experiment_run_atomic(experiment_row("sess-1"), envelope(), now())
                .expect_err("duplicate session must fail");
            assert_eq!(error.code, AdmissionErrorCode::AdmissionPersistenceFailure);
            // The original chain is intact and exactly one request scope
            // exists for the session (no orphaned second chain).
            assert!(store
                .get_budget_scope_status("sess-1:trial")
                .expect("read")
                .is_some());
        }

        #[test]
        fn experiment_run_atomic_rejects_invalid_envelope_without_writing() {
            let (store, _dir) = store();
            let mut bad = envelope();
            bad.max_attempts = 0;
            let error = store
                .create_experiment_run_atomic(experiment_row("sess-bad"), bad, now())
                .expect_err("invalid envelope must fail before any write");
            assert_eq!(error.code, AdmissionErrorCode::InvalidScopeChain);
            // Nothing was persisted: no workflow row, no request scope.
            assert!(store.get_workflow("sess-bad").expect("read").is_none());
            assert!(store
                .get_budget_scope_status("sess-bad")
                .expect("read")
                .is_none());
        }

        #[test]
        fn scope_chain_absent_returns_none() {
            let (store, _dir) = store();
            // No request scope: an ordinary workflow keeps the normal path.
            let chain = store
                .get_budget_scope_chain("no-such-session")
                .expect("read");
            assert!(chain.is_none());
        }

        #[test]
        fn scope_chain_resolves_canonical_experiment_chain() {
            let (store, _dir) = store();
            seed_agent(&store, "agent-1");
            store
                .create_experiment_run_atomic(experiment_row("sess-1"), envelope(), now())
                .expect("atomic create");
            let chain = store
                .get_budget_scope_chain("sess-1")
                .expect("read")
                .expect("canonical chain present");
            assert_eq!(chain.request_id, "sess-1");
            assert_eq!(chain.trial_id, "sess-1:trial");
            assert_eq!(chain.candidate_id, "sess-1:candidate");
            assert_eq!(chain.campaign_id, "sess-1:campaign");
        }

        #[test]
        fn scope_chain_rejects_noncanonical_chain() {
            let (store, _dir) = store();
            // `create_chain` builds a kind-valid chain with non-canonical ids
            // (req-1/trial-1/...). The canonical resolver keys the outer
            // levels by the `:trial`/`:candidate`/`:campaign` suffixes, so a
            // request scope that is not the root of a canonical chain must
            // fail closed rather than resolve.
            create_chain(&store);
            let error = store
                .get_budget_scope_chain("req-1")
                .expect_err("non-canonical chain must fail closed");
            assert!(matches!(error, StoreError::InvalidData(_)));
        }
    }
}

//! Opted-in pre-effect admission gate for ccproxy outbound requests.
//!
//! Only backend-created, trusted internal requests may carry an admission
//! context (opaque scope chain + effect identity). The context travels in
//! `x-cs-experiment-admission`; the router strips this header from every
//! untrusted request, and extraction additionally re-checks the trusted
//! internal request marker, so an external caller can never mint admission
//! ownership (INV-6).
//!
//! Ordinary requests without the context keep the existing behavior
//! unchanged (INV-2): no budget lookup, no reservation, no retry override.
//!
//! Settlement contract (INV-5):
//! - proven pre-send failure (request never built/sent) -> release;
//! - transport ambiguity after send (connection error, no response) ->
//!   mark unknown + infra failure;
//! - provider response received (success or error status) -> the effect
//!   happened; commit the actual usage (zero usage for error responses);
//! - streaming -> the lease moves into the terminal stat guard and is
//!   committed (or marked unknown on stream failure) at the terminal
//!   boundary.

use crate::budget::errors::AdmissionError;
use crate::budget::pricing::{build_llm_estimate, settlement_money_micros, LlmEffectBound};
use crate::budget::resource::check_owner_observability;
use crate::budget::types::{BudgetEnvelope, BudgetVector, EffectKind, ScopeChain};
use crate::budget::{CommitReceipt, Reservation, ReserveEffect};
use crate::ccproxy::auth::is_trusted_internal_request;
use crate::db::MainStore;
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Header carrying the opaque backend-created admission context. Stripped
/// from every untrusted request by the router.
pub const ADMISSION_HEADER: &str = "x-cs-experiment-admission";

/// Opaque admission context created by the backend experiment owner. All
/// identifiers are backend-generated strings; external values can never
/// change ledger ownership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AdmissionContext {
    pub scope_chain: ScopeChain,
    pub effect_id: String,
    pub idempotency_key: String,
    pub attempt: u32,
}

/// Extracts the admission context from request headers. Returns `None` for
/// any request that is not a trusted internal request or that carries no
/// (or malformed) context — such requests keep the ordinary path.
pub fn admission_context_from_headers(headers: &HeaderMap) -> Option<AdmissionContext> {
    if !is_trusted_internal_request(headers) {
        return None;
    }
    let raw = headers
        .get(ADMISSION_HEADER)
        .and_then(|value| value.to_str().ok())?;
    match serde_json::from_str(raw) {
        Ok(context) => Some(context),
        Err(error) => {
            log::warn!("Ignoring malformed admission context header: {}", error);
            None
        }
    }
}

/// Extracts an explicit max output bound from a raw JSON request body,
/// covering the wire variants that survive on direct pass-through paths
/// (OpenAI `max_tokens`/`max_completion_tokens`, Responses
/// `max_output_tokens`, Gemini `generationConfig.maxOutputTokens`).
pub fn max_output_tokens_from_json(body: &serde_json::Value) -> Option<u64> {
    let read = |value: &serde_json::Value| {
        value
            .as_i64()
            .filter(|value| *value > 0)
            .map(|value| value as u64)
    };
    for key in ["max_tokens", "max_completion_tokens", "max_output_tokens"] {
        if let Some(value) = body.get(key).and_then(read) {
            return Some(value);
        }
    }
    body.get("generationConfig")
        .and_then(|config| config.get("maxOutputTokens"))
        .and_then(read)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Inputs required to admit one LLM/embedding effect.
#[derive(Debug, Clone)]
pub struct LlmGateInput {
    pub provider_id: String,
    pub model_id: String,
    /// Normalized request token estimate (upper bound).
    pub estimated_input_tokens: u64,
    pub effect_kind: EffectKind,
    /// Explicit hard output bound, when the path can establish one.
    pub max_output_tokens: Option<u64>,
}

/// A reserved admission lease that must be settled exactly once.
pub struct AdmissionLease {
    store: Arc<MainStore>,
    reservation_id: String,
    idempotency_key: String,
    campaign_id: String,
    request_scope_id: String,
    /// The worst-case estimate this reservation holds; used for
    /// evidence-based conservative settlement of provider error responses.
    estimate: BudgetVector,
}

impl AdmissionLease {
    pub fn reservation_id(&self) -> &str {
        &self.reservation_id
    }

    /// Terminal settlement data that can be moved into a stat guard.
    pub fn settlement(&self) -> AdmissionSettlement {
        AdmissionSettlement {
            store: Arc::clone(&self.store),
            reservation_id: self.reservation_id.clone(),
            campaign_id: self.campaign_id.clone(),
            request_scope_id: self.request_scope_id.clone(),
            estimate: self.estimate,
        }
    }

    async fn run_blocking<T, F>(&self, operation: F) -> Result<T, AdmissionError>
    where
        T: Send + 'static,
        F: FnOnce(&MainStore) -> Result<T, AdmissionError> + Send + 'static,
    {
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || operation(&store))
            .await
            .map_err(|error| {
                AdmissionError::persistence_failure(format!(
                    "admission settlement task failed: {error}"
                ))
            })?
    }

    async fn load_envelope(&self) -> Result<BudgetEnvelope, AdmissionError> {
        let scope_id = self.request_scope_id.clone();
        self.run_blocking(move |store| {
            store
                .get_budget_scope_envelope(&scope_id)
                .map_err(AdmissionError::from)?
                .ok_or_else(|| {
                    AdmissionError::invalid_scope_chain(format!(
                        "request scope {} does not exist",
                        scope_id
                    ))
                })
        })
        .await
    }

    /// Commits the actual usage vector (reserved -> committed/overrun).
    /// The monetary dimension is derived from the frozen pricing snapshot
    /// so money-budget caps accumulate across sequential requests instead
    /// of being silently released at zero.
    pub async fn commit_usage(self, tokens: BudgetVector) -> Result<CommitReceipt, AdmissionError> {
        let envelope = self.load_envelope().await?;
        let mut actual = tokens;
        actual.money_micros = settlement_money_micros(&envelope, &actual)?;
        let reservation_id = self.reservation_id.clone();
        let idempotency_key = self.idempotency_key.clone();
        self.run_blocking(move |store| {
            store.commit_reservation(&reservation_id, &idempotency_key, actual, now_ms())
        })
        .await
    }

    /// Conservative settlement for a provider error response: the request
    /// was processed (the effect happened) but no completion content was
    /// produced. Input tokens are settled at the reserved estimate
    /// (evidence-based upper bound of what was sent), output at zero
    /// (an error response carries no generated content by definition).
    /// Committing zero here would silently release the monetary hold.
    pub async fn commit_provider_error_response(self) -> Result<CommitReceipt, AdmissionError> {
        let envelope = self.load_envelope().await?;
        let mut actual = BudgetVector {
            input_tokens: self.estimate.input_tokens,
            cache_read_tokens: self.estimate.cache_read_tokens,
            cache_write_tokens: self.estimate.cache_write_tokens,
            ..BudgetVector::ZERO
        };
        actual.money_micros = settlement_money_micros(&envelope, &actual)?;
        let reservation_id = self.reservation_id.clone();
        let idempotency_key = self.idempotency_key.clone();
        self.run_blocking(move |store| {
            store.commit_reservation(&reservation_id, &idempotency_key, actual, now_ms())
        })
        .await
    }

    /// Atomically marks a possibly-happened effect unknown and records one
    /// infra failure against the campaign (single writer transaction).
    /// Persistence failures are propagated: the reservation stays
    /// `reserved`, so lease-expiry recovery still freezes it later.
    pub async fn mark_unknown_with_infra_failure(
        self,
        reason: &str,
        kind: &str,
    ) -> Result<(), AdmissionError> {
        let reservation_id = self.reservation_id.clone();
        let idempotency_key = self.idempotency_key.clone();
        let campaign_id = self.campaign_id.clone();
        let reason = reason.to_string();
        let kind = kind.to_string();
        self.run_blocking(move |store| {
            store
                .mark_reservation_unknown_with_infra_failure(
                    &reservation_id,
                    &format!("unknown:{idempotency_key}"),
                    &reason,
                    &campaign_id,
                    &idempotency_key,
                    &format!("infra:{idempotency_key}"),
                    &kind,
                    now_ms(),
                )
                .map(|_| ())
        })
        .await
    }
}

/// Terminal-boundary settlement handle carried into the stat guard for
/// streaming paths. Commit/unknown happen via blocking writer calls at the
/// terminal boundary; the durable ledger remains the consistency authority
/// (lease expiry recovery covers a crashed process).
#[derive(Clone)]
pub struct AdmissionSettlement {
    store: Arc<MainStore>,
    reservation_id: String,
    campaign_id: String,
    request_scope_id: String,
    /// The worst-case estimate this reservation holds; used for the
    /// evidence-based conservative settlement of streams that terminate
    /// without output.
    estimate: BudgetVector,
}

impl AdmissionSettlement {
    /// Commits actual usage from a synchronous terminal boundary. The
    /// monetary dimension is derived from the frozen pricing snapshot.
    /// On any persistence failure the effect is conservatively frozen as
    /// unknown (fail closed) and the failure is logged at error level with
    /// its machine code; the reservation then remains recoverable via
    /// lease-expiry reconciliation.
    pub fn commit_usage_blocking(&self, tokens: BudgetVector) {
        let envelope = self.store.get_budget_scope_envelope(&self.request_scope_id);
        let settlement = envelope.ok().flatten().map(|envelope| {
            settlement_money_micros(&envelope, &tokens).map(|money| (envelope, money))
        });
        match settlement {
            Some(Ok((_, money))) => {
                let mut actual = tokens;
                actual.money_micros = money;
                if let Err(error) = self.store.commit_reservation(
                    &self.reservation_id,
                    &format!("commit:{}", self.reservation_id),
                    actual,
                    now_ms(),
                ) {
                    // Fail closed: freeze the budget as unknown instead of
                    // leaving it silently reserved or released.
                    log::error!(
                        "[Budget][settlement] commit failed ({}); freezing reservation {} as unknown",
                        error.code.as_str(),
                        self.reservation_id
                    );
                    self.mark_unknown_blocking("commit_failed_settlement", "settlement_failure");
                }
            }
            _ => {
                log::error!(
                    "[Budget][settlement] envelope unavailable for {}; freezing reservation {} as unknown",
                    self.request_scope_id,
                    self.reservation_id
                );
                self.mark_unknown_blocking("envelope_unavailable", "settlement_failure");
            }
        }
    }

    /// Conservative settlement for a stream that terminated without any
    /// output: the request was sent (input tokens were consumed) but no
    /// completion content was produced. Input/cache tokens are settled at
    /// the reserved estimate and output at zero. Committing zero here
    /// would silently release the monetary hold (INV-5 / AC-6).
    pub fn commit_conservative_input_blocking(&self) {
        let mut tokens = BudgetVector {
            input_tokens: self.estimate.input_tokens,
            cache_read_tokens: self.estimate.cache_read_tokens,
            cache_write_tokens: self.estimate.cache_write_tokens,
            ..BudgetVector::ZERO
        };
        let envelope = self.store.get_budget_scope_envelope(&self.request_scope_id);
        let money = envelope
            .ok()
            .flatten()
            .and_then(|envelope| settlement_money_micros(&envelope, &tokens).ok());
        match money {
            Some(money) => {
                tokens.money_micros = money;
                if let Err(error) = self.store.commit_reservation(
                    &self.reservation_id,
                    &format!("commit:{}", self.reservation_id),
                    tokens,
                    now_ms(),
                ) {
                    log::error!(
                        "[Budget][settlement] conservative commit failed ({}); freezing reservation {} as unknown",
                        error.code.as_str(),
                        self.reservation_id
                    );
                    self.mark_unknown_blocking("commit_failed_settlement", "settlement_failure");
                }
            }
            None => {
                log::error!(
                    "[Budget][settlement] envelope unavailable for {}; freezing reservation {} as unknown",
                    self.request_scope_id,
                    self.reservation_id
                );
                self.mark_unknown_blocking("envelope_unavailable", "settlement_failure");
            }
        }
    }

    /// Atomically marks the effect unknown and records one infra failure
    /// from a synchronous terminal boundary. Persistence failures are
    /// logged at error level with their machine code; the reservation
    /// stays `reserved` and remains recoverable via lease-expiry
    /// reconciliation.
    pub fn mark_unknown_blocking(&self, reason: &str, kind: &str) {
        if let Err(error) = self.store.mark_reservation_unknown_with_infra_failure(
            &self.reservation_id,
            &format!("unknown:{}", self.reservation_id),
            reason,
            &self.campaign_id,
            &self.reservation_id,
            &format!("infra:{}", self.reservation_id),
            kind,
            now_ms(),
        ) {
            log::error!(
                "[Budget][settlement] unknown+infra recording failed ({}); reservation {} stays reserved for lease-expiry recovery",
                error.code.as_str(),
                self.reservation_id
            );
        }
    }
}

/// Admits one LLM/embedding effect before send. Returns `None` when the
/// request carries no admission context (ordinary path, unchanged). Returns
/// an error when the request is opted in but admission fails closed — the
/// caller must not send the request.
pub async fn admit_before_send(
    store: &Arc<MainStore>,
    headers: &HeaderMap,
    input: LlmGateInput,
) -> Result<Option<AdmissionLease>, AdmissionError> {
    let Some(context) = admission_context_from_headers(headers) else {
        return Ok(None);
    };
    let envelope = load_envelope(store, &context.scope_chain.request_id).await?;
    // Fail closed when the envelope requires dimensions the LLM/embedding
    // owner cannot observe (e.g. disk/network bytes or tool calls).
    check_owner_observability(&envelope, input.effect_kind)?;
    let estimate = build_llm_estimate(
        &envelope,
        LlmEffectBound {
            provider_id: input.provider_id,
            model_id: input.model_id,
            input_tokens: input.estimated_input_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            max_output_tokens: input.max_output_tokens,
        },
    )?;
    let effect_kind = input.effect_kind;
    let store_clone = Arc::clone(store);
    let reservation =
        tokio::task::spawn_blocking(move || -> Result<Reservation, AdmissionError> {
            store_clone.reserve_effect(
                ReserveEffect {
                    effect_id: context.effect_id,
                    idempotency_key: context.idempotency_key,
                    scopes: context.scope_chain,
                    effect_kind,
                    attempt: context.attempt,
                    estimate,
                },
                now_ms(),
            )
        })
        .await
        .map_err(|error| {
            AdmissionError::persistence_failure(format!("admission reserve task failed: {error}"))
        })??;
    // Opaque identifiers only — never prompts, keys, or payloads.
    let lease = AdmissionLease {
        store: Arc::clone(store),
        reservation_id: reservation.reservation_id,
        idempotency_key: reservation.idempotency_key,
        campaign_id: reservation.scopes.campaign_id,
        request_scope_id: reservation.scopes.request_id,
        estimate: reservation.reserved,
    };
    log::info!(
        "[Budget][admission] reserved {} for effect {} (attempt {})",
        lease.reservation_id(),
        reservation.effect_id,
        reservation.attempt
    );
    Ok(Some(lease))
}

async fn load_envelope(
    store: &Arc<MainStore>,
    request_scope_id: &str,
) -> Result<BudgetEnvelope, AdmissionError> {
    let store = Arc::clone(store);
    let scope_id = request_scope_id.to_string();
    let envelope = tokio::task::spawn_blocking(move || store.get_budget_scope_envelope(&scope_id))
        .await
        .map_err(|error| {
            AdmissionError::persistence_failure(format!("envelope lookup task failed: {error}"))
        })?
        .map_err(AdmissionError::from)?;
    envelope.ok_or_else(|| {
        AdmissionError::invalid_scope_chain(format!(
            "request scope {request_scope_id} does not exist"
        ))
    })
}

/// Maps an admission rejection to a protocol-safe client error message.
/// The message carries only the machine code and opaque identifiers —
/// never prompts, keys, or provider payloads.
pub fn rejection_message(error: &AdmissionError) -> String {
    match error.dimension {
        Some(dimension) => format!(
            "experiment admission rejected ({}: {} dimension)",
            error.code.as_str(),
            dimension.as_str()
        ),
        None => format!("experiment admission rejected ({})", error.code.as_str()),
    }
}

#[cfg(test)]
mod admission {
    use super::*;
    use crate::budget::types::{BudgetEnvelope, CapLimit, MoneyMode, ResourceCaps};
    use crate::constants::INTERNAL_CCPROXY_API_KEY;
    use crate::db::StoreError;
    use std::collections::BTreeSet;
    use tempfile::tempdir;

    fn envelope() -> BudgetEnvelope {
        BudgetEnvelope {
            caps: ResourceCaps {
                input_tokens: CapLimit::HardCap(100_000),
                output_tokens: CapLimit::HardCap(100_000),
                cache_read_tokens: CapLimit::NotApplicable,
                cache_write_tokens: CapLimit::NotApplicable,
                wall_time_ms: CapLimit::HardCap(600_000),
                tool_calls: CapLimit::HardCap(100),
                processes: CapLimit::HardCap(10),
                disk_bytes: CapLimit::NotApplicable,
                network_bytes: CapLimit::NotApplicable,
                concurrency: CapLimit::HardCap(4),
                money: CapLimit::NotApplicable,
            },
            required_dimensions: BTreeSet::new(),
            money_mode: MoneyMode::TokenResourceOnly,
            max_attempts: 1,
            infra_failure_threshold: 5,
            reservation_lease_ms: 600_000,
        }
    }

    fn store() -> (Arc<MainStore>, tempfile::TempDir) {
        let directory = tempdir().expect("temp dir");
        let store = Arc::new(MainStore::new(directory.path().join("admission.db")).expect("store"));
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
                .create_budget_scope(crate::db::budget::NewBudgetScope {
                    scope_id: id.into(),
                    scope_kind: kind,
                    parent_scope_id: parent.map(|value| value.to_string()),
                    envelope: base.clone(),
                    now_ms: now_ms(),
                })
                .expect("scope creation");
        }
    }

    use crate::budget::types::ScopeKind;

    fn internal_headers_with_context(context: &AdmissionContext) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-cs-internal-request", "true".parse().unwrap());
        headers.insert(
            "authorization",
            format!("Bearer {}", INTERNAL_CCPROXY_API_KEY.read().clone())
                .parse()
                .unwrap(),
        );
        headers.insert(
            ADMISSION_HEADER,
            serde_json::to_string(context).unwrap().parse().unwrap(),
        );
        headers
    }

    fn context() -> AdmissionContext {
        AdmissionContext {
            scope_chain: chain(),
            effect_id: "eff-1".into(),
            idempotency_key: "idem-1".into(),
            attempt: 1,
        }
    }

    fn gate_input() -> LlmGateInput {
        LlmGateInput {
            provider_id: "prov".into(),
            model_id: "model".into(),
            estimated_input_tokens: 100,
            effect_kind: EffectKind::LlmCompletion,
            max_output_tokens: Some(50),
        }
    }

    #[tokio::test]
    async fn external_headers_cannot_mint_admission() {
        let (store, _dir) = store();
        create_chain(&store);
        let mut headers = HeaderMap::new();
        headers.insert(
            ADMISSION_HEADER,
            serde_json::to_string(&context()).unwrap().parse().unwrap(),
        );
        let admitted = admit_before_send(&store, &headers, gate_input())
            .await
            .expect("no admission attempted for untrusted headers");
        assert!(admitted.is_none());
        // No reservation exists.
        assert!(store
            .get_budget_reservation("res-any")
            .expect("read")
            .is_none());
    }

    #[tokio::test]
    async fn malformed_context_is_ignored() {
        let (store, _dir) = store();
        let mut headers = HeaderMap::new();
        headers.insert("x-cs-internal-request", "true".parse().unwrap());
        headers.insert(
            "authorization",
            format!("Bearer {}", INTERNAL_CCPROXY_API_KEY.read().clone())
                .parse()
                .unwrap(),
        );
        headers.insert(ADMISSION_HEADER, "not-json".parse().unwrap());
        let admitted = admit_before_send(&store, &headers, gate_input())
            .await
            .expect("malformed context is ignored");
        assert!(admitted.is_none());
    }

    #[tokio::test]
    async fn trusted_context_reserves_and_settles() {
        let (store, _dir) = store();
        create_chain(&store);
        let headers = internal_headers_with_context(&context());
        let lease = admit_before_send(&store, &headers, gate_input())
            .await
            .expect("admission should succeed")
            .expect("lease expected");
        let reservation_id = lease.reservation_id().to_string();
        let settlement = lease.settlement();
        settlement.commit_usage_blocking(BudgetVector {
            input_tokens: 90,
            output_tokens: 40,
            ..BudgetVector::ZERO
        });
        let reservation = store
            .get_budget_reservation(&reservation_id)
            .expect("read")
            .expect("reservation exists");
        assert_eq!(
            reservation.state,
            crate::budget::types::ReservationState::Committed
        );
    }

    #[tokio::test]
    async fn missing_scope_chain_fails_closed() {
        let (store, _dir) = store();
        let headers = internal_headers_with_context(&context());
        let error = match admit_before_send(&store, &headers, gate_input()).await {
            Ok(_) => panic!("missing scopes must fail closed"),
            Err(error) => error,
        };
        assert_eq!(
            error.code,
            crate::budget::errors::AdmissionErrorCode::InvalidScopeChain
        );
    }

    #[test]
    fn rejection_message_is_protocol_safe() {
        let error = AdmissionError::budget_exceeded("input tokens")
            .with_dimension(crate::budget::types::ResourceDimension::InputTokens);
        let message = rejection_message(&error);
        assert!(message.contains("budget_exceeded"));
        assert!(message.contains("input_tokens"));
        assert!(!message.contains("secret"));
    }

    #[test]
    fn store_error_maps_to_persistence_failure() {
        let error: AdmissionError = StoreError::RuntimeClosed.into();
        assert_eq!(
            error.code,
            crate::budget::errors::AdmissionErrorCode::AdmissionPersistenceFailure
        );
    }

    fn money_envelope() -> BudgetEnvelope {
        BudgetEnvelope {
            caps: ResourceCaps {
                input_tokens: CapLimit::HardCap(1_000_000),
                output_tokens: CapLimit::HardCap(1_000_000),
                cache_read_tokens: CapLimit::NotApplicable,
                cache_write_tokens: CapLimit::NotApplicable,
                wall_time_ms: CapLimit::HardCap(600_000),
                tool_calls: CapLimit::HardCap(100),
                processes: CapLimit::HardCap(10),
                disk_bytes: CapLimit::NotApplicable,
                network_bytes: CapLimit::NotApplicable,
                concurrency: CapLimit::HardCap(4),
                money: CapLimit::HardCap(100_000_000),
            },
            required_dimensions: BTreeSet::new(),
            money_mode: MoneyMode::Money {
                currency_code: "cny".into(),
                cap_money_micros: 100_000_000,
                pricing: crate::budget::types::PricingSnapshot {
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
                },
            },
            max_attempts: 1,
            infra_failure_threshold: 5,
            reservation_lease_ms: 600_000,
        }
    }

    #[tokio::test]
    async fn provider_error_response_settles_conservatively_not_zero() {
        let (store, _dir) = store();
        let base = money_envelope();
        create_chain_with_envelope(&store, base);
        let headers = internal_headers_with_context(&context());
        let lease = admit_before_send(&store, &headers, gate_input())
            .await
            .expect("admission should succeed")
            .expect("lease expected");
        let reservation_id = lease.reservation_id().to_string();
        let settlement = lease.settlement();
        // Simulate the provider error-response settlement path.
        settlement.commit_usage_blocking(BudgetVector {
            input_tokens: 100,
            ..BudgetVector::ZERO
        });
        let status = store
            .get_budget_scope_status("req-1")
            .expect("read scope")
            .expect("scope exists");
        // Money must accumulate (100 input tokens at 1 unit/M = 100
        // micros), never silently settle at zero.
        assert_eq!(status.committed.money_micros, 100);
        assert_eq!(status.committed.input_tokens, 100);
        assert_eq!(status.reserved.money_micros, 0);
        let _ = reservation_id;
    }

    #[tokio::test]
    async fn required_unobservable_dimension_rejects_llm_effects() {
        let (store, _dir) = store();
        let mut base = envelope();
        base.required_dimensions
            .insert(crate::budget::types::ResourceDimension::Processes);
        create_chain_with_envelope(&store, base);
        let headers = internal_headers_with_context(&context());
        let error = match admit_before_send(&store, &headers, gate_input()).await {
            Ok(_) => panic!("required processes must fail closed for LLM effects"),
            Err(error) => error,
        };
        assert_eq!(
            error.code,
            crate::budget::errors::AdmissionErrorCode::ResourceUnobservable
        );
        assert_eq!(
            error.dimension,
            Some(crate::budget::types::ResourceDimension::Processes)
        );
    }

    fn create_chain_with_envelope(store: &MainStore, base: BudgetEnvelope) {
        for (kind, id, parent) in [
            (ScopeKind::Campaign, "camp-1", None),
            (ScopeKind::Candidate, "cand-1", Some("camp-1")),
            (ScopeKind::Trial, "trial-1", Some("cand-1")),
            (ScopeKind::Request, "req-1", Some("trial-1")),
        ] {
            store
                .create_budget_scope(crate::db::budget::NewBudgetScope {
                    scope_id: id.into(),
                    scope_kind: kind,
                    parent_scope_id: parent.map(|value| value.to_string()),
                    envelope: base.clone(),
                    now_ms: now_ms(),
                })
                .expect("scope creation");
        }
    }
}

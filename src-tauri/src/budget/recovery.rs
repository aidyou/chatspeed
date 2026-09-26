//! Crash recovery and infra failure classification for the budget ledger
//! (Phase 2B).
//!
//! Recovery is conservative: a reservation whose lease expired while still
//! `reserved` may have produced a physical effect, so it is frozen as
//! `unknown` — never released and never settled at zero cost (INV-5).
//! Recovery never replays provider requests; it only reconciles durable
//! ledger state. Logs carry opaque identifiers and machine codes only.

use crate::budget::errors::AdmissionError;
use crate::db::MainStore;
use std::sync::Arc;

/// Classification of an effect failure. Only infrastructure failures
/// (transport, timeout, rate limit, stream failure) count against the
/// campaign infra threshold; auth/model-not-found are correctness
/// failures of the request itself and must not pause a campaign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfraFailureKind {
    /// Connection/transport error; the effect outcome is ambiguous.
    Transport,
    /// The request timed out; the effect outcome is ambiguous.
    Timeout,
    /// Provider rate limiting (429) exhausted its (zero) retry budget.
    RateLimit,
    /// The response stream failed mid-flight.
    StreamFailure,
    /// Authentication failure — a correctness failure, not infra.
    AuthFailure,
    /// Model not found — a correctness failure, not infra.
    ModelNotFound,
}

impl InfraFailureKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            InfraFailureKind::Transport => "transport",
            InfraFailureKind::Timeout => "timeout",
            InfraFailureKind::RateLimit => "rate_limit",
            InfraFailureKind::StreamFailure => "stream_failure",
            InfraFailureKind::AuthFailure => "auth_failure",
            InfraFailureKind::ModelNotFound => "model_not_found",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "transport" => Some(InfraFailureKind::Transport),
            "timeout" => Some(InfraFailureKind::Timeout),
            "rate_limit" => Some(InfraFailureKind::RateLimit),
            "stream_failure" => Some(InfraFailureKind::StreamFailure),
            "auth_failure" => Some(InfraFailureKind::AuthFailure),
            "model_not_found" => Some(InfraFailureKind::ModelNotFound),
            _ => None,
        }
    }

    /// Whether this kind counts against the campaign infra threshold.
    pub fn is_infra(&self) -> bool {
        matches!(
            self,
            InfraFailureKind::Transport
                | InfraFailureKind::Timeout
                | InfraFailureKind::RateLimit
                | InfraFailureKind::StreamFailure
        )
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Recovers expired reservations: every reservation whose lease elapsed
/// while still `reserved` is frozen as unknown. Returns the reconciled
/// reservation ids (opaque). Safe to call repeatedly; idempotent.
pub async fn recover_expired_reservations(
    store: &Arc<MainStore>,
) -> Result<Vec<String>, AdmissionError> {
    let store = Arc::clone(store);
    let reconciled =
        tokio::task::spawn_blocking(move || store.reconcile_expired_reservations(now_ms()))
            .await
            .map_err(|error| {
                AdmissionError::persistence_failure(format!("recovery task failed: {error}"))
            })??;
    if !reconciled.is_empty() {
        // Opaque identifiers only — never prompts, keys, or payloads.
        log::info!(
            "[Budget][recovery] froze {} expired reservation(s) as unknown: {}",
            reconciled.len(),
            reconciled.join(",")
        );
    }
    Ok(reconciled)
}

#[cfg(test)]
mod budget_recovery {
    use super::*;
    use crate::budget::types::{BudgetEnvelope, CapLimit, MoneyMode, ResourceCaps, ScopeKind};
    use crate::db::budget::NewBudgetScope;
    use std::collections::BTreeSet;
    use tempfile::tempdir;

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

    fn store() -> (Arc<MainStore>, tempfile::TempDir) {
        let directory = tempdir().expect("temp dir");
        let store = Arc::new(MainStore::new(directory.path().join("recovery.db")).expect("store"));
        (store, directory)
    }

    fn create_chain(store: &MainStore) {
        let base = envelope();
        for (kind, id, parent) in [
            (ScopeKind::Campaign, "camp-1".to_string(), None),
            (
                ScopeKind::Candidate,
                "cand-1".to_string(),
                Some("camp-1".to_string()),
            ),
            (
                ScopeKind::Trial,
                "trial-1".to_string(),
                Some("cand-1".to_string()),
            ),
            (
                ScopeKind::Request,
                "req-1".to_string(),
                Some("trial-1".to_string()),
            ),
        ] {
            store
                .create_budget_scope(NewBudgetScope {
                    scope_id: id,
                    scope_kind: kind,
                    parent_scope_id: parent,
                    envelope: base.clone(),
                    now_ms: 1_700_000_000_000,
                })
                .expect("scope creation");
        }
    }

    #[test]
    fn infra_classification_separates_correctness_failures() {
        assert!(InfraFailureKind::Transport.is_infra());
        assert!(InfraFailureKind::Timeout.is_infra());
        assert!(InfraFailureKind::RateLimit.is_infra());
        assert!(InfraFailureKind::StreamFailure.is_infra());
        assert!(!InfraFailureKind::AuthFailure.is_infra());
        assert!(!InfraFailureKind::ModelNotFound.is_infra());
        // Round-trip parsing.
        for kind in [
            InfraFailureKind::Transport,
            InfraFailureKind::Timeout,
            InfraFailureKind::RateLimit,
            InfraFailureKind::StreamFailure,
            InfraFailureKind::AuthFailure,
            InfraFailureKind::ModelNotFound,
        ] {
            assert_eq!(InfraFailureKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(InfraFailureKind::parse("bogus"), None);
    }

    #[tokio::test]
    async fn startup_recovery_freezes_expired_reservations_as_unknown() {
        let (store, _dir) = store();
        create_chain(&store);
        let reservation = store
            .reserve_effect(
                crate::budget::ReserveEffect {
                    effect_id: "eff-1".into(),
                    idempotency_key: "idem-1".into(),
                    scopes: crate::budget::types::ScopeChain {
                        request_id: "req-1".into(),
                        trial_id: "trial-1".into(),
                        candidate_id: "cand-1".into(),
                        campaign_id: "camp-1".into(),
                    },
                    effect_kind: crate::budget::types::EffectKind::LlmCompletion,
                    attempt: 1,
                    estimate: crate::budget::types::BudgetVector {
                        input_tokens: 10,
                        ..crate::budget::types::BudgetVector::ZERO
                    },
                },
                1_700_000_000_000,
            )
            .expect("reserve should succeed");

        let reconciled = recover_expired_reservations(&store)
            .await
            .expect("recovery should succeed");
        assert_eq!(reconciled, vec![reservation.reservation_id.clone()]);

        let stored = store
            .get_budget_reservation(&reservation.reservation_id)
            .expect("read")
            .expect("reservation exists");
        assert_eq!(
            stored.state,
            crate::budget::types::ReservationState::Unknown
        );
        // The budget stays frozen; recovery never releases (INV-5).
        let status = store
            .get_budget_scope_status("req-1")
            .expect("read scope")
            .expect("scope exists");
        assert_eq!(status.reserved.input_tokens, 10);

        // Repeated recovery is idempotent.
        let again = recover_expired_reservations(&store)
            .await
            .expect("recovery");
        assert!(again.is_empty());
    }

    #[tokio::test]
    async fn live_reservations_are_not_touched_by_recovery() {
        let (store, _dir) = store();
        create_chain(&store);
        let _live = store
            .reserve_effect(
                crate::budget::ReserveEffect {
                    effect_id: "eff-2".into(),
                    idempotency_key: "idem-2".into(),
                    scopes: crate::budget::types::ScopeChain {
                        request_id: "req-1".into(),
                        trial_id: "trial-1".into(),
                        candidate_id: "cand-1".into(),
                        campaign_id: "camp-1".into(),
                    },
                    effect_kind: crate::budget::types::EffectKind::LlmCompletion,
                    attempt: 1,
                    estimate: crate::budget::types::BudgetVector {
                        input_tokens: 5,
                        ..crate::budget::types::BudgetVector::ZERO
                    },
                },
                now_ms(),
            )
            .expect("reserve should succeed");
        let reconciled = recover_expired_reservations(&store)
            .await
            .expect("recovery");
        assert!(reconciled.is_empty());
    }
}

//! Tool/process effect admission and resource measurement boundary
//! (Phase 2B).
//!
//! A workflow session is gated when the backend has created a budget
//! request scope whose id equals the session id. Scope chains can only be
//! created through the backend-owned `MainStore` ledger API (no CLI, no
//! HTTP route), so an external caller can never opt a session into the
//! gate or forge its ownership (INV-6). Sessions without such a scope keep
//! the ordinary path unchanged (INV-2).
//!
//! Measurement honesty (A-4): tool call counts, process spawns and wall
//! time are observable at this boundary. Disk bytes and network bytes have
//! no reliable owner instrumentation yet, so an envelope that hard-caps
//! them fails admission closed (`resource_unobservable`) instead of
//! pretending zero usage.

use crate::budget::errors::AdmissionError;
use crate::budget::types::{
    BudgetEnvelope, BudgetVector, CapLimit, EffectKind, ReserveEffect, ResourceDimension,
};
use crate::budget::Reservation;
use crate::db::MainStore;
use std::sync::Arc;

/// Resource dimensions the LLM/embedding effect owner can reliably bound
/// at admission and measure at the terminal boundary.
const LLM_OBSERVABLE_DIMENSIONS: &[ResourceDimension] = &[
    ResourceDimension::InputTokens,
    ResourceDimension::OutputTokens,
    ResourceDimension::CacheReadTokens,
    ResourceDimension::CacheWriteTokens,
    ResourceDimension::WallTimeMs,
    ResourceDimension::Concurrency,
    ResourceDimension::Money,
];

/// Resource dimensions the tool/process effect owner can reliably bound at
/// admission and measure at the terminal boundary. Disk and network bytes
/// have no owner instrumentation yet (A-4).
const TOOL_OBSERVABLE_DIMENSIONS: &[ResourceDimension] = &[
    ResourceDimension::ToolCalls,
    ResourceDimension::Processes,
    ResourceDimension::WallTimeMs,
    ResourceDimension::Concurrency,
];

/// Fails closed when the frozen envelope requires a dimension that the
/// effect owner for `effect_kind` cannot reliably observe: such a
/// dimension must never be admitted with an implicit zero measurement
/// (AC-3 / A-4).
pub fn check_owner_observability(
    envelope: &BudgetEnvelope,
    effect_kind: EffectKind,
) -> Result<(), AdmissionError> {
    let observable: &[ResourceDimension] = match effect_kind {
        EffectKind::LlmCompletion | EffectKind::Embedding => LLM_OBSERVABLE_DIMENSIONS,
        EffectKind::ToolCall | EffectKind::Process => TOOL_OBSERVABLE_DIMENSIONS,
    };
    for dimension in &envelope.required_dimensions {
        if !observable.contains(dimension) {
            return Err(AdmissionError::resource_unobservable(format!(
                "required dimension {} cannot be reliably observed by the {:?} effect owner",
                dimension.as_str(),
                effect_kind
            ))
            .with_dimension(*dimension));
        }
    }
    Ok(())
}

/// A lease for one admitted physical tool effect. It must be settled
/// exactly once: committed after the tool terminal boundary, or marked
/// unknown when the outcome cannot be determined (e.g. cancellation while
/// the tool was running).
pub struct ToolAdmissionLease {
    reservation_id: String,
}

impl std::fmt::Debug for ToolAdmissionLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolAdmissionLease")
            .field("reservation_id", &self.reservation_id)
            .finish()
    }
}

impl ToolAdmissionLease {
    pub fn reservation_id(&self) -> &str {
        &self.reservation_id
    }
}

/// Whether the named native tool spawns an OS process. MCP tools and any
/// unknown tool are treated as potentially spawning processes.
pub fn spawns_process(tool_name: &str) -> bool {
    match tool_name {
        crate::tools::TOOL_BASH => true,
        crate::tools::TOOL_MCP_TOOL_EXECUTE => true,
        crate::tools::TOOL_MCP_TOOL_EXPAND => true,
        // Read-only / structural native tools do not spawn processes.
        crate::tools::TOOL_READ_FILE
        | crate::tools::TOOL_WRITE_FILE
        | crate::tools::TOOL_EDIT_FILE
        | crate::tools::TOOL_LIST_DIR
        | crate::tools::TOOL_GLOB
        | crate::tools::TOOL_GREP
        | crate::tools::TOOL_GIT_DIFF
        | crate::tools::TOOL_GIT_INSPECT
        | crate::tools::TOOL_WEB_SEARCH
        | crate::tools::TOOL_WEB_FETCH
        | crate::tools::TOOL_TODO_CREATE
        | crate::tools::TOOL_TODO_LIST
        | crate::tools::TOOL_TODO_UPDATE
        | crate::tools::TOOL_TODO_GET
        | crate::tools::TOOL_SKILL
        | crate::tools::TOOL_ASK_USER
        | crate::tools::TOOL_COMPLETE_WORKFLOW
        | crate::tools::TOOL_SUBMIT_RESULT
        | crate::tools::TOOL_SUBMIT_PLAN => false,
        // Unknown tools are conservatively treated as process-spawning.
        _ => true,
    }
}

/// Admits one physical tool effect for a workflow session. Returns `None`
/// when the session has no backend-created budget request scope (ordinary
/// path). Fails closed when the frozen envelope caps a dimension this
/// boundary cannot reliably bound or measure.
pub async fn admit_tool_effect(
    store: &Arc<MainStore>,
    session_id: &str,
    tool_name: &str,
) -> Result<Option<ToolAdmissionLease>, AdmissionError> {
    let envelope = match store.get_budget_scope_envelope(session_id) {
        Ok(Some(envelope)) => envelope,
        Ok(None) => return Ok(None),
        Err(error) => return Err(AdmissionError::from(error)),
    };
    envelope.validate()?;
    // Fail closed when the envelope requires dimensions this owner cannot
    // observe (e.g. disk/network bytes for tools).
    check_owner_observability(&envelope, crate::budget::types::EffectKind::ToolCall)?;
    // Fail closed on dimensions without reliable owner instrumentation.
    for dimension in [
        ResourceDimension::DiskBytes,
        ResourceDimension::NetworkBytes,
    ] {
        if matches!(envelope.caps.cap(dimension), CapLimit::HardCap(_)) {
            return Err(AdmissionError::resource_unobservable(format!(
                "required dimension {} cannot be reliably measured for tool effects",
                dimension.as_str()
            ))
            .with_dimension(dimension));
        }
    }
    let estimate = BudgetVector {
        tool_calls: 1,
        processes: if spawns_process(tool_name) { 1 } else { 0 },
        concurrency: 1,
        ..BudgetVector::ZERO
    };
    let effect_id = format!("tool:{session_id}:{}", uuid::Uuid::new_v4().simple());
    let idempotency_key = format!("idem:{effect_id}");
    let store_clone = Arc::clone(store);
    let session_id = session_id.to_string();
    let reservation =
        tokio::task::spawn_blocking(move || -> Result<Reservation, AdmissionError> {
            store_clone.reserve_effect(
                ReserveEffect {
                    effect_id,
                    idempotency_key,
                    scopes: crate::budget::types::ScopeChain {
                        request_id: session_id.clone(),
                        trial_id: format!("{session_id}:trial"),
                        candidate_id: format!("{session_id}:candidate"),
                        campaign_id: format!("{session_id}:campaign"),
                    },
                    effect_kind: crate::budget::types::EffectKind::ToolCall,
                    attempt: 1,
                    estimate,
                },
                now_ms(),
            )
        })
        .await
        .map_err(|error| {
            AdmissionError::persistence_failure(format!("tool admission task failed: {error}"))
        })??;
    Ok(Some(ToolAdmissionLease {
        reservation_id: reservation.reservation_id,
    }))
}

/// Commits the actual usage of one finished tool effect. Wall time is the
/// owner-observed execution duration and the process count is the real
/// spawned-process usage; the concurrency lease is released by the commit
/// (actual concurrency is zero at the terminal boundary). Persistence
/// failures are propagated: the caller must fall back to the conservative
/// unknown+infra settlement below.
pub async fn commit_tool_effect(
    store: &Arc<MainStore>,
    reservation_id: &str,
    tool_name: &str,
    wall_time_ms: u64,
) -> Result<(), AdmissionError> {
    let store = Arc::clone(store);
    let reservation_id = reservation_id.to_string();
    let tool_name = tool_name.to_string();
    tokio::task::spawn_blocking(move || {
        store.commit_reservation(
            &reservation_id,
            &format!("commit:{reservation_id}"),
            BudgetVector {
                tool_calls: 1,
                processes: if spawns_process(&tool_name) { 1 } else { 0 },
                wall_time_ms,
                ..BudgetVector::ZERO
            },
            now_ms(),
        )
    })
    .await
    .map_err(|error| {
        AdmissionError::persistence_failure(format!("tool commit task failed: {error}"))
    })??;
    Ok(())
}

/// Atomically marks a possibly-executed tool effect unknown AND records
/// one infra failure against the campaign (single writer transaction, so
/// the campaign infra count and threshold pause always track the unknown
/// fact). Persistence failures are propagated: the reservation stays
/// `reserved` and remains recoverable via lease-expiry reconciliation.
pub async fn mark_tool_effect_unknown(
    store: &Arc<MainStore>,
    reservation_id: &str,
    campaign_id: &str,
    effect_id: &str,
    reason: &str,
    kind: &str,
) -> Result<(), AdmissionError> {
    let store = Arc::clone(store);
    let reservation_id = reservation_id.to_string();
    let campaign_id = campaign_id.to_string();
    let effect_id = effect_id.to_string();
    let reason = reason.to_string();
    let kind = kind.to_string();
    tokio::task::spawn_blocking(move || {
        store
            .mark_reservation_unknown_with_infra_failure(
                &reservation_id,
                &format!("unknown:{reservation_id}"),
                &reason,
                &campaign_id,
                &effect_id,
                &format!("infra:{reservation_id}"),
                &kind,
                now_ms(),
            )
            .map(|_| ())
    })
    .await
    .map_err(|error| {
        AdmissionError::persistence_failure(format!("tool unknown task failed: {error}"))
    })?
}

/// Releases the reservation of a queued tool effect that is proven never
/// to have been dispatched (e.g. postponed and later dropped). Persistence
/// failures are propagated: the reservation stays `reserved` and remains
/// recoverable via lease-expiry reconciliation.
pub async fn release_tool_effect(
    store: &Arc<MainStore>,
    reservation_id: &str,
    reason: &str,
) -> Result<(), AdmissionError> {
    let store = Arc::clone(store);
    let reservation_id = reservation_id.to_string();
    let reason = reason.to_string();
    tokio::task::spawn_blocking(move || {
        store
            .release_reservation(
                &reservation_id,
                &format!("release:{reservation_id}"),
                &reason,
                now_ms(),
            )
            .map(|_| ())
    })
    .await
    .map_err(|error| {
        AdmissionError::persistence_failure(format!("tool release task failed: {error}"))
    })?
}

/// Releases a batch of reservations whose effects are proven never to
/// have been dispatched (e.g. a turn cancelled or failed between the
/// partition stage and physical dispatch). Proven no-effect paths must
/// release instead of freezing the budget as unknown (AC-6/AC-7).
/// Persistence failures are propagated after attempting every release.
pub async fn release_proven_not_dispatched(
    store: &Arc<MainStore>,
    reservation_ids: &[String],
    reason: &str,
) -> Result<(), AdmissionError> {
    let mut first_error: Option<AdmissionError> = None;
    for reservation_id in reservation_ids {
        if let Err(error) = release_tool_effect(store, reservation_id, reason).await {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod budget_resource {
    use super::*;
    use crate::budget::types::{BudgetEnvelope, MoneyMode, ResourceCaps, ScopeKind};
    use crate::db::budget::NewBudgetScope;
    use std::collections::BTreeSet;
    use tempfile::tempdir;

    fn envelope(disk_cap: CapLimit) -> BudgetEnvelope {
        BudgetEnvelope {
            caps: ResourceCaps {
                input_tokens: CapLimit::NotApplicable,
                output_tokens: CapLimit::NotApplicable,
                cache_read_tokens: CapLimit::NotApplicable,
                cache_write_tokens: CapLimit::NotApplicable,
                wall_time_ms: CapLimit::NotApplicable,
                tool_calls: CapLimit::HardCap(10),
                processes: CapLimit::HardCap(4),
                disk_bytes: disk_cap,
                network_bytes: CapLimit::NotApplicable,
                concurrency: CapLimit::HardCap(2),
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
        let store = Arc::new(MainStore::new(directory.path().join("resource.db")).expect("store"));
        (store, directory)
    }

    fn create_chain(store: &MainStore, session_id: &str, envelope: BudgetEnvelope) {
        for (kind, id, parent) in [
            (ScopeKind::Campaign, format!("{session_id}:campaign"), None),
            (
                ScopeKind::Candidate,
                format!("{session_id}:candidate"),
                Some(format!("{session_id}:campaign")),
            ),
            (
                ScopeKind::Trial,
                format!("{session_id}:trial"),
                Some(format!("{session_id}:candidate")),
            ),
            (
                ScopeKind::Request,
                session_id.to_string(),
                Some(format!("{session_id}:trial")),
            ),
        ] {
            store
                .create_budget_scope(NewBudgetScope {
                    scope_id: id,
                    scope_kind: kind,
                    parent_scope_id: parent,
                    envelope: envelope.clone(),
                    now_ms: now_ms(),
                })
                .expect("scope creation");
        }
    }

    #[tokio::test]
    async fn ungated_session_keeps_ordinary_path() {
        let (store, _dir) = store();
        let admitted = admit_tool_effect(&store, "plain-session", "bash")
            .await
            .expect("no gate for sessions without a scope");
        assert!(admitted.is_none());
    }

    #[tokio::test]
    async fn gated_session_reserves_and_commits_tool_effect() {
        let (store, _dir) = store();
        create_chain(&store, "sess-1", envelope(CapLimit::NotApplicable));
        let lease = admit_tool_effect(&store, "sess-1", "bash")
            .await
            .expect("admission should succeed")
            .expect("gated session must be admitted");
        let reservation_id = lease.reservation_id().to_string();
        commit_tool_effect(&store, &reservation_id, "bash", 1_500)
            .await
            .expect("commit should succeed");
        let status = store
            .get_budget_scope_status("sess-1")
            .expect("read scope")
            .expect("scope exists");
        assert_eq!(status.committed.tool_calls, 1);
        assert_eq!(status.committed.wall_time_ms, 1_500);
        assert_eq!(status.reserved.tool_calls, 0);
        assert_eq!(status.reserved.concurrency, 0);
    }

    #[tokio::test]
    async fn unobservable_required_dimension_fails_closed() {
        let (store, _dir) = store();
        create_chain(&store, "sess-2", envelope(CapLimit::HardCap(1_024)));
        let error = admit_tool_effect(&store, "sess-2", "bash")
            .await
            .expect_err("disk-capped envelope must fail closed");
        assert_eq!(
            error.code,
            crate::budget::errors::AdmissionErrorCode::ResourceUnobservable
        );
        assert_eq!(error.dimension, Some(ResourceDimension::DiskBytes));
    }

    #[tokio::test]
    async fn concurrency_cap_limits_parallel_tool_admissions() {
        let (store, _dir) = store();
        create_chain(&store, "sess-3", envelope(CapLimit::NotApplicable));
        let first = admit_tool_effect(&store, "sess-3", "bash")
            .await
            .expect("first admission")
            .expect("lease");
        let second = admit_tool_effect(&store, "sess-3", "read_file")
            .await
            .expect("second admission")
            .expect("lease");
        // Concurrency cap 2 is now exhausted; a third tool must be refused.
        let error = admit_tool_effect(&store, "sess-3", "web_fetch")
            .await
            .expect_err("third admission must exceed the concurrency cap");
        assert_eq!(
            error.code,
            crate::budget::errors::AdmissionErrorCode::BudgetExceeded
        );
        // Settling one lease frees its concurrency slot.
        commit_tool_effect(&store, first.reservation_id(), "bash", 10)
            .await
            .expect("commit");
        let third = admit_tool_effect(&store, "sess-3", "web_fetch")
            .await
            .expect("admission after settle");
        assert!(third.is_some());
        drop(second);
    }

    #[tokio::test]
    async fn process_cap_limits_spawning_tools() {
        let (store, _dir) = store();
        let mut base = envelope(CapLimit::NotApplicable);
        base.caps.processes = CapLimit::HardCap(1);
        create_chain(&store, "sess-p", base);
        // First bash admission occupies the single process slot.
        let first = admit_tool_effect(&store, "sess-p", "bash")
            .await
            .expect("first admission")
            .expect("lease");
        // A second process-spawning tool exceeds the process cap.
        let error = admit_tool_effect(&store, "sess-p", "bash")
            .await
            .expect_err("second spawning tool must exceed the process cap");
        assert_eq!(
            error.code,
            crate::budget::errors::AdmissionErrorCode::BudgetExceeded
        );
        // A non-spawning tool still fits (processes estimate 0).
        let non_spawning = admit_tool_effect(&store, "sess-p", "read_file")
            .await
            .expect("non-spawning admission");
        assert!(non_spawning.is_some());
        // Settling the first tool commits its real process usage; the
        // process cap is cumulative, so with cap 1 no further spawning
        // tool can ever be admitted.
        commit_tool_effect(&store, first.reservation_id(), "bash", 5)
            .await
            .expect("commit");
        let status = store
            .get_budget_scope_status("sess-p")
            .expect("read scope")
            .expect("scope exists");
        assert_eq!(status.committed.processes, 1);
        let error = admit_tool_effect(&store, "sess-p", "bash")
            .await
            .expect_err("cumulative process cap must refuse further spawns");
        assert_eq!(
            error.code,
            crate::budget::errors::AdmissionErrorCode::BudgetExceeded
        );
    }

    #[tokio::test]
    async fn required_unobservable_dimension_rejects_tool_effects() {
        let (store, _dir) = store();
        let mut base = envelope(CapLimit::NotApplicable);
        base.required_dimensions
            .insert(ResourceDimension::NetworkBytes);
        // A required dimension must carry its hard cap to form a valid
        // envelope; the tool owner still cannot observe it.
        base.caps.network_bytes = CapLimit::HardCap(1_024);
        create_chain(&store, "sess-r", base);
        let error = admit_tool_effect(&store, "sess-r", "bash")
            .await
            .expect_err("required network bytes must fail closed for tools");
        assert_eq!(
            error.code,
            crate::budget::errors::AdmissionErrorCode::ResourceUnobservable
        );
        assert_eq!(error.dimension, Some(ResourceDimension::NetworkBytes));
    }

    #[tokio::test]
    async fn proven_not_dispatched_reservations_are_released() {
        let (store, _dir) = store();
        create_chain(&store, "sess-5", envelope(CapLimit::NotApplicable));
        let first = admit_tool_effect(&store, "sess-5", "bash")
            .await
            .expect("admission")
            .expect("lease");
        let second = admit_tool_effect(&store, "sess-5", "read_file")
            .await
            .expect("admission")
            .expect("lease");
        let ids = vec![
            first.reservation_id().to_string(),
            second.reservation_id().to_string(),
        ];
        release_proven_not_dispatched(&store, &ids, "cancelled_before_dispatch")
            .await
            .expect("batch release should succeed");
        let status = store
            .get_budget_scope_status("sess-5")
            .expect("read scope")
            .expect("scope exists");
        assert_eq!(status.reserved.tool_calls, 0);
        assert_eq!(status.reserved.concurrency, 0);
        assert_eq!(status.committed.tool_calls, 0);
    }

    #[tokio::test]
    async fn unknown_outcome_freezes_the_budget() {
        let (store, _dir) = store();
        create_chain(&store, "sess-4", envelope(CapLimit::NotApplicable));
        let lease = admit_tool_effect(&store, "sess-4", "bash")
            .await
            .expect("admission")
            .expect("lease");
        let reservation_id = lease.reservation_id().to_string();
        mark_tool_effect_unknown(
            &store,
            &reservation_id,
            "sess-4:campaign",
            &reservation_id,
            "cancelled_while_running",
            "cancelled",
        )
        .await
        .expect("atomic unknown+infra should succeed");
        let status = store
            .get_budget_scope_status("sess-4")
            .expect("read scope")
            .expect("scope exists");
        assert_eq!(status.reserved.tool_calls, 1, "budget stays frozen");
        assert_eq!(status.committed.tool_calls, 0);
    }

    #[tokio::test]
    async fn tool_unknown_counts_infra_and_pauses_at_threshold() {
        let (store, _dir) = store();
        let mut base = envelope(CapLimit::NotApplicable);
        base.infra_failure_threshold = 2;
        create_chain(&store, "sess-t", base);
        for index in 1..=2 {
            let lease = admit_tool_effect(&store, "sess-t", "bash")
                .await
                .expect("admission")
                .expect("lease");
            mark_tool_effect_unknown(
                &store,
                lease.reservation_id(),
                "sess-t:campaign",
                lease.reservation_id(),
                "cancelled_while_running",
                "cancelled",
            )
            .await
            .expect("atomic unknown+infra should succeed");
            let status = store
                .get_budget_scope_status("sess-t:campaign")
                .expect("read scope")
                .expect("scope exists");
            assert_eq!(status.infra_failure_count, index);
        }
        // The threshold pause took effect atomically with the second
        // unknown recording.
        let status = store
            .get_budget_scope_status("sess-t:campaign")
            .expect("read scope")
            .expect("scope exists");
        assert_eq!(status.status, "paused");
        assert_eq!(
            admit_tool_effect(&store, "sess-t", "bash")
                .await
                .unwrap_err()
                .code,
            crate::budget::errors::AdmissionErrorCode::ScopePaused
        );
    }
}
